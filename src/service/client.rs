//! Reading one answer, whatever it turns out to be.
//!
//! Every call to the service goes through [`Service::read`], so there is one
//! place that knows what a bron envelope is and one place that turns a failure
//! into a [`Fault`]. A second path around it is how `bron find` came to report
//! that nothing matched a question the service had never been asked.

use chrono::NaiveDate;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::error::{Fault, Outcome, from_source};
use crate::sources::{Client, Refused};
use crate::wire;

use super::Service;

/// What a fault from this address is attributed to.
pub(super) const SERVICE: &str = "bron";

/// An answer, the limits the service stated on it, and the envelope it came in.
pub struct Answered<T> {
    pub data: T,
    pub caveats: Vec<wire::Caveat>,
    /// The moment the service bound the answer to, where it bound one. Read as
    /// dates, so an envelope stating anything else there is unreadable.
    pub as_of: Option<NaiveDate>,
    pub as_known_on: Option<NaiveDate>,
    /// The envelope exactly as the service sent it. The service's JSON is the
    /// contract, so `--json` prints this and nothing read out of it: a field
    /// this build has no name for still reaches the consumer.
    pub envelope: String,
}

/// Every answer the service gives, success or failure.
///
/// A failure is not a missing `data`: it is an `error` in the same envelope,
/// which is why it is read rather than inferred from a status code.
///
/// Deliberately permissive, and no serde tag makes the pair an enum. `caveats`
/// sits beside both fields, so a tagged enum fights the third field, and an
/// untagged one answers a real deserialisation failure with "matched no
/// variant", stripped of the field and position — on exactly the failure this
/// module exists to diagnose. The wire shape stays loose;
/// [`Answer::settle`] collapses it exactly once.
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: Option<T>,
    error: Option<Stated>,
    as_of: Option<NaiveDate>,
    as_known_on: Option<NaiveDate>,
    #[serde(default)]
    caveats: Vec<serde_json::Value>,
}

/// What an envelope legally says: an answer, or a failure in the service's
/// own terms. Two states, where the wire shape can express four.
enum Answer<T> {
    Data(Answered<T>),
    Failed(Stated),
}

/// The one place that decides what an envelope's fields mean together, and
/// the only way out of [`Envelope`], so a second call site cannot resolve the
/// pair differently. An error wins over any data beside it, and an envelope
/// stating neither is the service breaking its own contract, which is an
/// upstream fault.
impl<T> Answer<T> {
    /// `sent` is the text `envelope` was read out of.
    fn settle(envelope: Envelope<T>, sent: String) -> Result<Self, Fault> {
        if let Some(stated) = envelope.error {
            return Ok(Self::Failed(stated));
        }

        let Some(data) = envelope.data else {
            return Err(Fault::Upstream {
                service: SERVICE,
                source: anyhow::anyhow!("the service answered neither data nor an error"),
            });
        };

        Ok(Self::Data(Answered {
            data,
            caveats: envelope.caveats.iter().map(wire::Caveat::stated).collect(),
            as_of: envelope.as_of,
            as_known_on: envelope.as_known_on,
            envelope: sent,
        }))
    }
}

/// A failure in the service's own terms.
#[derive(Debug, Deserialize)]
struct Stated {
    kind: String,
    /// Which failure this is, finer than `kind`. Read as a string and never
    /// as an enum, so a code the service adds later crosses untouched.
    code: Option<String>,
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

        let fault = match self.kind.as_str() {
            "usage" => Fault::Usage(message),
            "not_found" => Fault::NotFound(message),
            // Throttled, corpus and source are the service failing to answer
            // a question it understood, which is what upstream means from here.
            _ => Fault::Unanswered(message),
        };

        match self.code {
            Some(code) => fault.stated_as(code),
            None => fault,
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
            .map_err(|e| refused(e).unwrap_or_else(|e| from_source(SERVICE, e)))?;

        // An unknown variant is this build being behind rather than the
        // service being broken, and the two want different things done about
        // them. Serde says which by naming the variant it did not know, the
        // same distinction `Caveat::Unrecognised` already draws for a limit
        // this bron has no name for.
        let unreadable = |e: serde_json::Error| {
            let known = e.to_string();
            let context = if known.starts_with("unknown variant") {
                format!(
                    "{} answered a shape this bron has no name for. Upgrade to read what it says.",
                    fetched.url,
                )
            } else {
                format!("{} did not answer the bron envelope", fetched.url)
            };

            Fault::Upstream {
                service: SERVICE,
                source: anyhow::Error::new(e).context(context),
            }
        };

        let envelope: Envelope<T> = serde_json::from_str(&fetched.text).map_err(unreadable)?;

        match Answer::settle(envelope, fetched.text)? {
            Answer::Data(answered) => Ok(answered),
            Answer::Failed(stated) => Err(stated.into_fault()),
        }
    }
}

