//! Reading a version's bytes through the bron service instead of the publisher.
//!
//! One setting, and the split inside it is the whole design.
//!
//! **Bytes are safe to take from a mirror.** The manifest states a SHA-512 for
//! every version it lists, bron reads that manifest from the publisher, and
//! bytes that do not hash to it are refused. A mirror therefore cannot lie
//! about text: the worst it can do is fail, and failing falls back to the
//! publisher.
//!
//! **Validity windows are not.** Nothing verifies a date. A mirror that shifted
//! one `in_force` bound by a day would have bron answer `--as-of` with the
//! wrong article and no way to notice. So the manifest is always read direct,
//! and only the bytes behind it are ever mirrored.
//!
//! What that buys is a collapsed rate limit against the government, and old
//! versions staying reachable after the publisher stops serving them.
//!
//! The expressions endpoint is read to learn which digest holds a version. That
//! is a pointer and not evidence: whatever it names still has to hash to the
//! publisher's digest before a single byte of it is used.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::ids::BwbId;
use crate::sources::{Client, Fetched, VersionMeta};

/// Where the service lives. Overrides [`DEFAULT`].
const ENV: &str = "BRON_MIRROR";

/// The service every install reads through unless told otherwise.
///
/// A default rather than an opt-in, because the reason the mirror exists is to
/// ask the government once for everybody, and a setting nobody sets collapses
/// no rate limit at all.
///
/// Three things keep that honest. Bytes are held to the digest the publisher
/// states in a manifest bron always reads directly, so this address cannot
/// change what a law says. Every answer names the route it took in
/// `resolved_via`. And `--direct`, or `BRON_MIRROR=`, takes it out of the path
/// entirely.
const DEFAULT: &str = "https://api.steddle.com/bron/v1";

/// Which of the two answered, so a stored answer can say where its bytes came
/// from. A dossier may not be able to hide that it read through an intermediary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedVia {
    /// Straight from the body that published it.
    Publisher,
    /// Through the bron service, and checked against the publisher's digest.
    Mirror,
}

impl ResolvedVia {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Publisher => "publisher",
            Self::Mirror => "mirror",
        }
    }
}

pub struct Mirror {
    base: String,
}

/// Just enough of the expressions answer to find a digest. Every other field
/// the service returns is deliberately ignored: the windows come from the
/// manifest and taking them from here would defeat the point.
#[derive(Deserialize)]
struct ExpressionsBody {
    #[serde(default)]
    data: Vec<ExpressionRow>,
}

#[derive(Deserialize)]
struct ExpressionRow {
    label: String,
    #[serde(default)]
    blob: Option<String>,
    /// The service's own statement that its stored bytes hash to the SHA-512
    /// the manifest names. Not taken on trust: the bytes are checked again on
    /// arrival either way. It is worth reading only because a row that says
    /// `false` is guaranteed to fail that check, so the fetch can be skipped.
    #[serde(default)]
    verified: bool,
}

/// Which blob to ask for, out of what the service listed.
fn digest_for(rows: Vec<ExpressionRow>, version: &str) -> Option<String> {
    rows.into_iter()
        .find(|row| row.label == version)
        .filter(|row| row.verified)
        .and_then(|row| row.blob)
}

/// What a search asks for. Mirrors the service's own body.
#[derive(Debug, Serialize)]
pub struct SearchRequest {
    pub q: String,
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_known_on: Option<String>,
}

/// What it answers with.
///
/// Only the fields the CLI renders are read, so a field the service adds later
/// does not break a build here.
#[derive(Debug, Deserialize)]
pub struct SearchAnswer {
    pub data: SearchData,
    #[serde(default)]
    pub caveats: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SearchData {
    /// Which corpora actually held vectors, so could have answered. A lane
    /// holding no vectors must not read as a lane holding no answers.
    pub searched: Vec<String>,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Deserialize)]
pub struct SearchHit {
    /// Which corpus this came from: `wetgeving` today, more later.
    pub source: String,
    /// `article` or `lid`.
    pub unit: String,
    pub regulation_id: String,
    pub expression: String,
    pub number: String,
    pub label: Option<String>,
    pub path: Option<String>,
    pub content_hash: String,
    pub distance: f64,
    #[serde(default)]
    pub lid: Option<SearchLid>,
}

#[derive(Debug, Deserialize)]
pub struct SearchLid {
    pub number: Option<String>,
}

