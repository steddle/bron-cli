//! The web archive, from the Internet Archive.
//!
//! Three surfaces. The CDX index lists what the archive holds for a URL and at
//! which timestamps, which is what makes `--as-of` a lookup here as much as it
//! is for a statute. The replay endpoint serves the captured bytes themselves.
//! Save Page Now asks the archive to make a capture that does not exist yet.
//!
//! What this lane answers is a different claim from what the other three
//! answer. A statute or a ruling carries authority in its own text; a capture
//! is a third party's record that a page said something on a day. Both are
//! citable and they are not the same thing, which is why every answer from here
//! is graded [`crate::wire::Grade::Archive`].

use std::borrow::Cow;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;

use super::{Client, FetchedCapture, Provenance};
use crate::ids::{CaptureId, Timestamp14};

const INDEX_ENDPOINT: &str = "https://web.archive.org/cdx/search/cdx";

/// The columns bron asks for, in order.
///
/// Pinned rather than defaulted, and then asserted against the header row that
/// comes back. Reading the default columns would leave every stored field
/// hostage to a server-side change nobody announced, and asserting the header
/// is what turns that from a silent misreading into a refusal.
const INDEX_FIELDS: [&str; 7] = [
    "urlkey",
    "original",
    "timestamp",
    "digest",
    "mimetype",
    "length",
    "statuscode",
];

/// What the site answered the archive when the capture was made.
///
/// `Unknown` is a real state and not a missing value: the archive writes `-`
/// when a crawl was excluded by robots.txt, which says *there is a capture at
/// this timestamp and I will not tell you what it returned*. Dropping those
/// rows would answer "no capture here", and a caller cannot tell the two apart
/// afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveStatus {
    Answered(u16),
    Unknown,
}

impl ArchiveStatus {
    /// Whether the site served the page itself.
    ///
    /// A capture of a 404 is a faithful capture of an error page, and citing it
    /// as the page is the mistake this distinction exists to prevent. When the
    /// archive withholds the status the honest answer is that nobody knows,
    /// which is why this is not a boolean.
    pub fn captured_the_page(self) -> crate::model::Captured {
        use crate::model::Captured;

        match self {
            Self::Answered(200) => Captured::Yes,
            Self::Answered(_) => Captured::No,
            Self::Unknown => Captured::Unknown,
        }
    }

    pub fn code(self) -> Option<u16> {
        match self {
            Self::Answered(code) => Some(code),
            Self::Unknown => None,
        }
    }
}

/// One capture, as the index describes it.
///
/// Available without fetching a single byte of the capture itself, which is
/// what makes both `--as-of` resolution and change detection cheap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRow {
    pub id: CaptureId,
    /// The address as captured. Provenance and display; identity is [`Self::id`].
    pub original: String,
    /// The archive's own hash of the payload, base32 over SHA-1.
    ///
    /// Absent on older rows. Two captures sharing a digest are the same bytes,
    /// so change detection costs nothing: no capture has to be fetched to know
    /// whether anything moved.
    pub digest: Option<String>,
    pub mime_type: Option<String>,
    pub length: Option<u64>,
    pub status: ArchiveStatus,
}

impl IndexRow {
    /// True when this row records that nothing changed since an earlier capture.
    ///
    /// The archive stores no new payload for a revisit and replays the original
    /// capture's bytes, so the digest verifies while the timestamp is not when
    /// those bytes were fetched. Anyone dating the content to this row's
    /// timestamp is off by however long the page sat unchanged.
    pub fn is_revisit(&self) -> bool {
        self.mime_type.as_deref() == Some("warc/revisit")
    }
}

/// One page of index rows, and the key to ask for the next.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Index {
    pub rows: Vec<IndexRow>,
    pub resume_key: Option<String>,
}

/// Reads the index for a URL.
///
/// An empty answer means the archive holds nothing, which is an answer. Only a
/// query that could not be answered is an error, because a sweep that renders
/// those the same records an absence it never established.
fn parse_index(body: &str) -> Result<Index> {
    let body = body.trim();

    // The archive answers a URL it holds nothing for with no body at all.
    if body.is_empty() {
        return Ok(Index::default());
    }

    // A blocked site comes back as a plain-text stack trace, which fails to
    // parse as JSON below. Named here so the operator reads why rather than a
    // decoder's complaint about byte 0.
    let lower = body.to_ascii_lowercase();
    if lower.contains("blocked site error")
        || lower.contains("administrativeaccesscontrolexception")
        || lower.contains("blocked query")
    {
        bail!("the archive refuses index queries for this site");
    }

    let mut rows: Vec<Vec<String>> =
        serde_json::from_str(body).context("the index returned a body that is not CDX JSON")?;

    // Some addresses come back with no body and others with a literal empty
    // array. Both say the archive holds nothing, and neither carries a header
    // row to check.
    if rows.is_empty() {
        return Ok(Index::default());
    }

    // The resume key arrives as a trailing single-field row, usually behind an
    // empty separator row. Detected by shape because no sentinel marks it.
    let resume_key = take_resume_key(&mut rows);

    // The resume key and its separator can account for every row, leaving
    // nothing to check the schema against. Taking the header positionally after
    // that reads off the end, and a body the archive controls must never be
    // able to abort the process.
    if rows.is_empty() {
        bail!("the index answered a resume key with no header row");
    }

    let header = rows.remove(0);

    if header != INDEX_FIELDS {
        bail!(
            "the index answered columns bron did not ask for: wanted {INDEX_FIELDS:?}, got {header:?}"
        );
    }

    let rows = rows
        .iter()
        .map(|row| parse_row(row))
        .collect::<Result<Vec<_>>>()?;

    Ok(Index { rows, resume_key })
}

fn parse_row(row: &[String]) -> Result<IndexRow> {
    // A row of the wrong width means the schema asked for is not the schema
    // returned, so every other row is suspect too. Fatal, not skipped.
    if row.len() != INDEX_FIELDS.len() {
        bail!(
            "the index answered a row of {} fields, expected {}",
            row.len(),
            INDEX_FIELDS.len()
        );
    }

    let field = |name: &str| -> &str {
        let at = INDEX_FIELDS
            .iter()
            .position(|f| *f == name)
            .expect("name is a pinned column");
        row[at].as_str()
    };

    let timestamp: Timestamp14 = field("timestamp")
        .parse()
        .with_context(|| format!("the index answered an unreadable timestamp: {row:?}"))?;

    let original = field("original").to_string();
    let urlkey = field("urlkey");

    if urlkey.is_empty() || original.is_empty() {
        bail!("the index answered a row naming no url: {row:?}");
    }

    let status = match field("statuscode") {
        // Robots-excluded: the capture exists, the status does not.
        "-" | "" => ArchiveStatus::Unknown,
        code => ArchiveStatus::Answered(
            code.parse()
                .with_context(|| format!("the index answered an unreadable status: {row:?}"))?,
        ),
    };

    Ok(IndexRow {
        id: CaptureId::new(timestamp, urlkey),
        original,
        digest: optional(field("digest")),
        mime_type: optional(field("mimetype")),
        length: optional(field("length")).and_then(|value| value.parse().ok()),
        status,
    })
}

/// Removes the resume-key rows, if the answer carries them.
fn take_resume_key(rows: &mut Vec<Vec<String>>) -> Option<String> {
    let last = rows.last()?;

    if rows.len() < 2 || last.len() != 1 || last[0].is_empty() {
        return None;
    }

    let key = rows.pop()?.remove(0);

    // The separator is usually there and is not guaranteed.
    if rows.last().is_some_and(Vec::is_empty) {
        rows.pop();
    }

    Some(key)
}

