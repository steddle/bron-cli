//! Reading one answer, whatever it turns out to be.
//!
//! Every call to the service goes through `Service::read`, so there is one
//! place that knows what a Bron envelope is and one place that turns a failure
//! into a [`Fault`]. A second path around it could report that nothing
//! matched a question the service was never asked.

use std::fmt::Write as _;

use chrono::NaiveDate;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;

use crate::error::{Fault, Outcome, Statement, from_source};
use crate::transport::{Client, Refused};
use crate::wire;

use super::Service;

/// What a fault from this address is attributed to.
pub(super) const SERVICE: &str = "Bron";

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
/// variant", stripped of the field and position, on the failure this module
/// exists to diagnose. The wire shape stays loose; [`Answer::settle`]
/// collapses it once.
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
/// own terms and the text it arrived in. Two states, where the wire shape can
/// express four.
enum Answer<T> {
    Data(Answered<T>),
    Failed(Stated, String),
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
            return Ok(Self::Failed(stated, sent));
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
    /// The seconds the service asks a caller to wait, which it also sends as
    /// its `Retry-After`.
    retry_after: Option<u64>,
}

/// A failure's error object as the service sent it.
#[derive(Deserialize)]
struct SentError {
    error: Box<RawValue>,
}

impl Stated {
    /// The same failure, in the kinds the CLI exits on.
    ///
    /// Both halves publish one vocabulary, so a failure is carried across
    /// rather than reclassified: a malformed `--limit` is a usage failure at
    /// both ends, and re-reading it here as 'the service answered something
    /// unreadable' would have a caller retry what will never work.
    ///
    /// `body` is the text the failure arrived in, whose error object `--json`
    /// prints as sent, so it answers what the API and the MCP answer: the
    /// kind, the code, the message and the detail, and a field added later.
    /// For the answer this whole lane exists for `detail` holds the labels of
    /// the withdrawn renditions covering a date, and a not-found without them
    /// says a date is uncovered when the truth is that what covers it has been
    /// withdrawn.
    ///
    /// `refusal` is the 429 or 503 the failure came under. The delay is read
    /// from the body where it states `retry_after`, and from the header
    /// otherwise, where it is written into the error object so `--json`
    /// carries it once.
    fn into_fault(self, body: &str, refusal: Option<&Refused>) -> Fault {
        let Ok(SentError { error: mut sent }) = serde_json::from_str(body) else {
            return Fault::Upstream {
                service: SERVICE,
                source: anyhow::anyhow!("the service's failure has no error object to read"),
            };
        };

        let header = refusal
            .and_then(|refusal| refusal.asked_for)
            .map(|wait| wait.as_secs());
        // Whether the object names the key at all, since one that states
        // `"retry_after": null` has stated it too, and a second key would
        // make the object two answers.
        let states_a_delay =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(sent.get())
                .is_ok_and(|object| object.contains_key("retry_after"));
        if !states_a_delay
            && let Some(seconds) = header
            && let Some(with) = with_retry_after(&sent, seconds)
        {
            sent = with;
        }

        let sentence = match self.retry_after.or(header) {
            Some(seconds) => format!("{} (the service asked for {seconds}s)", self.message),
            None => self.message.clone(),
        };

        let fault = match self.kind.as_str() {
            "usage" => Fault::Usage(sentence),
            "not_found" => Fault::NotFound(sentence),
            // Throttled, corpus and source are the service failing to answer
            // a question it understood, which is what upstream means from here.
            _ => Fault::Unanswered(sentence),
        };

        fault.stated(Statement {
            kind: self.kind,
            code: self.code,
            message: self.message,
            detail: self.detail,
            sent,
        })
    }
}

/// An error object with `retry_after` added as its last field, the rest of it
/// as sent. Written into the text rather than through a map, which would put
/// the service's fields in another order.
fn with_retry_after(sent: &RawValue, seconds: u64) -> Option<Box<RawValue>> {
    let open = sent.get().trim_end().strip_suffix('}')?.trim_end();
    let comma = if open.ends_with('{') { "" } else { "," };

    RawValue::from_string(format!("{open}{comma}\"retry_after\":{seconds}}}")).ok()
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
        // this Bron has no name for.
        let unreadable = |e: serde_json::Error| {
            let known = e.to_string();
            let context = if known.starts_with("unknown variant") {
                format!(
                    "{} answered a shape this Bron has no name for. Upgrade to read what it says.",
                    fetched.url,
                )
            } else {
                format!("{} did not answer the Bron envelope", fetched.url)
            };

            Fault::Upstream {
                service: SERVICE,
                source: anyhow::Error::new(e).context(context),
            }
        };

        let envelope: Envelope<T> = match serde_json::from_str(&fetched.text) {
            Ok(envelope) => envelope,
            Err(e) => {
                let host = reqwest::Url::parse(&fetched.url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_string))
                    .unwrap_or_default();

                return Err(match gateway(&host, fetched.status, None) {
                    Some(sentence) => Fault::NoAnswer {
                        sentence,
                        retry_after: None,
                    },
                    None => unreadable(e),
                });
            }
        };

        match Answer::settle(envelope, fetched.text)? {
            Answer::Data(answered) => Ok(answered),
            Answer::Failed(stated, body) => Err(stated.into_fault(&body, None)),
        }
    }
}

