//! What bron stores, as read back out.
//!
//! Dates are `NaiveDate`, not text: `--as-of` compares them, and a lexical
//! comparison over unvalidated strings is how you silently apply the wrong law.
//! Identifiers are newtypes for the same reason.
//!
//! Nothing here implements `Serialize`, on purpose. These types are shaped for
//! the corpus, not for a consumer, and a derive would make every field name
//! part of a public contract that no compiler checks. Everything that crosses
//! the process boundary is defined in [`crate::wire`].

use crate::blob::BlobId;
use crate::ids::{BwbId, CaptureId, Ecli, PublicationId};
use chrono::{DateTime, NaiveDate, Utc};

/// The date the BWB uses to mean "no end yet".
pub const OPEN_ENDED: &str = "9999-12-31";

/// A window of validity, closed at both ends.
///
/// Both bounds are inclusive, because the manifest's `einddatum` is the last
/// day the text applied rather than the first day it did not. Reading this as
/// half-open puts an as-of query one day out, which in this domain means
/// applying the wrong law.
///
/// Two different windows govern every published law. *Geldigheid* is when the
/// text was in force, which is the question to ask about conduct. *Zicht* is
/// when the register showed it, which is the question to ask about what someone
/// could have known. They diverge whenever a correction is published with
/// retroactive effect, and answering the first with the second is the mistake
/// this type exists to make hard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Validity {
    pub from: NaiveDate,
    /// `None` when open-ended. Upstream writes that as `9999-12-31`.
    pub until: Option<NaiveDate>,
}

impl Validity {
    pub fn contains(&self, date: NaiveDate) -> bool {
        date >= self.from && self.until.is_none_or(|until| date <= until)
    }

    pub fn is_open_ended(&self) -> bool {
        self.until.is_none()
    }
}

/// A byte range in the frozen document a value was read from.
///
/// This is what makes a stored fact citable: the digest names the document, the
/// span names the characters in it that state the fact.
///
/// Offsets are byte offsets into the blob, and they are also valid char
/// boundaries into the decoded text, because a fetch that is not UTF-8 is
/// rejected outright rather than lossily decoded. Slicing is therefore total in
/// practice and still returns `None` rather than panicking if it ever is not.
///
/// A span quotes the *source document*, which for BWB is XML. It is not the
/// same string as the rendered `content` beside it. Verifying a quotation means
/// comparing against the rendering; proving what the publisher wrote means
/// slicing the blob. Both are offered because they answer different questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// Byte ranges come from the XML parser and are bounded by the document,
    /// which cannot exceed `u32::MAX` because a larger fetch is refused before
    /// it is parsed. Returns `None` only if that ever stops being true.
    pub fn new(range: std::ops::Range<usize>) -> Option<Self> {
        Some(Self {
            start: u32::try_from(range.start).ok()?,
            end: u32::try_from(range.end).ok()?,
        })
    }

    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Lifts the exact bytes back out of the document this span refers to.
    pub fn slice<'a>(&self, document: &'a str) -> Option<&'a str> {
        document.get(self.start as usize..self.end as usize)
    }
}

/// A law, independent of any one version of its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Law {
    pub id: BwbId,
    pub title: String,
    pub version_count: u32,
}

/// One published version of a law's text.
#[derive(Debug, Clone, PartialEq)]
pub struct LawVersion {
    pub law_id: BwbId,
    /// Upstream's name for this version, e.g. `2026-07-16_0`.
    pub version: String,
    pub title: String,
    pub law_type: Option<String>,
    pub in_force: Validity,
    /// Absent when the manifest omitted the zicht dates. Never inferred from
    /// `in_force`: see [`Validity`] for why the two must not substitute.
    pub visible: Option<Validity>,
    /// The frozen document this version was parsed from.
    pub blob: BlobId,
    /// The publisher's own SHA-512, when the manifest supplied one. Lets a
    /// frozen copy be checked against the source rather than only against us.
    pub upstream_hash: Option<String>,
    pub fetched_at: DateTime<Utc>,
    pub article_count: u32,
}