fn optional(value: &str) -> Option<String> {
    match value.trim() {
        "" | "-" => None,
        value => Some(value.to_string()),
    }
}

// -- reading the index ---------------------------------------------------

/// How a query matches the address given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchType {
    #[default]
    Exact,
    Prefix,
    Host,
    Domain,
}

impl MatchType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Prefix => "prefix",
            Self::Host => "host",
            Self::Domain => "domain",
        }
    }
}

/// What to ask the index for.
#[derive(Debug, Clone, Default)]
pub struct IndexQuery {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    /// The most rows to answer with.
    ///
    /// A cap on the answer and not on the page. Upstream's own `limit` is per
    /// response and paging walks straight past it, so a caller who asked for
    /// five and got a hundred was told something false about what they asked.
    /// Reaching this is reported as truncation, never absorbed.
    pub limit: Option<u32>,
    pub match_type: MatchType,
}

/// Pages requested before bron stops asking.
///
/// A domain-wide enumeration of a large site runs to tens of megabytes and
/// millions of rows, so there has to be a stopping point. Reaching it is
/// reported rather than absorbed: a truncated answer that reads as a complete
/// one is how a sweep records an absence it never established.
const MAX_PAGES: usize = 20;
const MAX_ROWS: usize = 1_000_000;

/// Everything the index holds for an address, within the bounds asked for.
#[derive(Debug, Clone, Default)]
pub struct Captures {
    pub rows: Vec<IndexRow>,
    /// True when the archive still had more to say and bron stopped asking.
    pub truncated: bool,
}

/// The one capture that answers "this page, on this date".
///
/// The newest capture taken at or before the date, and nothing else. Upstream's
/// negative `limit` counts back from the newest, so asking for `-1` under a `to`
/// bound returns exactly that row in one response.
///
/// Worth having as its own call rather than a filter over [`captures`]. The
/// list form pages through the whole history and then picks a maximum out of
/// it, which for an address with hundreds of thousands of captures is tens of
/// megabytes and up to twenty requests to answer with one row.
pub fn capture_on(client: &Client, url: &str, on: NaiveDate) -> Result<Option<IndexRow>> {
    newest_capture(client, url, Some(on))
}

/// The newest capture the archive holds for an address, optionally bounded to a
/// date. One row, one request, and therefore nothing to truncate.
pub fn newest_capture(
    client: &Client,
    url: &str,
    on: Option<NaiveDate>,
) -> Result<Option<IndexRow>> {
    let mut request = client.get(INDEX_ENDPOINT).query(&[
        ("url", url),
        ("output", "json"),
        ("fl", &INDEX_FIELDS.join(",")),
        ("matchType", MatchType::Exact.as_str()),
        // Negative counts back from the newest, which is the whole trick.
        ("limit", "-1"),
    ]);

    if let Some(on) = on {
        request = request.query(&[("to", on.format("%Y%m%d235959").to_string())]);
    }

    Ok(parse_index(&client.fetch(request)?.text)?.rows.pop())
}

/// Whether a capture the archive has just reported is one bron caused.
///
/// Answered against the newest capture held before the submission, not against
/// the whole history. Save Page Now answers a resubmit with the capture it
/// already holds, and the one it already holds is the newest one, so a reported
/// timestamp that is not strictly newer than that is a capture bron found
/// rather than made.
///
/// Reading one row rather than the history is what keeps this honest. A paged
/// read of every capture can stop short of the end, and a row missing because
/// the read was truncated is indistinguishable from a row that was never there;
/// deciding provenance off that is how a corpus comes to claim it caused a
/// capture that predates it.
pub fn caused_by_bron(newest_before: Option<&IndexRow>, made: &Timestamp14) -> bool {
    newest_before.is_none_or(|before| made > before.id.timestamp())
}

/// Reads the index for an address, following resume keys.
pub fn captures(client: &Client, url: &str, query: &IndexQuery) -> Result<Captures> {
    let mut rows = Vec::new();
    let mut resume: Option<String> = None;
    let mut truncated = false;

    for page in 0..MAX_PAGES {
        let mut request = client.get(INDEX_ENDPOINT).query(&[
            ("url", url),
            ("output", "json"),
            ("fl", &INDEX_FIELDS.join(",")),
            ("matchType", query.match_type.as_str()),
            ("showResumeKey", "true"),
        ]);

        if let Some(from) = query.from {
            request = request.query(&[("from", from.format("%Y%m%d000000").to_string())]);
        }
        if let Some(to) = query.to {
            // Inclusive of the whole day: a date bound that silently excluded
            // captures made that afternoon would be wrong in the direction
            // nobody checks.
            request = request.query(&[("to", to.format("%Y%m%d235959").to_string())]);
        }
        if let Some(limit) = query.limit {
            request = request.query(&[("limit", limit.to_string())]);
        }
        if let Some(key) = &resume {
            request = request.query(&[("resumeKey", key)]);
        }

        let fetched = client.fetch(request)?;
        let index = parse_index(&fetched.text)?;

        rows.extend(index.rows);

        // The cap is on the answer, so it is applied here and not left to
        // upstream's per-page limit, which paging would simply walk past.
        if let Some(cap) = query.limit.map(|l| l as usize)
            && rows.len() >= cap
        {
            let more = rows.len() > cap || index.resume_key.is_some();
            rows.truncate(cap);
            return Ok(Captures {
                rows,
                truncated: more,
            });
        }

        match index.resume_key {
            None => return Ok(Captures { rows, truncated }),
            Some(key) => resume = Some(key),
        }

        if rows.len() >= MAX_ROWS || page + 1 == MAX_PAGES {
            truncated = true;
            break;
        }
    }

    Ok(Captures {
        rows,
        truncated: truncated || resume.is_some(),
    })
}

/// A key the index itself assigned. Identity.
///
/// Only ever built from a CDX row, which is why there is no public constructor
/// taking a string: an approximation must never be mistaken for the real thing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UrlKey(String);

impl UrlKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// bron's own guess at the key the index would assign an address.
///
/// A separate type from [`UrlKey`] with no conversion between them, so the rule
/// below is checked by the compiler rather than promised in a comment.
///
/// Used for exactly one thing: looking in the corpus for a capture that may
/// already be held, before asking the archive. Identity always comes from the
/// index's own `urlkey`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApproxUrlKey(String);

impl ApproxUrlKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Guesses the key the index would assign an address.
///
/// The archive keys its index on a SURT form: host labels reversed and
/// comma-joined, then `)/` and the path. `https://steddle.com/pricing` becomes
/// `com,steddle)/pricing`.
///
/// # The invariant
///
/// **This is never coarser than SURT.** Every simplification here must keep
/// apart at least everything the archive keeps apart. Two addresses the archive
/// would give different keys must never collapse to one key here.
///
/// That is the whole safety argument, and it is directional. Being *finer* than
/// SURT costs a lookup that misses and a fetch that was not needed. Being
/// *coarser* returns a capture of a different page, which is a wrong answer
/// wearing a cache hit. So: query parameters are not sorted, session ids are
/// not stripped, nothing is percent-decoded and nothing is punycoded, all of
/// which distinguish addresses the archive merges. Only the default port for
/// the scheme is dropped, because that is the only port the archive itself
/// treats as absent.
pub fn urlkey_of(url: &str) -> Result<ApproxUrlKey> {
    let url = url.trim();
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => (String::new(), url),
    };

    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, path),
        None => (rest, ""),
    };

    // Credentials are not part of the key.
    let authority = authority
        .rsplit('@')
        .next()
        .unwrap_or(authority)
        .to_ascii_lowercase();

    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority.as_str(), None),
    };

    // A non-default port is part of the address as far as the archive is
    // concerned, so dropping every port would merge two pages into one key and
    // hand back a capture of the wrong one.
    let default = match scheme.as_str() {
        "https" => "443",
        _ => "80",
    };
    let host = match port {
        Some(port) if port != default => &authority,
        _ => host,
    };

    if host.is_empty() {
        bail!("{url} names no host");
    }

    // www is not a distinct site as far as the index is concerned.
    let host = host.strip_prefix("www.").unwrap_or(host);
    let (host, suffix) = match host.split_once(':') {
        Some((host, port)) => (host, format!(":{port}")),
        None => (host, String::new()),
    };
    let reversed: Vec<&str> = host.split('.').rev().collect();

    Ok(ApproxUrlKey(format!(
        "{}{suffix})/{}",
        reversed.join(","),
        path.to_ascii_lowercase()
    )))
}

