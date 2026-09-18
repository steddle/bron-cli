//! What went wrong, in the five kinds a caller can act on differently.
//!
//! The exit-code contract is the reason this type exists. A pipeline branching
//! on the result of `bron` must be able to tell "this article does not exist"
//! from "the network was down" without parsing prose, and a single `anyhow`
//! chain collapses every outcome onto exit 1.
//!
//! Not-found is a [`Fault`] rather than a successful empty result, because an
//! empty JSON array cannot distinguish "nothing matched" from "no such thing".

use std::process::ExitCode;

/// Exit codes. Small, fixed, and documented, so a shell script can branch.
///
/// | code | meaning |
/// |------|---------|
/// | 0 | ok |
/// | 1 | usage: the command line was wrong |
/// | 2 | not found: well-formed request, no such thing |
/// | 3 | network: unreachable, timed out, refused |
/// | 4 | upstream: reachable, but the answer could not be read |
/// | 5 | local: this machine could not do its part |
pub mod exit {
    pub const OK: u8 = 0;
    pub const USAGE: u8 = 1;
    pub const NOT_FOUND: u8 = 2;
    pub const NETWORK: u8 = 3;
    pub const UPSTREAM: u8 = 4;
    pub const LOCAL: u8 = 5;
}

#[derive(Debug, thiserror::Error)]
pub enum Fault {
    /// The command line did not make sense. Includes an identifier that is not
    /// a valid identifier, which is a usage error rather than a lookup miss.
    #[error("{0}")]
    Usage(String),

    /// A well-formed request for something that does not exist.
    #[error("{0}")]
    NotFound(String),

    /// The service could not be reached at all.
    #[error("cannot reach {service}: {source}")]
    Network {
        service: &'static str,
        #[source]
        source: anyhow::Error,
    },

    /// The service answered, but not with something readable.
    #[error("{service} returned an unusable response: {source}")]
    Upstream {
        service: &'static str,
        #[source]
        source: anyhow::Error,
    },

    /// bron could not do its own part on this machine.
    ///
    /// Not the network and not what came back over it: an HTTP client that
    /// would not build, a rate-limit directory that cannot be created, a stdout
    /// that will not take the answer. Nothing here is worth retrying against
    /// the same machine, which is what tells it apart from the two above.
    #[error("{0}")]
    Local(#[source] anyhow::Error),

    /// A failure the service stated, with the `code` it stated beside the kind.
    ///
    /// `fault` is the same failure in the kinds bron exits on, so the exit
    /// code, the tag and the sentence are its. The code is the service's word,
    /// kept as sent and never read here: one this build has never heard of
    /// crosses like any other. Only a stated failure is built this way, which
    /// is what keeps a code off `network` and `local`.
    #[error("{fault}")]
    Stated { fault: Box<Fault>, code: String },
}

impl Fault {
    pub fn code(&self) -> u8 {
        match self {
            Self::Usage(_) => exit::USAGE,
            Self::NotFound(_) => exit::NOT_FOUND,
            Self::Network { .. } => exit::NETWORK,
            Self::Upstream { .. } => exit::UPSTREAM,
            Self::Local(_) => exit::LOCAL,
            Self::Stated { fault, .. } => fault.code(),
        }
    }

    /// Stable machine-readable tag. Part of the JSON contract, so renaming one
    /// of these is a breaking change to every consumer.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Usage(_) => "usage",
            Self::NotFound(_) => "not_found",
            Self::Network { .. } => "network",
            Self::Upstream { .. } => "upstream",
            Self::Local(_) => "local",
            Self::Stated { fault, .. } => fault.kind(),
        }
    }

    /// The `code` the service stated for this failure, as sent. Absent on a
    /// failure bron states itself, so presence says who stated it.
    pub fn stated_code(&self) -> Option<&str> {
        match self {
            Self::Stated { code, .. } => Some(code),
            _ => None,
        }
    }

    /// This failure, as one the service stated under `code`.
    pub fn stated_as(self, code: impl Into<String>) -> Self {
        Self::Stated {
            fault: Box::new(self),
            code: code.into(),
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code())
    }

    pub fn not_found(kind: &'static str, id: impl std::fmt::Display) -> Self {
        Self::NotFound(format!("no {kind} {id}"))
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    /// The full cause chain, one line per link.
    ///
    /// `.context()` is applied throughout the library so each link names the
    /// url, path or identifier involved; flattening it here keeps that detail
    /// rather than reporting only the outermost sentence.
    pub fn chain(&self) -> Vec<String> {
        if let Self::Stated { fault, .. } = self {
            return fault.chain();
        }

        let mut chain = vec![self.to_string()];
        let mut source = std::error::Error::source(self);
        while let Some(error) = source {
            let text = error.to_string();
            if !chain.contains(&text) {
                chain.push(text);
            }
            source = error.source();
        }
        chain
    }
}

