//! One HTTP client for every source.
//!
//! Every response is hashed before it is parsed and the digest travels with the
//! text, so a caller that states a fact can always name the exact bytes it read
//! that fact from. Nothing is kept: a digest beside the address it came from is
//! what makes an answer checkable, and a reader checks one by fetching that
//! address and hashing what arrives.
//!
//! There is no cache here and no store behind it. bron holds nothing between
//! runs but its share of the rate-limit budget.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::blocking::RequestBuilder;

use crate::blob::BlobId;
use crate::error::OwnDoing;
use crate::paths;
use crate::ratelimit::{Bucket, Policy};

/// Identifies bron honestly to the public services it reads.
const USER_AGENT: &str = concat!(
    "bron/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/steddle/bron-cli)"
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

/// Said when the shared budget itself is what bron cannot use. See
/// [`OwnDoing`]: the host on the other end had nothing to do with it.
const BUDGET: OwnDoing = OwnDoing("bron cannot use the rate-limit budget on this machine");

/// What a fetch does with what comes back.
#[derive(Debug, Clone, Copy)]
enum Reading {
    /// A publisher's answer. Hashed, and refused where the server answered an
    /// error status.
    ///
    /// The [`reqwest::Error`] carrying the code is what lets
    /// [`crate::error::from_source`] tell a 404 from a dead network.
    Hashed,
    /// Neither of those, and none of the budget.
    ///
    /// An answer from the bron service is not evidence: no offset points into
    /// it, no ledger row names it, and what it does point at is a blob already
    /// addressed by the hash of itself. Freezing one would put a copy of every
    /// answer ever read in a store that is deliberately never evicted, and a
    /// question put to `bron find` travels in a POST body precisely so it does
    /// not end up written down.
    ///
    /// A failure arrives inside that answer rather than beside it, carrying the
    /// kind to branch on and the detail that says why, so refusing it on a
    /// status code would leave bron reporting that something is missing without
    /// the labels the service named.
    ///
    /// And it takes no share of the budget. That budget is a promise to the
    /// public services bron reads: they rate-limit by address, and several
    /// processes on one machine must not together go three times too fast. The
    /// bron service is not one of them. It states its own limit in a header,
    /// refuses with a 429 this fetch still reports, and paces nobody, so
    /// queueing behind a one-second bucket spends two seconds on a
    /// three-kilobyte article. The hold-off is the worse half: a deadline
    /// written for one unreachable host is shared by every process on the
    /// machine, and this lane has no publisher left to fall back to.
    Envelope,
}

pub struct Client {
    inner: reqwest::blocking::Client,
    /// The same client, except that it reports a redirect rather than taking
    /// it. See [`Client::get_unredirected`].
    no_redirect: reqwest::blocking::Client,
    buckets: PathBuf,
    on_wait: Option<WaitReporter>,
}

impl Client {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: build(reqwest::redirect::Policy::default())?,
            no_redirect: build(reqwest::redirect::Policy::none())?,
            buckets: paths::bucket_dir().context(OwnDoing(
                "bron has nowhere to keep its share of the rate-limit budget",
            ))?,
            on_wait: None,
        })
    }

    /// A client whose rate-limit buckets land somewhere specific. For tests.
    pub fn with_buckets(buckets: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            inner: build(reqwest::redirect::Policy::default())?,
            no_redirect: build(reqwest::redirect::Policy::none())?,
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

    /// Sends a prepared request and hashes what comes back, undecoded.
    ///
    /// The URL is taken from the built request rather than from the caller, so
    /// provenance records the address actually dialled, query string and all.
    pub fn read_bytes(&self, request: RequestBuilder) -> Result<FetchedBytes> {
        self.fetch_on(&self.inner, request, Reading::Hashed)
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
    pub fn read_bytes_unredirected(&self, request: RequestBuilder) -> Result<FetchedBytes> {
        self.fetch_on(&self.no_redirect, request, Reading::Hashed)
    }

    fn fetch_on(
        &self,
        client: &reqwest::blocking::Client,
        request: RequestBuilder,
        reading: Reading,
    ) -> Result<FetchedBytes> {
        let request = request.build().context("cannot build request")?;
        let url = request.url().to_string();
        let host = request.url().host_str().unwrap_or_default().to_string();

        // The shared budget is for the services bron does not run. See
        // [`Reading::Envelope`].
        let bucket = match reading {
            Reading::Hashed => {
                let policy = Policy::for_request(&host, request.url().path());
                let bucket = Bucket::new(&self.buckets, &host, policy);
                let waited = bucket.acquire().context(BUDGET)?;
                if !waited.is_zero()
                    && let Some(report) = &self.on_wait
                {
                    report(&host, waited);
                }
                Some(bucket)
            }
            Reading::Envelope => None,
        };

        let response = client.execute(request);
        // Stamped on return rather than on dispatch: a call that spent a minute
        // inside its own retries must not hand the next one a free pass.
        if let Some(bucket) = &bucket {
            bucket.released().context(BUDGET)?;
        }

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
                let error = anyhow::Error::new(error);
                let error = match &bucket {
                    Some(bucket) => error.context(format!(
                        "cannot reach {host}; holding off for {}s",
                        bucket.unreachable().context(BUDGET)?.as_secs()
                    )),
                    None => error.context(format!("cannot reach {host}")),
                };
                return Err(error.context(format!("request to {url} failed")));
            }
        };

        if let Some(backoff) = refusal(&response) {
            // Reported either way, and the delay the host named is reported
            // either way. Only a host that shares the budget has a deadline
            // written for it; the service refusing is this caller's problem and
            // not every other process's, and this lane has no publisher to fall
            // back to, so the one thing it can be told is how long to wait.
            match &bucket {
                Some(bucket) => anyhow::bail!(
                    "{host} refused the request; holding off for {}s",
                    bucket.refused(backoff).context(BUDGET)?.as_secs()
                ),
                None => match backoff {
                    Some(wait) => anyhow::bail!(
                        "{host} refused the request and asked for {}s",
                        wait.as_secs()
                    ),
                    None => anyhow::bail!("{host} refused the request and named no delay"),
                },
            }
        }

        let response = match reading {
            Reading::Hashed => response
                .error_for_status()
                .with_context(|| format!("{url} returned an error status"))?,
            Reading::Envelope => response,
        };

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

        // Hashed before anything is decided about what it says, so a body that
        // no reading can decode still has a digest to be cited by.
        let blob = BlobId::of(&body);

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
    /// The lanes over publisher XML all want this. A lane whose bodies are not
    /// guaranteed UTF-8 wants [`Client::read_bytes`] and its own stated decode.
    pub fn read(&self, request: RequestBuilder) -> Result<Fetched> {
        decoded(self.read_bytes(request)?)
    }

    pub fn read_url(&self, url: &str) -> Result<Fetched> {
        self.read(self.get(url))
    }

    /// Reads an answer from a service that speaks the bron envelope. See
    /// [`Reading::Envelope`].
    pub fn fetch_answered(&self, request: RequestBuilder) -> Result<Fetched> {
        decoded(self.fetch_on(&self.inner, request, Reading::Envelope)?)
    }
}

fn decoded(raw: FetchedBytes) -> Result<Fetched> {
    let text = String::from_utf8(raw.bytes)
        .with_context(|| format!("{} returned a body that is not UTF-8", raw.url))?;

    Ok(Fetched {
        blob: raw.blob,
        text,
        url: raw.url,
    })
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