// -- digests -------------------------------------------------------------

/// How the replayed bytes stand against the digest the index recorded.
///
/// Three states, and none of them is a failure. That is a correction, arrived
/// at by measurement: this started as two states with a mismatch as a hard
/// error, on the assumption that the index digest is a checksum of the bytes
/// the replay serves. It is not.
///
/// The digest covers the payload *as stored in the WARC*. When the original
/// response used `Transfer-Encoding: chunked` — which is most of the web — the
/// stored payload carries the chunk framing and the replay strips it before
/// serving. The two therefore cannot agree, on a capture that is entirely
/// sound. Verified against the live archive with plain `curl`, which produces
/// exactly the same mismatch bron does: the index said
/// `JRV2UFDKO7P4KIBFRN2URU6M2YEX6Q6O` for a capture whose replayed body hashes
/// to `C2WZDLUS4PYKVDDGVXQ2A6QU33T5OSGK`, and whose stored length is 3970
/// against 13556 bytes served.
///
/// So the evidence is asymmetric, and the type says so. A match proves the
/// replay is byte-identical to what the crawl recorded. A difference proves
/// nothing either way. Treating the second as tampering failed 16 of 47
/// otherwise-good captures in one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verified {
    /// The replayed bytes hash to the digest the index recorded. The replay is
    /// exactly what the crawl stored, framing and all.
    Matches,
    /// They do not. Ordinary, and not evidence of anything: the stored payload
    /// and the served body are different encodings of the same capture.
    Differs,
    /// The index carried no digest for this row. Older captures are routinely
    /// like this.
    Absent,
}

/// Hashes bytes the way the index does: SHA-1, base32, unpadded.
pub fn digest_of(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    data_encoding::BASE32.encode(&Sha1::digest(bytes))
}

/// Bytes that came back through the `id_` replay path.
///
/// A separate type with a private field and no public constructor, because the
/// index's digest is a digest *of the capture's payload* and comparing it to
/// anything else is meaningless. Only [`fetch_replay`] builds one, and it is
/// the function that builds the `id_` URL, so the two cannot come apart.
///
/// Without this a caller could fetch the ordinary replay page, get a mismatch,
/// and make the failure go away by passing no digest — which would report the
/// crate's central claim as merely unchecked. Closing that route is the point.
pub struct ReplayBytes {
    fetched: super::FetchedBytes,
}

impl ReplayBytes {
    pub fn bytes(&self) -> &[u8] {
        &self.fetched.bytes
    }

    pub fn content_type(&self) -> Option<&str> {
        self.fetched.content_type.as_deref()
    }

    pub fn content_encoding(&self) -> Option<&str> {
        self.fetched.content_encoding.as_deref()
    }
}

/// Undoes the transfer encoding the archive replayed the capture in.
///
/// Three steps, kept visibly apart because collapsing any two of them breaks
/// something different:
///
/// 1. The digest hashes the payload **as stored**, upstream of this. The index
///    publishes its hash over what the WARC holds, so unpacking first would
///    have bron hashing something the archive never described, turning a check
///    that passes into one that always fails and reads like tampering.
/// 2. This undoes the encoding.
/// 3. Only then does charset detection run, over the decompressed bytes.
///
/// Decided from the header rather than from leading bytes: a body that merely
/// begins like gzip is not gzip, and a body the archive labelled is not a guess.
///
/// An encoding bron cannot undo is a hard error naming it. It must never fall
/// through to the caller: printing bytes nothing could decode, under a line
/// saying the digest verified, is the defect this exists to prevent, and it
/// would return with the next encoding rather than the next gzip.
fn decompress<'a>(bytes: &'a [u8], encoding: Option<&str>) -> Result<(Cow<'a, [u8]>, bool)> {
    use std::io::Read;

    let Some(encoding) = encoding.map(str::trim).filter(|e| !e.is_empty()) else {
        return Ok((Cow::Borrowed(bytes), false));
    };

    let mut out = Vec::new();

    match encoding.to_ascii_lowercase().as_str() {
        // What the archive answers for an uncompressed payload.
        "identity" => return Ok((Cow::Borrowed(bytes), false)),
        "gzip" | "x-gzip" => flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut out)
            .with_context(|| "the capture was labelled gzip and did not decompress")?,
        "deflate" => flate2::read::ZlibDecoder::new(bytes)
            .read_to_end(&mut out)
            .with_context(|| "the capture was labelled deflate and did not decompress")?,
        other => bail!(
            "the archive replayed this capture as {other}, which bron cannot decode; \
             its bytes are stored and citable by digest, but no text can be read from them"
        ),
    };

    Ok((Cow::Owned(out), true))
}

/// Fetches one capture's own bytes, unmodified.
///
/// `id_` is what asks the archive for the resource as captured, with no replay
/// banner and no rewritten links, so text derived over it behaves as it would
/// over a direct fetch. Without it the caller gets the archive's rendering of
/// the page, which is not the evidence.
fn fetch_replay(client: &Client, row: &IndexRow) -> Result<ReplayBytes> {
    let url = row.id.replay_url(&row.original);

    // Never following the redirect. A captured 301 replays as a real 301, and
    // taking it lands on a different capture whose bytes cannot match the
    // digest recorded for this one.
    Ok(ReplayBytes {
        fetched: client.fetch_bytes_unredirected(client.get(&url))?,
    })
}

/// Compares replayed bytes with the digest the index recorded for that row.
///
/// Never fails. See [`Verified`] for why a difference is not a finding: the
/// index digest covers the stored payload and the replay serves a de-framed
/// body, so the two disagree routinely on captures that are entirely sound.
///
/// Takes the row rather than a digest, so "there is nothing to compare
/// against" is a fact read off the index and never a choice at the call site.
pub fn verify(replay: &ReplayBytes, row: &IndexRow) -> Verified {
    check_digest(replay.bytes(), row.digest.as_deref())
}

fn check_digest(bytes: &[u8], index_digest: Option<&str>) -> Verified {
    match index_digest {
        None => Verified::Absent,
        Some(expected) if digest_of(bytes) == expected => Verified::Matches,
        Some(_) => Verified::Differs,
    }
}

// -- reading a capture ---------------------------------------------------

/// Where the encoding a capture was read with came from.
///
/// Recorded rather than inferred. An archived page is whatever the site served
/// in whatever year it served it, so the label is often absent and sometimes
/// wrong, and which rule fired is part of how the text traces back to the
/// bytes. A reading nobody can reproduce is not provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declared {
    /// A byte-order mark, which outranks every label.
    Bom,
    /// The `charset` on the Content-Type the archive replayed.
    Header,
    /// A `charset` declared inside the document itself.
    Document,
    /// Nothing said, so the web's historical default was assumed.
    Assumed,
}

