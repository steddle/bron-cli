//! Official identifiers, as types.
//!
//! Identity in bron is always the identifier the publisher assigns: a BWB id, an
//! ECLI, a document number. Never a URL, which is an address that can change
//! without the thing it addresses changing.
//!
//! Each type validates on the way in. That makes the difference between a lookup
//! and a search visible at the call site: `BwbId::from_str("burgerlijk wetboek")`
//! fails, so anything accepting free text is doing resolution, not retrieval.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use rusqlite::ToSql;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use serde::Serialize;

/// Error returned when text does not have the shape of an identifier.
#[derive(Debug, thiserror::Error)]
#[error("not a valid {kind}: {text}")]
pub struct InvalidId {
    pub kind: &'static str,
    pub text: String,
}

impl InvalidId {
    fn new(kind: &'static str, text: &str) -> Self {
        Self {
            kind,
            text: text.to_string(),
        }
    }
}

/// Generates the shared boilerplate: display, serde, and both SQL directions.
macro_rules! string_id {
    ($name:ident, $kind:literal) => {
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(self.0.as_str()))
            }
        }

        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                let text = value.as_str()?;
                text.parse().map_err(|e| FromSqlError::Other(Box::new(e)))
            }
        }

        impl TryFrom<&str> for $name {
            type Error = InvalidId;

            fn try_from(text: &str) -> Result<Self, Self::Error> {
                text.parse()
            }
        }

        impl $name {
            pub const KIND: &'static str = $kind;
        }
    };
}

/// A Basis Wetten Bestand identifier, e.g. `BWBR0005289`.
///
/// `BWBR` for regelingen, `BWBV` for verdragen.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BwbId(String);

string_id!(BwbId, "BWB id");

impl FromStr for BwbId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let valid = text.len() == 11
            && text.is_char_boundary(4)
            && matches!(&text[..4], "BWBR" | "BWBV")
            && text[4..].bytes().all(|b| b.is_ascii_digit());

        if valid {
            Ok(Self(text.to_string()))
        } else {
            Err(InvalidId::new(Self::KIND, text))
        }
    }
}

/// A European Case Law Identifier, e.g. `ECLI:NL:HR:2019:1278`.
///
/// Five colon-separated fields: the literal `ECLI`, a country code, the court
/// code, the year, and an ordinal. Only well-formedness is checked here;
/// whether the decision exists is a question for the network.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ecli(String);

string_id!(Ecli, "ECLI");

impl Ecli {
    /// Court code, e.g. `HR` or `RBAMS`.
    pub fn court(&self) -> &str {
        self.0.split(':').nth(2).unwrap_or_default()
    }

    /// Year of the decision as recorded in the identifier.
    pub fn year(&self) -> &str {
        self.0.split(':').nth(3).unwrap_or_default()
    }
}

impl FromStr for Ecli {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let fields: Vec<&str> = text.split(':').collect();

        let valid = match fields[..] {
            [prefix, country, court, year, ordinal] => {
                prefix.eq_ignore_ascii_case("ECLI")
                    && country.len() == 2
                    && country.bytes().all(|b| b.is_ascii_alphabetic())
                    && !court.is_empty()
                    && court.bytes().all(|b| b.is_ascii_alphanumeric())
                    && year.len() == 4
                    && year.bytes().all(|b| b.is_ascii_digit())
                    && !ordinal.is_empty()
                    && ordinal
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'.')
            }
            _ => false,
        };

        if valid {
            // The prefix and country are conventionally uppercase; the ordinal
            // is not, so only normalise what is safe to normalise.
            let mut parts = fields;
            let prefix = parts[0].to_ascii_uppercase();
            let country = parts[1].to_ascii_uppercase();
            parts[0] = &prefix;
            parts[1] = &country;
            Ok(Self(parts.join(":")))
        } else {
            Err(InvalidId::new(Self::KIND, text))
        }
    }
}

/// An officiële bekendmakingen document number, e.g. `kst-33669-6` or
/// `stb-2024-123`.
///
/// The collection prefix is open-ended, so this checks shape rather than
/// membership of a fixed list.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicationId(String);

string_id!(PublicationId, "publication id");

