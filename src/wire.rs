//! The JSON contract.
//!
//! Domain types in [`crate::model`] deliberately do not implement `Serialize`.
//! Everything a consumer parses is defined here, so renaming a field in the
//! corpus cannot silently break a parser on the other side of a pipe: it breaks
//! the build instead.
//!
//! Two properties every answer carries, because a caller cannot infer either:
//!
//! - **Grade.** A statute or a ruling speaks with its own authority. An archive
//!   capture is a third party's record that a page said something on a day.
//!   Both are citable and they are not the same claim.
//! - **The date the answer is bound to.** For an as-of query that is the
//!   resolved date, never today.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use crate::blob::BlobId;
use crate::db::Document;
use crate::error::Fault;
use crate::model;
use crate::sources;

/// Bumped when a consumer would have to change to keep reading this.
pub const CONTRACT: u32 = 1;

/// Where an answer's authority comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Grade {
    /// The source speaks for itself: a statute, a ruling, a published document.
    Authority,
    /// A third party recorded that something said this on a date.
    Archive,
}

/// Which of the two validity windows a date was resolved against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    /// Geldigheid: what was in force. The question to ask about conduct.
    InForce,
    /// Zicht: what the register showed. The question to ask about what someone
    /// could have known at the time.
    AsKnown,
}

/// Every response, success or failure.
#[derive(Debug, Serialize)]
pub struct Envelope<T> {
    /// Contract version. A consumer that does not recognise it should refuse to
    /// parse rather than guess.
    pub bron: u32,
    pub grade: Grade,
    /// The date this answer is bound to, when it was resolved against one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<View>,
    /// Which route the bytes behind this answer took.
    ///
    /// Present only where bytes were resolved at all, and never inferred. A
    /// dossier must not be able to hide that its text came through an
    /// intermediary, even one whose bytes were checked against the publisher's
    /// own digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_via: Option<crate::mirror::ResolvedVia>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
    /// Present only when something about this answer limits what may be
    /// concluded from it. Empty is omitted so a caller can treat presence as
    /// the signal.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub caveats: Vec<Caveat>,
}

impl<T: Serialize> Envelope<T> {
    pub fn ok(data: T) -> Self {
        Self {
            bron: CONTRACT,
            grade: Grade::Authority,
            resolved_via: None,
            as_of: None,
            view: None,
            data: Some(data),
            error: None,
            caveats: Vec::new(),
        }
    }

    /// An answer that rests on a third party's record rather than on a source
    /// speaking for itself.
    ///
    /// A separate constructor rather than a setter on [`Envelope::ok`], so the
    /// grade is settled where the envelope is built and cannot be left at the
    /// default by an arm that forgot to downgrade it.
    pub fn archive(data: T) -> Self {
        Self {
            grade: Grade::Archive,
            ..Self::ok(data)
        }
    }

    pub fn bound_to(mut self, as_of: NaiveDate, view: View) -> Self {
        self.as_of = Some(as_of);
        self.view = Some(view);
        self
    }

    /// Binds an answer to a date that was not resolved against a validity
    /// window.
    ///
    /// An archive capture has no geldigheid and no zicht: it is one record
    /// taken at one moment, and there is no register whose windows it could be
    /// read against. Naming a [`View`] here would assert a distinction that
    /// does not exist for this kind of answer, so the field stays absent and a
    /// consumer reads its absence as "this date is the answer's own".
    pub fn dated(mut self, as_of: NaiveDate) -> Self {
        self.as_of = Some(as_of);
        self
    }

    pub fn with_caveats(mut self, caveats: Vec<Caveat>) -> Self {
        self.caveats = caveats;
        self
    }

    /// Records which route the bytes behind this answer took.
    pub fn via(mut self, route: crate::mirror::ResolvedVia) -> Self {
        self.resolved_via = Some(route);
        self
    }
}

