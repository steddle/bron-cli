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
use crate::ratelimit::Bucket;

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
/// on the way out; the digest is taken first either way, so provenance holds
/// even for bytes that no reading will ever turn into text.
pub struct FetchedBytes {
    /// Digest of the exact bytes stored.
    pub blob: BlobId,
    pub bytes: Vec<u8>,
    /// The HTTP status they came under. Only a service that speaks the
    /// envelope answers a body under an error status; a publisher's is refused
    /// before it gets here.
    pub status: u16,
    /// The URL actually dialled. Recorded for provenance, never for identity.
    pub url: String,
}

/// A response, frozen and decoded as UTF-8.
pub struct Fetched {
    /// Digest of the exact bytes stored.
    pub blob: BlobId,
    /// Those bytes, decoded. Byte offsets into this are offsets into the blob.
    pub text: String,
    /// See [`FetchedBytes::status`].
    pub status: u16,
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
    /// it, and what it does point at is a blob already addressed by the hash of
    /// itself. Its digest would name nothing a reader could fetch again, and a
    /// question put to `bron find` travels in a POST body precisely so that
    /// nothing about it is written down.
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
    /// machine, and the service is the only route this lane has.
    Envelope,
}

pub struct Client {
    inner: reqwest::blocking::Client,
    buckets: PathBuf,
    on_wait: Option<WaitReporter>,
}

impl Client {
    pub fn new() -> Result<Self> {
        Self::with_buckets(paths::bucket_dir().context(OwnDoing(
            "bron has nowhere to keep its share of the rate-limit budget",
        ))?)
    }

