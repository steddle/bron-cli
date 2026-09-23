//! The JSON bron builds itself.
//!
//! The service's JSON is the contract, and a command that is one call to the
//! service prints the envelope it was sent. What is defined here is the rest:
//! the answers bron assembles, and the failure envelope.
//!
//! Domain types in [`crate::model`] deliberately do not implement `Serialize`.
//! Everything bron serialises is defined here, so renaming a field in the
//! domain cannot silently break a parser on the other side of a pipe: it breaks
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

use std::collections::BTreeMap;

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

/// Every response, success or failure.
#[derive(Debug, Serialize)]
pub struct Envelope<T> {
    /// Contract version. A consumer that does not recognise it should refuse to
    /// parse rather than guess.
    pub bron: u32,
    pub grade: Grade,
    /// The day the text is for, when the answer was resolved against one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<NaiveDate>,
    /// The day the register was read on, where the question named one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_known_on: Option<NaiveDate>,
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
            as_known_on: None,
            data: Some(data),
            error: None,
            caveats: Vec::new(),
        }
    }

    pub fn bound_to(mut self, as_of: NaiveDate, as_known_on: Option<NaiveDate>) -> Self {
        self.as_of = Some(as_of);
        self.as_known_on = as_known_on;
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
            as_known_on: None,
            data: None,
            // A failure the service stated is its error object as it arrived;
            // one bron states itself carries its own sentence and cause chain.
            error: Some(match fault.statement() {
                Some(statement) => ErrorBody::Stated(statement.sent.clone()),
                None => ErrorBody::Own {
                    kind: fault.kind(),
                    message: fault.to_string(),
                    detail: fault.chain(),
                    retry_after: fault.retry_after(),
                },
            }),
            caveats: Vec::new(),
        }
    }
}

/// A failure, in the `error` of the envelope.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ErrorBody {
    /// A failure the service stated: its error object as it arrived, so its
    /// kind, code, message and detail, and any field it adds later, read as
    /// the API and the MCP give them.
    Stated(Box<serde_json::value::RawValue>),
    /// A failure bron states itself, which carries no code: presence says who
    /// stated it.
    Own {
        /// Stable tag matching the exit code. Branch on this, not on the
        /// message.
        kind: &'static str,
        message: String,
        /// The cause chain, outermost first.
        detail: Vec<String>,
        /// The seconds a `Retry-After` header named where no envelope carried
        /// the failure. Present only where one was named.
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_after: Option<u64>,
    },
}

