//! Reading the wetgeving lane through the bron service.
//!
//! One address, and what travels over it is the whole design.
//!
//! **Which rendition a date resolves to is the service's answer.** It holds
//! every manifest the register publishes, it knows which renditions the
//! register has withdrawn, and it does that arithmetic once for everybody. A
//! second implementation of one rule is the one that falls behind: a withdrawn
//! rendition keeps its dates in the manifest, so a resolver that does not know
//! it was withdrawn goes on choosing it.
//!
//! **Nothing has to be taken on trust.** Every answer names the blob it was
//! read from, addressed by the hash of its own content, and the offsets of the
//! article inside it. Fetch those bytes, hash them, slice them, and either they
//! say what bron said or they do not.
//!
//! There is no second route. `BRON_MIRROR` points this at another deployment
//! and nothing points it at a publisher, because resolving a date against the
//! register means holding every manifest the register publishes and that is
//! what the service is for.

pub mod client;
pub mod model;
pub mod search;

use crate::error::{Fault, Outcome};
use crate::ids::BwbId;
use crate::sources::Client;

pub use client::Answered;
pub use model::{
    Article, Articles, Ask, Change, Changes, Compare, Expression, Feed, Image, Paragraph, Parts,
    Published, RegisterEvent, Regulation, Span, Summary, Units, Window, name,
};
pub use search::{Cited, Place, SearchData, SearchHit, SearchLid, SearchRequest};

/// Where the service lives. Overrides [`DEFAULT`].
const ENV: &str = "BRON_MIRROR";

/// The service every install reads through unless told otherwise.
///
/// A default rather than an opt-in, because the reason the service exists is to
/// ask the government once for everybody, and a setting nobody sets collapses
/// no rate limit at all. What keeps that honest is that every span it carries
/// points into a blob named by the hash of its own content, so a reader who
/// wants to check a quotation can fetch those bytes and cut them again.
const DEFAULT: &str = "https://bron.steddle.com/api/v1";

/// What `limit` reads as the whole set, on the two routes one regulation
/// bounds. The service pages both by default, and refuses the word on a route
/// the corpus bounds.
const WHOLE: &str = "none";

#[derive(Debug)]
pub struct Service {
    base: String,
}

impl Service {
    /// The service this invocation reads through.
    ///
    /// Always one. Every lane either reads the service or reads a publisher
    /// directly, and no lane does both, so there is nothing an empty setting
    /// could turn the service off in favour of. A setting that names no service
    /// is refused rather than silently ignored, because whoever wrote it meant
    /// something by it and is owed the news that it cannot be done.
    ///
    /// The refusal quotes what was set, and asks whether the value names a
    /// service rather than whether it survived normalising. `""`, `"/"` and
    /// `"/api/v1"` all normalise away to nothing, but `"example.com"` and
    /// `"/some/path"` do not: carried to the first request they fail there as
    /// `upstream`, a value the caller typed wrong charged to the service. One
    /// rule, an address with a scheme and a host, refuses all five on the same
    /// sentence and at the caller's own exit code.
    pub fn resolve() -> Outcome<Self> {
        let Ok(configured) = std::env::var(ENV) else {
            return Ok(Self::at(DEFAULT));
        };

        let service = Self::at(&configured);

        let names_a_service = reqwest::Url::parse(&service.base)
            .is_ok_and(|url| url.has_host() && matches!(url.scheme(), "http" | "https"));

        if !names_a_service {
            return Err(Fault::usage(format!(
                "{ENV} is set to [{configured}], which names no service. The legislation lane \
                 reads the bron service and nothing else; unset it to use {DEFAULT}, or point \
                 it at another deployment."
            )));
        }

        Ok(service)
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

        self.read(client, client.get(&url).query(&[("limit", WHOLE)]))
    }

    /// Which renditions the register published inside a window, across laws.
    ///
    /// A window is required and both axes may be named at once. The service
    /// defaults nothing to today, so a window that has not happened answers
    /// empty rather than silently becoming a question about now.
    pub fn published(
        &self,
        client: &Client,
        ask: &Published<'_>,
    ) -> Outcome<Answered<Vec<Expression>>> {
        let url = format!("{}/api/v1/expressions", self.base);

        let mut query = vec![("limit", ask.limit.to_string())];

        for (name, day) in [
            ("in_force_from", ask.from),
            ("in_force_to", ask.to),
            ("visible_from", ask.known_from),
            ("visible_to", ask.known_to),
        ] {
            if let Some(day) = day {
                query.push((name, day.to_string()));
            }
        }

        if let Some(kind) = ask.kind {
            query.push(("type", kind.to_string()));
        }

        if let Some(law) = ask.regulation {
            query.push(("regulation", law.to_string()));
        }

        self.read(client, client.get(&url).query(&query))
    }

