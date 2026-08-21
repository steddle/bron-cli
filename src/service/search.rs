//! Ranking the shared corpus against a question.
//!
//! The one call with no publisher behind it. Bytes and manifests both have a
//! source of last resort; a ranking over the shared corpus does not, because
//! the corpus is the source.
//!
//! A POST, alone among these, because the term is the caller's own question
//! rather than an identifier. A question in a URL is a question in every access
//! log the request passes, which is also why the answer to one is never written
//! to disk here; see [`crate::sources`] and `Reading::Envelope`.

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_known_on: Option<String>,
}

/// A ranking, and which corpora it could actually have come from.
#[derive(Debug, Deserialize)]
pub struct SearchData {
    /// Which corpora held vectors, so could have answered. A lane holding no
    /// vectors must not read as a lane holding no answers.
    pub searched: Vec<String>,
    pub hits: Vec<SearchHit>,
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
    pub distance: f64,
    #[serde(default)]
    pub lid: Option<SearchLid>,
}

#[derive(Debug, Deserialize)]
pub struct SearchLid {
    pub number: Option<String>,
}

impl Service {
    /// Asks the service which held texts are nearest a question.
    pub fn search(&self, client: &Client, request: &SearchRequest) -> Outcome<Answered<SearchData>> {
        let url = format!("{}/api/v1/search", self.base());

        self.read(client, client.post(&url).json(request))
    }
}
