//! Asking the shared corpus a question, or looking a citation up in it.
//!
//! One address and two answers. A question is ranked by a vector; a term that
//! parses as a citation is looked up, because an identifier is arithmetic and
//! `3:305a` put through a vector comes back as the Wft's numerically similar
//! articles and never as artikel 3:305a BW. The two shapes stay apart here
//! because a resolved citation carries no distance, and inventing one would
//! say the answer came about a way it did not.
//!
//! The one call with no publisher behind it. Bytes and manifests both have a
//! source of last resort; the shared corpus does not, because it is the source.
//!
//! A POST, alone among these, because the term is the caller's own question
//! rather than an identifier. A question in a URL is a question in every access
//! log the request passes, which is also why the answer to one is never written
//! to disk here; see [`crate::sources`] and `Reading::Envelope`.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::error::Outcome;
use crate::sources::Client;

use super::Service;
use super::client::Answered;

/// What a search asks for. Mirrors the service's own body.
#[derive(Debug, Serialize)]
pub struct SearchRequest {
    pub q: String,
    pub limit: u32,
    pub as_of: NaiveDate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_known_on: Option<NaiveDate>,
}

/// What the term turned out to be.
///
/// Read at `kind`, never guessed at from which keys are present. The two shapes
/// do happen to share no key, so telling them apart by shape works, and it
/// works right up until something is wrong: a renamed field or a `distance`
/// that arrives as a string makes the ranking fail to match, falls through to
/// the lookup, fails there too, and reports that the answer matched no variant
/// at all. No field, no line, no column, on exactly the failure this has to
/// catch. With the tag read first, the shape is settled before a single field
/// is, and what is wrong with it is named.
///
/// `kind` is the word [`crate::wire::Caveat`] and the service's own `Fault`
/// already use for the value a consumer switches on.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchData {
    /// The term was a question.
    Ranking {
        /// Which corpora held vectors, so could have answered. A lane holding
        /// no vectors must not read as a lane holding no answers.
        searched: Vec<String>,
        hits: Vec<SearchHit>,
    },
    /// The term was a citation, and every reading of it that resolved.
    Lookup {
        /// The citation as the service read it, which is not always as it was
        /// typed: `6:162 BW` pins the book and is read as `6:162`.
        citation: String,
        resolved: Vec<Cited>,
    },
}

/// One reading of a citation, resolved to an article in a rendition.
///
/// No distance, and none is coming. A citation that resolves is not an 87%
/// match, and a number beside it invites a threshold on something that is not
/// an ordering.
#[derive(Debug, Deserialize)]
pub struct Cited {
    /// How the citation was read. `book` takes the first segment as a book of
    /// the Burgerlijk Wetboek; `literal` takes the whole number as one a
    /// regulation prints itself.
    pub reading: String,
    pub source: String,
    pub regulation_id: String,
    pub expression: String,
    /// Whether the service holds this rendition's text.
    pub held: bool,
    /// The number as the regulation prints it, which for a book reading is the
    /// citation with its book taken off.
    pub number: String,
    /// Absent where the rendition's text is not held: the dates resolved and
    /// the rendition is named, and whether it carries the article is a fact
    /// nobody has yet.
    pub label: Option<String>,
    pub path: Option<String>,
    pub content_hash: Option<String>,
    /// What limits this reading in particular, as against the answer as a
    /// whole. The rendition it resolved to may be one the publisher does not
    /// vouch for, or one of several covering the date.
    #[serde(default)]
    pub caveats: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SearchHit {
    /// Which corpus this came from: `wetgeving` today, more later.
    pub source: String,
    /// `article` or `lid`.
    pub unit: String,
    pub regulation_id: String,
    pub expression: String,
    pub number: String,
    pub label: Option<String>,
    pub path: Option<String>,
    pub content_hash: String,
    /// The cosine distance of the hit's own vector to the question. Absent on
    /// a text that has no vector, which only full text can find.
    #[serde(default)]
    pub distance: Option<f64>,
    #[serde(default)]
    pub lid: Option<SearchLid>,
}

#[derive(Debug, Deserialize)]
pub struct SearchLid {
    pub number: Option<String>,
}

impl Service {
    /// Asks the service which held texts are nearest a question.
    pub fn search(
        &self,
        client: &Client,
        request: &SearchRequest,
    ) -> Outcome<Answered<SearchData>> {
        let url = format!("{}/api/v1/search", self.base());

        self.read(client, client.post(&url).json(request))
    }
}
