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

use crate::ids::{Ecli, PublicationId};
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

// -- search-index hits, not yet fetched ----------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseHit {
    pub id: Ecli,
    pub court: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub decided_on: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationHit {
    pub id: PublicationId,
    pub title: Option<String>,
    pub dossier: Option<String>,
    pub published_on: Option<NaiveDate>,
}
