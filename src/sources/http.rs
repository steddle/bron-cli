//! One HTTP client for every source.
//!
//! Every response is frozen into the blob store before it is parsed, and the
//! digest travels with the text. A caller that stores a fact can therefore
//! always name the exact bytes it read that fact from.
//!
//! There is no freshness cache here. A content-addressed store cannot be
//! consulted before a fetch, because the key is the answer. Skipping a refetch
//! is the corpus's decision, made from what it already holds: the BWB manifest
//! names the current version, so an update compares versions, not bodies.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::blocking::RequestBuilder;

use crate::blob::{BlobId, BlobStore};
use crate::paths;
use crate::ratelimit::{Bucket, Policy};

/// Identifies bron honestly to the public services it reads.
const USER_AGENT: &str = concat!(
    "bron/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/mischasigtermans/bron)"
);

/// rechtspraak's search endpoint refuses anything that does not look like the
/// browser app it serves. It is applied to that one request, not to the
/// overheid.nl services, which have no such requirement and where being
/// indistinguishable from a browser in their logs is the wrong outcome.
pub const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                                      AppleWebKit/537.36 (KHTML, like Gecko) \
                                      Chrome/126.0 Safari/537.36";

/// A response, frozen, before anything has been decided about its encoding.
///
/// The primitive every fetch goes through. Publisher XML is UTF-8 and decodes
/// on the way out, but an archived page is whatever the site served in whatever
/// year it served it, and a capture can be a PDF that never decodes at all. The
/// blob is written either way, so provenance holds for bytes that no reading
/// will ever turn into text.
pub struct FetchedBytes {
    /// Digest of the exact bytes stored.
    pub blob: BlobId,
    pub bytes: Vec<u8>,
    /// The URL actually dialled. Recorded for provenance, never for identity.
    pub url: String,
    /// What the server said these bytes are, verbatim and unparsed.
    pub content_type: Option<String>,
    /// The transfer encoding the server applied, verbatim. Read from the
    /// header rather than guessed from the leading bytes, so an encoding bron
    /// cannot undo is named rather than mistaken for content.
    pub content_encoding: Option<String>,
}

/// A response, frozen and decoded as UTF-8.
pub struct Fetched {
    /// Digest of the exact bytes stored.
    pub blob: BlobId,
    /// Those bytes, decoded. Byte offsets into this are offsets into the blob.
    pub text: String,
    /// The URL actually dialled. Recorded for provenance, never for identity.
    pub url: String,
}

impl Fetched {
    pub fn byte_len(&self) -> usize {
        self.text.len()
    }
}

/// Called when a request had to wait for the shared budget, with the host and
/// the wait. A sweep that pauses for two minutes must be able to say so rather
/// than look hung.
pub type WaitReporter = Box<dyn Fn(&str, Duration) + Send + Sync>;

pub struct Client {
    inner: reqwest::blocking::Client,
    /// The same client, except that it reports a redirect rather than taking
    /// it. See [`Client::get_unredirected`].
    no_redirect: reqwest::blocking::Client,
    blobs: BlobStore,
    buckets: PathBuf,
    on_wait: Option<WaitReporter>,
}