/// A capture, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    Text {
        /// The replayed bytes were compressed and were unpacked to get here.
        /// Stated because a reader told only that the digest matched is being
        /// shown text one step removed from the bytes that were hashed.
        decompressed: bool,
        text: String,
        /// The encoding actually used, by its WHATWG name.
        encoding: &'static str,
        declared: Declared,
        /// True when bytes could not be represented and were replaced.
        lossy: bool,
    },
    /// Not text at all. A capture of a PDF is evidence as bytes and a digest;
    /// running a text extractor over it would invent a document.
    Binary,
}

/// How many bytes to search for a document's own charset declaration.
const SNIFF_WINDOW: usize = 1024;

/// Reads captured bytes as text, or declines to.
pub fn read(bytes: &[u8], content_type: Option<&str>, decompressed: bool) -> Reading {
    let media = content_type.map(media_type);

    if media.is_some_and(|media| !is_text(media)) {
        return Reading::Binary;
    }

    let (encoding, declared) = if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(bytes) {
        (encoding, Declared::Bom)
    } else if let Some(encoding) = content_type.and_then(charset_of) {
        (encoding, Declared::Header)
    } else if let Some(encoding) = sniff_charset(bytes) {
        (encoding, Declared::Document)
    } else {
        // What a browser assumes when nothing says otherwise, and what most of
        // the unlabelled web actually is. Never UTF-8-lossy, which would drop
        // exactly the accented characters a Dutch or German page is full of.
        (encoding_rs::WINDOWS_1252, Declared::Assumed)
    };

    let (text, encoding, lossy) = encoding.decode(bytes);

    Reading::Text {
        decompressed,
        text: text.into_owned(),
        encoding: encoding.name(),
        declared,
        lossy,
    }
}

fn media_type(content_type: &str) -> &str {
    content_type.split(';').next().unwrap_or("").trim()
}

fn is_text(media: &str) -> bool {
    media.starts_with("text/")
        || matches!(media, "application/xhtml+xml" | "application/xml" | "application/json")
        || media.ends_with("+xml")
        || media.ends_with("+json")
        // A revisit record carries no payload type of its own.
        || media == "warc/revisit"
}

fn charset_of(content_type: &str) -> Option<&'static encoding_rs::Encoding> {
    let lowered = content_type.to_ascii_lowercase();
    let (_, rest) = lowered.split_once("charset=")?;
    let label = rest
        .trim_matches(['"', '\'', ' '])
        .split(';')
        .next()?
        .trim();

    encoding_rs::Encoding::for_label(label.as_bytes())
}

fn sniff_charset(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let window = &bytes[..bytes.len().min(SNIFF_WINDOW)];
    let head = String::from_utf8_lossy(window).to_ascii_lowercase();
    let (_, rest) = head.split_once("charset=")?;

    let label: String = rest
        .trim_start_matches(['"', '\''])
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();

    encoding_rs::Encoding::for_label(label.as_bytes())
}

/// Fetches one capture and checks it against the index before reading it.
pub fn fetch_capture(client: &Client, row: &IndexRow) -> Result<FetchedCapture> {
    let replay = fetch_replay(client, row)?;

    // Hash as stored, then unpack, then decide the charset. See `decompress`.
    let verified = verify(&replay, row);
    let (bytes, decompressed) = decompress(replay.bytes(), replay.content_encoding())?;
    let reading = read(&bytes, replay.content_type(), decompressed);

    Ok(FetchedCapture {
        id: row.id.clone(),
        original: row.original.clone(),
        status: row.status,
        digest: row.digest.clone(),
        verified,
        is_revisit: row.is_revisit(),
        mime_type: row.mime_type.clone(),
        content_type: replay.fetched.content_type,
        reading,
        provenance: Provenance {
            blob: replay.fetched.blob,
            url: replay.fetched.url,
            byte_len: replay.fetched.bytes.len(),
        },
    })
}

// -- asking for a capture ------------------------------------------------

const SAVE_ENDPOINT: &str = "https://web.archive.org/save";
const STATUS_ENDPOINT: &str = "https://web.archive.org/save/status";

/// Internet Archive account keys, from `archive.org/account/s3.php`.
///
/// The reading half of this lane needs none of these, so their absence must
/// never stop bron starting or stop an index query. They are checked when a
/// capture is asked for and not before.
pub struct Credentials {
    access: String,
    secret: String,
}

impl Credentials {
    pub const ACCESS_VAR: &'static str = "BRON_IA_ACCESS_KEY";
    pub const SECRET_VAR: &'static str = "BRON_IA_SECRET_KEY";

    /// Reads the keys, refusing before a request is built rather than after
    /// the archive answers 401.
    pub fn from_env() -> Result<Self> {
        let read = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };

        Self::require(read(Self::ACCESS_VAR), read(Self::SECRET_VAR))
    }

    /// The decision itself, separated from where the values came from, so it
    /// can be tested without a process-wide environment every other test in
    /// the binary shares.
    fn require(access: Option<String>, secret: Option<String>) -> Result<Self> {
        match (access, secret) {
            (Some(access), Some(secret)) => Ok(Self { access, secret }),
            _ => bail!(
                "Save Page Now refuses an unauthenticated caller. Set {} and {} from \
                 archive.org/account/s3.php. Reading the archive needs neither.",
                Self::ACCESS_VAR,
                Self::SECRET_VAR
            ),
        }
    }

    /// The IAS3 authorization header, which is what both save endpoints want.
    fn header(&self) -> String {
        format!("LOW {}:{}", self.access, self.secret)
    }
}

/// Written by hand and not derived, so that a secret cannot reach a log, an
/// error chain or a panic message by way of someone formatting a struct that
/// happens to contain it.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// What to ask the archive to capture.
#[derive(Debug, Clone, Default)]
pub struct SaveRequest {
    pub url: String,
    /// Do not make a new capture when one exists inside this window.
    pub if_not_archived_within_days: Option<u32>,
    /// Also capture a rendered PNG. Worth more than it looks when the claim is
    /// in the layout of a pricing table rather than in prose.
    pub screenshot: bool,
}

/// How far a submitted job has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
    Pending,
    Success,
    /// The archive gave up. Its own words travel in [`SaveStatus::message`].
    Failed,
}

/// The archive's answer about one job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveStatus {
    pub state: SaveState,
    /// Present once the job finishes. Same format the index uses, so the
    /// capture is addressable the moment this arrives.
    pub timestamp: Option<Timestamp14>,
    pub original_url: Option<String>,
    /// What the *site* answered the *archive*.
    ///
    /// A job that succeeded is not a page that was there: `success` alongside
    /// 404 is a faithful capture of an error page, and reporting that as a
    /// capture of the page is the mistake most likely to put a worthless
    /// citation into a legal document.
    pub http_status: Option<u16>,
    pub message: Option<String>,
}

impl SaveStatus {
    /// True only when the archive finished *and* the site served the page.
    pub fn captured_the_page(&self) -> bool {
        self.state == SaveState::Success && self.http_status == Some(200)
    }
}

