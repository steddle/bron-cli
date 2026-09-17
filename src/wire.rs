//! The JSON contract.
//!
//! Domain types in [`crate::model`] deliberately do not implement `Serialize`.
//! Everything a consumer parses is defined here, so renaming a field in the
//! corpus cannot silently break a parser on the other side of a pipe: it breaks
//! the build instead.
//!
//! Two properties every answer carries, because a caller cannot infer either:
//!
//! - **Grade.** Where an answer's authority comes from. Every source bron
//!   reads speaks with its own authority, so today this is always
//!   `authority`; it is stated rather than left implicit so a consumer never
//!   has to infer it.
//! - **The date the answer is bound to.** For an as-of query that is the
//!   resolved date, never today.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use crate::blob::BlobId;
use crate::error::Fault;
use crate::model;
use crate::sources;

/// Bumped when a consumer would have to change to keep reading this.
pub const CONTRACT: u32 = 1;

/// Where an answer's authority comes from.
///
/// One value, kept on the wire so a consumer reads it rather than infers it:
/// a claim's grade is the one field the contract says never to guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Grade {
    /// The source speaks for itself: a statute, a ruling, a published document.
    Authority,
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
            as_of: None,
            view: None,
            data: Some(data),
            error: None,
            caveats: Vec::new(),
        }
    }

    pub fn bound_to(mut self, as_of: NaiveDate, view: View) -> Self {
        self.as_of = Some(as_of);
        self.view = Some(view);
        self
    }

    pub fn with_caveats(mut self, caveats: Vec<Caveat>) -> Self {
        self.caveats = caveats;
        self
    }
}