impl Mirror {
    /// The mirror this invocation should use, if any.
    ///
    /// `--direct` wins over everything, so a single run can always be pinned to
    /// the publisher without editing anyone's shell profile. Setting the
    /// variable to an empty string is the same opt-out made durable; unsetting
    /// it restores [`DEFAULT`], because "I never configured this" and "I turned
    /// this off" are different intentions and only one of them is silence.
    pub fn resolve(direct: bool) -> Option<Self> {
        if direct {
            return None;
        }

        let configured = std::env::var(ENV);
        let base = normalise(configured.as_deref().unwrap_or(DEFAULT));

        if base.is_empty() {
            return None;
        }

        Some(Self { base })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Asks the service which held texts are nearest a question.
    ///
    /// The one call here with no publisher behind it. Bytes and manifests both
    /// have a source of last resort; a ranking over the shared corpus does not,
    /// because the corpus is the source. So a caller with no mirror configured
    /// is told so rather than handed an empty list, which would report an
    /// absence bron never established.
    ///
    /// A POST, alone among these, because the term is the caller's own question
    /// rather than an identifier. A question in a URL is a question in every
    /// access log the request passes.
    pub fn search(&self, client: &Client, request: &SearchRequest) -> Result<SearchAnswer> {
        let url = format!("{}/v1/search", self.base);
        let fetched = client
            .fetch(client.post(&url).json(request))
            .context("cannot reach the corpus to search it")?;

        serde_json::from_str(&fetched.text)
            .context("the service answered something that is not the bron envelope")
    }

    /// A version's bytes, when the mirror has them.
    ///
    /// `Ok(None)` is the mirror saying it does not hold this version, which is
    /// an ordinary answer and a reason to ask the publisher. `Err` is the
    /// mirror failing, which is also a reason to ask the publisher but is worth
    /// saying out loud: a mirror that is quietly unreachable looks exactly like
    /// one that is quietly empty, and the difference is the whole question of
    /// whether the thing is working.
    pub fn version_bytes(
        &self,
        client: &Client,
        law: &BwbId,
        meta: &VersionMeta,
    ) -> Result<Option<Fetched>> {
        // Without a stated digest there is nothing to hold the answer to, so
        // there is no safe way to accept it from anywhere but the publisher.
        if meta.upstream_hash.is_none() {
            return Ok(None);
        }

        let Some(digest) = self.digest_of(client, law, &meta.version)? else {
            return Ok(None);
        };

        self.blob(client, &digest).map(Some)
    }

    /// Which blob holds a version, according to the service.
    fn digest_of(&self, client: &Client, law: &BwbId, version: &str) -> Result<Option<String>> {
        let url = format!("{}/v1/regulations/{law}/expressions", self.base);
        let fetched = client
            .fetch_url(&url)
            .with_context(|| format!("cannot read expressions for {law} from the mirror"))?;

        let body: ExpressionsBody = serde_json::from_str(&fetched.text)
            .context("the mirror answered expressions that are not the bron envelope")?;

        Ok(digest_for(body.data, version))
    }

    fn blob(&self, client: &Client, digest: &str) -> Result<Fetched> {
        if !is_sha256_hex(digest) {
            bail!("the mirror named a blob that is not a sha-256 digest: {digest}");
        }

        let url = format!("{}/v1/blobs/{digest}", self.base);
        client
            .fetch_url(&url)
            .with_context(|| format!("cannot read blob {digest} from the mirror"))
    }
}

/// Trims a configured base down to what the paths below are appended to.
///
/// The service versions itself in the path and bron adds `/v1` itself, so a
/// base copied out of a browser with `/v1` already on the end would ask for
/// `/v1/v1` and get a 404. That failure is the quiet kind: the mirror is
/// skipped, the publisher answers, and the only trace is `resolved_via` saying
/// `publisher` when someone expected `mirror`. Cheaper to accept both spellings
/// than to make everyone find out once.
fn normalise(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    base.strip_suffix("/v1")
        .unwrap_or(base)
        .trim_end_matches('/')
        .to_string()
}

/// Checked before the digest reaches a URL, so a value the service controls
/// cannot walk out of the blobs path.
fn is_sha256_hex(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `std::env` is process-wide, so these run under one lock rather than
    /// racing each other on the same variable.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let restore = std::env::var(ENV).ok();

        unsafe {
            match value {
                Some(value) => std::env::set_var(ENV, value),
                None => std::env::remove_var(ENV),
            }
        }

        let out = body();

        unsafe {
            match restore {
                Some(previous) => std::env::set_var(ENV, previous),
                None => std::env::remove_var(ENV),
            }
        }

        out
    }

