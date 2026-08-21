//! Reading one answer, whatever it turns out to be.
//!
//! Every call to the service goes through [`Service::read`], so there is one
//! place that knows what a bron envelope is and one place that turns a failure
//! into a [`Fault`]. A second path around it is how `bron find` came to report
//! that nothing matched a question the service had never been asked.

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::error::{Fault, Outcome, from_source};
use crate::sources::Client;
use crate::wire;

use super::Service;

/// What a fault from this address is attributed to.
pub(super) const SERVICE: &str = "bron";

/// An answer and the limits the service stated on it.
pub struct Answered<T> {
    pub data: T,
    pub caveats: Vec<wire::Caveat>,
}

/// Every answer the service gives, success or failure.
///
/// A failure is not a missing `data`: it is an `error` in the same envelope,
/// which is why it is read rather than inferred from a status code.
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    #[serde(default = "none")]
    data: Option<T>,
    #[serde(default = "none")]
    error: Option<Stated>,
    #[serde(default)]
    caveats: Vec<serde_json::Value>,
}

fn none<T>() -> Option<T> {
    None
}

/// A failure in the service's own terms.
#[derive(Debug, Deserialize)]
struct Stated {
    kind: String,
    message: String,
    #[serde(default)]
    detail: Vec<String>,
}

impl Stated {
    /// The same failure, in the kinds the CLI exits on.
    ///
    /// Both halves publish one vocabulary, so a failure is carried across
    /// rather than reclassified: a malformed `--limit` is a usage failure at
    /// both ends, and re-reading it here as 'the service answered something
    /// unreadable' would have a caller retry what will never work.
    ///
    /// `detail` travels with the message rather than being dropped. For the
    /// answer this whole lane exists for it holds the labels of the withdrawn
    /// renditions covering a date, and a not-found without them says a date is
    /// uncovered when the truth is that what covers it has been withdrawn.
    fn into_fault(self) -> Fault {
        let message = if self.detail.is_empty() {
            self.message
        } else {
            format!("{}: {}", self.message, self.detail.join(", "))
        };

        match self.kind.as_str() {
            "usage" => Fault::Usage(message),
            "not_found" => Fault::NotFound(message),
            // Throttled and corpus are both the service failing to answer a
            // question it understood, which is what upstream means from here.
            _ => Fault::Upstream {
                service: SERVICE,
                source: anyhow::anyhow!("{message}"),
            },
        }
    }
}

impl Service {
    /// Reads one answer, and reports a failure in the service's own terms.
    pub(super) fn read<T: DeserializeOwned>(
        &self,
        client: &Client,
        request: reqwest::blocking::RequestBuilder,
    ) -> Outcome<Answered<T>> {
        let fetched = client
            .fetch_answered(request)
            .map_err(|e| from_source(SERVICE, e))?;

        let unreadable = |e: serde_json::Error| Fault::Upstream {
            service: SERVICE,
            source: anyhow::Error::new(e)
                .context(format!("{} did not answer the bron envelope", fetched.url)),
        };

        let envelope: Envelope<T> = serde_json::from_str(&fetched.text).map_err(unreadable)?;

        if let Some(stated) = envelope.error {
            return Err(stated.into_fault());
        }

        let data = envelope.data.ok_or_else(|| Fault::Upstream {
            service: SERVICE,
            source: anyhow::anyhow!("{} answered neither data nor an error", fetched.url),
        })?;

        Ok(Answered {
            data,
            caveats: envelope
                .caveats
                .into_iter()
                .map(wire::Caveat::stated)
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure keeps the kind the service gave it. Re-reading a usage failure
    /// as an unreadable answer would have a caller retry what will never work.
    #[test]
    fn a_stated_failure_keeps_its_kind() {
        let stated = |kind: &str| Stated {
            kind: kind.into(),
            message: "something".into(),
            detail: Vec::new(),
        };

        assert_eq!(stated("usage").into_fault().kind(), "usage");
        assert_eq!(stated("not_found").into_fault().kind(), "not_found");
        assert_eq!(stated("throttled").into_fault().kind(), "upstream");
        assert_eq!(stated("corpus").into_fault().kind(), "upstream");
    }

    /// The labels of the withdrawn renditions covering a date are the whole
    /// point of that failure. Reporting the date as uncovered without them says
    /// the register never had it, which is a different fact.
    #[test]
    fn the_detail_of_a_failure_travels_with_its_message() {
        let stated = Stated {
            kind: "not_found".into(),
            message: "no published rendition of BWBR0037132 in force on [2016-01-01]".into(),
            detail: vec!["2016-01-01_0".into(), "2016-06-16_0".into()],
        };

        let message = stated.into_fault().to_string();
        assert!(message.contains("2016-01-01_0"), "{message}");
        assert!(message.contains("2016-06-16_0"), "{message}");
    }
}