/// A 429 or a 503 whose body states the failure, as that failure with the
/// delay the service named beside its words. Anything else is handed back.
fn refused(error: anyhow::Error) -> Result<Fault, anyhow::Error> {
    let Some(refusal) = error.downcast_ref::<Refused>() else {
        return Err(error);
    };
    let Ok(Envelope::<serde::de::IgnoredAny> {
        error: Some(mut stated),
        ..
    }) = serde_json::from_str(&refusal.body)
    else {
        return Err(error);
    };

    stated.message = format!("{} ({})", stated.message, refusal.sentence);
    Ok(stated.into_fault())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure beside leftover data is still a failure. Reading the data
    /// would report success on an answer the service disowned.
    #[test]
    fn an_error_wins_over_data_beside_it() {
        let envelope: Envelope<u32> = serde_json::from_str(
            r#"{"data":7,"error":{"kind":"usage","message":"limit must be positive"}}"#,
        )
        .unwrap();

        match Answer::settle(envelope, String::new()) {
            Ok(Answer::Failed(stated)) => assert_eq!(stated.kind, "usage"),
            Ok(Answer::Data(answered)) => panic!("read {} as success", answered.data),
            Err(fault) => panic!("read a stated failure as {fault}"),
        }
    }

    /// An envelope stating neither is the service breaking its own contract,
    /// which is upstream's doing and not the caller's.
    #[test]
    fn an_envelope_stating_neither_is_an_upstream_fault() {
        let envelope: Envelope<u32> = serde_json::from_str(r#"{"bron":1}"#).unwrap();

        match Answer::settle(envelope, String::new()) {
            Err(fault) => assert_eq!(fault.kind(), "upstream"),
            Ok(Answer::Data(answered)) => panic!("read {} out of thin air", answered.data),
            Ok(Answer::Failed(stated)) => panic!("invented a failure: {}", stated.message),
        }
    }

    /// A failure keeps the kind the service gave it. Re-reading a usage failure
    /// as an unreadable answer would have a caller retry what will never work.
    #[test]
    fn a_stated_failure_keeps_its_kind() {
        let stated = |kind: &str| Stated {
            kind: kind.into(),
            code: None,
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
            code: Some("rendition_withdrawn".into()),
            message: "no published rendition of BWBR0037132 in force on [2016-01-01]".into(),
            detail: vec!["2016-01-01_0".into(), "2016-06-16_0".into()],
        };

        let message = stated.into_fault().to_string();
        assert!(message.contains("2016-01-01_0"), "{message}");
        assert!(message.contains("2016-06-16_0"), "{message}");
    }

    /// The code is the service's word and crosses as sent. A code this build
    /// has never heard of crosses too: the kind beside it already says what to
    /// do, so nothing here has to know what the code means.
    #[test]
    fn the_code_of_a_stated_failure_crosses_as_sent() {
        let stated = |kind: &str, code: Option<&str>| {
            let body = match code {
                Some(code) => format!(r#"{{"kind":"{kind}","code":"{code}","message":"m"}}"#),
                None => format!(r#"{{"kind":"{kind}","message":"m"}}"#),
            };
            serde_json::from_str::<Stated>(&body).unwrap().into_fault()
        };

        let fault = stated("not_found", Some("regulation_not_found"));
        assert_eq!(fault.stated_code(), Some("regulation_not_found"));
        assert_eq!(fault.kind(), "not_found");
        assert_eq!(fault.exit_byte(), 2);

        let fault = stated("usage", Some("unknown_parameter"));
        assert_eq!(fault.stated_code(), Some("unknown_parameter"));
        assert_eq!(fault.exit_byte(), 1);

        let fault = stated("source", Some("a_code_this_bron_predates"));
        assert_eq!(fault.stated_code(), Some("a_code_this_bron_predates"));
        assert_eq!(fault.kind(), "upstream");
        assert_eq!(fault.exit_byte(), 4);

        // A deployment that states no code is read as it states itself.
        assert_eq!(stated("not_found", None).stated_code(), None);

        let null: Stated =
            serde_json::from_str(r#"{"kind":"not_found","code":null,"message":"m"}"#).unwrap();
        assert_eq!(null.into_fault().stated_code(), None);
    }

    /// A code is a string on the wire, like the kind beside it. One that is
    /// anything else fails the envelope it arrived in.
    #[test]
    fn a_code_that_is_not_a_string_is_not_the_bron_envelope() {
        let read = serde_json::from_str::<Envelope<u32>>(
            r#"{"bron":1,"error":{"kind":"not_found","code":7,"message":"m"}}"#,
        );

        assert!(read.is_err());
    }

    /// The code rides beside the failure and changes nothing a person reads:
    /// the same lines reach stderr and `detail` with it as without.
    #[test]
    fn a_code_leaves_the_cause_chain_as_it_was() {
        let stated = |code: Option<&str>| Stated {
            kind: "source".into(),
            code: code.map(str::to_string),
            message: "the register did not serve 2016-07-01_0".into(),
            detail: Vec::new(),
        };

        assert_eq!(
            stated(Some("source_unavailable")).into_fault().chain(),
            stated(None).into_fault().chain()
        );
    }
}
