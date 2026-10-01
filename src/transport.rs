//! The one HTTP client, and every call to the bron service goes through it.
//!
//! A failure arrives inside the service's answer rather than beside it,
//! carrying the kind to branch on and the detail that says why, so a body
//! is read whatever its status: refusing it on a status code would leave bron
//! reporting that something is missing without the labels the service named.
//!
//! There is no cache here and no store behind it. bron writes nothing.

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::blocking::RequestBuilder;

use crate::blob::BlobId;

/// Identifies bron honestly to the service it reads.
const USER_AGENT: &str = concat!(
    "bron/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/steddle/bron-cli)"
);

/// A response, frozen, before anything has been decided about its encoding.
///
/// The digest is taken first, so a document the service froze is named by the
/// bytes that arrived, which is what `bron blob` checks against the digest it
/// asked for.
pub struct FetchedBytes {
    /// Digest of the exact bytes received.
    pub blob: BlobId,
    pub bytes: Vec<u8>,
    /// The HTTP status they came under. The service answers its envelope
    /// under an error status too.
    pub status: u16,
    /// The URL actually dialled.
    pub url: String,
}

/// A response, frozen and decoded as UTF-8.
pub struct Fetched {
    /// Digest of the exact bytes received.
    pub blob: BlobId,
    pub text: String,
    /// See [`FetchedBytes::status`].
    pub status: u16,
    /// The URL actually dialled.
    pub url: String,
}

pub struct Client {
    inner: reqwest::blocking::Client,
}

impl Client {
    pub fn new() -> Result<Self> {
        Ok(Self { inner: build()? })
    }

    pub fn get(&self, url: &str) -> RequestBuilder {
        self.inner.get(url)
    }

    pub fn post(&self, url: &str) -> RequestBuilder {
        self.inner.post(url)
    }

    /// Reads an answer from the service, decoded as UTF-8.
    pub fn fetch_answered(&self, request: RequestBuilder) -> Result<Fetched> {
        decoded(self.fetch(request)?)
    }

    /// Reads the bytes the service serves on success, undecoded: a document it
    /// froze, which may be no text at all. A failure still comes in the
    /// envelope, under its status.
    pub fn fetch_raw(&self, request: RequestBuilder) -> Result<FetchedBytes> {
        self.fetch(request)
    }

    /// The URL is taken from the built request rather than from the caller, so
    /// what is reported is the address actually dialled, query string and all.
    fn fetch(&self, request: RequestBuilder) -> Result<FetchedBytes> {
        let request = request.build().context("cannot build request")?;
        let url = request.url().to_string();
        let host = request.url().host_str().unwrap_or_default().to_string();

        let response = self
            .inner
            .execute(request)
            .context(format!("cannot reach {host}"))
            .with_context(|| format!("request to {url} failed"))?;

        if let Some(backoff) = refusal(&response) {
            let sentence = match backoff {
                Refusal::After(wait) => {
                    format!(
                        "{host} refused the request and asked for {}s",
                        wait.as_secs()
                    )
                }
                Refusal::Unstated => format!("{host} refused the request and named no delay"),
            };
            // The body may be the envelope stating which refusal this is. A
            // body that cannot be read leaves the sentence alone.
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

        let status = response.status().as_u16();
        let body = response
            .bytes()
            .with_context(|| format!("{url} returned an unreadable body"))?;

        Ok(FetchedBytes {
            blob: BlobId::of(&body),
            bytes: body.to_vec(),
            status,
            url,
        })
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

/// A 429 or a 503 from the service, and the body it came with. The status and
/// the delay are the refusal; the body, where it is the envelope, says which
/// one it was.
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