/// A stated limit on what an answer supports.
///
/// Most of these arrive from the service, which is where the corpus and the
/// arithmetic over it live. Both halves state one vocabulary, so a limit keeps
/// its name through [`Caveat::stated`], and one the service knows about and
/// this crate does not is carried whole rather than dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Caveat {
    /// Ordering came from a relevance score that is not reproducible over time.
    UnstableOrdering,
    /// Filters ran over a capped window; matches may remain unseen.
    Truncated { window: u32 },
    /// Articles in the source yielded no text and were not stored.
    SkippedArticles { count: u32 },
    /// An article number named more than one article in the rendition, and the
    /// service answered all of them. `paths` names each, including any that
    /// `--paragraph` then drops.
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

    /// Words of the question full text did not search: too common to fit its
    /// bound, or past the words it considers, as the lexemes the `dutch`
    /// configuration makes of them. The vector list read the whole question.
    WordsUnsearched { words: Vec<String> },

    /// These hits' places were read up to `bound` in the article or lid list.
    /// Their counts are lower bounds, and the places named are first among
    /// those read, not necessarily first across all places.
    PlacesRead {
        bound: u32,
        content_hashes: Vec<String>,
    },

    /// Renditions for the moment asked that the index does not hold as the
    /// corpus does. Their texts rank as last indexed, or not at all.
    IndexBehind { scope: String, count: u32 },

    /// A treaty prints `number` in its authentic text and in a translation,
    /// and the authentic text is what answered. `lang` names the languages the
    /// translations stand in and `paths` where each is printed.
    TranslationLeftOut {
        number: String,
        lang: Vec<String>,
        paths: Vec<String>,
    },

    /// Texts too long to index for full text, held whole and found by no word
    /// in them. `scope` is `corpus`, as on [`Caveat::UnrankableTexts`]. They
    /// stay answerable by identifier.
    TextsUnindexed { scope: String, count: u32 },

    /// The ranking holds fewer hits than its window: the vector list reached
    /// `reached` of the `asked` nearest texts at the moment, and its index
    /// scan may have stopped at a bound before it found more.
    RankingShort { asked: u32, reached: u32 },

    /// The counts named here, by the kind each would have raised
    /// (`unrankable_texts`, `texts_unindexed`), were not taken for this
    /// search. Their absence is nothing checked, not the corpus holding none.
    CountsNotTaken { caveats: Vec<String> },

    /// The citation reads more than one way and every reading is answered.
    ///
    /// Not [`Caveat::AmbiguousRendition`]: nothing is discarded here. `3:305a`
    /// is Boek 3 artikel 305a and is also, in a law that prints the colon
    /// itself, artikel 3:305a, and which the reader meant is theirs to say.
    ///
    /// `regulations` names them where the register gives one name to several
    /// regulations, and is empty where it does not.
    SeveralReadings {
        readings: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        regulations: Vec<String>,
    },

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
    /// Such a provision carries `status` `nogniet`, and its text is what the
    /// rendition prints for it: the register's stub, as BW Boek 6 artikel 119b
    /// has, or the settled text, as the Wft's 38 have. Settled text read
    /// without this is a rule that does not yet apply, quoted as though it did.
    NotCommenced { articles: Vec<String> },
    /// An article in this answer prints an image, and bron holds no images.
    ///
    /// Not cosmetic. Where a provision states a formula, the register renders
    /// the formula itself as an image and states only its variables as text, so
    /// the stored article introduces a calculation, defines A, B and C, and
    /// never says what to do with them. Silence there reads as a complete
    /// provision.
    IllustrationOmitted { articles: Vec<String> },
    /// The held bytes do not hash to the SHA-512 the publisher's manifest
    /// states, and the manifest states the same hash it stated when they were
    /// fetched.
    ///
    /// Deliberately not fatal. The register disagrees with itself in the wild:
    /// for a large share of renditions the manifest's hash and size describe a
    /// gzip of the manifestation rather than what that manifestation's own URL
    /// serves, and BWBR0002638 is affected in every rendition it has. Refusing
    /// would make bron unable to read a law the publisher is willing to hand
    /// over. So the text is answered and the disagreement is stated, which
    /// leaves the reader to decide what it is worth.
    ///
    /// The publisher's hash does not travel. The comparison is the service's,
    /// made against bytes it holds, and the `blob` digest describes those bytes
    /// all the same.
    UpstreamHashDiffers,
    /// The publisher rewrote this rendition after the service read it: the
    /// manifest's hash has moved since the fetch.
    ///
    /// The answer is the earlier publication, and stays so until the service
    /// takes the rendition again. [`Caveat::UpstreamHashDiffers`] is the other
    /// case: the hash stood still and the bytes never hashed to it. Only the
    /// publisher can settle that one; this one settles itself at the next
    /// fetch.
    UpstreamHashRewritten,
    /// The manifest published no hash for this rendition, so these bytes rest
    /// on our own capture alone.
    ///
    /// A different fact from a hash that disagrees, and a consumer may care
    /// about the difference: 'nobody vouched for this' is not 'the voucher and
    /// the bytes do not match'.
    UpstreamHashAbsent,
    /// The cursor a walk asked from is past everything the service holds.
    ///
    /// A walk that finds nothing and a cursor that belongs to another corpus
    /// answer alike without this: both are an empty list. Only one of them
    /// means the caller should throw its cursor away and start again from the
    /// head, which is why `origin` travels beside it.
    CursorAhead { asked: u64, head: u64 },
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

    /// Units a comparison left out because the register used their stam-id
    /// more than once inside one rendition, where a match would have to pick
    /// one and calling that an identity would be a guess.
    ///
    /// Left out of the counts too, which is why this has to be said: without it
    /// `law changes` answers "nothing changed" where the answer is "this could
    /// not be compared".
    ArticlesNotCompared { repeated: Vec<String> },

    /// A term shaped like a citation was ranked as a question, because the
    /// register gives the name in it to no regulation.
    ///
    /// `number` is the article number read and `name` the part taken for the
    /// law's name, as typed. `because` is `name_not_stated`, a name the
    /// register states for nothing, or `name_not_standing`, one it states and
    /// not on the day asked. The hits are the nearest texts to an identifier,
    /// which may have nothing to do with the article meant.
    CitationNotRead {
        number: String,
        name: String,
        because: String,
    },

    /// Renditions in a history whose text the service does not hold, `count`
    /// of the `renditions` the history lists.
    RenditionsNotHeld { count: u32, renditions: u32 },

    /// Renditions in a history whose bytes the service holds and did not parse
    /// into articles, `count` of the `renditions` the history lists.
    ///
    /// Held is not read: such a rendition has no article to answer, and a
    /// history that listed it beside the others without this would read as one
    /// whose every held text can be quoted.
    RenditionsUnread { count: u32, renditions: u32 },

    /// These renditions were parsed for articles alone, so the answer covers
    /// their articles and no other units.
    ReadForArticles { renditions: Vec<String> },

    /// `count` treaties print units as not yet in force and were left out of
    /// a list of what waits to apply. What a treaty prints so is no
    /// announcement: some treaties never enter into force.
    TreatiesLeftOut { count: u32 },

    /// The list holds what the BWB prints and the acts the Staatsblad
    /// published, as far as the service holds the Staatsblad, that the BWB
    /// does not. An act published before that and never printed in the BWB,
    /// what [`Caveat::ActsLeftOut`] counts, and EU regulations are not in it,
    /// and nothing in the list can say which are missing.
    NotInBwb,

    /// `count` units printed as not yet in force are left out of what waits,
    /// because the register's own remark on each says the change will not
    /// take effect.
    UnitsWithdrawnByRemark { count: u32 },

    /// The rendition `expression` prints words that no unit holds: beside the
    /// units of a chapter or a bijlage that holds some, or under an element
    /// without the publisher's identity. `units` counts them per element.
    ///
    /// No answer covers those words and no comparison compared them.
    TextOutsideArticles {
        expression: String,
        units: BTreeMap<String, u32>,
    },

    /// Acts the Staatsblad published and the BWB prints nowhere that are
    /// nothing to come, left out of the `published` stage and counted here by
    /// kind: `budget`, `treaty_approval`, `constitution_first_reading`. A kind
    /// with none is absent, and a kind this build has no phrase for is named
    /// by its own key rather than dropped.
    ActsLeftOut { counts: BTreeMap<String, u32> },

    /// Regulations whose WTI bron has not read, so no `established_by` names
    /// the act that founded each. While any is left, such an act can stand as
    /// `published` though the BWB prints it.
    FoundingActsUnread { regulations: u32 },

    /// Held inwerkingtredingsbesluiten that name no act by `extref` or
    /// `ketenid`, so bron cannot say what they commence and raises no
    /// [`Caveat::CommencementPending`] for them. `unlinked` counts every one
    /// the service holds, of any age; `decrees` names the ones published in
    /// the last 15 days, and is empty where none was.
    DecreesUnmatched {
        unlinked: u32,
        decrees: Vec<UnmatchedDecree>,
    },

    /// A besluit the Staatsblad published names, by `extref` or `ketenid`,
    /// the act behind units this answer prints as not yet in force, and no
    /// rendition the BWB holds names it as `commenced_by` yet.
    ///
    /// Whether its day has come stands in its own words, which bron does not
    /// read. Raised only where a besluit names its act that way: its absence
    /// does not mean no commencement is pending.
    CommencementPending { decrees: Vec<PendingDecree> },

    /// The publication was passed on and not kept. Its digest is of the bytes
    /// bron read, not one `/blobs` can serve again.
    PublicationNotHeld,

    /// The publisher's own count of matches is an estimate, so walking a
    /// search to its end can read a few hits more or fewer than it stated.
    SourceTotalEstimated,

    /// The service stated a limit this bron has no sentence for.
    ///
    /// Carried whole rather than dropped: its name and every field it came
    /// with, so a reader sees what was said even where this build cannot put it
    /// in words.
    Unrecognised { stated: serde_json::Value },
}