impl PublicationId {
    /// Collection prefix, e.g. `kst`, `stb`, `stcrt`.
    pub fn collection(&self) -> &str {
        self.0.split('-').next().unwrap_or_default()
    }
}

impl FromStr for PublicationId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let mut fields = text.split('-');

        let prefix_ok = fields
            .next()
            .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase()));
        let rest: Vec<&str> = fields.collect();
        let rest_ok = !rest.is_empty()
            && rest
                .iter()
                .all(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_alphanumeric()));

        if prefix_ok && rest_ok {
            Ok(Self(text.to_string()))
        } else {
            Err(InvalidId::new(Self::KIND, text))
        }
    }
}

impl PublicationId {
    /// Resolves a BWB source reference to the publication it names.
    ///
    /// BWB writes these as `Stb.2023-41`; the publication register addresses
    /// the same document as `stb-2023-41`. Translating between them is what
    /// turns an article into the amending act that produced it, and the
    /// amending act is where transitional law lives.
    pub fn from_bwb_reference(reference: &str) -> Option<Self> {
        let (kind, rest) = reference.trim().split_once('.')?;
        let (year, number) = rest.split_once('-')?;

        let valid = !kind.is_empty()
            && kind.bytes().all(|b| b.is_ascii_alphabetic())
            && year.len() == 4
            && year.bytes().all(|b| b.is_ascii_digit())
            && !number.is_empty()
            && number.bytes().all(|b| b.is_ascii_alphanumeric());

        valid.then(|| Self(format!("{}-{year}-{number}", kind.to_ascii_lowercase())))
    }
}

/// An Internet Archive capture timestamp: fourteen digits, `YYYYMMDDHHMMSS`.
///
/// Held as the digits the archive assigned rather than as a parsed instant, so
/// the identifier round-trips through the corpus byte for byte. The archive
/// states these in UTC, which is why the parsed form is a [`DateTime<Utc>`] and
/// not a `NaiveDateTime`: a naive value would claim the zone is unknown when it
/// is known, and that is how a capture ends up an hour from where it happened.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp14(String);

impl Timestamp14 {
    pub const KIND: &'static str = "archive timestamp";

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The instant these digits name.
    pub fn to_utc(&self) -> DateTime<Utc> {
        NaiveDateTime::parse_from_str(&self.0, "%Y%m%d%H%M%S")
            .expect("validated on construction")
            .and_utc()
    }

    pub fn date(&self) -> NaiveDate {
        self.to_utc().date_naive()
    }
}

impl FromStr for Timestamp14 {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();

        // Fourteen digits is necessary and not sufficient: 20261332000000 is
        // fourteen digits and not a date. Parsing is the validation.
        let valid = text.len() == 14
            && text.bytes().all(|b| b.is_ascii_digit())
            && NaiveDateTime::parse_from_str(text, "%Y%m%d%H%M%S").is_ok();

        if valid {
            Ok(Self(text.to_string()))
        } else {
            Err(InvalidId::new(Self::KIND, text))
        }
    }
}

impl fmt::Display for Timestamp14 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One capture in the Internet Archive, as the archive identifies it.
///
/// The archive assigns the timestamp; the URL is what was captured. A timestamp
/// alone is not unique and a URL alone is not a capture, so identity is the
/// pair. This is the one identifier in bron with two fields, and it is still
/// the publisher's and not an address.
///
/// The URL half is CDX's `urlkey`, its own canonical form, never `original`.
/// Two spellings of one address canonicalize together, so keying on `original`
/// would mint two identities for one record and leave `Eq` and `Hash` wrong
/// about the thing they are supposed to be right about. `original` is what the
/// capture is *of*, and it travels on the capture.
///
/// `Display` renders the identifier, never a web.archive.org URL. Whatever
/// `Display` emits is what lands in the corpus, in the JSON and in a citation,
/// and a replay address there would be the URL-as-identity this module exists
/// to refuse. The two URL forms are built on demand and named for what they are.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaptureId {
    timestamp: Timestamp14,
    urlkey: String,
}