impl Client {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: build(reqwest::redirect::Policy::default())?,
            no_redirect: build(reqwest::redirect::Policy::none())?,
            blobs: BlobStore::new(paths::blob_dir()?),
            buckets: paths::bucket_dir()?,
            on_wait: None,
        })
    }

    /// A client whose blobs and buckets land somewhere specific. For tests.
    pub fn with_dirs(blobs: impl Into<PathBuf>, buckets: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            inner: build(reqwest::redirect::Policy::default())?,
            no_redirect: build(reqwest::redirect::Policy::none())?,
            blobs: BlobStore::new(blobs),
            buckets: buckets.into(),
            on_wait: None,
        })
    }

    /// Installs a [`WaitReporter`].
    pub fn on_wait(mut self, f: impl Fn(&str, Duration) + Send + Sync + 'static) -> Self {
        self.on_wait = Some(Box::new(f));
        self
    }

    pub fn get(&self, url: &str) -> RequestBuilder {
        self.inner.get(url)
    }

    pub fn post(&self, url: &str) -> RequestBuilder {
        self.inner.post(url)
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    /// Sends a prepared request and freezes what comes back, undecoded.
    ///
    /// The URL is taken from the built request rather than from the caller, so
    /// provenance records the address actually dialled, query string and all.
    pub fn fetch_bytes(&self, request: RequestBuilder) -> Result<FetchedBytes> {
        self.fetch_on(&self.inner, request)
    }

    /// Freezes what comes back without taking any redirect it offers.
    ///
    /// Following a redirect returns the bytes of whatever it points at, under
    /// the address that was asked for. For an ordinary fetch that is what a
    /// caller wants. For a captured redirect it is not: the archive replays a
    /// captured 301 as a real 301, so following it lands on a *different*
    /// capture, and checking those bytes against the digest of the one that
    /// was asked for fails every time, on a capture that is perfectly sound.
    ///
    /// Measured on a 400-address sweep: 30% failed this way, and one capture
    /// produced a different hash on each attempt, because the chain ended
    /// somewhere that moved between requests.
    ///
    /// The policy belongs to the client that *executes* the request rather
    /// than the one that built it, which is why this is a separate fetch and
    /// not a separate builder.
    pub fn fetch_bytes_unredirected(&self, request: RequestBuilder) -> Result<FetchedBytes> {
        self.fetch_on(&self.no_redirect, request)
    }

    fn fetch_on(
        &self,
        client: &reqwest::blocking::Client,
        request: RequestBuilder,
    ) -> Result<FetchedBytes> {
        let request = request.build().context("cannot build request")?;
        let url = request.url().to_string();
        let host = request.url().host_str().unwrap_or_default().to_string();

        let policy = Policy::for_request(&host, request.url().path());
        let bucket = Bucket::new(&self.buckets, &host, policy);
        let waited = bucket.acquire()?;
        if !waited.is_zero()
            && let Some(report) = &self.on_wait
        {
            report(&host, waited);
        }

        let response = client.execute(request);
        // Stamped on return rather than on dispatch: a call that spent a minute
        // inside its own retries must not hand the next one a free pass.
        bucket.released()?;

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                // No answer to read. Refused connection, DNS, TLS, timeout:
                // deliberately all of them, because backing off is right
                // whatever the cause and none of them is worth telling apart
                // here. Dropping the connection is how the archive says no
                // when it has decided about you, and it must slow every
                // caller exactly as a 429 would; otherwise a run answers
                // "cannot reach" a few hundred times without ever pausing.
                let held = bucket.unreachable()?;
                return Err(anyhow::Error::new(error)
                    .context(format!(
                        "cannot reach {host}; holding off for {}s",
                        held.as_secs()
                    ))
                    .context(format!("request to {url} failed")));
            }
        };

        if let Some(backoff) = refusal(&response) {
            let held = bucket.refused(backoff)?;
            anyhow::bail!(
                "{host} refused the request; holding off for {}s",
                held.as_secs()
            );
        }

        let response = response
            .error_for_status()
            .with_context(|| format!("{url} returned an error status"))?;

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let content_encoding = response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let body = response
            .bytes()
            .with_context(|| format!("{url} returned an unreadable body"))?;

        // Frozen before anything is decided about what it says, so a body that
        // no reading can decode still has a digest to be cited by.
        let blob = self.blobs.put(&body)?;

        Ok(FetchedBytes {
            content_encoding,
            blob,
            bytes: body.to_vec(),
            url,
            content_type,
        })
    }

    /// Sends a prepared request and decodes what comes back as UTF-8.
    ///
    /// The three lanes over publisher XML all want this. A lane whose bodies
    /// are not guaranteed UTF-8 wants [`Client::fetch_bytes`] and its own
    /// stated decode.
    pub fn fetch(&self, request: RequestBuilder) -> Result<Fetched> {
        let raw = self.fetch_bytes(request)?;
        let text = String::from_utf8(raw.bytes)
            .with_context(|| format!("{} returned a body that is not UTF-8", raw.url))?;

        Ok(Fetched {
            blob: raw.blob,
            text,
            url: raw.url,
        })
    }

    pub fn fetch_url(&self, url: &str) -> Result<Fetched> {
        self.fetch(self.get(url))
    }
}

/// A rate-limit refusal, and how long the host asked us to wait.
///
/// `Retry-After` is read from the header, which is where it lives. A well
/// known MCP implementation parses it out of the response body instead, so it
/// silently never honours it and always falls back to a default.
fn refusal(response: &reqwest::blocking::Response) -> Option<Option<Duration>> {
    if !matches!(response.status().as_u16(), 429 | 503) {
        return None;
    }

    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);

    Some(retry_after)
}

fn build(redirects: reqwest::redirect::Policy) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(redirects)
        .timeout(Duration::from_secs(180))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .context("cannot build HTTP client")
}