impl Envelope<()> {
    /// A failure, graded by the lane that produced it.
    ///
    /// A failed archive lookup is not an authority answer that happened to go
    /// wrong. Nothing branches on this yet, but grade is the one field in the
    /// contract a consumer is told never to infer, and defaulting it here would
    /// be bron inferring it.
    pub fn failed(fault: &Fault, grade: Grade) -> Self {
        Self {
            bron: CONTRACT,
            grade,
            resolved_via: None,
            as_of: None,
            view: None,
            data: None,
            error: Some(ErrorBody {
                kind: fault.kind(),
                message: fault.to_string(),
                detail: fault.chain(),
            }),
            caveats: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    /// Stable tag matching the exit code. Branch on this, not on the message.
    pub kind: &'static str,
    pub message: String,
    /// The cause chain, outermost first.
    pub detail: Vec<String>,
}

/// A stated limit on what an answer supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Caveat {
    /// Ordering came from a relevance score that is not reproducible over time.
    UnstableOrdering,
    /// Filters ran over a capped window; matches may remain unseen.
    Truncated { window: u32 },
    /// Articles in the source yielded no text and were not stored.
    SkippedArticles { count: u32 },
    /// An article number named more than one article, and all of them are in
    /// the answer.
    ///
    /// Numbers are not unique within a law: the Awb restarts at Artikel 1 in
    /// each of its annexes, and those annex articles carry no label at all,
    /// only a number. Answering twice is right, because discarding one would
    /// be bron deciding which the caller meant. Answering twice without saying
    /// so is not: a reader who sees one heading "1" twice has no way to tell
    /// an ambiguous number from a duplicated row, and `paths` is what
    /// disambiguates it.
    AmbiguousArticle { number: String, paths: Vec<String> },
    /// An article in this answer prints a table, and its text carries one line
    /// per row with cells separated rather than a grid.
    ///
    /// Named per article because the reader who needs to know is the one
    /// quoting that article. Wet IB 2001 artikel 2.10 is the case to think
    /// about: linearised, every figure is present and no rate is tied to the
    /// band it applies to unless the reader knows to split the line.
    ///
    /// `cell_separator` is stated rather than assumed, so a consumer splits on
    /// a published value. The service emits the same one from the same
    /// reasoning; it is a contract between the two halves.
    TableFlattened {
        articles: Vec<String>,
        cell_separator: &'static str,
    },
    /// A corpus bron is meant to cover holds no vectors, so was not searched.
    ///
    /// A reader who asks a question about case law and receives only statutes
    /// has to be told the difference between 'the corpus holds no answer' and
    /// 'that half of it was never indexed'.
    LanesUnsearched { lanes: Vec<String> },
    /// An article in this answer has not commenced.
    ///
    /// The register publishes such a provision twice: once as the stub 'Dit
    /// onderdeel is nog niet inwerking getreden' and once as the text it will
    /// have, both under one `versie-id` and told apart only by `status`. bron
    /// holds both and answers with the text, which is the more useful of the
    /// two and the more dangerous: read without this it is a rule that does not
    /// yet apply, quoted as though it did.
    NotCommenced { articles: Vec<String> },
    /// An article in this answer prints an image, and bron holds no images.
    ///
    /// Not cosmetic. Where a provision states a formula, the register renders
    /// the formula itself as an image and states only its variables as text, so
    /// the stored article introduces a calculation, defines A, B and C, and
    /// never says what to do with them. Silence there reads as a complete
    /// provision.
    IllustrationOmitted { articles: Vec<String> },
    /// The bytes this answer quotes are not the bytes the publisher's manifest
    /// describes.
    ///
    /// Deliberately not fatal. The register disagrees with itself in the wild:
    /// for a large share of renditions the manifest's hash and size describe a
    /// gzip of the manifestation rather than what that manifestation's own URL
    /// serves, and BWBR0002638 is affected in every rendition it has. Refusing
    /// would make bron unable to read a law the publisher is willing to hand
    /// over. So the text is answered and the disagreement is stated, which
    /// leaves the reader to decide what it is worth.
    ///
    /// Distinct from [`Caveat::DigestUnverifiable`], which is the web archive's
    /// index digest. Two unrelated mechanisms, and collapsing them would make
    /// one verification story indistinguishable from the other.
    UpstreamDigestDiffers { published: String, ours: String },
    /// The manifest published no hash for this rendition, so these bytes rest
    /// on our own capture alone.
    ///
    /// A different fact from a hash that disagrees, and a consumer may care
    /// about the difference: 'nobody vouched for this' is not 'the voucher and
    /// the bytes do not match'.
    UpstreamDigestAbsent,
    /// The manifest carried no zicht dates, so no as-known-on query can be
    /// answered for this version.
    NoZichtWindow,
    /// This text applied from before the register showed it.
    Retroactive {
        in_force_from: NaiveDate,
        first_visible: NaiveDate,
    },
    /// The two windows resolve this date to different texts.
    ///
    /// Which one governs is a question of transitional law and not one bron
    /// answers. Both versions are named so the caller can fetch either and
    /// decide, rather than being handed one without being told the other exists.
    WindowsDisagree { in_force: String, as_known: String },
    /// The index recorded no digest for this capture, so the replayed bytes
    /// could not be checked against what the crawl stored. Not a failure and
    /// not a clean bill of health.
    DigestUnverifiable,
    /// The capture's bytes carried no usable encoding label and were read with
    /// an assumed one. Which rule fired is part of how the text traces back to
    /// the bytes.
    DecodedAs { encoding: String, declared: String },
    /// Bytes that could not be represented in the chosen encoding were replaced.
    LossyDecoding,
    /// The archive stored no new payload for this capture and replayed an
    /// earlier one's bytes. The content is older than the capture timestamp by
    /// however long the page sat unchanged.
    Revisit,
    /// The archive declines to say what the site answered, which is what it
    /// records for a robots-excluded crawl. There is a capture; what it
    /// returned is withheld.
    ArchiveStatusUnknown,
    /// This capture exists because bron asked for it, not because the archive
    /// already held it.
    RequestedByBron,
    /// The archive had more to say and bron stopped asking.
    MoreCaptures,
}

// -- documents -----------------------------------------------------------

/// Where a value came from, in the form a consumer can verify.
#[derive(Debug, Clone, Serialize)]
pub struct Provenance {
    /// SHA-256 of the frozen document, hex.
    pub blob: BlobId,
    pub url: String,
    pub byte_len: usize,
    pub fetched_at: DateTime<Utc>,
}

impl From<Document> for Provenance {
    fn from(doc: Document) -> Self {
        Self {
            blob: doc.blob,
            url: doc.url,
            byte_len: doc.byte_len,
            fetched_at: doc.fetched_at,
        }
    }
}

/// A byte range plus the bytes themselves.
///
/// `text` is the exact slice of the frozen document, and `rendered` is bron's
/// reading of it. They are different strings and both are needed: a consumer
/// verifying a quotation compares against `rendered`, and a consumer proving
/// what the publisher actually wrote slices the blob at `start..end` and gets
/// `text` back. Shipping only the offsets would mean a bron span could not
/// verify a bron quote.
#[derive(Debug, Clone, Serialize)]
pub struct Excerpt {
    pub start: u32,
    pub end: u32,
    /// The raw slice of the source document, markup included.
    pub text: String,
    /// bron's rendering of that slice, which is what `content` is built from.
    pub rendered: String,
}

impl Excerpt {
    /// Builds an excerpt, given the document the span points into.
    ///
    /// Returns `None` when the span does not lie inside the document, which
    /// means the blob is not the one the span was recorded against.
    pub fn new(span: model::Span, rendered: &str, document: &str) -> Option<Self> {
        Some(Self {
            start: span.start,
            end: span.end,
            text: span.slice(document)?.to_string(),
            rendered: rendered.to_string(),
        })
    }
}

// -- laws ----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Law {
    pub id: String,
    pub title: String,
    pub version_count: u32,
}