/// Reads the answer to a submission.
///
/// A refusal arrives inside an HTTP 200: a caller that checks only the status
/// code reads `{"status":"error","status_ext":"error:invalid-host-resolution"}`
/// as success. Any unrecognised `status_ext` is treated as a refusal and its
/// message is surfaced verbatim rather than mapped onto a guess.
fn parse_submit(body: &str) -> Result<String> {
    let value: serde_json::Value =
        serde_json::from_str(body).context("Save Page Now returned a body that is not JSON")?;

    let message = value.get("message").and_then(serde_json::Value::as_str);

    if value.get("status").and_then(serde_json::Value::as_str) == Some("error") {
        let detail = value.get("status_ext").and_then(serde_json::Value::as_str);
        bail!(
            "the archive refused to capture the page: {}{}",
            message.unwrap_or("no reason given"),
            detail.map(|d| format!(" ({d})")).unwrap_or_default()
        );
    }

    match value.get("job_id").and_then(serde_json::Value::as_str) {
        Some(job) if !job.is_empty() => Ok(job.to_string()),
        _ => bail!("the archive answered no job: {}", message.unwrap_or(body)),
    }
}

fn parse_status(body: &str) -> Result<SaveStatus> {
    let value: serde_json::Value = serde_json::from_str(body)
        .context("the save status endpoint returned a body that is not JSON")?;

    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };

    let state = match value.get("status").and_then(serde_json::Value::as_str) {
        Some("pending") => SaveState::Pending,
        Some("success") => SaveState::Success,
        Some(_) => SaveState::Failed,
        None => bail!("the save status endpoint answered no status"),
    };

    Ok(SaveStatus {
        state,
        timestamp: text("timestamp").and_then(|t| t.parse().ok()),
        original_url: text("original_url"),
        http_status: value
            .get("http_status")
            .and_then(serde_json::Value::as_u64)
            .map(|s| s as u16),
        message: text("message"),
    })
}

/// How long to leave between polls, and how many to allow.
///
/// Measured: one capture took 16 seconds of archive-side work and resolved
/// within about 30 seconds of polling at six-second intervals.
const POLL_INTERVAL: Duration = Duration::from_secs(6);
const MAX_POLLS: u32 = 50;

/// What to do next, given where a job has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    Wait(Duration),
    Done(Box<SaveStatus>),
    GaveUp(String),
}

/// Decides the next move. Pure, so every path is a table test rather than a
/// five-minute wait.
///
/// Time arrives as an argument the way `fetched_at` does everywhere else in
/// bron: a decision that reads the clock itself is a decision nobody can test.
pub fn next(status: &SaveStatus, attempt: u32, elapsed: Duration, budget: Duration) -> Poll {
    match status.state {
        SaveState::Success => Poll::Done(Box::new(status.clone())),
        SaveState::Failed => Poll::GaveUp(
            status
                .message
                .clone()
                .unwrap_or_else(|| "the archive gave no reason".to_string()),
        ),
        SaveState::Pending if elapsed >= budget => Poll::GaveUp(format!(
            "the capture was still running after {}s; the archive may still finish it",
            elapsed.as_secs()
        )),
        SaveState::Pending if attempt >= MAX_POLLS => Poll::GaveUp(format!(
            "the capture was still running after {MAX_POLLS} checks; \
             the archive may still finish it"
        )),
        SaveState::Pending => Poll::Wait(POLL_INTERVAL.min(budget.saturating_sub(elapsed))),
    }
}

/// Asks the archive to capture a page. Returns the job, which is not evidence.
///
/// Submitting a URL again returns the job of the capture already held, with or
/// without `if_not_archived_within`, so a job id never establishes that a new
/// capture was made. Only an index row postdating the submission does.
pub fn submit(client: &Client, credentials: &Credentials, request: &SaveRequest) -> Result<String> {
    let mut form = vec![("url".to_string(), request.url.clone())];

    if let Some(days) = request.if_not_archived_within_days {
        form.push(("if_not_archived_within".to_string(), format!("{days}d")));
    }
    if request.screenshot {
        form.push(("capture_screenshot".to_string(), "1".to_string()));
    }

    let fetched = client.fetch(
        client
            .post(SAVE_ENDPOINT)
            .header("Authorization", credentials.header())
            .header("Accept", "application/json")
            .form(&form),
    )?;

    parse_submit(&fetched.text)
}

