//! Upstream services, one module each, plus the shared HTTP and XML plumbing.
//!
//! Every module here returns a `Fetched*` value: parsed, carrying the digest of
//! the document it was parsed from and byte spans into it, but not yet stored
//! and not yet stamped with a fetch time. Persisting is [`crate::db`]'s job.

pub mod bwb;
pub mod publications;
pub mod rechtspraak;
pub mod wayback;

mod http;
mod xml;

pub use http::{Client, Fetched, FetchedBytes};

use chrono::NaiveDate;

use crate::blob::BlobId;
use crate::ids::{BwbId, Ecli, PublicationId};
use crate::model::{Relation, Span, Validity};

/// A document to parse, paired with everything needed to record where it came
/// from.
///
/// All four fields travel together because the provenance ledger needs the url
/// and the length, and a parse boundary that drops them leaves no path able to
/// write that ledger. Splitting the digest from the text is separately how
/// offsets end up pointing at nothing.
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

/// Where a stored row came from, carried alongside it so the two can be written
/// in one call and the ledger cannot be forgotten.
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

/// One published version of a law, as the manifest describes it.
///
/// Available without downloading the text, which is what makes both `--as-of`
/// resolution and update checking cheap.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionMeta {
    /// Upstream's label, e.g. `2026-07-16_0`.
    pub version: String,
    pub in_force: Validity,
    /// Absent when the manifest omits the zicht dates.
    ///
    /// Never defaulted to `in_force`: substituting one window for the other is
    /// exactly what [`crate::model::Validity`] exists to prevent, and a default
    /// performs that substitution silently.
    pub visible: Option<Validity>,
    /// The publisher's own digest of the version XML, when supplied.
    pub upstream_hash: Option<String>,
    pub byte_len: Option<u64>,
}

impl VersionMeta {
    /// True when this text applied before the register showed it.
    ///
    /// Retroactive effect is the ordinary case in tax legislation: 612 of the
    /// 733 versions of the Wet IB 2001 apply from before they were visible. It
    /// is what makes the two windows answer differently, and it is a fact read
    /// off the manifest rather than a judgement about which one governs.
    pub fn is_retroactive(&self) -> bool {
        self.visible
            .is_some_and(|zicht| zicht.from > self.in_force.from)
    }
}

/// A law and its articles, as published in a single version.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedLaw {
    pub id: BwbId,
    pub title: String,
    pub law_type: Option<String>,
    pub version: String,
    pub in_force: Validity,
    /// Absent when the manifest omits the zicht dates. See [`VersionMeta`].
    pub visible: Option<Validity>,
    pub upstream_hash: Option<String>,
    pub provenance: Provenance,
    pub articles: Vec<FetchedArticle>,
    /// Articles present in the source that yielded no text, and were therefore
    /// not stored. A legal corpus that drops articles must at least count them.
    pub skipped_articles: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FetchedArticle {
    /// Upstream's stable identity for this article across versions.
    ///
    /// Numbers repeat within a law and paths move when a law is restructured;
    /// this does neither. It is what makes a diff between two versions an
    /// identity match rather than a guess: of the 340 articles in BW6, 335
    /// carry the same stam-id in the 2023 text as in the 2026 one.
    pub stam_id: Option<String>,
    pub number: String,
    pub label: Option<String>,
    pub title: Option<String>,
    pub path: Option<String>,
    /// Rendered text, derived from the leden for reading and indexing.
    pub content: String,
    /// Byte range of the whole `<artikel>` element in the source document.
    pub span: Span,
    pub leden: Vec<FetchedLid>,
    /// The publication that put this article in this form, e.g. `stb-2023-41`.
    ///
    /// This is the amending act, which is where transitional law lives. bron
    /// does not read it; it makes it reachable.
    pub amended_by: Option<PublicationId>,
    /// What that publication did: `nieuwe-regeling`, `wijziging`, `nieuw`,
    /// `vervallen`. Upstream's vocabulary, unmapped.
    pub effect: Option<String>,
    /// The day this article took the form it has here.
    ///
    /// For a repealed provision it is the day it went, which is the difference
    /// between knowing a provision is gone and knowing when.
    pub in_force_from: Option<NaiveDate>,
    /// How many tables this article prints, so an answer can say its text is a
    /// linearisation rather than a grid.
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

#[derive(Debug, Clone, PartialEq)]
pub struct FetchedLid {
    pub number: Option<String>,
    pub content: String,
    /// Byte range of the `<lid>` element, or of the article's own text when the
    /// article has no lid structure.
    pub span: Span,
    /// The Juriconnect reference the register states for this lid.
    pub jci: Option<String>,
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

/// One capture of one page, as the archive replayed it.
///
/// Two dates matter here and they are not the same date. `id` carries when the
/// archive made the capture; the fetch time bron stamps on the way into the
/// corpus is when bron read it. An archive answer is bound to the first and
/// never to the second, which is the whole reason the lane exists.
///
/// `status` is what the *site* answered the *archive*, kept apart from anything
/// a direct fetch would report, because "the site told us" and "a third party
/// recorded" are different claims about different moments.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedCapture {
    pub id: crate::ids::CaptureId,
    /// The address captured, as written. Provenance; identity is `id`.
    pub original: String,
    pub status: wayback::ArchiveStatus,
    /// The archive's own hash of the payload, when the index carried one.
    pub digest: Option<String>,
    pub verified: wayback::Verified,
    /// True when the archive stored no new payload and replayed an earlier
    /// capture's bytes, so the content is older than this capture's timestamp.
    pub is_revisit: bool,
    /// The index's media type for the capture.
    pub mime_type: Option<String>,
    /// What the replay actually served it as.
    pub content_type: Option<String>,
    pub reading: wayback::Reading,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FetchedPublication {
    pub id: PublicationId,
    pub title: Option<String>,
    pub dossier: Option<String>,
    pub document_type: Option<String>,
    pub vergaderjaar: Option<String>,
    pub published_on: Option<NaiveDate>,
    pub url: Option<String>,
    pub full_text: Option<String>,
    /// Absent when the record links no XML body.
    pub provenance: Option<Provenance>,
}
