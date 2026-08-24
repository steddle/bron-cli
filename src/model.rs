//! What bron parsed, before anything decided how to say it.
//!
//! Dates are `NaiveDate`, not text: a lexical comparison over unvalidated
//! strings is how you silently apply the wrong law. Identifiers are newtypes
//! for the same reason.
//!
//! Nothing here implements `Serialize`, on purpose. These types are shaped for
//! reading a document, not for a consumer, and a derive would make every field
//! name part of a public contract that no compiler checks. Everything that
//! crosses the process boundary is defined in [`crate::wire`].

use crate::blob::BlobId;
use crate::ids::{CaptureId, Ecli, PublicationId};
use chrono::{DateTime, NaiveDate, Utc};

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