impl Service {
    /// Reads the bytes a route serves on success, where the success is no
    /// envelope: the one such route is a frozen document. A failure is read
    /// as on every other route, from the envelope the service states it in.
    pub(super) fn read_raw(
        &self,
        client: &Client,
        request: reqwest::blocking::RequestBuilder,
    ) -> Outcome<crate::transport::FetchedBytes> {
        let fetched = client
            .fetch_raw(request)
            .map_err(|e| refused(e).unwrap_or_else(|e| from_source(SERVICE, e)))?;

        if (200..300).contains(&fetched.status) {
            return Ok(fetched);
        }

        let body = String::from_utf8_lossy(&fetched.bytes);
        match serde_json::from_str::<Envelope<serde::de::IgnoredAny>>(&body) {
            Ok(Envelope {
                error: Some(stated),
                ..
            }) => Err(stated.into_fault(&body, None)),
            _ => {
                let host = reqwest::Url::parse(&fetched.url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_string))
                    .unwrap_or_default();

                Err(match gateway(&host, fetched.status, None) {
                    Some(sentence) => Fault::NoAnswer {
                        sentence,
                        retry_after: None,
                    },
                    None => Fault::Upstream {
                        service: SERVICE,
                        source: anyhow::anyhow!(
                            "{} answered {} and no Bron envelope with it",
                            fetched.url,
                            fetched.status
                        ),
                    },
                })
            }
        }
    }
}

/// A 429 or a 503, as the failure its body states with the delay the service
/// named beside its words. Where the body is no envelope, as the refusal or
/// the gateway failure it is. Anything else is handed back.
fn refused(error: anyhow::Error) -> Result<Fault, anyhow::Error> {
    let Some(refusal) = error.downcast_ref::<Refused>() else {
        return Err(error);
    };

    match serde_json::from_str(&refusal.body) {
        Ok(Envelope::<serde::de::IgnoredAny> {
            error: Some(stated),
            ..
        }) => Ok(stated.into_fault(&refusal.body, Some(refusal))),
        _ => Ok(Fault::NoAnswer {
            sentence: gateway(&refusal.host, refusal.status, refusal.asked_for)
                .unwrap_or_else(|| refusal.sentence.clone()),
            retry_after: refusal.asked_for.map(|wait| wait.as_secs()),
        }),
    }
}