/// One article within a version of a law.
#[derive(Debug, Clone, PartialEq)]
pub struct Article {
    pub law_id: BwbId,
    pub version: String,
    /// Stable identity across versions. The key a diff matches on.
    pub stam_id: Option<String>,
    /// Article number as printed, e.g. `162` or `162a`. Not unique within a
    /// law: annexes carry their own article numbering.
    pub number: String,
    pub label: Option<String>,
    pub title: Option<String>,
    /// Structural path, e.g. `/Boek6/Titeldeel3/Afdeling1/Artikel162`. This is
    /// what disambiguates a repeated number and what makes `6:162` resolvable.
    pub path: Option<String>,
    /// Rendered text, derived from the leden below for reading and indexing.
    /// The leden are the citable representation; this is a convenience.
    pub content: String,
    pub span: Span,
    pub leden: Vec<Lid>,
    /// The publication that put this article in this form.
    pub amended_by: Option<PublicationId>,
    pub effect: Option<String>,
    /// The day this article took the form it has here. For a repealed
    /// provision it is the day it went.
    pub in_force_from: Option<NaiveDate>,
    /// How many tables this article prints. Its text carries one line per row
    /// with cells separated, which is a linearisation and not a grid.
    pub tables: u32,
    /// How many images this article prints, none of which bron holds.
    pub illustrations: u32,
    /// Upstream's identity for this text of this article.
    pub versie_id: Option<String>,
    /// `goed`, or `nogniet` where the provision has not commenced.
    pub status: Option<String>,
    /// The Juriconnect reference the register states for this article.
    pub jci: Option<String>,
}

/// One lid of an article, the unit Dutch practice actually cites.
#[derive(Debug, Clone, PartialEq)]
pub struct Lid {
    /// Lid number as printed. Absent for articles with unnumbered text.
    pub number: Option<String>,
    pub content: String,
    pub span: Span,
    /// The Juriconnect reference the register states for this lid.
    pub jci: Option<String>,
}

/// A published court decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub id: Ecli,
    pub court: Option<String>,
    pub decided_on: Option<NaiveDate>,
    pub published_on: Option<NaiveDate>,
    pub procedure_type: Option<String>,
    pub summary: Option<String>,
    pub full_text: Option<String>,
    pub blob: BlobId,
    pub fetched_at: DateTime<Utc>,
    pub relations: Vec<Relation>,
}

/// A formal link between two decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub target: Ecli,
    /// Upstream's name for the step, e.g. `cassatie` or `conclusie`.
    pub kind: Option<String>,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Points back to a decision that came before this one.
    Earlier,
    /// Points forward to a decision that came after it.
    Later,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Earlier => "earlier",
            Self::Later => "later",
        }
    }
}

/// What the register can honestly say about whether a decision was appealed.
///
/// Never a boolean. Rechtspraak records appeal relations in both directions, so
/// a forward link missing from a decision it publishes is evidence. It is not
/// proof of no appeal: an appeal that settled, was withdrawn, or ended in an
/// unpublished decision leaves no trace here. Collapsing this to `true` and
/// `false` is how a tool comes to assert that none of these judgments were
/// appealed.
///
/// A negative also carries the moment it was true at. It is a claim about what
/// the register held when bron read the decision, and it began going stale as
/// it was made: a decision read a year ago yields a negative indistinguishable
/// from one read this morning unless the answer says when it was looked at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppealStatus {
    /// A later decision exists and is published.
    Appealed { successors: Vec<Ecli> },
    /// The decision carries no forward link, as of the moment bron read it.
    NoPublishedSuccessor { as_of: DateTime<Utc> },
}

/// A parliamentary or official publication.
#[derive(Debug, Clone, PartialEq)]
pub struct Publication {
    pub id: PublicationId,
    pub title: Option<String>,
    pub dossier: Option<String>,
    pub document_type: Option<String>,
    pub vergaderjaar: Option<String>,
    pub published_on: Option<NaiveDate>,
    pub url: Option<String>,
    pub full_text: Option<String>,
    pub blob: Option<BlobId>,
    pub fetched_at: DateTime<Utc>,
}