impl Envelope<()> {
    /// A failure, in the same envelope as any answer.
    pub fn failed(fault: &Fault) -> Self {
        Self {
            bron: CONTRACT,
            grade: Grade::Authority,
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
///
/// Most of these now arrive from the service, which is where the corpus and the
/// arithmetic over it live. [`Caveat::stated`] is the one place that translates
/// them, so a limit the service knows about and this crate does not is carried
/// under its own name rather than dropped.
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
    /// `cell_separator` is carried from the service rather than restated here,
    /// so a consumer splits on the value that actually produced the line.
    TableFlattened {
        articles: Vec<String>,
        cell_separator: String,
    },
    /// A corpus bron is meant to cover holds no vectors, so was not searched.
    ///
    /// A reader who asks a question about case law and receives only statutes
    /// has to be told the difference between 'the corpus holds no answer' and
    /// 'that half of it was never indexed'.
    LanesUnsearched { lanes: Vec<String> },

    /// Texts in a searched lane that carry no vector, because the model will
    /// not take them in one input, so no ranking can reach them.
    ///
    /// Counted over the corpus and not over this answer, which is what `scope`
    /// says: the same texts are out of every ranking, so narrowing by
    /// regulation or by date does not narrow the number. They stay answerable
    /// by identifier, which is the route that does not go through a vector.
    UnrankableTexts { scope: String, count: u32 },

    /// The citation reads more than one way and every reading is answered.
    ///
    /// Not [`Caveat::AmbiguousRendition`]: nothing is discarded here. `3:305a`
    /// is Boek 3 artikel 305a and is also, in a law that prints the colon
    /// itself, artikel 3:305a, and which the reader meant is theirs to say.
    SeveralReadings { readings: Vec<String> },

    /// The service said two things about one reading of a citation that cannot
    /// both be true, so the reading is answered as though its text is not held.
    ///
    /// `held` and the three fields that belong to it travel separately on the
    /// wire, so the two can disagree: a rendition said to be held with no hash
    /// to resolve, or one said not to be held with a hash beside it. Answering
    /// either as though nothing were wrong would put a reader one step from a
    /// citation nobody can check, and dropping the whole answer would take the
    /// other readings of the citation with it.
    ReadingContradicted { reading: String, stated: String },

    /// A reading of a citation that came to nothing, and why.
    ///
    /// `because` is the service's own word: `not_in_corpus`,
    /// `no_regulation_prints_it`, `no_rendition_in_force`, `withdrawn`,
    /// `not_carried`. Four of those would otherwise render as one absence, and
    /// a reader handed the other reading alone has to be told this one was
    /// looked for.
    ReadingUnresolved {
        reading: String,
        because: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        regulation_id: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        renditions: Vec<String>,
    },

    /// The answer is drawn from the regulations whose text the corpus holds,
    /// and says how many that is of the ones it knows. Four held out of 25.462
    /// known is four of an unknown number, and nothing else in an answer can
    /// tell a caller that.
    HeldTextOnly {
        held_regulations: u32,
        known_regulations: u32,
    },
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
    /// Neither digest travels. The comparison is the service's, made against
    /// bytes it holds, and printing two hashes here would read as bron having
    /// hashed something.
    UpstreamDigestDiffers,
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
    /// Renditions the publisher has withdrawn cover this date and were passed
    /// over, whatever their windows say.
    ///
    /// The reason this lane goes through the service at all. A withdrawn
    /// rendition keeps its dates in the manifest, so it wins an ordering by
    /// date and is still not what the register says applies; asking the
    /// publisher for its text ends in a redirect loop, because the text is no
    /// longer there. They are named so a reader can see which they were.
    WithdrawnRenditions { excluded: Vec<String> },

    /// More than one published rendition covers this date, which the register
    /// does. The answer is the one that commenced last; the rest are named so a
    /// caller can fetch either and decide.
    AmbiguousRendition {
        chosen: String,
        discarded: Vec<String>,
    },

    /// The register has since revised what it shows for this date, so this is
    /// what was believed then rather than what is believed now.
    SupersededKnowledge { visible_until: NaiveDate },

    /// The bytes beside each span are the service's word for what stands there.
    ///
    /// bron used to cut them out of a document whose digest it had checked, and
    /// refused outright a span that did not fit that document. It no longer
    /// holds the document: checking this text means fetching the blob it came
    /// out of, and not having to is the whole reason it travels. Being handed a
    /// slice and having cut one are different facts, and only one of them is a
    /// proof.
    UnverifiedSlice,

    /// Articles a comparison could not place, and why.
    ///
    /// `unidentified_from` and `unidentified_to` count articles carrying no
    /// `stam-id`, one count per rendition. Nothing identifies them, so whether
    /// the one in the earlier rendition is the one in the later is exactly what
    /// the register declined to say, and one total would read as two articles.
    ///
    /// `repeated` are stam-ids the register used more than once inside one
    /// rendition, where a match would have to pick one and calling that an
    /// identity would be a guess.
    ///
    /// Both are left out of the comparison and out of its counts, which is why
    /// this has to be said: without it `law changes` answers "nothing changed"
    /// where the answer is "this could not be compared".
    ArticlesNotCompared {
        unidentified_from: u32,
        unidentified_to: u32,
        repeated: Vec<String>,
    },

    /// The service stated a limit this bron has no name for.
    ///
    /// Carried rather than dropped. A limit on what an answer supports that
    /// goes unsaid because the reader is one release behind is the failure
    /// every caveat here exists to prevent.
    Unrecognised { stated: String },
}

impl Caveat {
    /// One limit the service stated, read into this crate's vocabulary.
    pub fn stated(value: serde_json::Value) -> Self {
        #[derive(serde::Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Stated {
            Withdrawn {
                excluded: Vec<String>,
            },
            Ambiguous {
                chosen: String,
                discarded: Vec<String>,
            },
            NoZichtWindow,
            Retroactive {
                in_force_from: NaiveDate,
                first_visible: NaiveDate,
            },
            SupersededKnowledge {
                visible_until: NaiveDate,
            },
            Unverified,
            HashDisagrees,
            Truncated {
                window: u32,
            },
            SkippedArticles {
                count: u32,
            },
            TableFlattened {
                articles: Vec<String>,
                cell_separator: String,
            },
            NotCommenced {
                articles: Vec<String>,
            },
            IllustrationOmitted {
                articles: Vec<String>,
            },
            LanesUnsearched {
                lanes: Vec<String>,
            },
            UnrankableTexts {
                scope: String,
                count: u32,
            },
            SeveralReadings {
                readings: Vec<String>,
            },
            ReadingUnresolved {
                reading: String,
                because: String,
                #[serde(default)]
                regulation_id: Option<String>,
                #[serde(default)]
                renditions: Vec<String>,
            },
            HeldTextOnly {
                held_regulations: u32,
                known_regulations: u32,
            },
        }

        // Read before the value is consumed, so an unrecognised limit can still
        // be reported by the name the service gave it.
        let kind = value
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("a limit with no kind")
            .to_string();

        match serde_json::from_value::<Stated>(value) {
            Ok(Stated::Withdrawn { excluded }) => Self::WithdrawnRenditions { excluded },
            Ok(Stated::Ambiguous { chosen, discarded }) => {
                Self::AmbiguousRendition { chosen, discarded }
            }
            Ok(Stated::NoZichtWindow) => Self::NoZichtWindow,
            Ok(Stated::Retroactive {
                in_force_from,
                first_visible,
            }) => Self::Retroactive {
                in_force_from,
                first_visible,
            },
            Ok(Stated::SupersededKnowledge { visible_until }) => {
                Self::SupersededKnowledge { visible_until }
            }
            Ok(Stated::Unverified) => Self::UpstreamDigestAbsent,
            Ok(Stated::HashDisagrees) => Self::UpstreamDigestDiffers,
            Ok(Stated::Truncated { window }) => Self::Truncated { window },
            Ok(Stated::SkippedArticles { count }) => Self::SkippedArticles { count },
            Ok(Stated::UnrankableTexts { scope, count }) => Self::UnrankableTexts { scope, count },
            Ok(Stated::TableFlattened {
                articles,
                cell_separator,
            }) => Self::TableFlattened {
                articles,
                cell_separator,
            },
            Ok(Stated::NotCommenced { articles }) => Self::NotCommenced { articles },
            Ok(Stated::IllustrationOmitted { articles }) => Self::IllustrationOmitted { articles },
            Ok(Stated::LanesUnsearched { lanes }) => Self::LanesUnsearched { lanes },
            Ok(Stated::SeveralReadings { readings }) => Self::SeveralReadings { readings },
            Ok(Stated::ReadingUnresolved {
                reading,
                because,
                regulation_id,
                renditions,
            }) => Self::ReadingUnresolved {
                reading,
                because,
                regulation_id,
                renditions,
            },
            Ok(Stated::HeldTextOnly {
                held_regulations,
                known_regulations,
            }) => Self::HeldTextOnly {
                held_regulations,
                known_regulations,
            },
            Err(_) => Self::Unrecognised { stated: kind },
        }
    }
}

// -- documents -----------------------------------------------------------

/// Who froze the bytes an answer rests on.
///
/// Two facts, said outright rather than left to be inferred from whether an
/// unrelated timestamp happens to be there. What a consumer may conclude from a
/// span differs between them: one is bron's own reading of bytes it hashed, the
/// other is the service's word for what stands at an offset.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "frozen_by")]
pub enum Frozen {
    /// bron. It fetched these bytes, filed them under the digest of themselves,
    /// and knows when it did because it did it.
    Bron { fetched_at: DateTime<Utc> },
    /// The service. bron has the digest and the address and has not read a
    /// byte: checking one means fetching the blob, and not having to is what
    /// this shape buys.
    ///
    /// No moment travels with it, and none should. The service does not say
    /// when it froze a document, deliberately: a reader who can see when
    /// something was fetched can see what is being worked on.
    Service,
}

impl Frozen {
    /// Whether a slice cut from this document is a report rather than a cut
    /// bron made. See [`Caveat::UnverifiedSlice`].
    pub fn reports(&self) -> bool {
        matches!(self, Self::Service)
    }
}

/// Where a value came from, in the form a consumer can verify.
#[derive(Debug, Clone, Serialize)]
pub struct Provenance {
    /// SHA-256 of the document, hex.
    pub blob: BlobId,
    pub url: String,
    pub byte_len: usize,
    #[serde(flatten)]
    pub frozen: Frozen,
}

impl Provenance {
    /// Provenance for a document the service froze and serves, named by its
    /// digest.
    pub fn served(blob: BlobId, url: String, byte_len: usize) -> Self {
        Self {
            blob,
            url,
            byte_len,
            frozen: Frozen::Service,
        }
    }