impl From<model::Law> for Law {
    fn from(law: model::Law) -> Self {
        Self {
            id: law.id.into_string(),
            title: law.title,
            version_count: law.version_count,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Window {
    pub from: NaiveDate,
    /// Absent when open-ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<NaiveDate>,
}

impl From<model::Validity> for Window {
    fn from(validity: model::Validity) -> Self {
        Self {
            from: validity.from,
            until: validity.until,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LawVersion {
    pub law_id: String,
    pub version: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub law_type: Option<String>,
    /// Geldigheid: when this text was in force.
    pub in_force: Window,
    /// Zicht: when the register showed it. Absent when upstream did not say,
    /// and never inferred from `in_force`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_known: Option<Window>,
    pub article_count: u32,
    /// The publisher's own digest, when the manifest supplied one. A consumer
    /// can check a frozen copy against the source rather than only against us.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_hash: Option<String>,
    pub provenance: Provenance,
}

/// One entry from a law's manifest: a version that exists, and whether the
/// corpus holds its text.
#[derive(Debug, Serialize)]
pub struct VersionSummary {
    pub version: String,
    pub in_force: Window,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_known: Option<Window>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_len: Option<u64>,
    /// True when this version's text is already stored locally.
    pub held: bool,
}

#[derive(Debug, Serialize)]
pub struct Article {
    pub law_id: String,
    pub version: String,
    pub number: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Structural path. Disambiguates a number an annex repeats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub content: String,
    pub excerpt: Excerpt,
    pub leden: Vec<Lid>,
    /// The publication that produced this text, e.g. `stb-2023-41`. Fetch it
    /// with `bron publication show` to read the transitional provisions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amended_by: Option<String>,
    /// What that publication did, in upstream's own words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    /// `nogniet` where the register published this provision as a stub because
    /// it has not commenced. Absent where the provision is ordinary.
    ///
    /// Carried because 'this provision does not exist' and 'this provision has
    /// not started' are different answers, and the stub text reads the same as
    /// a real one to anyone not looking for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// The day this article took the form it has here.
    ///
    /// For a repealed provision it is the day it went, which is the difference
    /// between knowing a provision is gone and knowing when. Read from the
    /// register's own `inwerking`, never derived from the rendition's window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_force_from: Option<NaiveDate>,
    /// The Juriconnect reference the register states for this article.
    ///
    /// The publisher's own citation string, taken rather than constructed, and
    /// carrying this rendition's zicht and geldigheid dates. It is what Dutch
    /// practice already cites with, so a consumer that speaks Juriconnect needs
    /// no mapping from ours.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jci: Option<String>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Serialize)]
pub struct Lid {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    pub content: String,
    pub excerpt: Excerpt,
    /// The Juriconnect reference the register states for this lid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jci: Option<String>,
}

/// One candidate for a question, named by identifier and never by text.
///
/// The service finds and the CLI proves, so a ranking answers with what a
/// citation is made of and leaves quoting to `law show`, which resolves it to
/// bytes with an offset. An approximate ordering is the one part of bron that
/// may not be quoted.
#[derive(Debug, Serialize)]
pub struct Hit {
    /// Which corpus this came from: `wetgeving` today, more later. Present from
    /// the first day so a consumer written now keeps working when a second
    /// corpus lands beside it.
    pub source: String,
    /// `article` or `lid`. Both are ranked, because a lid is the unit a
    /// citation names and an article is the unit a reader wants, and neither
    /// is derivable from the other.
    pub unit: String,
    pub law_id: String,
    pub version: String,
    pub number: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lid: Option<String>,
    /// SHA-256 of the text that matched, which is what `law show` resolves.
    pub content_hash: String,
    /// Cosine distance, reported as stored rather than as a percentage. A
    /// similarity rendered as 87% invites a threshold, and a threshold on an
    /// approximate ordering is a relevance judgement dressed as a fact.
    pub distance: f64,
}

/// A ranking, and which corpora it could actually have come from.
#[derive(Debug, Serialize)]
pub struct Ranking {
    /// The corpora that held vectors. Anything bron is meant to cover and is
    /// missing here was not searched, which is a different answer from finding
    /// nothing in it.
    pub searched: Vec<String>,
    pub hits: Vec<Hit>,
}

impl Article {
    /// Renders an article against the document it was parsed from.
    pub fn new(article: model::Article, document: &str, provenance: Provenance) -> Option<Self> {
        Some(Self {
            law_id: article.law_id.into_string(),
            version: article.version,
            number: article.number,
            label: article.label,
            title: article.title,
            path: article.path,
            excerpt: Excerpt::new(article.span, &article.content, document)?,
            content: article.content,
            amended_by: article.amended_by.map(|p| p.into_string()),
            effect: article.effect,
            in_force_from: article.in_force_from,
            jci: article.jci,
            status: article.status,
            leden: article
                .leden
                .into_iter()
                .map(|lid| {
                    Some(Lid {
                        excerpt: Excerpt::new(lid.span, &lid.content, document)?,
                        number: lid.number,
                        content: lid.content,
                        jci: lid.jci,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
            provenance,
        })
    }
}

/// What happened to one article between two versions of a law.
///
/// Both sides carry their own text, digest and offsets, from two different
/// frozen documents. A consumer never has to trust our comparison: it can run
/// its own over `before.content` and `after.content`, or verify either against
/// the bytes its span points at.
#[derive(Debug, Serialize)]
pub struct ArticleChange {
    /// `unchanged`, `amended`, `added`, `repealed` or `withdrawn`.
    pub change: &'static str,
    pub number: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Side>,
}

/// One article as it stood in one version.
#[derive(Debug, Serialize)]
pub struct Side {
    pub version: String,
    pub content: String,
    pub excerpt: Excerpt,
    /// The act that produced this text. Where transitional law lives.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amended_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    pub provenance: Provenance,
}

/// A comparison of two versions of one law.
#[derive(Debug, Serialize)]
pub struct Changes {
    pub law_id: String,
    pub from: ComparedVersion,
    pub to: ComparedVersion,
    pub articles: Vec<ArticleChange>,
    /// Counts per outcome, so a caller can size the answer before reading it.
    pub summary: ChangeSummary,
}

#[derive(Debug, Serialize)]
pub struct ComparedVersion {
    pub version: String,
    pub in_force: Window,
    pub blob: BlobId,
}

#[derive(Debug, Default, Serialize)]
pub struct ChangeSummary {
    pub unchanged: u32,
    pub amended: u32,
    pub added: u32,
    pub repealed: u32,
    pub withdrawn: u32,
}

// -- decisions -----------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Case {
    pub ecli: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub court: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_on: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_on: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_text: Option<String>,
    pub relations: Vec<Relation>,
    pub appeal: AppealStatus,
    pub provenance: Provenance,
}

#[derive(Debug, Serialize)]
pub struct Relation {
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub direction: &'static str,
}

impl From<model::Relation> for Relation {
    fn from(relation: model::Relation) -> Self {
        Self {
            target: relation.target.into_string(),
            kind: relation.kind,
            direction: relation.direction.as_str(),
        }
    }
}

/// Three-valued, never a boolean.
///
/// `no_published_successor` is a claim about what bron holds on the date it
/// last looked, not a claim that no appeal was brought. An appeal that settled,
/// was withdrawn, or ended in an unpublished decision leaves no trace.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum AppealStatus {
    Appealed { successors: Vec<String> },
    NoPublishedSuccessor { as_of: DateTime<Utc> },
    Unknown,
}

impl From<model::AppealStatus> for AppealStatus {
    fn from(status: model::AppealStatus) -> Self {
        match status {
            model::AppealStatus::Appealed { successors } => Self::Appealed {
                successors: successors.into_iter().map(|e| e.into_string()).collect(),
            },
            model::AppealStatus::NoPublishedSuccessor { as_of } => {
                Self::NoPublishedSuccessor { as_of }
            }
            model::AppealStatus::Unknown => Self::Unknown,
        }
    }
}

// -- search hits ---------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct LawHit {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub law_type: Option<String>,
}

impl From<model::LawHit> for LawHit {
    fn from(hit: model::LawHit) -> Self {
        Self {
            id: hit.id.into_string(),
            title: hit.title,
            law_type: hit.law_type,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CaseHit {
    pub ecli: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_on: Option<NaiveDate>,
}

impl From<model::CaseHit> for CaseHit {
    fn from(hit: model::CaseHit) -> Self {
        Self {
            ecli: hit.id.into_string(),
            title: hit.title,
            summary: hit.summary,
            decided_on: hit.decided_on,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PublicationHit {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dossier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_on: Option<NaiveDate>,
}

impl From<model::PublicationHit> for PublicationHit {
    fn from(hit: model::PublicationHit) -> Self {
        Self {
            id: hit.id.into_string(),
            title: hit.title,
            dossier: hit.dossier,
            published_on: hit.published_on,
        }
    }
}

// -- captures ------------------------------------------------------------

/// Whether a capture holds the page itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Captured {
    Yes,
    No,
    /// The archive withholds what the site answered, so this is not knowable.
    /// A consumer must not read it as `no`.
    Unknown,
}

impl From<model::Captured> for Captured {
    fn from(captured: model::Captured) -> Self {
        match captured {
            model::Captured::Yes => Self::Yes,
            model::Captured::No => Self::No,
            model::Captured::Unknown => Self::Unknown,
        }
    }
}

/// How a capture's bytes stand against the digest the index recorded.
///
/// `matches` proves the replay is byte-identical to what the crawl stored.
/// `differs` proves nothing: the index digest covers the payload as stored,
/// chunk framing included, and the replay strips that before serving, so the
/// two disagree routinely on sound captures. `absent` means the index carried
/// no digest. None of the three is a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestCheck {
    Matches,
    Differs,
    Absent,
}

impl From<model::DigestCheck> for DigestCheck {
    fn from(check: model::DigestCheck) -> Self {
        match check {
            model::DigestCheck::Matches => Self::Matches,
            model::DigestCheck::Differs => Self::Differs,
            model::DigestCheck::Absent => Self::Absent,
        }
    }
}

/// One capture as the index describes it, without its bytes.
#[derive(Debug, Serialize)]
pub struct CaptureHit {
    /// The archive's identifier for this capture.
    pub id: String,
    /// The address to cite, which is the page a reader should open.
    pub url: String,
    /// When the archive took the capture. Never today.
    pub captured_at: DateTime<Utc>,
    pub original_url: String,
    /// What the site answered the archive. Absent means the archive declines
    /// to say, not that there is no capture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_status: Option<u16>,
    /// Whether the capture holds the page itself. `unknown` when the archive
    /// withholds the status, which is not the same claim as `no`.
    pub captured_the_page: Captured,
    /// The archive's own hash of the payload. Two captures sharing it are the
    /// same bytes, which is change detection without a single fetch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// The content is older than `captured_at`; see [`Caveat::Revisit`].
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub revisit: bool,
}

impl From<&sources::wayback::IndexRow> for CaptureHit {
    fn from(row: &sources::wayback::IndexRow) -> Self {
        Self {
            id: row.id.to_string(),
            url: row.id.citation_url(&row.original),
            captured_at: row.id.captured_at(),
            original_url: row.original.clone(),
            archive_status: row.status.code(),
            captured_the_page: row.status.captured_the_page().into(),
            digest: row.digest.clone(),
            mime_type: row.mime_type.clone(),
            revisit: row.is_revisit(),
        }
    }
}

/// One capture, read.
#[derive(Debug, Serialize)]
pub struct Capture {
    pub id: String,
    pub url: String,
    pub captured_at: DateTime<Utc>,
    pub original_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_status: Option<u16>,
    pub captured_the_page: Captured,
    /// How the bytes stand against the digest the index recorded. See
    /// [`DigestCheck`]: only `matches` is positive evidence, and none of the
    /// three is a failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<DigestCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub revisit: bool,
    /// bron asked the archive for this capture rather than finding it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub requested_by_bron: bool,
    /// Absent when the capture is not text, which is not a failure: a PDF is
    /// evidence as bytes and a digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// When bron read it, which is not when it was captured.
    pub fetched_at: DateTime<Utc>,
}

impl From<model::Capture> for Capture {
    fn from(capture: model::Capture) -> Self {
        // Read before the struct is taken apart, since it borrows the whole.
        let captured_the_page = capture.captured_the_page();

        Self {
            id: capture.id.to_string(),
            url: capture.citation_url(),
            captured_at: capture.captured_at,
            original_url: capture.original_url,
            archive_status: capture.archive_status,
            captured_the_page: captured_the_page.into(),
            verified: capture.verified.map(Into::into),
            digest: capture.index_digest,
            mime_type: capture.mime_type,
            revisit: capture.is_revisit,
            requested_by_bron: capture.requested,
            content: capture.content,
            encoding: capture.encoding,
            fetched_at: capture.fetched_at,
        }
    }
}

// -- housekeeping --------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Stats {
    pub path: String,
    pub laws: u32,
    pub versions: u32,
    pub articles: u32,
    pub cases: u32,
    pub publications: u32,
    pub documents: u32,
}

impl From<crate::db::Stats> for Stats {
    fn from(stats: crate::db::Stats) -> Self {
        Self {
            path: stats.path.display().to_string(),
            laws: stats.laws,
            versions: stats.versions,
            articles: stats.articles,
            cases: stats.cases,
            publications: stats.publications,
            documents: stats.documents,
        }
    }
}

/// What `bron verify` can and cannot say.
///
/// Every count here is about the corpus alone: whether the bytes on this disk
/// still hash to the name they are filed under. It says nothing about whether
/// those bytes are the ones the publisher vouched for. A document whose
/// upstream digest disagrees is `intact` all the same, because the two are
/// different claims and collapsing them is the inference this crate exists to
/// refuse.
#[derive(Debug, Serialize)]
pub struct Verification {
    pub documents: u32,
    pub intact: u32,
    pub missing: Vec<String>,
    pub corrupt: Vec<String>,
}

impl Verification {
    pub fn is_intact(&self) -> bool {
        self.missing.is_empty() && self.corrupt.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_carries_the_contract_version() {
        let json = serde_json::to_string(&Envelope::ok(42)).unwrap();
        assert!(
            json.starts_with(r#"{"bron":1,"grade":"authority""#),
            "{json}"
        );
    }

    #[test]
    fn an_as_of_answer_states_which_window_it_used() {
        let date: NaiveDate = "2023-06-01".parse().unwrap();
        let json = serde_json::to_string(&Envelope::ok(1).bound_to(date, View::InForce)).unwrap();
        assert!(json.contains(r#""as_of":"2023-06-01""#), "{json}");
        assert!(json.contains(r#""view":"in_force""#), "{json}");
    }

    #[test]
    fn a_failure_is_still_valid_json_with_a_branchable_kind() {
        let fault = Fault::not_found("law", "BWBR0000001");
        let json = serde_json::to_string(&Envelope::failed(&fault, Grade::Authority)).unwrap();
        assert!(json.contains(r#""kind":"not_found""#), "{json}");
        assert!(
            !json.contains(r#""data""#),
            "no data key on a failure: {json}"
        );
    }

    #[test]
    fn absent_optionals_are_omitted_rather_than_null() {
        let json = serde_json::to_string(&Envelope::ok(1)).unwrap();
        assert!(!json.contains("null"), "{json}");
        assert!(
            !json.contains("caveats"),
            "an empty caveat list is not shipped: {json}"
        );
    }

    #[test]
    fn an_excerpt_carries_both_the_raw_slice_and_the_rendering() {
        let document = "<lid><lidnr>1</lidnr><al>Eerste lid.</al></lid>";
        let span = model::Span {
            start: 0,
            end: document.len() as u32,
        };
        let excerpt = Excerpt::new(span, "1. Eerste lid.", document).unwrap();

        // The consumer can verify the quote against the rendering, and prove the
        // publisher's own bytes by slicing the blob at the same offsets.
        assert_eq!(excerpt.rendered, "1. Eerste lid.");
        assert_eq!(excerpt.text, document);
        assert_eq!(
            &document[excerpt.start as usize..excerpt.end as usize],
            excerpt.text
        );
    }

    #[test]
    fn an_excerpt_against_the_wrong_document_is_refused() {
        let span = model::Span { start: 0, end: 500 };
        assert!(Excerpt::new(span, "x", "short document").is_none());
    }

    #[test]
    fn a_failed_archive_lookup_is_not_graded_as_an_authority() {
        let fault = Fault::not_found("capture", "https://example.com");
        let envelope = Envelope::failed(&fault, Grade::Archive);

        // Grade is the one field a consumer is told never to infer, so a
        // failure carries the lane's own grade rather than a default.
        assert_eq!(envelope.grade, Grade::Archive);
    }
}