/// Asks how a job is getting on.
pub fn job_status(client: &Client, credentials: &Credentials, job: &str) -> Result<SaveStatus> {
    let fetched = client.fetch(
        client
            .get(&format!("{STATUS_ENDPOINT}/{job}"))
            .header("Authorization", credentials.header())
            .header("Accept", "application/json"),
    )?;

    parse_status(&fetched.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER_ROW: &str =
        r#"["urlkey","original","timestamp","digest","mimetype","length","statuscode"]"#;

    fn header() -> String {
        format!("[{HEADER_ROW}]")
    }

    fn with_rows(rows: &str) -> String {
        format!("[{HEADER_ROW},{rows}]")
    }

    #[test]
    fn an_empty_body_is_an_answer_and_not_a_failure() {
        // "The archive holds nothing for this URL" and "the query could not be
        // answered" must never render the same.
        assert_eq!(parse_index("").unwrap(), Index::default());
        assert_eq!(parse_index("   \n ").unwrap(), Index::default());
    }

    #[test]
    fn a_header_alone_is_an_empty_answer() {
        assert_eq!(parse_index(&header()).unwrap().rows, vec![]);
    }

    #[test]
    fn a_blocked_site_is_a_failure_and_says_so() {
        let body = "org.archive.wayback.exception.AdministrativeAccessControlException: \
                    Blocked Site Error";
        let error = parse_index(body).unwrap_err().to_string();
        assert!(error.contains("refuses index queries"), "{error}");
    }

    #[test]
    fn columns_bron_did_not_ask_for_are_refused() {
        // The server changing its default columns must break loudly, not be
        // read positionally into the wrong fields.
        let body =
            r#"[["timestamp","original","statuscode"],["20260806153132","https://x/","200"]]"#;
        let error = parse_index(body).unwrap_err().to_string();
        assert!(error.contains("columns bron did not ask for"), "{error}");
    }

    #[test]
    fn a_row_is_read_by_column_name() {
        let body = with_rows(
            r#"["com,steddle)/","https://steddle.com/","20260806153132","ZHYT52YPEOCHJD5FZINSDYXGQZI22WJ4","text/html","4242","200"]"#,
        );
        let row = &parse_index(&body).unwrap().rows[0];

        assert_eq!(row.id.to_string(), "20260806153132/com,steddle)/");
        assert_eq!(row.original, "https://steddle.com/");
        assert_eq!(
            row.digest.as_deref(),
            Some("ZHYT52YPEOCHJD5FZINSDYXGQZI22WJ4")
        );
        assert_eq!(row.mime_type.as_deref(), Some("text/html"));
        assert_eq!(row.length, Some(4242));
        assert_eq!(row.status, ArchiveStatus::Answered(200));
        assert_eq!(row.status.captured_the_page(), crate::model::Captured::Yes);
    }

    #[test]
    fn a_robots_excluded_row_is_kept_with_its_status_unknown() {
        // The capture exists. What it returned is what the archive withholds,
        // and that is a different fact from there being no capture.
        let body = with_rows(
            r#"["com,steddle)/","https://steddle.com/","20260806153132","-","text/html","-","-"]"#,
        );
        let index = parse_index(&body).unwrap();

        assert_eq!(
            index.rows.len(),
            1,
            "the row carries evidence and must survive"
        );
        assert_eq!(index.rows[0].status, ArchiveStatus::Unknown);
        assert_eq!(index.rows[0].digest, None);
        assert_eq!(index.rows[0].length, None);
        // Not `No`. Nobody knows, and saying `No` would invent an answer.
        assert_eq!(
            index.rows[0].status.captured_the_page(),
            crate::model::Captured::Unknown
        );
    }

    #[test]
    fn a_captured_error_page_is_not_a_captured_page() {
        let body = with_rows(
            r#"["com,steddle)/gone","https://steddle.com/gone","20260806153132","D1","text/html","9","404"]"#,
        );
        let row = &parse_index(&body).unwrap().rows[0];

        assert_eq!(row.status, ArchiveStatus::Answered(404));
        assert_eq!(
            row.status.captured_the_page(),
            crate::model::Captured::No,
            "a 404 capture must not pass for the page"
        );
    }

    #[test]
    fn a_revisit_row_is_flagged_because_its_bytes_are_older_than_its_timestamp() {
        let body = with_rows(
            r#"["com,steddle)/","https://steddle.com/","20260806153132","D1","warc/revisit","0","200"]"#,
        );
        assert!(parse_index(&body).unwrap().rows[0].is_revisit());
    }

    #[test]
    fn a_row_of_the_wrong_width_is_fatal() {
        // Unlike a sentinel, this means the shape itself is wrong, so every
        // other row in the answer is suspect.
        let body = with_rows(r#"["com,steddle)/","https://steddle.com/","20260806153132"]"#);
        let error = parse_index(&body).unwrap_err().to_string();
        assert!(error.contains("row of 3 fields"), "{error}");
    }

    #[test]
    fn an_unreadable_timestamp_is_fatal() {
        let body = with_rows(
            r#"["com,steddle)/","https://steddle.com/","not-a-stamp","D1","text/html","9","200"]"#,
        );
        assert!(parse_index(&body).is_err());
    }

    #[test]
    fn the_resume_key_is_found_by_shape() {
        let body = with_rows(
            r#"["com,steddle)/","https://steddle.com/","20260806153132","D1","text/html","9","200"],[],["com,steddle)/x 20260101"]"#,
        );
        let index = parse_index(&body).unwrap();

        assert_eq!(index.resume_key.as_deref(), Some("com,steddle)/x 20260101"));
        assert_eq!(index.rows.len(), 1, "the key rows are not data");
    }

    #[test]
    fn a_resume_key_without_its_separator_is_still_found() {
        let body = with_rows(
            r#"["com,steddle)/","https://steddle.com/","20260806153132","D1","text/html","9","200"],["resume-here"]"#,
        );
        let index = parse_index(&body).unwrap();

        assert_eq!(index.resume_key.as_deref(), Some("resume-here"));
        assert_eq!(index.rows.len(), 1);
    }

    #[test]
    fn a_last_page_carries_no_resume_key() {
        let body = with_rows(
            r#"["com,steddle)/","https://steddle.com/","20260806153132","D1","text/html","9","200"]"#,
        );
        assert_eq!(parse_index(&body).unwrap().resume_key, None);
    }

    #[test]
    fn an_empty_json_array_is_an_answer_and_not_a_crash() {
        // The archive answers some addresses with no body at all and others
        // with a literal empty array. Both mean the same thing and neither is
        // a failure, let alone a panic.
        assert_eq!(parse_index("[]").unwrap(), Index::default());
        assert_eq!(parse_index(" [] ").unwrap(), Index::default());
    }

    #[test]
    fn a_body_that_is_not_json_is_a_failure() {
        assert!(parse_index("<html>404</html>").is_err());
    }

    // -- provenance of a capture ------------------------------------------

    fn row_at(timestamp: &str) -> IndexRow {
        let body = with_rows(&format!(
            r#"["com,steddle)/","https://steddle.com/","{timestamp}","D1","text/html","9","200"]"#
        ));
        parse_index(&body).unwrap().rows.pop().expect("one row")
    }

    fn stamp(timestamp: &str) -> Timestamp14 {
        timestamp.parse().unwrap()
    }

    #[test]
    fn a_capture_postdating_everything_held_is_one_bron_caused() {
        let before = row_at("20260101000000");
        assert!(caused_by_bron(Some(&before), &stamp("20260806153132")));
    }

    #[test]
    fn a_capture_the_archive_already_held_is_not_one_bron_caused() {
        // Save Page Now answers a resubmit with the capture it already holds,
        // so the reported timestamp comes back equal to one already indexed.
        // Recording that as bron's would be a false provenance claim.
        let before = row_at("20260806153132");
        assert!(!caused_by_bron(Some(&before), &stamp("20260806153132")));
        assert!(!caused_by_bron(Some(&before), &stamp("20250101000000")));
    }

    #[test]
    fn the_first_capture_of_an_address_is_one_bron_caused() {
        assert!(caused_by_bron(None, &stamp("20260806153132")));
    }

    #[test]
    fn the_provenance_question_cannot_be_asked_of_a_truncated_read() {
        // Not an assertion about values: the decision takes the single newest
        // row, so there is no paged set to stop short of and no way to express
        // the truncated read that used to answer this question wrongly. The
        // test exists so a change back to a history-wide set fails to compile
        // here rather than silently reintroducing the false claim.
        let newest: Option<&IndexRow> = None;
        assert!(caused_by_bron(newest, &stamp("20260806153132")));
    }

    #[test]
    fn a_resume_key_that_leaves_no_header_is_a_failure_and_not_a_crash() {
        // Every row can be consumed before the header is reached: a separator
        // and a resume key with nothing in front of them. Taking the header
        // positionally after that reads off the end of the slice, and a body
        // the archive controls must never be able to abort the process.
        assert!(parse_index(r#"[[],["com,steddle)/x 20260101"]]"#).is_err());
    }

    // -- urlkeys ----------------------------------------------------------

    fn key(url: &str) -> String {
        urlkey_of(url).unwrap().as_str().to_string()
    }

    #[test]
    fn an_address_reduces_to_the_key_the_index_sorts_on() {
        assert_eq!(key("https://steddle.com/"), "com,steddle)/");
        assert_eq!(key("https://steddle.com"), "com,steddle)/");
        assert_eq!(key("https://steddle.com/pricing"), "com,steddle)/pricing");
        assert_eq!(key("https://a.b.example.org/x"), "org,example,b,a)/x");
    }

    #[test]
    fn spellings_of_one_address_reduce_together() {
        // The whole reason identity is the key and not the address: these are
        // one record, and keying on the address would make them several.
        let canonical = key("https://steddle.com/pricing");

        for spelling in [
            "http://steddle.com/pricing",
            "https://www.steddle.com/pricing",
            "https://STEDDLE.COM/Pricing",
            // The default port for the scheme, which the archive treats as
            // absent too.
            "https://steddle.com:443/pricing",
            "http://steddle.com:80/pricing",
        ] {
            assert_eq!(key(spelling), canonical, "{spelling}");
        }
    }

    #[test]
    fn a_non_default_port_is_kept_because_the_index_keeps_it() {
        // The direction that matters. Being finer than the archive costs a
        // lookup that misses; being coarser hands back a capture of a
        // different page, which is a wrong answer wearing a cache hit.
        assert_ne!(
            key("https://steddle.com:8080/x"),
            key("https://steddle.com/x")
        );
        assert_ne!(
            key("http://steddle.com:8080/x"),
            key("http://steddle.com/x")
        );

        // ...and the port survives into the key rather than being smuggled
        // into a host label.
        assert!(key("https://steddle.com:8080/x").contains("8080"));
    }

    #[test]
    fn queries_are_left_alone_because_touching_them_could_only_merge() {
        // Not sorted, not stripped, not decoded: every one of those would risk
        // collapsing two addresses the archive keeps apart.
        assert_ne!(
            key("https://steddle.com/x?a=1&b=2"),
            key("https://steddle.com/x?b=2&a=1")
        );
        assert_ne!(
            key("https://steddle.com/x?a=1"),
            key("https://steddle.com/x")
        );
    }

    #[test]
    fn an_address_with_no_host_is_refused() {
        assert!(urlkey_of("https:///nothing").is_err());
        assert!(urlkey_of("").is_err());
    }

    // -- digests ----------------------------------------------------------

    #[test]
    fn the_digest_is_the_one_the_index_publishes() {
        // Base32 over SHA-1, which is what a CDX row carries. A SHA-1 is 160
        // bits and base32 packs five to a character, so it never pads.
        // Cross-checked against `hashlib.sha1` + `base64.b32encode`.
        assert_eq!(digest_of(b""), "3I42H3S6NNFQ2MSVX7XZKYAYSCX5QBYJ");
        assert_eq!(digest_of(b"hello"), "VL2MMHO4YXUKFWV63YHTWSBM3GXKSQ2N");
        assert_eq!(
            digest_of(b"<html>steddle</html>"),
            "KT6FH5JONWBHAAKLLQTH6AYP453C2F4M"
        );
        assert!(
            !digest_of(b"hello").contains('='),
            "a CDX digest is never padded"
        );
    }

    #[test]
    fn bytes_matching_the_index_are_checked() {
        let bytes = b"<html>steddle</html>";
        let digest = digest_of(bytes);
        assert_eq!(check_digest(bytes, Some(&digest)), Verified::Matches);
    }

    #[test]
    fn a_row_with_no_digest_is_unverifiable_and_not_verified() {
        // Two different answers. Reading "nothing to check" as "checked" is how
        // an unverified capture ends up cited as a verified one.
        assert_eq!(check_digest(b"anything", None), Verified::Absent);
    }

    // -- reading a capture ------------------------------------------------

    fn text(reading: &Reading) -> &str {
        match reading {
            Reading::Text { text, .. } => text,
            Reading::Binary => panic!("expected text"),
        }
    }

    fn declared(reading: &Reading) -> Declared {
        match reading {
            Reading::Text { declared, .. } => *declared,
            Reading::Binary => panic!("expected text"),
        }
    }

    #[test]
    fn a_latin1_page_is_read_by_the_charset_the_archive_replayed() {
        // 0xEB is 'ë' in ISO-8859-1 and not valid UTF-8 at all. A UTF-8-lossy
        // read would turn a Dutch page into replacement characters.
        let bytes = b"beh\xEBren";
        let reading = read(bytes, Some("text/html; charset=ISO-8859-1"), false);

        assert_eq!(text(&reading), "behëren");
        assert_eq!(declared(&reading), Declared::Header);
    }

    #[test]
    fn an_unlabelled_page_is_read_as_the_webs_historical_default() {
        let reading = read(b"beh\xEBren", Some("text/html"), false);

        assert_eq!(text(&reading), "behëren");
        assert_eq!(declared(&reading), Declared::Assumed);
        // Stated, so a caller can see the reading was assumed and not told.
        assert!(matches!(
            reading,
            Reading::Text {
                encoding: "windows-1252",
                ..
            }
        ));
    }

    #[test]
    fn a_page_that_declares_its_own_charset_is_read_that_way() {
        let bytes =
            b"<html><head><meta charset=\"utf-8\"></head><body>beh\xc3\xabren</body></html>";
        let reading = read(bytes, Some("text/html"), false);

        assert_eq!(declared(&reading), Declared::Document);
        assert!(text(&reading).contains("behëren"));
    }

    #[test]
    fn a_byte_order_mark_outranks_every_label() {
        // The bytes are UTF-8 with a BOM while the header insists on latin-1.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("behëren".as_bytes());

        let reading = read(&bytes, Some("text/html; charset=ISO-8859-1"), false);

        assert_eq!(declared(&reading), Declared::Bom);
        assert_eq!(text(&reading), "behëren");
    }

    #[test]
    fn the_header_wins_over_the_document_when_both_speak() {
        let bytes = b"<meta charset=\"utf-8\">beh\xEBren";
        let reading = read(bytes, Some("text/html; charset=ISO-8859-1"), false);

        assert_eq!(declared(&reading), Declared::Header);
        assert!(text(&reading).contains("behëren"));
    }

    // -- transfer encoding ------------------------------------------------

    fn gzipped(bytes: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn a_gzip_capture_is_unpacked_before_anything_reads_it() {
        let packed = gzipped(b"<!doctype html><p>Rijksoverheid</p>");
        let (bytes, decompressed) = decompress(&packed, Some("gzip")).unwrap();

        assert!(decompressed);
        assert!(bytes.starts_with(b"<!doctype html>"));
    }

    #[test]
    fn an_unlabelled_capture_is_left_exactly_as_it_arrived() {
        // Decided from the header, never from the leading bytes: a body that
        // merely begins like gzip is not gzip.
        let (bytes, decompressed) = decompress(b"<!doctype html>", None).unwrap();
        assert!(!decompressed);
        assert_eq!(&*bytes, b"<!doctype html>");

        let (bytes, decompressed) = decompress(b"plain", Some("identity")).unwrap();
        assert!(!decompressed);
        assert_eq!(&*bytes, b"plain");
    }

    #[test]
    fn an_encoding_bron_cannot_undo_fails_and_names_itself() {
        // The defect this replaces printed undecodable bytes under a line
        // saying the digest verified. Falling through is never an option.
        let error = decompress(b"\x1b\x2f", Some("br")).unwrap_err().to_string();
        assert!(error.contains("br"), "{error}");
        assert!(error.contains("cannot decode"), "{error}");
    }

    #[test]
    fn a_body_that_lies_about_being_gzip_fails_rather_than_yielding_rubbish() {
        assert!(decompress(b"not actually gzip", Some("gzip")).is_err());
    }

    #[test]
    fn a_pdf_capture_is_never_turned_into_text() {
        // Evidence as bytes and a digest. A text extractor here would invent a
        // document that the capture does not contain.
        assert_eq!(
            read(
                b"%PDF-1.4\n%\xE2\xE3\xCF\xD3",
                Some("application/pdf"),
                false
            ),
            Reading::Binary
        );
        assert_eq!(
            read(b"\x89PNG\r\n", Some("image/png"), false),
            Reading::Binary
        );
    }

    #[test]
    fn xml_and_json_captures_still_read_as_text() {
        assert!(matches!(
            read(b"{}", Some("application/json"), false),
            Reading::Text { .. }
        ));
        assert!(matches!(
            read(b"<a/>", Some("application/xhtml+xml"), false),
            Reading::Text { .. }
        ));
        assert!(matches!(
            read(b"<a/>", Some("image/svg+xml"), false),
            Reading::Text { .. }
        ));
    }

    #[test]
    fn an_unreadable_byte_is_reported_as_lossy_rather_than_hidden() {
        // 0x80 is not a valid UTF-8 start byte; decoding replaces it.
        let reading = read(b"ok\x80", Some("text/html; charset=utf-8"), false);
        assert!(matches!(reading, Reading::Text { lossy: true, .. }));
    }

    #[test]
    fn bytes_the_index_disagrees_with_are_a_fact_and_not_a_failure() {
        // Measured against the live archive: an index digest of
        // JRV2UFDKO7P4KIBFRN2URU6M2YEX6Q6O over a replay body hashing to
        // C2WZDLUS4PYKVDDGVXQ2A6QU33T5OSGK, reproduced by plain curl. The
        // stored payload carries chunk framing the replay strips, so the two
        // cannot agree on a capture that is entirely sound.
        assert_eq!(
            check_digest(b"tampered", Some("ZHYT52YPEOCHJD5FZINSDYXGQZI22WJ4")),
            Verified::Differs
        );
    }

    // -- Save Page Now ----------------------------------------------------
    //
    // Every body below is the shape Save Page Now actually answered on
    // 2026-08-06, recorded before any of this was written. They are measured
    // ground truth, not invented examples, so the literals stay as they are.

    #[test]
    fn a_submission_answers_a_job_and_no_timestamp() {
        let body = r#"{"url":"https://steddle.com","job_id":"spn2-549a2df6ae55d13c4ac977e0166fdaf85032364d"}"#;
        assert_eq!(
            parse_submit(body).unwrap(),
            "spn2-549a2df6ae55d13c4ac977e0166fdaf85032364d"
        );
    }

    #[test]
    fn a_refusal_inside_an_http_200_is_still_a_refusal() {
        // The trap: this arrives with a 200 around it, so a caller checking
        // only the status code reads it as success.
        let body = r#"{"status":"error","status_ext":"error:invalid-host-resolution",
                       "message":"Cannot resolve host nx.example."}"#;
        let error = parse_submit(body).unwrap_err().to_string();

        assert!(error.contains("Cannot resolve host nx.example."), "{error}");
        // Surfaced verbatim rather than mapped: the full list of status_ext
        // codes is unknown, so a mapping would quietly mislabel the unseen ones.
        assert!(error.contains("error:invalid-host-resolution"), "{error}");
    }

    #[test]
    fn an_unrecognised_refusal_code_is_still_surfaced_whole() {
        let body = r#"{"status":"error","status_ext":"error:something-nobody-has-seen",
                       "message":"A new failure."}"#;
        let error = parse_submit(body).unwrap_err().to_string();

        assert!(error.contains("A new failure."), "{error}");
        assert!(error.contains("error:something-nobody-has-seen"), "{error}");
    }

    #[test]
    fn an_unauthenticated_answer_names_the_reason() {
        let body = r#"{"message":"You need to be logged in to use Save Page Now."}"#;
        let error = parse_submit(body).unwrap_err().to_string();
        assert!(error.contains("logged in"), "{error}");
    }

    #[test]
    fn credentials_are_refused_before_anything_is_sent() {
        // Reading the archive needs no keys, so this must be checked at the
        // point of asking for a capture and never at startup.
        let some = || Some("k".to_string());

        for (access, secret) in [(None, None), (some(), None), (None, some())] {
            let error = Credentials::require(access, secret)
                .unwrap_err()
                .to_string();
            assert!(error.contains("BRON_IA_ACCESS_KEY"), "{error}");
            assert!(
                error.contains("Reading the archive needs neither"),
                "{error}"
            );
        }

        assert!(Credentials::require(some(), some()).is_ok());
    }

    #[test]
    fn the_authorization_header_is_the_ias3_form() {
        let credentials = Credentials::require(some_key("acc"), some_key("sec")).unwrap();
        assert_eq!(credentials.header(), "LOW acc:sec");
    }

    #[test]
    fn a_secret_cannot_escape_through_a_debug_format() {
        let credentials =
            Credentials::require(some_key("K3YV4LU3"), some_key("SUP3RS3CR3T")).unwrap();
        let shown = format!("{credentials:?}");

        assert!(!shown.contains("SUP3RS3CR3T"), "{shown}");
        assert!(!shown.contains("K3YV4LU3"), "{shown}");
    }

    fn some_key(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    #[test]
    fn a_running_job_is_pending_with_nothing_to_cite_yet() {
        let body = r#"{"status":"pending","job_id":"spn2-abc","resources":[]}"#;
        let status = parse_status(body).unwrap();

        assert_eq!(status.state, SaveState::Pending);
        assert_eq!(status.timestamp, None);
        assert!(!status.captured_the_page());
    }

    #[test]
    fn a_finished_job_carries_the_timestamp_the_index_will_use() {
        let body = r#"{"status":"success","timestamp":"20260806153132",
                       "original_url":"https://steddle.com/","duration_sec":16.14,
                       "http_status":200,"job_id":"spn2-abc","resources":[],"outlinks":[]}"#;
        let status = parse_status(body).unwrap();

        assert_eq!(status.state, SaveState::Success);
        assert_eq!(
            status.timestamp.as_ref().unwrap().as_str(),
            "20260806153132"
        );
        assert_eq!(status.http_status, Some(200));
        assert!(status.captured_the_page());
    }

    #[test]
    fn a_successful_job_that_captured_a_404_did_not_capture_the_page() {
        // Two independent facts: the archive finished, and the site was not
        // there. Collapsing them is what puts a worthless citation in a filing.
        let body = r#"{"status":"success","timestamp":"20260806153132",
                       "original_url":"https://steddle.com/gone","http_status":404,
                       "job_id":"spn2-abc"}"#;
        let status = parse_status(body).unwrap();

        assert_eq!(status.state, SaveState::Success, "the job did succeed");
        assert!(!status.captured_the_page(), "the page was not there");
    }

    // -- the poll decision ------------------------------------------------

    fn pending() -> SaveStatus {
        SaveStatus {
            state: SaveState::Pending,
            timestamp: None,
            original_url: None,
            http_status: None,
            message: None,
        }
    }

    #[test]
    fn a_pending_job_waits_the_measured_interval() {
        let poll = next(&pending(), 1, Duration::ZERO, Duration::from_secs(300));
        assert_eq!(poll, Poll::Wait(Duration::from_secs(6)));
    }

    #[test]
    fn the_last_wait_never_overshoots_the_budget() {
        let poll = next(
            &pending(),
            1,
            Duration::from_secs(298),
            Duration::from_secs(300),
        );
        assert_eq!(poll, Poll::Wait(Duration::from_secs(2)));
    }

    #[test]
    fn a_spent_budget_gives_up_and_says_the_job_may_still_land() {
        let poll = next(
            &pending(),
            3,
            Duration::from_secs(300),
            Duration::from_secs(300),
        );

        let Poll::GaveUp(reason) = poll else {
            panic!("expected to give up")
        };
        assert!(reason.contains("may still finish"), "{reason}");
    }

    #[test]
    fn a_wrong_clock_cannot_spin_forever() {
        // Attempts are bounded independently of elapsed time, so a clock that
        // never advances still terminates.
        let poll = next(
            &pending(),
            MAX_POLLS,
            Duration::ZERO,
            Duration::from_secs(86_400),
        );
        assert!(matches!(poll, Poll::GaveUp(_)));
    }

    #[test]
    fn a_failed_job_reports_the_archives_own_words() {
        let status = SaveStatus {
            state: SaveState::Failed,
            ..pending()
        };
        let status = SaveStatus {
            message: Some("Live page is not available.".to_string()),
            ..status
        };

        let Poll::GaveUp(reason) = next(&status, 1, Duration::ZERO, Duration::from_secs(300))
        else {
            panic!("expected to give up")
        };
        assert_eq!(reason, "Live page is not available.");
    }

    #[test]
    fn a_finished_job_stops_the_loop() {
        let status = SaveStatus {
            state: SaveState::Success,
            ..pending()
        };
        assert!(matches!(
            next(&status, 1, Duration::ZERO, Duration::from_secs(300)),
            Poll::Done(_)
        ));
    }
}
