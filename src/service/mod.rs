//! Reading the wetgeving lane through the bron service.
//!
//! One address, and what travels over it is the whole design.
//!
//! **Which rendition a date resolves to is the service's answer.** It holds
//! every manifest the register publishes, it knows which renditions the
//! register has withdrawn, and it does that arithmetic once for everybody.
//! bron used to do it a second time from a corpus of its own, and a second
//! implementation of one rule is one that falls behind: the CLI went on
//! choosing a withdrawn rendition for months after the service stopped.
//!
//! **Bytes are safe to take from it.** A blob is named by the SHA-256 of its
//! own content, and the manifest states a SHA-512 for every rendition, so a
//! body that does not hash to what was asked for is refused. `update` still
//! holds the service to the publisher's digest and falls back to the publisher
//! when it cannot answer.
//!
//! **An answer says it came through here.** [`crate::wire::ResolvedVia`] is on
//! every wetgeving answer, and `--direct`, or `BRON_MIRROR=`, takes this
//! address out of the path entirely, which for this lane means there is no
//! answer to give.
//!
//! The environment variable, the flag and the value `mirror` in `resolved_via`
//! are published contract and keep their names. Only this module was renamed,
//! because there is no publisher behind the address any more for a caller to
//! be mirrored from.

pub mod client;
pub mod model;
pub mod search;

use anyhow::{Context, Result, bail};

use crate::error::{Fault, Outcome};
use crate::ids::BwbId;
use crate::sources::{Client, Fetched, VersionMeta};

pub use client::Answered;
pub use model::{
    Article, Articles, Ask, BlobProvenance, Expression, Paragraph, Regulation, Span, Verified,
    Window,
};
pub use search::{Cited, SearchData, SearchHit, SearchLid, SearchRequest};

use client::SERVICE;

/// Where the service lives. Overrides [`DEFAULT`].
const ENV: &str = "BRON_MIRROR";

/// The service every install reads through unless told otherwise.
///
/// A default rather than an opt-in, because the reason the service exists is to
/// ask the government once for everybody, and a setting nobody sets collapses
/// no rate limit at all.
///
/// Two things keep that honest. Every answer names the route it took in
/// `resolved_via`, and every span it carries points into a blob named by the
/// hash of its own content, so a reader who wants to check a quotation can
/// fetch those bytes and cut them again. `--direct`, or `BRON_MIRROR=`, takes
/// this address out of the path.
const DEFAULT: &str = "https://bron.steddle.com/api/v1";

pub struct Service {
    base: String,
}

impl Service {
    /// The service this invocation should use, if any.
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

        let service = Self::at(std::env::var(ENV).as_deref().unwrap_or(DEFAULT));

        if service.base.is_empty() {
            return None;
        }

        Some(service)
    }

    /// The service at one address. [`Service::resolve`] decides which one a run
    /// uses; this names one outright.
    pub fn at(base: &str) -> Self {
        Self {
            base: normalise(base),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Regulations whose name matches a term.
    ///
    /// The service matches on every name the register gives a regulation in its
    /// WTI, so `Awb` finds the Algemene wet bestuursrecht where a title search
    /// never did.
    pub fn regulations(
        &self,
        client: &Client,
        query: &str,
        limit: u32,
    ) -> Outcome<Answered<Vec<Regulation>>> {
        let url = format!("{}/api/v1/regulations", self.base);

        self.read(
            client,
            client
                .get(&url)
                .query(&[("q", query), ("limit", &limit.to_string())]),
        )
    }

    /// Every rendition a regulation has ever had, held or not.
    pub fn expressions(&self, client: &Client, law: &BwbId) -> Outcome<Answered<Vec<Expression>>> {
        let url = format!("{}/api/v1/regulations/{law}/expressions", self.base);

        self.read(client, client.get(&url))
    }

    /// The rendition a point in both windows resolves to, and its articles.
    pub fn articles(
        &self,
        client: &Client,
        law: &BwbId,
        ask: &Ask<'_>,
    ) -> Outcome<Answered<Articles>> {
        let url = format!("{}/api/v1/regulations/{law}/articles", self.base);

        let mut query = vec![
            ("as_of".to_string(), ask.as_of.to_string()),
            // Always, because the answer is built from the leden and their
            // text is what `rendered` carries.
            ("rendered".to_string(), "1".to_string()),
        ];

        if let Some(date) = ask.as_known_on {
            query.push(("as_known_on".to_string(), date.to_string()));
        }
        if let Some(number) = ask.number {
            query.push(("number".to_string(), number.to_string()));
        }
        if ask.raw {
            query.push(("raw".to_string(), "1".to_string()));
        }

        self.read(client, client.get(&url).query(&query))
    }

    /// How long a blob is. See [`BlobProvenance`].
    pub fn blob_at(&self, client: &Client, digest: &str) -> Outcome<Answered<BlobProvenance>> {
        if !is_sha256_hex(digest) {
            return Err(Fault::Upstream {
                service: SERVICE,
                source: anyhow::anyhow!(
                    "the service named a blob that is not a sha-256 digest: {digest}"
                ),
            });
        }

        let url = format!("{}/api/v1/blobs/{digest}/provenance", self.base);

        self.read(client, client.get(&url))
    }

    /// Where a blob is readable, which is the address a provenance names.
    pub fn blob_url(&self, digest: &str) -> String {
        format!("{}/api/v1/blobs/{digest}", self.base)
    }

    /// A version's bytes, when the service has them.
    ///
    /// `Ok(None)` is the service saying it does not hold this version, which is
    /// an ordinary answer and a reason to ask the publisher. `Err` is the
    /// service failing, which is also a reason to ask the publisher but is
    /// worth saying out loud: a service that is quietly unreachable looks
    /// exactly like one that is quietly empty, and the difference is the whole
    /// question of whether the thing is working.
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
        let answered = self.expressions(client, law).map_err(anyhow::Error::new)?;

        Ok(digest_for(answered.data, version))
    }

    fn blob(&self, client: &Client, digest: &str) -> Result<Fetched> {
        if !is_sha256_hex(digest) {
            bail!("the service named a blob that is not a sha-256 digest: {digest}");
        }

        client
            .fetch_url(&self.blob_url(digest))
            .with_context(|| format!("cannot read blob {digest} from the service"))
    }
}

