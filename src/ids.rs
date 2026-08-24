//! Official identifiers, as types.
//!
//! Identity in bron is always the identifier the publisher assigns: a BWB id, an
//! ECLI, a document number. Never a URL, which is an address that can change
//! without the thing it addresses changing.
//!
//! Each type validates on the way in. That makes the difference between a lookup
//! and a search visible at the call site: `BwbId::from_str("burgerlijk wetboek")`
//! fails, so anything accepting free text is doing resolution, not retrieval.

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

/// Generates the shared boilerplate: display, serde, and both SQL directions.
macro_rules! string_id {
    ($name:ident, $kind:literal) => {
        impl $name {
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

        impl $name {
            pub const KIND: &'static str = $kind;
        }
    };
}

/// A Basis Wetten Bestand identifier, e.g. `BWBR0005289`.
///
/// `BWBR` for regelingen, `BWBV` for verdragen.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BwbId(String);

string_id!(BwbId, "BWB id");

impl FromStr for BwbId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let valid = text.len() == 11
            && text.is_char_boundary(4)
            && matches!(&text[..4], "BWBR" | "BWBV")
            && text[4..].bytes().all(|b| b.is_ascii_digit());

        if valid {
            Ok(Self(text.to_string()))
        } else {
            Err(InvalidId::new(Self::KIND, text))
        }
    }
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

        let valid = match fields[..] {
            [prefix, country, court, year, ordinal] => {
                prefix.eq_ignore_ascii_case("ECLI")
                    && country.len() == 2
                    && country.bytes().all(|b| b.is_ascii_alphabetic())
                    && !court.is_empty()
                    && court.bytes().all(|b| b.is_ascii_alphanumeric())
                    && year.len() == 4
                    && year.bytes().all(|b| b.is_ascii_digit())
                    && !ordinal.is_empty()
                    && ordinal
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'.')
            }
            _ => false,
        };

        if valid {
            // The prefix and country are conventionally uppercase; the ordinal
            // is not, so only normalise what is safe to normalise.
            let mut parts = fields;
            let prefix = parts[0].to_ascii_uppercase();
            let country = parts[1].to_ascii_uppercase();
            parts[0] = &prefix;
            parts[1] = &country;
            Ok(Self(parts.join(":")))
        } else {
            Err(InvalidId::new(Self::KIND, text))
        }
    }
}

/// An officiële bekendmakingen document number, e.g. `kst-33669-6` or
/// `stb-2024-123`.
///
/// The collection prefix is open-ended, so this checks shape rather than
/// membership of a fixed list.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicationId(String);

string_id!(PublicationId, "publication id");

impl PublicationId {
    /// Collection prefix, e.g. `kst`, `stb`, `stcrt`.
    pub fn collection(&self) -> &str {
        self.0.split('-').next().unwrap_or_default()
    }
}

impl FromStr for PublicationId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let mut fields = text.split('-');

        let prefix_ok = fields
            .next()
            .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase()));
        let rest: Vec<&str> = fields.collect();
        let rest_ok = !rest.is_empty()
            && rest
                .iter()
                .all(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_alphanumeric()));

        if prefix_ok && rest_ok {
            Ok(Self(text.to_string()))
        } else {
            Err(InvalidId::new(Self::KIND, text))
        }
    }
}

impl PublicationId {
    /// Resolves a BWB source reference to the publication it names.
    ///
    /// BWB writes these as `Stb.2023-41`; the publication register addresses
    /// the same document as `stb-2023-41`. Translating between them is what
    /// turns an article into the amending act that produced it, and the
    /// amending act is where transitional law lives.
    pub fn from_bwb_reference(reference: &str) -> Option<Self> {
        let (kind, rest) = reference.trim().split_once('.')?;
        let (year, number) = rest.split_once('-')?;

        let valid = !kind.is_empty()
            && kind.bytes().all(|b| b.is_ascii_alphabetic())
            && year.len() == 4
            && year.bytes().all(|b| b.is_ascii_digit())
            && !number.is_empty()
            && number.bytes().all(|b| b.is_ascii_alphanumeric());

        valid.then(|| Self(format!("{}-{year}-{number}", kind.to_ascii_lowercase())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_real_bwb_ids() {
        assert_eq!(
            "BWBR0005289".parse::<BwbId>().unwrap().as_str(),
            "BWBR0005289"
        );
        assert!("BWBV0001234".parse::<BwbId>().is_ok());
        assert!("  BWBR0005289  ".parse::<BwbId>().is_ok());
    }

    #[test]
    fn rejects_free_text_as_a_bwb_id() {
        // The point of the newtype: this is a search, and the type says so.
        assert!("burgerlijk wetboek".parse::<BwbId>().is_err());
        assert!("BWBR005289".parse::<BwbId>().is_err());
        assert!("BWBX0005289".parse::<BwbId>().is_err());
        assert!("BWBR000528X".parse::<BwbId>().is_err());
        assert!("".parse::<BwbId>().is_err());
    }

    #[test]
    fn bwb_rejection_is_not_a_panic_on_multibyte_input() {
        assert!("Bä€".parse::<BwbId>().is_err());
        assert!("日本語のテキストです".parse::<BwbId>().is_err());
    }

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

    #[test]
    fn accepts_publication_ids_across_collections() {
        assert_eq!(
            "kst-33669-6".parse::<PublicationId>().unwrap().collection(),
            "kst"
        );
        assert_eq!(
            "stb-2024-123"
                .parse::<PublicationId>()
                .unwrap()
                .collection(),
            "stb"
        );
        assert!("stcrt-2024-1234".parse::<PublicationId>().is_ok());
        assert!("ah-tk-20232024-1234".parse::<PublicationId>().is_ok());
    }

    #[test]
    fn rejects_malformed_publication_ids() {
        assert!("kst".parse::<PublicationId>().is_err());
        assert!("KST-33669-6".parse::<PublicationId>().is_err());
        assert!("kst--6".parse::<PublicationId>().is_err());
    }

    #[test]
    fn bwb_source_references_resolve_to_publications() {
        assert_eq!(
            PublicationId::from_bwb_reference("Stb.2023-41")
                .unwrap()
                .as_str(),
            "stb-2023-41"
        );
        assert_eq!(
            PublicationId::from_bwb_reference("Stb.1991-600")
                .unwrap()
                .as_str(),
            "stb-1991-600"
        );
        assert_eq!(
            PublicationId::from_bwb_reference("Stcrt.2020-12345")
                .unwrap()
                .as_str(),
            "stcrt-2020-12345"
        );
    }

    #[test]
    fn a_reference_that_is_not_one_resolves_to_nothing() {
        assert!(PublicationId::from_bwb_reference("onbekend").is_none());
        assert!(PublicationId::from_bwb_reference("Stb.91-600").is_none());
        assert!(PublicationId::from_bwb_reference("").is_none());
    }
}