    /// What the register has done since a cursor.
    pub fn feed(
        &self,
        client: &Client,
        since: u64,
        limit: u32,
        regulation: Option<&BwbId>,
    ) -> Outcome<Answered<Feed>> {
        let url = format!("{}/api/v1/feed", self.base);

        let mut query = vec![("since", since.to_string()), ("limit", limit.to_string())];

        if let Some(law) = regulation {
            query.push(("regulation", law.to_string()));
        }

        self.read(client, client.get(&url).query(&query))
    }

    /// The rendition a point in both windows resolves to, and its articles.
    ///
    /// All of them, always: at a terminal the whole law is what `law show`
    /// means. Where `number` narrows the answer the set is a handful, so asking
    /// for the whole of it costs nothing.
    pub fn articles(
        &self,
        client: &Client,
        law: &BwbId,
        ask: &Ask<'_>,
    ) -> Outcome<Answered<Articles>> {
        let url = format!("{}/api/v1/regulations/{law}/articles", self.base);

        let mut query = vec![
            ("as_of", ask.as_of.to_string()),
            // Always, because the answer is built from the leden and their
            // text is what `rendered` carries.
            ("rendered", "1".to_string()),
            ("limit", WHOLE.to_string()),
        ];

        if let Some(date) = ask.as_known_on {
            query.push(("as_known_on", date.to_string()));
        }
        if let Some(number) = ask.number {
            query.push(("number", number.to_string()));
        }
        if let Some(paragraph) = ask.paragraph {
            query.push(("paragraph", paragraph.to_string()));
        }
        if ask.raw {
            query.push(("raw", "1".to_string()));
        }

        if ask.not_commenced {
            query.push(("not_commenced", "1".to_string()));
        }

        self.read(client, client.get(&url).query(&query))
    }

    /// The rendition a point in both windows resolves to, and the units of
    /// text asked of it.
    pub fn parts(&self, client: &Client, law: &BwbId, ask: &Units<'_>) -> Outcome<Answered<Parts>> {
        let url = format!("{}/api/v1/regulations/{law}/parts", self.base);

        let mut query = vec![
            ("as_of", ask.as_of.to_string()),
            ("rendered", "1".to_string()),
            ("limit", ask.limit.to_string()),
            ("offset", ask.offset.to_string()),
        ];

        if let Some(date) = ask.as_known_on {
            query.push(("as_known_on", date.to_string()));
        }
        if let Some(kind) = ask.kind {
            query.push(("kind", kind.to_string()));
        }
        if let Some(number) = ask.number {
            query.push(("number", number.to_string()));
        }
        if let Some(position) = ask.position {
            query.push(("position", position.to_string()));
        }
        if ask.raw {
            query.push(("raw", "1".to_string()));
        }

        self.read(client, client.get(&url).query(&query))
    }

    /// The service's comparison of a regulation between two days.
    ///
    /// Always rendered, because a person reads what changed in the words.
    pub fn changes(
        &self,
        client: &Client,
        law: &BwbId,
        ask: &Compare<'_>,
    ) -> Outcome<Answered<Changes>> {
        let url = format!("{}/api/v1/regulations/{law}/changes", self.base);

        let mut query = vec![
            ("from", ask.from.to_string()),
            ("to", ask.to.to_string()),
            ("rendered", "1".to_string()),
        ];

        if let Some(kind) = ask.kind {
            query.push(("kind", kind.to_string()));
        }
        if let Some(article) = ask.article {
            query.push(("article", article.to_string()));
        }
        if ask.all {
            query.push(("all", "1".to_string()));
        }
        if ask.raw {
            query.push(("raw", "1".to_string()));
        }

        self.read(client, client.get(&url).query(&query))
    }

