//! Reading the officiële publicaties through the bron service.
//!
//! The service holds what step 0 of `research/26-09-23-the-order-from-here.md`
//! measured to name no person outside an office, passes on unkept what it has
//! not measured, and refuses what it knows to name private persons. bron keeps
//! no publisher client of its own for this lane any more: a lookup and a
//! search are each one call, read the same way `law show` and `find` are.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::error::Outcome;
use crate::sources::Client;

use super::Service;
use super::client::Answered;
use super::model::Span;

/// What a search over the officiële publicaties asks for. Mirrors the
/// service's own body: at least one of `q`, `collection` and `dossier`, which
/// the service refuses the absence of rather than bron.
#[derive(Debug, Serialize)]
pub struct PublicationQuery<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q: Option<&'a str>,
    /// One collection by its own short code: `kst`, `stb`, `stcrt`, `trb`,
    /// `blg`. Sent as typed; the service says whether it is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dossier: Option<&'a str>,
    /// As the service reads it: digits, exactly as typed, and left out
    /// where untyped, so the service's window is the default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<&'a str>,
    /// Where to ask on, as a previous answer's own `next`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<&'a str>,
}

/// The publisher's own ranking of a question, asked live: bron keeps no index
/// of the officiële publicaties.
#[derive(Debug, Deserialize)]
pub struct PublicationSearch {
    pub hits: Vec<PublicationHit>,
    /// The publisher's own count of matches, which it calls an estimate.
    pub source_total: u64,
    /// Where to ask on. Absent on the last page.
    pub next: Option<String>,
}

/// A hit in the publisher's order. `publication_type`, `title` and
/// `published_on` are stated only for a kind the service holds, because a
/// title can name a person.
#[derive(Debug, Deserialize)]
pub struct PublicationHit {
    pub id: String,
    pub held: bool,
    #[serde(rename = "type")]
    pub publication_type: Option<String>,
    pub title: Option<String>,
    pub published_on: Option<NaiveDate>,
}

/// One official publication: its record, its manifestations under the digests
/// that name them, the dossier it states as its own, and where a
/// manifestation is XML the articles it prints at their spans.
#[derive(Debug, Deserialize)]
pub struct Publication {
    pub id: String,
    #[serde(rename = "type")]
    pub publication_type: Option<String>,
    pub title: Option<String>,
    pub published_on: Option<NaiveDate>,
    pub signed_on: Option<NaiveDate>,
    /// The kamerstukdossier a kamerstuk belongs to.
    pub dossier: Option<String>,
    /// Whether the corpus keeps it. False where it was passed on, under a
    /// `publication_not_held` caveat.
    pub held: bool,
    pub manifestations: Vec<Manifestation>,
    /// The dossier the publication states as its own, in its `<histnoot>`. On
    /// a publication not held, `stated_in` is the digest of the bytes bron
    /// read and passed on, which `/blobs` does not serve.
    pub dossiers: Vec<PublicationDossier>,
    /// The articles the XML prints, at their spans in the XML manifestation.
    /// Absent where no manifestation is XML.
    pub units: Option<Vec<PublicationUnit>>,
}

/// One manifestation of a publication, in one format.
#[derive(Debug, Deserialize)]
pub struct Manifestation {
    /// The publisher's own name for it: `xml`, `pdf`, `html`, `odt`,
    /// `metadata`, `metadataowms`.
    pub format: String,
    /// SHA-256 of its bytes. Absent where the publication was passed on and
    /// this format was not the one item asked for.
    pub blob: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PublicationDossier {
    pub dossier: String,
    pub stated_in: String,
}

/// One unit the XML manifestation prints, at its span in that document.
#[derive(Debug, Deserialize)]
pub struct PublicationUnit {
    /// `artikel` or `wijzig-artikel`, the outermost of either.
    pub kind: String,
    /// The heading as printed: `ARTIKEL I`, `Enig artikel`.
    pub label: Option<String>,
    pub span: Span,
    /// SHA-256 of bron's rendering.
    pub content_hash: String,
    /// bron's rendering, present only where `rendered` was asked.
    pub rendered: Option<String>,
    /// The publisher's bytes at the span, present only where `raw` was asked.
    pub raw: Option<String>,
}

impl PublicationUnit {
    /// What a reader calls this unit: its label, else its kind.
    pub fn name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.kind)
    }
}

impl Service {
    /// One publication by identifier: held, passed through, or refused, the
    /// service's call. `limit` and `offset` page its units.
    pub fn publication(
        &self,
        client: &Client,
        id: &str,
        limit: Option<&str>,
        offset: Option<&str>,
        rendered: bool,
        raw: bool,
    ) -> Outcome<Answered<Publication>> {
        let url = self.route(&["publications", id])?;

        let mut query = Vec::new();
        if let Some(limit) = limit {
            query.push(("limit", limit));
        }
        if let Some(offset) = offset {
            query.push(("offset", offset));
        }
        if rendered {
            query.push(("rendered", "1"));
        }
        if raw {
            query.push(("raw", "1"));
        }

        self.read(client, client.get(&url).query(&query))
    }

    /// The publisher's own ranking of a question over the officiële
    /// publicaties, asked live.
    pub fn publication_search(
        &self,
        client: &Client,
        ask: &PublicationQuery<'_>,
    ) -> Outcome<Answered<PublicationSearch>> {
        let url = format!("{}/api/v1/publications/search", self.base());

        self.read(client, client.post(&url).json(ask))
    }
}
