//! Official identifiers, as types.
//!
//! Identity in bron is always the identifier the publisher assigns: a BWB id, an
//! ECLI, a document number. Never a URL, which is an address that can change
//! without the thing it addresses changing.
//!
//! An ECLI is asked of rechtspraak directly, so it validates on the way in.
//! That makes the difference between a lookup and a search visible at the
//! call site: `Ecli::from_str("hoge raad")` fails, so anything accepting free
//! text is doing resolution, not retrieval. A BWB id and a publication id go
//! to the service as typed, and the service says whether either is one.

use std::fmt;
use std::str::FromStr;

use serde::Serialize;

/// Error returned when text does not have the shape of an identifier.
#[derive(Debug, thiserror::Error)]
#[error("not a valid {kind}: {text}")]
pub struct InvalidId {
    pub kind: &'static str,
    pub text: String,
}

impl InvalidId {
    fn new(kind: &'static str, text: &str) -> Self {
        Self {
            kind,
            text: text.to_string(),
        }
    }
}

/// Generates the shared boilerplate: accessors, display, serde and
/// `TryFrom<&str>`.
macro_rules! string_id {
    ($name:ident, $kind:literal) => {
        impl $name {
            pub const KIND: &'static str = $kind;

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = InvalidId;

            fn try_from(text: &str) -> Result<Self, Self::Error> {
                text.parse()
            }
        }
    };
}

/// A European Case Law Identifier, e.g. `ECLI:NL:HR:2019:1278`.
///
/// Five colon-separated fields: the literal `ECLI`, a country code, the court
/// code, the year, and an ordinal. Only well-formedness is checked here;
/// whether the decision exists is a question for the network.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ecli(String);

string_id!(Ecli, "ECLI");

impl Ecli {
    /// Court code, e.g. `HR` or `RBAMS`.
    pub fn court(&self) -> &str {
        self.0.split(':').nth(2).unwrap_or_default()
    }

    /// Year of the decision as recorded in the identifier.
    pub fn year(&self) -> &str {
        self.0.split(':').nth(3).unwrap_or_default()
    }
}

impl FromStr for Ecli {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let fields: Vec<&str> = text.split(':').collect();

        match fields[..] {
            [prefix, country, court, year, ordinal]
                if prefix.eq_ignore_ascii_case("ECLI")
                    && country.len() == 2
                    && country.bytes().all(|b| b.is_ascii_alphabetic())
                    && !court.is_empty()
                    && court.bytes().all(|b| b.is_ascii_alphanumeric())
                    && year.len() == 4
                    && year.bytes().all(|b| b.is_ascii_digit())
                    && !ordinal.is_empty()
                    && ordinal
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'.') =>
            {
                // The prefix and country are conventionally uppercase; the
                // ordinal is not, so only normalise what is safe to normalise.
                Ok(Self(format!(
                    "ECLI:{}:{court}:{year}:{ordinal}",
                    country.to_ascii_uppercase()
                )))
            }
            _ => Err(InvalidId::new(Self::KIND, text)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_normalises_eclis() {
        let ecli: Ecli = "ecli:nl:HR:2019:1278".parse().unwrap();
        assert_eq!(ecli.as_str(), "ECLI:NL:HR:2019:1278");
        assert_eq!(ecli.court(), "HR");
        assert_eq!(ecli.year(), "2019");
    }

    #[test]
    fn rejects_malformed_eclis() {
        assert!("ECLI:NL:HR:2019".parse::<Ecli>().is_err());
        assert!("ECLI:NL:HR:19:1278".parse::<Ecli>().is_err());
        assert!("ECLI:NLD:HR:2019:1278".parse::<Ecli>().is_err());
        assert!(
            "http://deeplink.rechtspraak.nl/uitspraak?id=x"
                .parse::<Ecli>()
                .is_err()
        );
    }
}