    /// A client whose rate-limit buckets land somewhere specific. `new` points
    /// it at the machine's shared budget; tests point it at a tempdir.
    ///
    /// The directory is created here, once, so that acquiring a bucket does not
    /// re-check it on every wakeup of its wait loop.
    pub fn with_buckets(buckets: impl Into<PathBuf>) -> Result<Self> {
        let buckets = buckets.into();
        std::fs::create_dir_all(&buckets)
            .with_context(|| format!("cannot create {}", buckets.display()))
            .context(BUDGET)?;
        Ok(Self {
            inner: build()?,
            buckets,
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
        self.fetch_on(request, Reading::Hashed)
    }

    fn fetch_on(&self, request: RequestBuilder, reading: Reading) -> Result<FetchedBytes> {
        let request = request.build().context("cannot build request")?;
        let url = request.url().to_string();
        let host = request.url().host_str().unwrap_or_default().to_string();

        // The shared budget is for the services bron does not run. See
        // [`Reading::Envelope`].
        let bucket = match reading {
            Reading::Hashed => {
                let bucket = Bucket::for_host(&self.buckets, &host);
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

        let response = self.inner.execute(request);
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
                // here. Dropping the connection is how a host says no when it
                // has decided about you, and it must slow every caller
                // exactly as a 429 would; otherwise a run answers "cannot
                // reach" a few hundred times without ever pausing.
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
            // not every other process's, and the service is the only route its
            // lane has, so the one thing it can be told is how long to wait.
            match &bucket {
                Some(bucket) => anyhow::bail!(
                    "{host} refused the request; holding off for {}s",
                    bucket
                        .refused(backoff.asked_for())
                        .context(BUDGET)?
                        .as_secs()
                ),
                None => {
                    let sentence = match backoff {
                        Refusal::After(wait) => {
                            format!(
                                "{host} refused the request and asked for {}s",
                                wait.as_secs()
                            )
                        }
                        Refusal::Unstated => {
                            format!("{host} refused the request and named no delay")
                        }
                    };
                    // The body may be the envelope stating which refusal this
                    // is. A body that cannot be read leaves the sentence alone.
                    let status = response.status().as_u16();
                    let body = response.text().unwrap_or_default();
                    return Err(Refused {
                        sentence,
                        status,
                        host,
                        asked_for: backoff.asked_for(),
                        body,
                    }
                    .into());
                }
            }
        }

        let status = response.status().as_u16();

        let response = match reading {
            Reading::Hashed => response
                .error_for_status()
                .with_context(|| format!("{url} returned an error status"))?,
            Reading::Envelope => response,
        };

        let body = response
            .bytes()
            .with_context(|| format!("{url} returned an unreadable body"))?;

        // Hashed before anything is decided about what it says, so a body that
        // no reading can decode still has a digest to be cited by.
        let blob = BlobId::of(&body);

        Ok(FetchedBytes {
            blob,
            bytes: body.to_vec(),
            status,
            url,
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
        decoded(self.fetch_on(request, Reading::Envelope)?)
    }

    /// Reads the bytes a service that speaks the envelope serves on success,
    /// undecoded: a document it froze, which may be no text at all. A failure
    /// still comes in the envelope, under its status.
    pub fn fetch_raw(&self, request: RequestBuilder) -> Result<FetchedBytes> {
        self.fetch_on(request, Reading::Envelope)
    }
}

fn decoded(raw: FetchedBytes) -> Result<Fetched> {
    let text = String::from_utf8(raw.bytes)
        .with_context(|| format!("{} returned a body that is not UTF-8", raw.url))?;

    Ok(Fetched {
        blob: raw.blob,
        text,
        status: raw.status,
        url: raw.url,
    })
}

/// A 429 or a 503 from a host that speaks the bron envelope, and the body it
/// came with. The status and the delay are the refusal; the body, where it
/// is the envelope, says which one it was.
#[derive(Debug, thiserror::Error)]
#[error("{sentence}")]
pub struct Refused {
    /// The refusal in words, with the delay the host named.
    pub sentence: String,
    pub status: u16,
    pub host: String,
    /// The delay the host named, where it named one.
    pub asked_for: Option<Duration>,
    pub body: String,
}

/// How a host qualified its refusal: it either asked for a delay or named
/// none. A fact with more than two answers gets a type, so that "no refusal"
/// and "a refusal with no delay" cannot be confused at a call site.
#[derive(Clone, Copy)]
enum Refusal {
    After(Duration),
    Unstated,
}

impl Refusal {
    /// The delay the host asked for, in the shape [`Bucket::refused`] takes.
    fn asked_for(self) -> Option<Duration> {
        match self {
            Self::After(wait) => Some(wait),
            Self::Unstated => None,
        }
    }
}

/// A rate-limit refusal, and how long the host asked us to wait.
///
/// `Retry-After` is read from the header, which is where it lives. A well
/// known MCP implementation parses it out of the response body instead, so it
/// silently never honours it and always falls back to a default.
fn refusal(response: &reqwest::blocking::Response) -> Option<Refusal> {
    if !matches!(response.status().as_u16(), 429 | 503) {
        return None;
    }

    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| asked_for(v.trim()));

    Some(retry_after.map_or(Refusal::Unstated, Refusal::After))
}

/// The delay a `Retry-After` names, written either way RFC 9110 allows: a
/// count of seconds, or the date the wait ends. A date already past is no
/// wait at all, and a header neither shape names nothing.
fn asked_for(header: &str) -> Option<Duration> {
    if let Ok(seconds) = header.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let until = chrono::DateTime::parse_from_rfc2822(header).ok()?;
    let ahead = until.signed_duration_since(chrono::Utc::now());

    Some(ahead.to_std().unwrap_or(Duration::ZERO))
}

fn build() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(180))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .context("cannot build HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both shapes RFC 9110 allows, and neither read as silence: a host that
    /// named a wait is told back as having named one.
    #[test]
    fn a_retry_after_is_read_as_seconds_or_as_the_date_the_wait_ends() {
        assert_eq!(asked_for("60"), Some(Duration::from_secs(60)));

        let at = chrono::Utc::now() + chrono::Duration::seconds(40);
        let named = asked_for(&at.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
            .expect("a date names a delay");
        assert!(
            (30..=45).contains(&named.as_secs()),
            "40s, got {}s",
            named.as_secs()
        );
    }

    /// A date already past asks for no wait, which is not the same as asking
    /// for one that cannot be measured.
    #[test]
    fn a_date_already_past_asks_for_no_wait() {
        let gone = chrono::Utc::now() - chrono::Duration::hours(1);
        assert_eq!(
            asked_for(&gone.format("%a, %d %b %Y %H:%M:%S GMT").to_string()),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn a_header_of_neither_shape_names_nothing() {
        assert_eq!(asked_for("soon"), None);
        assert_eq!(asked_for(""), None);
    }
}