/// Sorted the way the index sorts: urlkey, then timestamp.
///
/// Written out rather than derived, because a derive orders by field
/// declaration and would make the order of every answer bron gives a
/// consequence of which field someone typed first.
impl Ord for CaptureId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.urlkey
            .cmp(&other.urlkey)
            .then_with(|| self.timestamp.cmp(&other.timestamp))
    }
}

impl PartialOrd for CaptureId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl CaptureId {
    pub const KIND: &'static str = "capture id";

    pub fn new(timestamp: Timestamp14, urlkey: impl Into<String>) -> Self {
        Self {
            timestamp,
            urlkey: urlkey.into(),
        }
    }

    pub fn timestamp(&self) -> &Timestamp14 {
        &self.timestamp
    }

    pub fn urlkey(&self) -> &str {
        &self.urlkey
    }

    pub fn captured_at(&self) -> DateTime<Utc> {
        self.timestamp.to_utc()
    }

    /// The address the captured bytes are served from, unmodified.
    ///
    /// `id_` is what asks the archive for the resource as it was captured,
    /// without the replay banner and without rewritten links. Text derived over
    /// this behaves as it would over a direct fetch; text derived over the
    /// replay page is derived over the archive's furniture.
    pub fn replay_url(&self, original: &str) -> String {
        format!(
            "https://web.archive.org/web/{}id_/{original}",
            self.timestamp
        )
    }

    /// The address a citation names, which is the page a reader should open.
    pub fn citation_url(&self, original: &str) -> String {
        format!("https://web.archive.org/web/{}/{original}", self.timestamp)
    }

    /// Reads an id back out of a pasted web.archive.org URL.
    ///
    /// A convenience for a caller holding a link, not the identity parse: the
    /// URL carries `original` where the identifier carries `urlkey`, so what
    /// comes back is keyed on the address as written. Prefer minting the id
    /// from a CDX row, which states the canonical form.
    pub fn from_replay_url(url: &str) -> Result<(Self, String), InvalidId> {
        let fail = || InvalidId::new(Self::KIND, url);

        let rest = url
            .trim()
            .strip_prefix("https://web.archive.org/web/")
            .or_else(|| url.trim().strip_prefix("http://web.archive.org/web/"))
            .ok_or_else(fail)?;

        let (stamp, original) = rest.split_once('/').ok_or_else(fail)?;
        // The modifier rides on the timestamp with no separator: `...20260806153132id_/https://...`
        let stamp = stamp
            .strip_suffix("id_")
            .or_else(|| stamp.strip_suffix("if_"))
            .unwrap_or(stamp);

        if original.is_empty() {
            return Err(fail());
        }

        Ok((
            Self::new(stamp.parse()?, original.to_string()),
            original.to_string(),
        ))
    }
}

impl fmt::Display for CaptureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.timestamp, self.urlkey)
    }
}

impl FromStr for CaptureId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let (stamp, urlkey) = text
            .split_once('/')
            .ok_or_else(|| InvalidId::new(Self::KIND, text))?;

        if urlkey.is_empty() {
            return Err(InvalidId::new(Self::KIND, text));
        }

        Ok(Self::new(stamp.parse()?, urlkey.to_string()))
    }
}

impl Serialize for CaptureId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl ToSql for CaptureId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}

impl FromSql for CaptureId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

impl TryFrom<&str> for CaptureId {
    type Error = InvalidId;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        text.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_real_bwb_ids() {
        assert_eq!(
            "BWBR0005289".parse::<BwbId>().unwrap().as_str(),
            "BWBR0005289"
        );
        assert!("BWBV0001234".parse::<BwbId>().is_ok());
        assert!("  BWBR0005289  ".parse::<BwbId>().is_ok());
    }

    #[test]
    fn rejects_free_text_as_a_bwb_id() {
        // The point of the newtype: this is a search, and the type says so.
        assert!("burgerlijk wetboek".parse::<BwbId>().is_err());
        assert!("BWBR005289".parse::<BwbId>().is_err());
        assert!("BWBX0005289".parse::<BwbId>().is_err());
        assert!("BWBR000528X".parse::<BwbId>().is_err());
        assert!("".parse::<BwbId>().is_err());
    }

    #[test]
    fn bwb_rejection_is_not_a_panic_on_multibyte_input() {
        assert!("Bä€".parse::<BwbId>().is_err());
        assert!("日本語のテキストです".parse::<BwbId>().is_err());
    }