/// What a 502, 503 or 504 without the envelope is, in words, with its status.
///
/// None of the three is an answer. The service states its own failures in the
/// envelope, so one of these with any other body came from the edge in front
/// of it or from the service before it could answer, and what a caller does
/// about each is not what it does about an unreadable answer.
fn gateway(host: &str, status: u16, asked_for: Option<std::time::Duration>) -> Option<String> {
    let mut sentence = match status {
        504 => format!(
            "{host} answered 504 Gateway Timeout: the service, or something in front of it, \
             did not answer in time. Nothing is wrong with the question; ask again later, or \
             narrow it"
        ),
        503 => format!(
            "{host} answered 503 Service Unavailable: the service, or something in front of \
             it, did not answer"
        ),
        502 => format!(
            "{host} answered 502 Bad Gateway: something in front of the service got no answer \
             from it that it could pass on"
        ),
        _ => return None,
    };

    if let Some(wait) = asked_for {
        let _ = write!(sentence, ", and asked for {}s", wait.as_secs());
    }
    sentence.push('.');

    Some(sentence)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure as the service states it in `error`, read the way `read`
    /// reads one, under a 429 or a 503 where `refusal` names it.
    fn stated(error: &str, refusal: Option<&Refused>) -> Fault {
        let body = format!(r#"{{"bron":1,"grade":"authority","error":{error}}}"#);
        let envelope: Envelope<serde::de::IgnoredAny> = serde_json::from_str(&body).unwrap();

        envelope.error.unwrap().into_fault(&body, refusal)
    }

    fn refusal(seconds: Option<u64>) -> Refused {
        Refused {
            sentence: "bron.test refused the request".into(),
            status: 503,
            host: "bron.test".into(),
            asked_for: seconds.map(std::time::Duration::from_secs),
            body: String::new(),
        }
    }

    /// A failure beside leftover data is still a failure. Reading the data
    /// would report success on an answer the service disowned.
    #[test]
    fn an_error_wins_over_data_beside_it() {
        let envelope: Envelope<u32> = serde_json::from_str(
            r#"{"data":7,"error":{"kind":"usage","message":"limit must be positive"}}"#,
        )
        .unwrap();

        match Answer::settle(envelope, String::new()) {
            Ok(Answer::Failed(stated, _)) => assert_eq!(stated.kind, "usage"),
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
            Ok(Answer::Failed(stated, _)) => panic!("invented a failure: {}", stated.message),
        }
    }

    /// A failure exits on the kind the service gave it, and carries that kind
    /// as sent. Re-reading a usage failure as an unreadable answer would have
    /// a caller retry what will never work, and calling `throttled` upstream
    /// in `--json` would answer otherwise than the API does.
    #[test]
    fn a_stated_failure_exits_on_its_kind_and_carries_it_as_sent() {
        for (kind, exits_as) in [
            ("usage", "usage"),
            ("not_found", "not_found"),
            ("throttled", "upstream"),
            ("corpus", "upstream"),
            ("source", "upstream"),
        ] {
            let fault = stated(&format!(r#"{{"kind":"{kind}","message":"m"}}"#), None);

            assert_eq!(fault.kind(), exits_as);
            assert_eq!(fault.statement().unwrap().kind, kind);
        }
    }

    /// The labels of the withdrawn renditions covering a date are the whole
    /// point of that failure. Reporting the date as uncovered without them says
    /// the register never had it, which is a different fact. The error object
    /// travels as the service sent it, byte for byte, a field this build has
    /// no name for included.
    #[test]
    fn the_error_object_travels_as_sent() {
        let error = r#"{"kind":"not_found","code":"rendition_withdrawn","message":"no published rendition of BWBR0037132 in force on [2016-01-01]","detail":["2016-01-01_0","2016-06-16_0"],"later":{"x":1}}"#;
        let fault = stated(error, None);
        let statement = fault.statement().expect("the service stated it");

        assert_eq!(statement.sent.get(), error);
        assert_eq!(statement.detail, ["2016-01-01_0", "2016-06-16_0"]);
        assert_eq!(
            statement.message,
            "no published rendition of BWBR0037132 in force on [2016-01-01]"
        );
    }

    /// The code is the service's word and crosses as sent. A code this build
    /// has never heard of crosses too: the kind beside it already says what to
    /// do, so nothing here has to know what the code means.
    #[test]
    fn the_code_of_a_stated_failure_crosses_as_sent() {
        let with = |kind: &str, code: Option<&str>| match code {
            Some(code) => stated(
                &format!(r#"{{"kind":"{kind}","code":"{code}","message":"m"}}"#),
                None,
            ),
            None => stated(&format!(r#"{{"kind":"{kind}","message":"m"}}"#), None),
        };

        let fault = with("not_found", Some("regulation_not_found"));
        assert_eq!(fault.stated_code(), Some("regulation_not_found"));
        assert_eq!(fault.exit_byte(), 2);

        let fault = with("usage", Some("unknown_parameter"));
        assert_eq!(fault.stated_code(), Some("unknown_parameter"));
        assert_eq!(fault.exit_byte(), 1);

        let fault = with("source", Some("a_code_this_bron_predates"));
        assert_eq!(fault.stated_code(), Some("a_code_this_bron_predates"));
        assert_eq!(fault.exit_byte(), 4);

        // A deployment that states no code is read as it states itself.
        assert_eq!(with("not_found", None).stated_code(), None);
        assert_eq!(
            stated(r#"{"kind":"not_found","code":null,"message":"m"}"#, None).stated_code(),
            None
        );
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
    /// the same lines reach stderr with it as without.
    #[test]
    fn a_code_leaves_the_cause_chain_as_it_was() {
        let message = "the register did not serve 2016-07-01_0";

        assert_eq!(
            stated(
                &format!(
                    r#"{{"kind":"source","code":"source_unavailable","message":"{message}"}}"#
                ),
                None
            )
            .chain(),
            stated(
                &format!(r#"{{"kind":"source","message":"{message}"}}"#),
                None
            )
            .chain()
        );
    }

    /// The delay is the body's where it states one, and the error object then
    /// crosses untouched, whatever the header said: never written twice.
    #[test]
    fn a_delay_the_body_states_is_the_one_that_crosses() {
        let error = r#"{"kind":"source","code":"source_unavailable","message":"m","detail":[],"retry_after":60}"#;
        let fault = stated(error, Some(&refusal(Some(30))));

        assert_eq!(fault.statement().unwrap().sent.get(), error);
        assert!(fault.to_string().contains("60s"), "{fault}");
    }

    /// Where the body states no delay and the header named one, the delay is
    /// written into the error object once, after the service's own fields.
    #[test]
    fn a_delay_only_the_header_named_is_written_into_the_error_once() {
        let fault = stated(
            r#"{"kind":"throttled","code":"rate_limited","message":"m","detail":[]}"#,
            Some(&refusal(Some(30))),
        );

        assert_eq!(
            fault.statement().unwrap().sent.get(),
            r#"{"kind":"throttled","code":"rate_limited","message":"m","detail":[],"retry_after":30}"#
        );
        assert!(fault.to_string().contains("30s"), "{fault}");

        let null = r#"{"kind":"source","message":"m","retry_after":null}"#;
        assert_eq!(
            stated(null, Some(&refusal(Some(30))))
                .statement()
                .unwrap()
                .sent
                .get(),
            null
        );

        let unasked = stated(r#"{"kind":"source","message":"m"}"#, Some(&refusal(None)));
        assert_eq!(
            unasked.statement().unwrap().sent.get(),
            r#"{"kind":"source","message":"m"}"#
        );
    }
}