/// One besluit [`Caveat::DecreesUnmatched`] names: published in the last 15
/// days and naming no act bron can match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct UnmatchedDecree {
    /// The besluit, as `stb-2026-280`.
    pub id: String,
    pub published_on: NaiveDate,
}

/// One besluit [`Caveat::CommencementPending`] names: it names its act, and
/// no rendition of that act's regulation has taken it in as `commenced_by`
/// yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct PendingDecree {
    /// The besluit, as `stb-2026-8`.
    pub decree: String,
    pub published_on: NaiveDate,
    /// The act the besluit names, as `stb-2025-62`.
    pub act: String,
}

impl Caveat {
    /// One limit the service stated, under the name the service gave it.
    pub fn stated(value: &serde_json::Value) -> Self {
        #[derive(serde::Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Stated {
            WithdrawnRenditions {
                excluded: Vec<String>,
            },
            AmbiguousRendition {
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
            UpstreamHashAbsent,
            UpstreamHashDiffers,
            UpstreamHashRewritten,
            CursorAhead {
                asked: u64,
                head: u64,
            },
            Truncated {
                window: u32,
            },
            SkippedArticles {
                count: u32,
            },
            AmbiguousArticle {
                number: String,
                paths: Vec<String>,
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
            TextsUnindexed {
                scope: String,
                count: u32,
            },
            RankingShort {
                asked: u32,
                reached: u32,
            },
            CountsNotTaken {
                caveats: Vec<String>,
            },
            UnitsWithdrawnByRemark {
                count: u32,
            },
            WordsUnsearched {
                words: Vec<String>,
            },
            PlacesRead {
                bound: u32,
                content_hashes: Vec<String>,
            },
            IndexBehind {
                scope: String,
                count: u32,
            },
            TranslationLeftOut {
                number: String,
                lang: Vec<String>,
                paths: Vec<String>,
            },
            SeveralReadings {
                readings: Vec<String>,
                #[serde(default)]
                regulations: Vec<String>,
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
            CitationNotRead {
                number: String,
                name: String,
                because: String,
            },
            RenditionsNotHeld {
                count: u32,
                renditions: u32,
            },
            RenditionsUnread {
                count: u32,
                renditions: u32,
            },
            ReadForArticles {
                renditions: Vec<String>,
            },
            TreatiesLeftOut {
                count: u32,
            },
            NotInBwb,
            TextOutsideArticles {
                expression: String,
                units: BTreeMap<String, u32>,
            },
            ArticlesNotCompared {
                repeated: Vec<String>,
            },
            ActsLeftOut {
                counts: BTreeMap<String, u32>,
            },
            FoundingActsUnread {
                regulations: u32,
            },
            DecreesUnmatched {
                unlinked: u32,
                decrees: Vec<UnmatchedDecree>,
            },
            CommencementPending {
                decrees: Vec<PendingDecree>,
            },
            PublicationNotHeld,
            SourceTotalEstimated,
        }

        match <Stated as serde::Deserialize>::deserialize(value) {
            Ok(Stated::WithdrawnRenditions { excluded }) => Self::WithdrawnRenditions { excluded },
            Ok(Stated::AmbiguousRendition { chosen, discarded }) => {
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
            Ok(Stated::CursorAhead { asked, head }) => Self::CursorAhead { asked, head },
            Ok(Stated::UpstreamHashAbsent) => Self::UpstreamHashAbsent,
            Ok(Stated::UpstreamHashDiffers) => Self::UpstreamHashDiffers,
            Ok(Stated::UpstreamHashRewritten) => Self::UpstreamHashRewritten,
            Ok(Stated::Truncated { window }) => Self::Truncated { window },
            Ok(Stated::SkippedArticles { count }) => Self::SkippedArticles { count },
            Ok(Stated::AmbiguousArticle { number, paths }) => {
                Self::AmbiguousArticle { number, paths }
            }
            Ok(Stated::UnrankableTexts { scope, count }) => Self::UnrankableTexts { scope, count },
            Ok(Stated::TextsUnindexed { scope, count }) => Self::TextsUnindexed { scope, count },
            Ok(Stated::RankingShort { asked, reached }) => Self::RankingShort { asked, reached },
            Ok(Stated::CountsNotTaken { caveats }) => Self::CountsNotTaken { caveats },
            Ok(Stated::UnitsWithdrawnByRemark { count }) => Self::UnitsWithdrawnByRemark { count },
            Ok(Stated::WordsUnsearched { words }) => Self::WordsUnsearched { words },
            Ok(Stated::PlacesRead {
                bound,
                content_hashes,
            }) => Self::PlacesRead {
                bound,
                content_hashes,
            },
            Ok(Stated::IndexBehind { scope, count }) => Self::IndexBehind { scope, count },
            Ok(Stated::TranslationLeftOut {
                number,
                lang,
                paths,
            }) => Self::TranslationLeftOut {
                number,
                lang,
                paths,
            },
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
            Ok(Stated::SeveralReadings {
                readings,
                regulations,
            }) => Self::SeveralReadings {
                readings,
                regulations,
            },
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
            Ok(Stated::CitationNotRead {
                number,
                name,
                because,
            }) => Self::CitationNotRead {
                number,
                name,
                because,
            },
            Ok(Stated::RenditionsNotHeld { count, renditions }) => {
                Self::RenditionsNotHeld { count, renditions }
            }
            Ok(Stated::RenditionsUnread { count, renditions }) => {
                Self::RenditionsUnread { count, renditions }
            }
            Ok(Stated::ReadForArticles { renditions }) => Self::ReadForArticles { renditions },
            Ok(Stated::TreatiesLeftOut { count }) => Self::TreatiesLeftOut { count },
            Ok(Stated::NotInBwb) => Self::NotInBwb,
            Ok(Stated::TextOutsideArticles { expression, units }) => {
                Self::TextOutsideArticles { expression, units }
            }
            Ok(Stated::ArticlesNotCompared { repeated }) => Self::ArticlesNotCompared { repeated },
            Ok(Stated::ActsLeftOut { counts }) => Self::ActsLeftOut { counts },
            Ok(Stated::FoundingActsUnread { regulations }) => {
                Self::FoundingActsUnread { regulations }
            }
            Ok(Stated::DecreesUnmatched { unlinked, decrees }) => {
                Self::DecreesUnmatched { unlinked, decrees }
            }
            Ok(Stated::CommencementPending { decrees }) => Self::CommencementPending { decrees },
            Ok(Stated::PublicationNotHeld) => Self::PublicationNotHeld,
            Ok(Stated::SourceTotalEstimated) => Self::SourceTotalEstimated,
            Err(_) => Self::Unrecognised {
                stated: value.clone(),
            },
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

// -- laws ----------------------------------------------------------------

// -- documents -----------------------------------------------------------

/// A document bron fetched by its digest and hashed itself, and the part of it
/// that was asked for.
#[derive(Debug, Serialize)]
pub struct Blob {
    pub provenance: Provenance,
    /// The byte range cut, where one was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<ByteRange>,
    /// The bytes handed over, where they are text. Absent where they are not,
    /// an image, which the human form hands over as bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
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
pub struct CaseHit {
    pub ecli: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub court: Option<String>,
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
            court: hit.court,
            title: hit.title,
            summary: hit.summary,
            decided_on: hit.decided_on,
        }
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
    fn an_answer_states_the_coordinates_it_was_asked_for() {
        let day: NaiveDate = "2023-06-01".parse().unwrap();
        let read_on: NaiveDate = "2024-01-01".parse().unwrap();

        let json = serde_json::to_string(&Envelope::ok(1).bound_to(day, Some(read_on))).unwrap();
        assert!(json.contains(r#""as_of":"2023-06-01""#), "{json}");
        assert!(json.contains(r#""as_known_on":"2024-01-01""#), "{json}");

        let json = serde_json::to_string(&Envelope::ok(1).bound_to(day, None)).unwrap();
        assert!(!json.contains("as_known_on"), "{json}");
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

    /// Presence is the signal. A code is the service's word for a failure the
    /// service stated, so a failure bron states itself carries none.
    #[test]
    fn a_failure_carries_a_code_only_where_the_service_stated_one() {
        let sent = r#"{"kind":"not_found","code":"regulation_not_found","message":"no regulation BWBR0000001","detail":[]}"#;
        let stated = Fault::not_found("law", "BWBR0000001").stated(crate::error::Statement {
            kind: "not_found".into(),
            code: Some("regulation_not_found".into()),
            message: "no regulation BWBR0000001".into(),
            detail: Vec::new(),
            sent: serde_json::value::RawValue::from_string(sent.into()).unwrap(),
        });
        let json = serde_json::to_string(&Envelope::failed(&stated)).unwrap();
        assert_eq!(
            json,
            format!(r#"{{"bron":1,"grade":"authority","error":{sent}}}"#)
        );

        for ours in [
            Fault::usage("name a window"),
            Fault::not_found("lid", "9"),
            Fault::Network {
                service: "bron",
                source: anyhow::anyhow!("connection refused"),
            },
            Fault::Upstream {
                service: "bron",
                source: anyhow::anyhow!("not the bron envelope"),
            },
            Fault::Local(anyhow::anyhow!("no bucket directory")),
        ] {
            let json = serde_json::to_value(Envelope::failed(&ours)).unwrap();
            assert!(json["error"].get("code").is_none(), "{json}");
        }
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

    /// A limit the service knows about and this crate does not is carried
    /// whole. Dropping it, or keeping only its name, would leave a reader one
    /// release behind unable to see what was said.
    #[test]
    fn a_limit_with_no_name_here_is_carried_with_its_fields() {
        let stated = serde_json::json!({"kind": "something_later", "detail": 3});

        assert_eq!(Caveat::stated(&stated), Caveat::Unrecognised { stated });
    }

    #[test]
    fn search_and_article_coverage_limits_keep_their_names_and_fields() {
        for stated in [
            serde_json::json!({"kind": "places_read", "bound": 100, "content_hashes": ["a".repeat(64), "b".repeat(64)]}),
            serde_json::json!({"kind": "index_behind", "scope": "moment", "count": 7}),
            serde_json::json!({"kind": "read_for_articles", "renditions": ["2025-01-01_0", "2026-01-01_0"]}),
        ] {
            assert_eq!(
                serde_json::to_value(Caveat::stated(&stated)).unwrap(),
                stated
            );
        }
    }

    #[test]
    fn unknown_limits_and_unreadable_new_limits_keep_the_whole_payload() {
        for stated in [
            serde_json::json!({"kind": "something_later", "detail": {"nested": [1, "two", null]}}),
            serde_json::json!({"kind": "places_read", "bound": 100}),
            serde_json::json!({"kind": "places_read", "bound": "many", "content_hashes": ["a".repeat(64)]}),
            serde_json::json!({"kind": "index_behind", "count": 7}),
            serde_json::json!({"kind": "index_behind", "scope": "moment", "count": "seven"}),
            serde_json::json!({"kind": "read_for_articles", "renditions": [7], "detail": {"why": "later"}}),
        ] {
            let read = Caveat::stated(&stated);
            assert_eq!(
                read,
                Caveat::Unrecognised {
                    stated: stated.clone()
                }
            );
            assert_eq!(
                serde_json::to_value(read).unwrap(),
                serde_json::json!({"kind": "unrecognised", "stated": stated})
            );
        }
    }

    /// The units are a map, and a map the shape here does not read still
    /// crosses whole: a count that is not a number is the service saying
    /// something this build cannot name, not something to drop.
    #[test]
    fn a_map_of_units_is_read_and_one_this_build_cannot_read_is_carried_whole() {
        assert_eq!(
            Caveat::stated(&serde_json::json!({
                "kind": "text_outside_articles",
                "expression": "2026-09-12_0",
                "units": {"bijlage": 10, "aanhef": 1},
            })),
            Caveat::TextOutsideArticles {
                expression: "2026-09-12_0".into(),
                units: [("aanhef".to_string(), 1), ("bijlage".to_string(), 10)].into(),
            }
        );

        for stated in [
            serde_json::json!({
                "kind": "text_outside_articles",
                "expression": "2026-09-12_0",
                "units": {"bijlage": "ten"},
            }),
            serde_json::json!({"kind": "something_later", "units": {"bijlage": {"n": 10}}}),
        ] {
            let read = Caveat::stated(&stated);

            assert_eq!(
                read,
                Caveat::Unrecognised {
                    stated: stated.clone()
                }
            );
            assert_eq!(
                serde_json::to_value(&read).unwrap(),
                serde_json::json!({"kind": "unrecognised", "stated": stated})
            );
        }
    }

    /// The count of texts no ranking reaches is read under its own name, and
    /// its scope with it: the number is of the corpus, so a search narrowed to
    /// one regulation reports the same figure and has to say why.
    #[test]
    fn texts_no_ranking_reaches_are_read_with_their_scope() {
        assert_eq!(
            Caveat::stated(&serde_json::json!({
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

    /// The count of texts no word finds is read under its own name, the twin
    /// of the texts no ranking reaches.
    #[test]
    fn texts_no_word_finds_are_read_with_their_scope() {
        assert_eq!(
            Caveat::stated(&serde_json::json!({
                "kind": "texts_unindexed",
                "scope": "corpus",
                "count": 3
            })),
            Caveat::TextsUnindexed {
                scope: "corpus".into(),
                count: 3
            }
        );
    }

    /// The words full text did not search are the answer's to state: a reader
    /// who does not know which words were dropped cannot tell a thin ranking
    /// from a thin corpus.
    #[test]
    fn the_words_full_text_did_not_search_are_read_by_name() {
        assert_eq!(
            Caveat::stated(&serde_json::json!({
                "kind": "words_unsearched",
                "words": ["van", "de"]
            })),
            Caveat::WordsUnsearched {
                words: vec!["van".into(), "de".into()],
            }
        );
    }

    /// A treaty prints one number twice, in its authentic text and in a
    /// translation, and which was answered is a fact about the answer.
    #[test]
    fn a_translation_left_out_is_read_with_its_languages_and_paths() {
        assert_eq!(
            Caveat::stated(&serde_json::json!({
                "kind": "translation_left_out",
                "number": "3",
                "lang": ["nl", "fr"],
                "paths": ["/Verdragtekst2/Artikel3", "/Verdragtekst3/Artikel3"]
            })),
            Caveat::TranslationLeftOut {
                number: "3".into(),
                lang: vec!["nl".into(), "fr".into()],
                paths: vec![
                    "/Verdragtekst2/Artikel3".into(),
                    "/Verdragtekst3/Artikel3".into()
                ],
            }
        );
    }

    /// Where the register gives one name to several regulations, the service
    /// names them, and the reading keeps them.
    #[test]
    fn several_readings_keep_the_regulations_that_share_a_name() {
        assert_eq!(
            Caveat::stated(&serde_json::json!({
                "kind": "several_readings",
                "readings": ["literal"],
                "regulations": ["BWBR0005537", "BWBR0005538"]
            })),
            Caveat::SeveralReadings {
                readings: vec!["literal".into()],
                regulations: vec!["BWBR0005537".into(), "BWBR0005538".into()],
            }
        );
    }

    /// A rendition that lost articles says how many. Read as `unrecognised` it
    /// would tell a reader on a current build to upgrade.
    #[test]
    fn articles_a_rendition_could_not_hold_are_counted() {
        assert_eq!(
            Caveat::stated(&serde_json::json!({"kind": "skipped_articles", "count": 14})),
            Caveat::SkippedArticles { count: 14 }
        );
    }

    /// The three hash caveats answer different questions, and a consumer may
    /// care about one and not the others.
    #[test]
    fn the_three_hash_limits_stay_apart() {
        assert_eq!(
            Caveat::stated(&serde_json::json!({"kind": "upstream_hash_absent"})),
            Caveat::UpstreamHashAbsent
        );
        assert_eq!(
            Caveat::stated(&serde_json::json!({"kind": "upstream_hash_differs"})),
            Caveat::UpstreamHashDiffers
        );
        assert_eq!(
            Caveat::stated(&serde_json::json!({"kind": "upstream_hash_rewritten"})),
            Caveat::UpstreamHashRewritten
        );
    }

    #[test]
    fn a_withdrawn_rendition_is_named_rather_than_only_counted() {
        assert_eq!(
            Caveat::stated(&serde_json::json!({
                "kind": "withdrawn_renditions",
                "excluded": ["2017-07-01_0"],
            })),
            Caveat::WithdrawnRenditions {
                excluded: vec!["2017-07-01_0".into()]
            }
        );
    }

    /// One vocabulary for both halves: a limit leaves under the name it
    /// arrived under, so a consumer matches one word whichever half built the
    /// envelope.
    #[test]
    fn a_limit_the_service_states_is_answered_under_the_same_name() {
        for stated in [
            serde_json::json!({"kind": "withdrawn_renditions", "excluded": ["2017-07-01_0"]}),
            serde_json::json!({
                "kind": "ambiguous_rendition",
                "chosen": "2020-01-01_1",
                "discarded": ["2020-01-01_0"],
            }),
            serde_json::json!({"kind": "no_zicht_window"}),
            serde_json::json!({
                "kind": "retroactive",
                "in_force_from": "2019-01-01",
                "first_visible": "2019-03-01",
            }),
            serde_json::json!({"kind": "superseded_knowledge", "visible_until": "2020-01-01"}),
            serde_json::json!({"kind": "upstream_hash_absent"}),
            serde_json::json!({"kind": "upstream_hash_differs"}),
            serde_json::json!({"kind": "upstream_hash_rewritten"}),
            serde_json::json!({"kind": "cursor_ahead", "asked": 90, "head": 40}),
            serde_json::json!({"kind": "truncated", "window": 20}),
            serde_json::json!({"kind": "skipped_articles", "count": 14}),
            serde_json::json!({
                "kind": "ambiguous_article",
                "number": "1",
                "paths": ["/Bijlage2/Artikel1", "/Bijlage3/Artikel1"],
            }),
            serde_json::json!({
                "kind": "table_flattened",
                "articles": ["2.10"],
                "cell_separator": " | ",
            }),
            serde_json::json!({"kind": "not_commenced", "articles": ["7"]}),
            serde_json::json!({"kind": "illustration_omitted", "articles": ["3.2"]}),
            serde_json::json!({"kind": "lanes_unsearched", "lanes": ["rechtspraak"]}),
            serde_json::json!({"kind": "unrankable_texts", "scope": "corpus", "count": 64}),
            serde_json::json!({"kind": "texts_unindexed", "scope": "corpus", "count": 3}),
            serde_json::json!({"kind": "words_unsearched", "words": ["van", "de"]}),
            serde_json::json!({
                "kind": "translation_left_out",
                "number": "3",
                "lang": ["nl"],
                "paths": ["/Verdragtekst2/Artikel3"],
            }),
            serde_json::json!({"kind": "several_readings", "readings": ["book", "literal"]}),
            serde_json::json!({
                "kind": "several_readings",
                "readings": ["literal"],
                "regulations": ["BWBR0005537", "BWBR0005538"],
            }),
            serde_json::json!({
                "kind": "reading_unresolved",
                "reading": "literal",
                "because": "withdrawn",
                "regulation_id": "BWBR0005537",
                "renditions": ["2017-07-01_0"],
            }),
            serde_json::json!({
                "kind": "held_text_only",
                "held_regulations": 4,
                "known_regulations": 25462,
            }),
            serde_json::json!({"kind": "renditions_unread", "count": 2, "renditions": 41}),
            serde_json::json!({"kind": "renditions_not_held", "count": 30, "renditions": 41}),
            serde_json::json!({
                "kind": "citation_not_read",
                "number": "3:305a",
                "name": "WAMCA",
                "because": "name_not_stated",
            }),
            serde_json::json!({
                "kind": "text_outside_articles",
                "expression": "2026-09-12_0",
                "units": {"aanhef": 1, "bijlage": 10},
            }),
        ] {
            assert_eq!(
                serde_json::to_value(Caveat::stated(&stated)).unwrap(),
                stated
            );
        }
    }

    /// Provenance names who froze the bytes: bron fetched them itself and
    /// stamps when, which is what a reader checks a quotation against.
    #[test]
    fn provenance_says_who_froze_the_bytes() {
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
    }
}