    /// Provenance for a document bron fetched and froze itself, stamped with
    /// the moment it did.
    pub fn frozen(source: &sources::Provenance, fetched_at: DateTime<Utc>) -> Self {
        Self {
            blob: source.blob,
            url: source.url.clone(),
            byte_len: source.byte_len,
            frozen: Frozen::Bron { fetched_at },
        }
    }
}

/// Where a value sits in the frozen document, and optionally what stands there.
///
/// The offsets are the answer. `blob` in the same provenance names a document
/// whose address is the hash of its own content, so a consumer that wants to
/// prove a quotation fetches those bytes and slices them at `start..end`.
///
/// `text` is that slice, sent only when `--raw` asks for it, and it is the
/// service's word for what stands there rather than a cut bron made. bron used
/// to take the slice itself out of a document whose digest it had checked, and
/// refused a span that did not fit. It no longer holds that document: checking
/// this string means fetching the blob it came out of, and not having to is
/// exactly why it travels. [`Caveat::UnverifiedSlice`] says so on any answer
/// that carries one.
#[derive(Debug, Clone, Serialize)]
pub struct Excerpt {
    pub start: u32,
    pub end: u32,
    /// The publisher's own bytes at this range, markup included. Absent unless
    /// `--raw`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Excerpt {
    /// Whether this excerpt carries the bytes at its range, rather than only
    /// pointing at them.
    pub fn quotes(&self) -> bool {
        self.text.is_some()
    }
}

// -- laws ----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Window {
    pub from: NaiveDate,
    /// Absent when open-ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<NaiveDate>,
}