    #[test]
    fn accepts_and_normalises_eclis() {
        let ecli: Ecli = "ecli:nl:HR:2019:1278".parse().unwrap();
        assert_eq!(ecli.as_str(), "ECLI:NL:HR:2019:1278");
        assert_eq!(ecli.court(), "HR");
        assert_eq!(ecli.year(), "2019");
    }

    #[test]
    fn rejects_malformed_eclis() {
        assert!("ECLI:NL:HR:2019".parse::<Ecli>().is_err());
        assert!("ECLI:NL:HR:19:1278".parse::<Ecli>().is_err());
        assert!("ECLI:NLD:HR:2019:1278".parse::<Ecli>().is_err());
        assert!(
            "http://deeplink.rechtspraak.nl/uitspraak?id=x"
                .parse::<Ecli>()
                .is_err()
        );
    }

    #[test]
    fn accepts_publication_ids_across_collections() {
        assert_eq!(
            "kst-33669-6".parse::<PublicationId>().unwrap().collection(),
            "kst"
        );
        assert_eq!(
            "stb-2024-123"
                .parse::<PublicationId>()
                .unwrap()
                .collection(),
            "stb"
        );
        assert!("stcrt-2024-1234".parse::<PublicationId>().is_ok());
        assert!("ah-tk-20232024-1234".parse::<PublicationId>().is_ok());
    }

    #[test]
    fn rejects_malformed_publication_ids() {
        assert!("kst".parse::<PublicationId>().is_err());
        assert!("KST-33669-6".parse::<PublicationId>().is_err());
        assert!("kst--6".parse::<PublicationId>().is_err());
    }

    #[test]
    fn bwb_source_references_resolve_to_publications() {
        assert_eq!(
            PublicationId::from_bwb_reference("Stb.2023-41")
                .unwrap()
                .as_str(),
            "stb-2023-41"
        );
        assert_eq!(
            PublicationId::from_bwb_reference("Stb.1991-600")
                .unwrap()
                .as_str(),
            "stb-1991-600"
        );
        assert_eq!(
            PublicationId::from_bwb_reference("Stcrt.2020-12345")
                .unwrap()
                .as_str(),
            "stcrt-2020-12345"
        );
    }

    #[test]
    fn a_reference_that_is_not_one_resolves_to_nothing() {
        assert!(PublicationId::from_bwb_reference("onbekend").is_none());
        assert!(PublicationId::from_bwb_reference("Stb.91-600").is_none());
        assert!(PublicationId::from_bwb_reference("").is_none());
    }

    #[test]
    fn round_trips_through_sqlite() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (bwb TEXT, ecli TEXT)", [])
            .unwrap();

        let bwb: BwbId = "BWBR0005289".parse().unwrap();
        let ecli: Ecli = "ECLI:NL:HR:2019:1278".parse().unwrap();
        conn.execute(
            "INSERT INTO t VALUES (?1, ?2)",
            rusqlite::params![bwb, ecli],
        )
        .unwrap();

        let (back_bwb, back_ecli): (BwbId, Ecli) = conn
            .query_row("SELECT bwb, ecli FROM t", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(back_bwb, bwb);
        assert_eq!(back_ecli, ecli);
    }

    #[test]
    fn a_corrupt_row_fails_loudly_rather_than_yielding_a_bad_id() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (ecli TEXT)", []).unwrap();
        conn.execute("INSERT INTO t VALUES ('not-an-ecli')", [])
            .unwrap();

