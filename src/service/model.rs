//! What the service answers with.
//!
//! Received shapes and nothing else: no rendering, no policy, no decisions.
//! Only the fields bron reads are named, so a field the service adds later does
//! not break a build here.

use chrono::NaiveDate;
use serde::Deserialize;

/// A validity window, either of the two.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Window {
    pub from: NaiveDate,
    /// Absent when open-ended.
    pub until: Option<NaiveDate>,
}

/// One regulation, as the index lists it.
#[derive(Debug, Deserialize)]
pub struct Regulation {
    pub id: String,
    pub title: Option<String>,
    #[serde(rename = "type")]
    pub regulation_type: Option<String>,
}

/// How the service's copy of a rendition stands against the publisher's digest.
///
/// Three answers, never two. Collapsing `Unstated` onto "not verified" would
/// have a service that stopped sending the field route every fetch to the
/// publisher, with `resolved_via: publisher` as the only symptom anyone could
/// see. It is also the wrong answer: nothing was said, so nothing is known, and
/// bytes bron has not yet been given cannot be ruled out in advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(from = "Option<bool>")]
pub enum Verified {
    /// Checked, and the stored bytes hash to the SHA-512 the manifest names.
    Matches,
    /// Checked, and they do not. Asking for them would fail the same check on
    /// arrival, so the round trip buys nothing.
    Differs,
    /// The service did not say.
    #[default]
    Unstated,
}

impl From<Option<bool>> for Verified {
    fn from(stated: Option<bool>) -> Self {
        match stated {
            Some(true) => Self::Matches,
            Some(false) => Self::Differs,
            None => Self::Unstated,
        }
    }
}

/// One rendition, as the manifest describes it and the corpus holds it.
#[derive(Debug, Deserialize)]
pub struct Expression {
    pub label: String,
    pub in_force: Window,
    /// Absent where the manifest carried no zicht dates, and never filled in
    /// from `in_force`.
    pub as_known: Option<Window>,
    pub upstream_hash: Option<String>,
    /// The size the manifest states, which is not the length of the document
    /// the spans point into. See [`BlobProvenance`].
    pub size: Option<u64>,
    /// True where the service holds this rendition's text.
    pub held: bool,
    #[serde(default)]
    pub verified: Verified,
    pub blob: Option<String>,
}

/// What a rendition's articles resolved to.
#[derive(Debug, Deserialize)]
pub struct Articles {
    pub regulation_id: String,
    pub expression: Expression,
    pub articles: Vec<Article>,
}

/// One article, where it sits in the frozen bytes, and what stands there.
#[derive(Debug, Clone, Deserialize)]
pub struct Article {
    pub number: String,
    pub label: Option<String>,
    pub title: Option<String>,
    pub path: Option<String>,
    /// The publisher's stable identity for this article across renditions,
    /// which is what makes a comparison an identity match rather than a guess.
    pub stam_id: Option<String>,
    pub amended_by: Option<String>,
    pub effect: Option<String>,
    pub status: Option<String>,
    /// The day this article took the form it has here, as the register states
    /// it. A date and not a moment, and absent where the register named no day.
    pub in_force_from: Option<NaiveDate>,
    pub jci: Option<String>,
    pub span: Span,
    /// The publisher's own bytes at `span`, present only where `raw=1` asked
    /// for them. On the service's word: checking it means fetching the blob it
    /// was cut from, and not having to is why it is here.
    pub raw: Option<String>,
    pub paragraphs: Vec<Paragraph>,
}

/// One lid, as the register numbers it.
#[derive(Debug, Clone, Deserialize)]
pub struct Paragraph {
    pub number: Option<String>,
    /// The service's reading of the bytes at `span`, present only where
    /// `rendered=1` asked for it.
    pub rendered: Option<String>,
    pub raw: Option<String>,
    pub jci: Option<String>,
    pub span: Span,
}

impl Paragraph {
    /// The service's reading, or the empty string where it sent none.
    pub fn text(&self) -> &str {
        self.rendered.as_deref().unwrap_or_default()
    }
}

/// A byte range into the rendition's document, absolute.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

/// How long a blob is.
///
/// The publisher's own address travels too and is not read: what a provenance
/// names is where these bytes can be fetched and hashed, and only one of the
/// two addresses answers under the digest.
///
/// The length is the one thing an answer cannot do without: every span in it
/// points inside that document, and the size the manifest states is not it. For
/// a large share of renditions the manifest describes a gzip of the
/// manifestation rather than what its own URL serves, and BWBR0008691
/// 2016-07-01_0 is one: 49.606 bytes stated against 547.732 held.
#[derive(Debug, Deserialize)]
pub struct BlobProvenance {
    pub size: usize,
}

/// Which rendition a question is about, and how much of its text to send.
///
/// Both coordinates travel, because the service resolves a point in both
/// windows and defaults the one it is not given to today. Asking what the
/// register showed on a past date without saying which date's text is meant
/// resolves against a rendition in force now, which for anything amended since
/// is no rendition at all.
#[derive(Debug)]
pub struct Ask<'a> {
    pub as_of: NaiveDate,
    /// Stated only where the caller asked what the register showed. One date
    /// names both coordinates of that point: the text for a day, as the
    /// register had it that day.
    pub as_known_on: Option<NaiveDate>,
    pub number: Option<&'a str>,
    /// Ask for the publisher's own bytes at every span. Off by default, so no
    /// markup travels for an answer nobody will read it in.
    pub raw: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checked-and-matching, checked-and-not, and nothing said are three
    /// answers. A service that stops sending the field must not read as one
    /// that checked and found a mismatch.
    #[test]
    fn a_verification_the_service_did_not_state_is_its_own_answer() {
        #[derive(Deserialize)]
        struct Row {
            #[serde(default)]
            verified: Verified,
        }

        let read = |json: &str| serde_json::from_str::<Row>(json).unwrap().verified;

        assert_eq!(read(r#"{"verified":true}"#), Verified::Matches);
        assert_eq!(read(r#"{"verified":false}"#), Verified::Differs);
        assert_eq!(read(r#"{"verified":null}"#), Verified::Unstated);
        assert_eq!(read("{}"), Verified::Unstated);
    }
}
