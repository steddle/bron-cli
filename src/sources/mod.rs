//! Upstream services, one module each, plus the shared HTTP and XML plumbing.
//!
//! Every module here returns a `Fetched*` value: parsed, carrying the digest of
//! the document it was parsed from and byte spans into it. Nothing is kept. The
//! command layer stamps the moment it read one, because a claim about what a
//! register held is dated to when bron looked rather than to today.

pub mod publications;
pub mod rechtspraak;

mod http;
mod xml;

pub use http::{Client, Fetched, FetchedBytes};

use chrono::NaiveDate;

use crate::blob::BlobId;
use crate::ids::{Ecli, PublicationId};
use crate::model::Relation;

/// A document to parse, paired with everything needed to say where it came
/// from.
///
/// All four fields travel together because an answer's provenance states the
/// digest, the url and the length, and a parse boundary that drops one leaves
/// nothing able to state it. Splitting the digest from the text is separately
/// how offsets end up pointing at nothing.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    pub blob: BlobId,
    pub text: &'a str,
    /// The url actually dialled. Provenance only; identity is the identifier.
    pub url: &'a str,
    pub byte_len: usize,
}

impl<'a> Source<'a> {
    pub fn new(blob: BlobId, text: &'a str, url: &'a str) -> Self {
        Self {
            blob,
            text,
            url,
            byte_len: text.len(),
        }
    }
}

impl<'a> From<&'a Fetched> for Source<'a> {
    fn from(fetched: &'a Fetched) -> Self {
        Self {
            blob: fetched.blob,
            text: &fetched.text,
            url: &fetched.url,
            byte_len: fetched.byte_len(),
        }
    }
}

/// Where a document came from, carried with every value read out of it, so a
/// claim can always name the bytes and the address they were fetched from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub blob: BlobId,
    pub url: String,
    pub byte_len: usize,
}

impl From<Source<'_>> for Provenance {
    fn from(source: Source<'_>) -> Self {
        Self {
            blob: source.blob,
            url: source.url.to_string(),
            byte_len: source.byte_len,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FetchedCase {
    pub id: Ecli,
    pub court: Option<String>,
    pub decided_on: Option<NaiveDate>,
    pub published_on: Option<NaiveDate>,
    pub procedure_type: Option<String>,
    pub summary: Option<String>,
    pub full_text: Option<String>,
    pub provenance: Provenance,
    pub relations: Vec<Relation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FetchedPublication {
    pub id: PublicationId,
    pub title: Option<String>,
    pub dossier: Option<String>,
    pub published_on: Option<NaiveDate>,
    /// Absent when the record links no XML body.
    pub provenance: Option<Provenance>,
}