        let result: rusqlite::Result<Ecli> = conn.query_row("SELECT ecli FROM t", [], |r| r.get(0));
        assert!(result.is_err());
    }

    // -- archive captures -------------------------------------------------

    #[test]
    fn a_timestamp_is_fourteen_digits_that_are_also_a_date() {
        assert_eq!(
            "20260806153132".parse::<Timestamp14>().unwrap().as_str(),
            "20260806153132"
        );

        assert!(
            "2026080615313".parse::<Timestamp14>().is_err(),
            "thirteen digits"
        );
        assert!(
            "20260806153132x".parse::<Timestamp14>().is_err(),
            "not all digits"
        );
        // Fourteen digits is not enough on its own.
        assert!("20261332000000".parse::<Timestamp14>().is_err(), "month 13");
        assert!(
            "20260231000000".parse::<Timestamp14>().is_err(),
            "31 February"
        );
    }

    #[test]
    fn a_timestamp_is_read_as_utc_because_the_archive_states_it_in_utc() {
        let stamp: Timestamp14 = "20260806153132".parse().unwrap();
        assert_eq!(stamp.to_utc().to_rfc3339(), "2026-08-06T15:31:32+00:00");
        assert_eq!(stamp.date(), NaiveDate::from_ymd_opt(2026, 8, 6).unwrap());
    }

    #[test]
    fn a_capture_id_displays_as_an_identifier_and_never_as_a_url() {
        let id = CaptureId::new("20260806153132".parse().unwrap(), "com,steddle)/");

        // What Display emits is what lands in the corpus, the JSON and a
        // citation. A replay address here is URL-as-identity.
        assert_eq!(id.to_string(), "20260806153132/com,steddle)/");
        assert!(!id.to_string().contains("web.archive.org"));
    }

    #[test]
    fn a_capture_id_round_trips_through_its_own_display() {
        let id = CaptureId::new("20260806153132".parse().unwrap(), "com,steddle)/");
        assert_eq!(id.to_string().parse::<CaptureId>().unwrap(), id);
    }

    #[test]
    fn the_two_url_forms_are_different_and_both_named() {
        let id = CaptureId::new("20260806153132".parse().unwrap(), "com,steddle)/");
        let original = "https://steddle.com/";

        // id_ is the bytes as captured; the bare form is the page a reader opens.
        assert_eq!(
            id.replay_url(original),
            "https://web.archive.org/web/20260806153132id_/https://steddle.com/"
        );
        assert_eq!(
            id.citation_url(original),
            "https://web.archive.org/web/20260806153132/https://steddle.com/"
        );
    }

    #[test]
    fn a_pasted_replay_url_reads_back_with_or_without_a_modifier() {
        for url in [
            "https://web.archive.org/web/20260806153132id_/https://steddle.com/",
            "https://web.archive.org/web/20260806153132if_/https://steddle.com/",
            "https://web.archive.org/web/20260806153132/https://steddle.com/",
            "http://web.archive.org/web/20260806153132/https://steddle.com/",
        ] {
            let (id, original) = CaptureId::from_replay_url(url).unwrap();
            assert_eq!(id.timestamp().as_str(), "20260806153132", "{url}");
            assert_eq!(original, "https://steddle.com/", "{url}");
        }
    }

    #[test]
    fn a_url_that_is_not_a_capture_is_refused() {
        assert!(CaptureId::from_replay_url("https://steddle.com/").is_err());
        assert!(CaptureId::from_replay_url("https://web.archive.org/web/20260806153132/").is_err());
        assert!(CaptureId::from_replay_url("https://web.archive.org/web/nope/https://x/").is_err());
    }

    #[test]
    fn capture_ids_sort_the_way_the_index_does() {
        // CDX orders by urlkey, then timestamp. Ordering that a caller can rely
        // on has to be stated, not inherited from whatever the rows arrived in.
        let mut ids = [
            CaptureId::new("20260806153132".parse().unwrap(), "com,steddle)/b"),
            CaptureId::new("20240101000000".parse().unwrap(), "com,steddle)/b"),
            CaptureId::new("20250101000000".parse().unwrap(), "com,steddle)/a"),
        ];
        ids.sort();

        let order: Vec<String> = ids.iter().map(ToString::to_string).collect();
        assert_eq!(
            order,
            [
                "20250101000000/com,steddle)/a",
                "20240101000000/com,steddle)/b",
                "20260806153132/com,steddle)/b",
            ]
        );
    }

    #[test]
    fn a_capture_id_survives_the_corpus() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id TEXT)", []).unwrap();

        let id = CaptureId::new("20260806153132".parse().unwrap(), "com,steddle)/");
        conn.execute("INSERT INTO t VALUES (?1)", [&id]).unwrap();

        let back: CaptureId = conn
            .query_row("SELECT id FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(back, id);
    }
}