    #[test]
    fn an_unset_environment_falls_back_to_the_default_service() {
        let mirror = with_env(None, || Mirror::resolve(false));
        assert_eq!(
            mirror.expect("the default").base(),
            "https://api.steddle.com/bron"
        );
    }

    #[test]
    fn the_default_is_normalised_like_any_other_base() {
        // It is written with the /v1 a reader would paste, and must not end up
        // asking for /v1/v1 any more than a hand-set value would.
        assert_eq!(normalise(DEFAULT), "https://api.steddle.com/bron");
    }

    #[test]
    fn direct_beats_the_environment() {
        // A single run has to be pinnable to the publisher without editing a
        // shell profile, so the flag wins.
        let mirror = with_env(Some("https://bron.test"), || Mirror::resolve(true));
        assert!(mirror.is_none());
    }

    #[test]
    fn a_configured_mirror_is_used_and_its_trailing_slash_is_not() {
        let mirror = with_env(Some("https://bron.test/"), || Mirror::resolve(false));
        // Joined with "/v1/..." below, so a kept slash would ask for "//v1".
        assert_eq!(mirror.expect("a mirror").base(), "https://bron.test");
    }

    #[test]
    fn a_base_that_already_names_the_api_version_is_accepted() {
        // The obvious thing to paste is the URL you were just reading, and that
        // one ends in /v1. Both spellings have to land on the same place.
        assert_eq!(
            normalise("https://api.steddle.com/bron/v1"),
            "https://api.steddle.com/bron"
        );
        assert_eq!(
            normalise("https://api.steddle.com/bron/v1/"),
            "https://api.steddle.com/bron"
        );
        assert_eq!(normalise("https://bron.test/v1"), "https://bron.test");
        assert_eq!(normalise("https://bron.test"), "https://bron.test");
    }

    #[test]
    fn a_host_that_merely_ends_in_v1_is_left_alone() {
        // Only a path segment counts; "v1" inside a hostname is not one.
        assert_eq!(
            normalise("https://v1.example.com"),
            "https://v1.example.com"
        );
    }

    #[test]
    fn a_blank_setting_turns_the_mirror_off_rather_than_restoring_the_default() {
        // The durable form of --direct. Silence means the default; an empty
        // value means someone decided.
        assert!(with_env(Some(""), || Mirror::resolve(false)).is_none());
        assert!(with_env(Some("   "), || Mirror::resolve(false)).is_none());
    }

    fn row(label: &str, blob: Option<&str>, verified: bool) -> ExpressionRow {
        ExpressionRow {
            label: label.to_string(),
            blob: blob.map(str::to_string),
            verified,
        }
    }

    #[test]
    fn the_blob_for_a_version_is_the_one_listed_against_its_label() {
        let rows = vec![
            row("2024-01-01_0", Some("aa"), true),
            row("2026-07-16_0", Some("bb"), true),
        ];
        assert_eq!(digest_for(rows, "2026-07-16_0").as_deref(), Some("bb"));
    }

    #[test]
    fn a_row_the_service_could_not_verify_is_not_worth_fetching() {
        // Its bytes are guaranteed to fail the publisher's digest on arrival,
        // so asking for them costs a round trip and buys nothing.
        let rows = vec![row("2024-01-01_0", Some("aa"), false)];
        assert_eq!(digest_for(rows, "2024-01-01_0"), None);
    }

    #[test]
    fn a_version_the_service_does_not_hold_yields_nothing() {
        let rows = vec![row("2024-01-01_0", None, true)];
        assert_eq!(digest_for(rows, "2024-01-01_0"), None);
        assert_eq!(digest_for(Vec::new(), "2024-01-01_0"), None);
    }

    #[test]
    fn only_a_sha256_digest_can_reach_the_blobs_path() {
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(is_sha256_hex(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));

        // A value the service controls must not be able to walk out of the path.
        assert!(!is_sha256_hex("../../etc/passwd"));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(&"z".repeat(64)));
        assert!(!is_sha256_hex(""));
    }

    #[test]
    fn the_route_taken_is_nameable_on_the_wire() {
        assert_eq!(ResolvedVia::Publisher.as_str(), "publisher");
        assert_eq!(ResolvedVia::Mirror.as_str(), "mirror");
    }
}