impl From<crate::service::Window> for Window {
    fn from(window: crate::service::Window) -> Self {
        Self {
            from: window.from,
            until: window.until,
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

/// Whether the rendition a reading resolved to has its text held, and what
/// that makes available.
///
/// One type rather than a flag beside three optional fields. `held: true` with
/// no `content_hash` is not a state that can exist, and saying so in prose left
/// sixteen combinations spellable for two facts.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "held")]
pub enum Held {
    /// It is, and it carries this article. `content_hash` is the SHA-256 of the
    /// text, which is what `law show` resolves.
    Text {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        content_hash: String,
    },
    /// It is not. An answer and not a miss: the dates resolved and the
    /// rendition is named, and whether it carries this article is a fact nobody
    /// has yet.
    None,
}

/// One reading of a citation, resolved to an article in a rendition.
///
/// No `distance`, and there will not be one. A ranking orders candidates by a
/// number that only orders; a citation is looked up, and arrives or does not.
/// Putting a distance here would say the answer came about a way it did not,
/// and would invite a threshold on something that is not an ordering.
#[derive(Debug, Serialize)]
pub struct Cited {
    /// How the citation was read. `book` takes the first segment as a book of
    /// the Burgerlijk Wetboek, so `6:162` is Boek 6 artikel 162; `literal`
    /// takes the whole number as one a regulation prints itself, which the Awb
    /// and the Wft do. Which the reader meant is theirs to say.
    pub reading: String,
    /// Which corpus this came from: `wetgeving` today, more later.
    pub source: String,
    pub law_id: String,
    pub version: String,
    /// The number as that regulation prints it, which for a book reading is the
    /// citation with its book taken off.
    pub number: String,
    #[serde(flatten)]
    pub held: Held,
    /// What limits this reading in particular, as against the answer as a
    /// whole. The rendition it resolved to may be one the publisher does not
    /// vouch for, or one of several covering the date.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub caveats: Vec<Caveat>,
}

/// A citation, and every reading of it that resolved.
#[derive(Debug, Serialize)]
pub struct Lookup {
    /// The citation as the service read it, which is not always as it was
    /// typed: `6:162 BW` pins the book and reads as `6:162`.
    pub citation: String,
    /// In the order the service read them. An answer with more than one is a
    /// citation that reads more than one way, and picking one of them would be
    /// bron deciding what it means.
    pub resolved: Vec<Cited>,
}

/// What a term turned out to be.
///
/// Two shapes rather than one with an optional field, and a tag rather than a
/// key a consumer has to test for. A ranking is an approximate ordering over
/// vectors and a lookup is arithmetic over identifiers; the one part of bron
/// that may not be quoted sits in the first and not in the second, and a
/// consumer branches on `kind` to know which it is holding.
///
/// `kind` is the word [`Caveat`] and [`ErrorBody`] already use for the value a
/// consumer switches on, and the word the service publishes for this one. One
/// vocabulary at both ends of the pipe.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Found {
    Ranking(Ranking),
    Lookup(Lookup),
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

impl Article {
    /// Whether this answer carries bytes bron did not cut out of a document it
    /// hashed. See [`Caveat::UnverifiedSlice`].
    ///
    /// Read off the answer rather than off the flag that asked for it: a caveat
    /// raised from `--raw` says something about the request, and only this says
    /// something about what is in the envelope.
    pub fn reports_a_slice(&self) -> bool {
        self.provenance.frozen.reports()
            && (self.excerpt.quotes() || self.leden.iter().any(|lid| lid.excerpt.quotes()))
    }
}

impl Side {
    /// See [`Article::reports_a_slice`].
    pub fn reports_a_slice(&self) -> bool {
        self.provenance.frozen.reports() && self.excerpt.quotes()
    }
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

/// What the decision's own relations say, and never a boolean.
///
/// `no_published_successor` is a claim about what the register held at the
/// moment bron read it, dated to that moment, and not a claim that no appeal
/// was brought. An appeal that settled, was withdrawn, or ended in an
/// unpublished decision leaves no trace.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum AppealStatus {
    Appealed { successors: Vec<String> },
    NoPublishedSuccessor { as_of: DateTime<Utc> },
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

/// Whether the register links a document for a publication.
///
/// Said outright rather than left to a missing key. The digest is how an answer
/// from this lane is checked at all: a reader fetches the address beside it and
/// hashes what arrives. A record the register publishes without a body has
/// nothing to hash, and simply leaving the provenance out would read as an
/// answer whose digest went missing.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "body")]
pub enum Body {
    /// The register links an XML document. This is where those bytes came from
    /// and what they hash to.
    Linked { provenance: Provenance },
    /// The register links none. The record is the whole of what it publishes
    /// under this identifier, so there is nothing to fetch and nothing to hash.
    Absent,
}

/// One publication, and the bytes behind it where there are any.
#[derive(Debug, Serialize)]
pub struct Publication {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dossier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_on: Option<NaiveDate>,
    #[serde(flatten)]
    pub body: Body,
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

// -- housekeeping --------------------------------------------------------

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
        let json = serde_json::to_string(&Envelope::failed(&fault)).unwrap();
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

    /// Offsets alone by default. The slice is markup, it is megabytes over a
    /// whole law, and a consumer that wants it asks.
    #[test]
    fn an_excerpt_carries_its_offsets_and_nothing_else_unasked() {
        let excerpt = Excerpt {
            start: 7108,
            end: 15301,
            text: None,
        };
        let json = serde_json::to_string(&excerpt).unwrap();

        assert_eq!(json, r#"{"start":7108,"end":15301}"#);
    }

    #[test]
    fn a_slice_that_was_asked_for_travels_beside_its_offsets() {
        let excerpt = Excerpt {
            start: 0,
            end: 46,
            text: Some("<lid><lidnr>1</lidnr><al>Eerste lid.</al></lid>".into()),
        };
        let json = serde_json::to_string(&excerpt).unwrap();

        assert!(json.contains(r#""text":"<lid>"#), "{json}");
    }

    /// A limit the service knows about and this crate does not is carried under
    /// the name the service gave it. Dropping it would leave a reader one
    /// release behind unable to see that anything was said.
    #[test]
    fn a_limit_with_no_name_here_is_still_reported() {
        let stated = serde_json::json!({"kind": "something_later", "detail": 3});

        assert_eq!(
            Caveat::stated(stated),
            Caveat::Unrecognised {
                stated: "something_later".into()
            }
        );
    }

    /// The count of texts no ranking reaches is read under its own name, and
    /// its scope with it: the number is of the corpus, so a search narrowed to
    /// one regulation reports the same figure and has to say why.
    #[test]
    fn texts_no_ranking_reaches_are_read_with_their_scope() {
        assert_eq!(
            Caveat::stated(serde_json::json!({
                "kind": "unrankable_texts",
                "scope": "corpus",
                "count": 64
            })),
            Caveat::UnrankableTexts {
                scope: "corpus".into(),
                count: 64
            }
        );
    }

    /// A rendition that lost articles says how many. Read as `unrecognised` it
    /// would tell a reader on a current build to upgrade.
    #[test]
    fn articles_a_rendition_could_not_hold_are_counted() {
        assert_eq!(
            Caveat::stated(serde_json::json!({"kind": "skipped_articles", "count": 14})),
            Caveat::SkippedArticles { count: 14 }
        );
    }

    /// The service's two hash caveats answer different questions, and a
    /// consumer may care about one and not the other.
    #[test]
    fn the_two_hash_limits_stay_apart() {
        assert_eq!(
            Caveat::stated(serde_json::json!({"kind": "unverified"})),
            Caveat::UpstreamDigestAbsent
        );
        assert_eq!(
            Caveat::stated(serde_json::json!({"kind": "hash_disagrees"})),
            Caveat::UpstreamDigestDiffers
        );
    }

    #[test]
    fn a_withdrawn_rendition_is_named_rather_than_only_counted() {
        assert_eq!(
            Caveat::stated(serde_json::json!({
                "kind": "withdrawn",
                "excluded": ["2017-07-01_0"],
            })),
            Caveat::WithdrawnRenditions {
                excluded: vec!["2017-07-01_0".into()]
            }
        );
    }

    /// A held rendition and an unheld one are two states, and one type says
    /// which. A flag beside three optional fields left sixteen combinations
    /// spellable, fourteen of which cannot happen.
    #[test]
    fn a_reading_says_what_the_service_holds_for_it() {
        let cited = |held| Cited {
            reading: "book".into(),
            source: "wetgeving".into(),
            law_id: "BWBR0005291".into(),
            version: "2025-07-01_0".into(),
            number: "305a".into(),
            held,
            caveats: Vec::new(),
        };

        let json = serde_json::to_string(&cited(Held::Text {
            label: Some("Artikel 305a".into()),
            path: None,
            content_hash: "8fc1e1eb".into(),
        }))
        .unwrap();

        assert!(json.contains(r#""held":"text""#), "{json}");
        assert!(json.contains(r#""content_hash":"8fc1e1eb""#), "{json}");

        let json = serde_json::to_string(&cited(Held::None)).unwrap();

        assert!(json.contains(r#""held":"none""#), "{json}");
        assert!(!json.contains("content_hash"), "{json}");
    }

    /// A record the register publishes without a body says so, rather than
    /// arriving as one whose provenance went missing. The digest is the whole
    /// of how an answer from this lane is checked.
    #[test]
    fn a_publication_says_whether_the_register_links_a_document() {
        let linked = Publication {
            id: "kst-33669-6".into(),
            title: None,
            dossier: None,
            published_on: None,
            body: Body::Linked {
                provenance: Provenance::frozen(
                    &sources::Provenance {
                        blob: BlobId::of(b"<kamerstuk/>"),
                        url: "https://zoek.officielebekendmakingen.nl/kst-33669-6.xml".into(),
                        byte_len: 12,
                    },
                    "2026-08-24T09:00:00Z".parse().unwrap(),
                ),
            },
        };
        let json = serde_json::to_string(&linked).unwrap();

        assert!(json.contains(r#""body":"linked""#), "{json}");
        assert!(json.contains(r#""frozen_by":"bron""#), "{json}");

        let bare = Publication {
            id: "kst-33669-6".into(),
            title: None,
            dossier: None,
            published_on: None,
            body: Body::Absent,
        };
        let json = serde_json::to_string(&bare).unwrap();

        assert!(json.contains(r#""body":"absent""#), "{json}");
        assert!(!json.contains("provenance"), "{json}");
    }

    /// A consumer tells the two shapes apart by reading one key, never by
    /// testing whether another is present. A ranking that happened to resolve
    /// to one hit and a citation that resolved to one article are different
    /// answers, and only one of them may be ordered.
    #[test]
    fn a_ranking_and_a_lookup_say_which_they_are() {
        let ranking = Found::Ranking(Ranking {
            searched: vec!["wetgeving".into()],
            hits: Vec::new(),
        });
        let json = serde_json::to_string(&ranking).unwrap();

        assert!(json.starts_with(r#"{"kind":"ranking""#), "{json}");
        assert!(json.contains(r#""searched":["wetgeving"]"#), "{json}");

        let lookup = Found::Lookup(Lookup {
            citation: "3:305a".into(),
            resolved: vec![Cited {
                reading: "book".into(),
                source: "wetgeving".into(),
                law_id: "BWBR0005291".into(),
                version: "2025-07-01_0".into(),
                number: "305a".into(),
                held: Held::Text {
                    label: Some("Artikel 305a".into()),
                    path: None,
                    content_hash: "8fc1e1eb".into(),
                },
                caveats: Vec::new(),
            }],
        });
        let json = serde_json::to_string(&lookup).unwrap();

        assert!(json.starts_with(r#"{"kind":"lookup""#), "{json}");

        // The one field a lookup may never carry. A citation that resolves is
        // not an 87% match, and a number here invites a threshold on something
        // that is not an ordering.
        assert!(!json.contains("distance"), "{json}");
    }

    /// Provenance names who froze the bytes. A slice cut from a document bron
    /// holds is bron's own reading of bytes it hashed; one beside a span the
    /// service serves is the service's word for what stands there. Those are
    /// different claims, and the envelope says which one an answer is making.
    #[test]
    fn provenance_says_who_froze_the_bytes() {
        let served = Provenance::served(
            BlobId::of(b"<toestand/>"),
            "https://example.invalid/x.xml".into(),
            11,
        );
        let json = serde_json::to_string(&served).unwrap();

        assert!(json.contains(r#""frozen_by":"service""#), "{json}");
        assert!(!json.contains("fetched_at"), "{json}");
        assert!(served.frozen.reports());

        let ours = Provenance::frozen(
            &sources::Provenance {
                blob: BlobId::of(b"<toestand/>"),
                url: "https://data.rechtspraak.nl/x".into(),
                byte_len: 11,
            },
            "2026-08-20T09:00:00Z".parse().unwrap(),
        );
        let json = serde_json::to_string(&ours).unwrap();

        assert!(json.contains(r#""frozen_by":"bron""#), "{json}");
        assert!(
            json.contains(r#""fetched_at":"2026-08-20T09:00:00Z""#),
            "{json}"
        );
        assert!(!ours.frozen.reports());
    }
}