    /// The bytes a digest names, checked against it.
    ///
    /// Hashed here, from the bytes as they arrived, so what is handed on is
    /// what the digest names or nothing at all: bytes that hash to anything
    /// else are refused rather than quoted.
    pub fn blob(&self, client: &Client, digest: &str) -> Outcome<crate::sources::FetchedBytes> {
        if !is_sha256_hex(digest) {
            return Err(Fault::usage(format!(
                "[{digest}] is not a SHA-256 digest as 64 lowercase hex characters"
            )));
        }

        let url = format!("{}/api/v1/blobs/{digest}", self.base);
        let fetched = self.read_raw(client, client.get(&url))?;

        if fetched.blob.to_string() != digest {
            return Err(Fault::Upstream {
                service: client::SERVICE,
                source: anyhow::anyhow!(
                    "the {} bytes served under {digest} hash to {}, which is not the document \
                     that digest names",
                    fetched.bytes.len(),
                    fetched.blob
                ),
            });
        }

        Ok(fetched)
    }

    /// Where a blob is readable, for a digest that names one.
    ///
    /// The digest is the service's own word and is checked before it reaches a
    /// URL, so an address bron prints for a reader to fetch is one that can
    /// name a document rather than whatever arrived in that field.
    pub fn blob_url(&self, digest: &str) -> Option<String> {
        is_sha256_hex(digest).then(|| format!("{}/api/v1/blobs/{digest}", self.base))
    }
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
/// cannot walk out of the blobs path. The service's own rule: 64 lowercase hex
/// characters, and it refuses anything else as `invalid_digest`.
fn is_sha256_hex(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
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
        let service = with_env(None, Service::resolve);
        assert_eq!(
            service.expect("the default").base(),
            "https://bron.steddle.com"
        );
    }

    /// A setting is refused for naming no service, not for surviving
    /// `normalise` badly. Two of these reached the first request and came back
    /// as `upstream`, which charged the service for what the caller typed.
    #[test]
    fn a_setting_that_names_no_service_is_a_usage_failure() {
        for named in [
            "",
            "   ",
            "/",
            "/api/v1",
            "example.com",
            "/some/path",
            "bron.steddle.com",
        ] {
            let refused = with_env(Some(named), Service::resolve);

            let fault = refused
                .err()
                .unwrap_or_else(|| panic!("[{named}] names no service and must be refused"));

            assert_eq!(fault.kind(), "usage", "[{named}]");
            assert!(fault.to_string().contains(named.trim()), "[{named}]");
        }
    }

    #[test]
    fn an_address_with_a_scheme_and_a_host_is_taken_either_way_it_is_written() {
        for named in [
            "https://bron.test",
            "https://bron.test/",
            "https://bron.test/api/v1",
            "http://127.0.0.1:8080",
        ] {
            let service = with_env(Some(named), Service::resolve);
            assert!(service.is_ok(), "[{named}] names a service");
        }
    }

    #[test]
    fn only_a_sha256_digest_can_name_a_document() {
        let service = Service::at("https://bron.test");

        assert_eq!(
            service.blob_url(&"a".repeat(64)),
            Some(format!("https://bron.test/api/v1/blobs/{}", "a".repeat(64)))
        );

        // A value the service controls must not be able to walk out of the
        // path, or send a reader to an address that names no document.
        assert_eq!(service.blob_url("../../etc/passwd"), None);
        assert_eq!(service.blob_url(&"a".repeat(63)), None);
        assert_eq!(service.blob_url(&"z".repeat(64)), None);
        // The service's rule is lowercase, and it refuses the upper case.
        assert_eq!(service.blob_url(&"A".repeat(64)), None);
        assert_eq!(service.blob_url(""), None);
    }

    #[test]
    fn the_default_is_normalised_like_any_other_base() {
        // It is written with the /api/v1 a reader would paste, and must not end
        // up asking for /api/v1/api/v1 any more than a hand-set value would.
        assert_eq!(normalise(DEFAULT), "https://bron.steddle.com");
    }

    #[test]
    fn a_configured_service_is_used_and_its_trailing_slash_is_not() {
        let service = with_env(Some("https://bron.test/"), Service::resolve);
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

    /// An empty setting has nothing to turn the service off in favour of, so it
    /// is refused rather than quietly read as silence: whoever wrote it meant
    /// something by it.
    ///
    /// `/api/v1` and `/` are not empty and normalise away to nothing all the
    /// same, so the refusal quotes what was set rather than calling it empty.
    #[test]
    fn a_setting_that_names_no_service_is_refused_and_quoted_back() {
        for nothing in ["", "   ", "/", "/api/v1"] {
            let refused =
                with_env(Some(nothing), Service::resolve).expect_err("this names no service");

            assert_eq!(refused.kind(), "usage");
            assert!(
                refused.to_string().contains(&format!("[{nothing}]")),
                "the refusal must quote what was set: {refused}"
            );
        }
    }
}