/// Which blob to ask for, out of what the service listed.
///
/// A rendition the service checked and found wrong is skipped: its bytes would
/// fail the publisher's digest on arrival too, so asking for them costs a round
/// trip and buys nothing. One it said nothing about is asked for, because
/// nothing was said and the check on arrival is what decides either way.
fn digest_for(rows: Vec<Expression>, version: &str) -> Option<String> {
    rows.into_iter()
        .find(|row| row.label == version)
        .filter(|row| row.verified != Verified::Differs)
        .and_then(|row| row.blob)
}

/// Trims a configured base down to what the paths above are appended to.
///
/// The service versions itself in the path and bron adds `/api/v1` itself, so a
/// base copied out of a browser with `/api/v1` already on the end would ask for
/// `/api/v1/api/v1` and get a 404. That failure is the quiet kind: the request
/// fails, and the only trace is a lane that stopped answering. Cheaper to
/// accept both spellings than to make everyone find out once.
fn normalise(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    base.strip_suffix("/api/v1")
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
        let service = with_env(None, || Service::resolve(false));
        assert_eq!(
            service.expect("the default").base(),
            "https://bron.steddle.com"
        );
    }

    #[test]
    fn the_default_is_normalised_like_any_other_base() {
        // It is written with the /api/v1 a reader would paste, and must not end
        // up asking for /api/v1/api/v1 any more than a hand-set value would.
        assert_eq!(normalise(DEFAULT), "https://bron.steddle.com");
    }

    #[test]
    fn direct_beats_the_environment() {
        // A single run has to be pinnable to the publisher without editing a
        // shell profile, so the flag wins.
        let service = with_env(Some("https://bron.test"), || Service::resolve(true));
        assert!(service.is_none());
    }

    #[test]
    fn a_configured_service_is_used_and_its_trailing_slash_is_not() {
        let service = with_env(Some("https://bron.test/"), || Service::resolve(false));
        // Joined with "/api/v1/..." above, so a kept slash would ask for "//api/v1".
        assert_eq!(service.expect("a service").base(), "https://bron.test");
    }

    #[test]
    fn a_base_that_already_names_the_api_version_is_accepted() {
        // The obvious thing to paste is the URL you were just reading, and that
        // one ends in /api/v1. Both spellings have to land on the same place.
        assert_eq!(
            normalise("https://bron.steddle.com/api/v1"),
            "https://bron.steddle.com"
        );
        assert_eq!(
            normalise("https://bron.steddle.com/api/v1/"),
            "https://bron.steddle.com"
        );
        assert_eq!(normalise("https://bron.test/api/v1"), "https://bron.test");
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
    fn a_blank_setting_turns_the_service_off_rather_than_restoring_the_default() {
        // The durable form of --direct. Silence means the default; an empty
        // value means someone decided.
        assert!(with_env(Some(""), || Service::resolve(false)).is_none());
        assert!(with_env(Some("   "), || Service::resolve(false)).is_none());
    }

    fn row(label: &str, blob: Option<&str>, verified: Verified) -> Expression {
        Expression {
            label: label.to_string(),
            in_force: Window {
                from: "2024-01-01".parse().unwrap(),
                until: None,
            },
            as_known: None,
            upstream_hash: None,
            size: None,
            held: blob.is_some(),
            verified,
            blob: blob.map(str::to_string),
        }
    }

    #[test]
    fn the_blob_for_a_version_is_the_one_listed_against_its_label() {
        let rows = vec![
            row("2024-01-01_0", Some("aa"), Verified::Matches),
            row("2026-07-16_0", Some("bb"), Verified::Matches),
        ];
        assert_eq!(digest_for(rows, "2026-07-16_0").as_deref(), Some("bb"));
    }

    #[test]
    fn a_rendition_the_service_checked_and_found_wrong_is_not_worth_fetching() {
        // Its bytes would fail the publisher's digest on arrival, so asking for
        // them costs a round trip and buys nothing.
        let rows = vec![row("2024-01-01_0", Some("aa"), Verified::Differs)];
        assert_eq!(digest_for(rows, "2024-01-01_0"), None);
    }

    /// Nothing said is not the same as checked and wrong. Reading it as a
    /// refusal would route every fetch to the publisher the day the service
    /// stopped sending the field, with nothing but `resolved_via` to show it.
    #[test]
    fn a_rendition_the_service_said_nothing_about_is_still_asked_for() {
        let rows = vec![row("2024-01-01_0", Some("aa"), Verified::Unstated)];
        assert_eq!(digest_for(rows, "2024-01-01_0").as_deref(), Some("aa"));
    }

    #[test]
    fn a_version_the_service_does_not_hold_yields_nothing() {
        let rows = vec![row("2024-01-01_0", None, Verified::Matches)];
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
}