/// How a capture's bytes stand against the digest the index recorded.
///
/// The evidence is asymmetric and the type says so. A match proves the replay
/// is byte-identical to what the crawl stored. A difference proves nothing:
/// the index digest covers the payload as stored, chunk framing included, and
/// the replay strips that before serving. Neither is a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestCheck {
    Matches,
    Differs,
    /// The index carried no digest for this row.
    Absent,
}

/// Whether the capture holds the page itself.
///
/// Three states, because the archive is entitled to withhold the answer. A
/// robots-excluded crawl records a capture whose result is unknown, and calling
/// that `false` asserts the page was not captured when nobody knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Captured {
    Yes,
    No,
    Unknown,
}

/// One capture of one page, as read back out of the corpus.
///
/// The date this is bound to is [`Self::captured_at`], never [`Self::fetched_at`]
/// and never today. That is the whole distinction between an archive answer and
/// an authority: a statute states what it states, while a capture states only
/// that a page said something on a day.
#[derive(Debug, Clone, PartialEq)]
pub struct Capture {
    pub id: CaptureId,
    pub original_url: String,
    /// When the archive took the capture.
    pub captured_at: DateTime<Utc>,
    /// What the site answered the archive. `None` is the archive declining to
    /// say, which is what it records for a robots-excluded crawl.
    pub archive_status: Option<u16>,
    pub index_digest: Option<String>,
    /// `None` when the bytes were never fetched, so there was no reading to
    /// check in the first place.
    pub verified: Option<DigestCheck>,
    /// The archive replayed an earlier capture's bytes, so the content is older
    /// than `captured_at`.
    pub is_revisit: bool,
    pub mime_type: Option<String>,
    pub encoding: Option<String>,
    pub encoding_from: Option<String>,
    /// bron asked for this capture rather than finding one already held.
    pub requested: bool,
    pub content: Option<String>,
    pub blob: Option<BlobId>,
    /// When bron read it, which is not when it was captured.
    pub fetched_at: DateTime<Utc>,
}

impl Capture {
    /// Whether this capture holds the page itself.
    pub fn captured_the_page(&self) -> Captured {
        match self.archive_status {
            Some(200) => Captured::Yes,
            Some(_) => Captured::No,
            // The archive withheld the status. It may or may not have got the
            // page, and saying either would be an invention.
            None => Captured::Unknown,
        }
    }

    /// The address a citation names.
    pub fn citation_url(&self) -> String {
        self.id.citation_url(&self.original_url)
    }
}

// -- search-index hits, not yet fetched ----------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseHit {
    pub id: Ecli,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub decided_on: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationHit {
    pub id: PublicationId,
    pub title: Option<String>,
    pub dossier: Option<String>,
    pub vergaderjaar: Option<String>,
    pub published_on: Option<NaiveDate>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn validity_covers_a_closed_window_inclusively() {
        let window = Validity {
            from: date("2002-04-12"),
            until: Some(date("2002-11-30")),
        };
        assert!(window.contains(date("2002-04-12")));
        assert!(window.contains(date("2002-11-30")));
        assert!(!window.contains(date("2002-04-11")));
        assert!(!window.contains(date("2002-12-01")));
    }

    #[test]
    fn an_open_ended_window_has_no_upper_bound() {
        let window = Validity {
            from: date("2026-07-16"),
            until: None,
        };
        assert!(window.is_open_ended());
        assert!(window.contains(date("2026-07-16")));
        assert!(window.contains(date("9999-01-01")));
        assert!(!window.contains(date("2026-07-15")));
    }

    #[test]
    fn a_span_lifts_the_exact_source_text_back_out() {
        let document = "<al>Verbintenissen kunnen slechts ontstaan.</al>";
        let text = "Verbintenissen kunnen slechts ontstaan";
        let start = document.find(text).unwrap() as u32;
        let span = Span {
            start,
            end: start + text.len() as u32,
        };
        assert_eq!(span.slice(document), Some(text));
        assert_eq!(span.len(), text.len() as u32);
    }

    #[test]
    fn a_span_past_the_end_yields_nothing_rather_than_panicking() {
        assert_eq!(
            Span {
                start: 0,
                end: 9_999
            }
            .slice("short"),
            None
        );
    }
}