pub type Outcome<T> = std::result::Result<T, Fault>;

/// Marks a failure as this machine's rather than a service's, and says what
/// bron was trying to do at the time.
///
/// A lane wraps everything it does in [`from_source`], which knows how to tell
/// a 404 from a dead network and reads anything else as the service answering
/// badly. That is right for a malformed body and wrong for a rate-limit
/// directory bron cannot create: reported as the publisher misbehaving, it is
/// a failure a pipeline would retry against a host that was never at fault.
///
/// It carries the sentence rather than being a bare tag, because it ends up
/// outermost and is therefore what a reader sees first; what actually went
/// wrong follows it in the cause chain.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct OwnDoing(pub &'static str);

/// Classifies a failure from a source module.
///
/// Reqwest is the only thing that can tell reachability from unreadability, and
/// by the time an error reaches the command layer it is an opaque chain, so the
/// split is made by looking for a transport error anywhere in that chain.
pub fn from_source(service: &'static str, error: anyhow::Error) -> Fault {
    // bron's own end, wherever in the chain it was marked. Checked first: a
    // failure this machine caused is not the service's to answer for, whatever
    // else the chain says about it.
    if error.downcast_ref::<OwnDoing>().is_some() {
        return Fault::Local(error);
    }

    let transport_errors = || {
        error
            .chain()
            .filter_map(|e| e.downcast_ref::<reqwest::Error>())
    };

    // A 404 is upstream answering the question, not upstream failing. Reporting
    // it as a transport problem would have a pipeline retry something that will
    // never exist.
    if transport_errors()
        .filter_map(reqwest::Error::status)
        .any(|status| matches!(status.as_u16(), 404 | 410))
    {
        // The outermost context line is where the caller named the thing.
        return Fault::NotFound(format!("{service} has nothing here: {error}"));
    }

    if transport_errors().any(|e| e.is_timeout() || e.is_connect() || e.is_request()) {
        Fault::Network {
            service,
            source: error,
        }
    } else {
        Fault::Upstream {
            service,
            source: error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_maps_to_its_documented_code() {
        assert_eq!(Fault::usage("bad flag").code(), 1);
        assert_eq!(Fault::not_found("law", "BWBR0000001").code(), 2);
        assert_eq!(
            Fault::Network {
                service: "rechtspraak",
                source: anyhow::anyhow!("x")
            }
            .code(),
            3
        );
        assert_eq!(
            Fault::Upstream {
                service: "rechtspraak",
                source: anyhow::anyhow!("x")
            }
            .code(),
            4
        );
        assert_eq!(Fault::Local(anyhow::anyhow!("x")).code(), 5);
    }

    #[test]
    fn not_found_reads_as_an_answer_not_an_accident() {
        let fault = Fault::not_found("law", "BWBR0000001");
        assert_eq!(fault.to_string(), "no law BWBR0000001");
        assert_eq!(fault.kind(), "not_found");
    }

    #[test]
    fn the_chain_keeps_the_context_each_layer_added() {
        let inner = anyhow::anyhow!("connection refused")
            .context("request to https://example.invalid/x failed")
            .context("cannot read the manifest for BWBR0005289");

        let chain = Fault::Network {
            service: "rechtspraak",
            source: inner,
        }
        .chain();
        assert!(
            chain.iter().any(|l| l.contains("manifest for BWBR0005289")),
            "{chain:?}"
        );
        assert!(
            chain.iter().any(|l| l.contains("connection refused")),
            "{chain:?}"
        );
    }

    /// A directory bron cannot create is not the publisher answering badly,
    /// and a pipeline told otherwise retries a host that was never at fault.
    #[test]
    fn a_failure_of_our_own_is_not_charged_to_the_service() {
        let ours = anyhow::Error::new(std::io::Error::other("Not a directory"))
            .context("cannot create /etc/hosts/buckets")
            .context(OwnDoing(
                "bron cannot use the rate-limit budget on this machine",
            ));

        assert_eq!(from_source("data.rechtspraak.nl", ours).kind(), "local");
    }

    #[test]
    fn a_non_transport_failure_is_upstream_rather_than_network() {
        let fault = from_source("rechtspraak", anyhow::anyhow!("malformed XML"));
        assert_eq!(fault.code(), exit::UPSTREAM);
    }

    #[test]
    fn a_not_found_message_reads_as_a_sentence() {
        assert_eq!(
            Fault::not_found("law", "BWBR0000001").to_string(),
            "no law BWBR0000001"
        );
    }
}
