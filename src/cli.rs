//! The command line.
//!
//! English throughout. Everything else a caller touches is already English
//! (flags, JSON keys, error kinds), the main consumer is code rather than a
//! person, and the sources will not stay Dutch: `wet` is the wrong word for an
//! EU directive.
//!
//! Dutch survives only where a term has no English equivalent and translating
//! it would lose precision: `citeertitel`, `geldigheid`, `zicht`, `lid`.

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "bron",
    version,
    about = "Dutch primary legal sources, quotable to the byte",
    long_about = "Answers what Dutch law said on a date, and what the courts and the \
                  official publications say, naming the document behind every answer and \
                  the byte range inside it.\n\n\
                  Exit codes: 0 ok, 1 usage, 2 not found, 3 network, 4 upstream, 5 local, \
                  6 a quotation `quote check` read is not found where it cites."
)]
pub struct Cli {
    /// Machine-readable output.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Legislation from the Basis Wetten Bestand (BWB), and the acts of the EU
    /// from CELLAR.
    #[command(subcommand)]
    Law(Law),

    /// Rulings and conclusies of the Dutch courts, from rechtspraak, and the
    /// judgments and opinions of the EU's courts, from CELLAR.
    #[command(subcommand)]
    Case(Case),

    /// Official publications (Officiële Bekendmakingen): kamerstukken,
    /// Staatsblad, Staatscourant, Tractatenblad.
    #[command(subcommand)]
    Publication(Publication),

    /// The parliamentary history behind a law: a kamerstukdossier as the act
    /// behind a text the service holds names it.
    #[command(subcommand)]
    Dossier(DossierCmd),

    /// Quotations, read again in the texts they cite.
    #[command(subcommand)]
    Quote(QuoteCmd),

    /// Answers which article, from a question or from a citation.
    ///
    /// Distinct from `law search`, which resolves a citeertitel to a BWB id and
    /// answers which law. This answers which article, across every corpus bron
    /// holds vectors for, and its answer says which of them it searched.
    ///
    /// A term that reads as a citation is looked up rather than ranked. `3:305a`
    /// put through a vector comes back as the Wft's numerically similar articles
    /// and never as artikel 3:305a BW, because an identifier is arithmetic and
    /// nothing about it is approximate. Where a citation reads more than one
    /// way, every reading is answered: `8:4` is Boek 8 artikel 4 and is also
    /// Awb artikel 8:4, and choosing between them is interpretation.
    ///
    /// The ranking is the one approximate thing in bron and it only orders. A
    /// lookup carries no distance at all. Every field of either is read from a
    /// stored row, and resolving one to bytes is `law show`, which is
    /// arithmetic.
    Find {
        /// The question in words, or the citation.
        query: Vec<String>,

        /// Search inside this law only, by its BWB identifier. Repeat it to
        /// search inside several.
        #[arg(long, value_name = "BWB")]
        regulation: Vec<String>,

        /// The jurisdiction to search, e.g. nl; untyped, the service's. Sent
        /// exactly as typed, and the service says whether it reads one.
        #[arg(long)]
        jurisdiction: Option<String>,

        /// How many hits to answer; untyped, the service's window. Sent
        /// exactly as typed; the service pages this route and refuses what it
        /// refuses.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        #[command(flatten)]
        moment: Moment,
    },

    /// The bytes a digest names, checked against it.
    ///
    /// What every answer's `source:` line invites: fetch the document, hash
    /// it, and cut the span again. bron hashes the bytes as they arrive and
    /// hands them over untouched on stdout, so `bron blob D | shasum -a 256`
    /// prints D, and refuses bytes that hash to anything else. With --span the
    /// slice is cut from those bytes. A note on stderr says what was checked.
    ///
    /// With --json the answer is what was checked, and the text where the
    /// bytes are text; bytes that are no text, an image, are for the human
    /// form to hand over.
    Blob {
        /// SHA-256 of the document, as 64 lowercase hex characters.
        digest: String,

        /// A byte range to cut, as an answer prints it: 349072..361267.
        #[arg(long, value_name = "START..END", value_parser = byte_range)]
        span: Option<(usize, usize)>,
    },
}

#[derive(Debug, Subcommand)]
pub enum QuoteCmd {
    /// Whether the text a quotation cites holds it, word for word once white
    /// space and quotation marks are folded alike, and never by meaning.
    ///
    /// One quotation from the command line, citing an article of a regulation
    /// or a ruling by its ECLI, or up to ten from --file. Each answers a
    /// verdict: found, with the unit that holds it; not as quoted, with each
    /// difference; not where cited, with the unit of the same article or
    /// ruling that holds it; or citation unresolved, with what the service
    /// answered. Exits 0 where every quotation is found and 6 where one is
    /// not, so a script can branch without reading the verdicts.
    Check {
        /// The quotation, as it stands in the text that relies on it.
        #[arg(required_unless_present = "file", conflicts_with = "file")]
        text: Option<String>,

        /// The regulation it cites, as `law show` takes it: a BWB id or
        /// celex:32016R0679. Sent exactly as typed.
        #[arg(long)]
        regulation: Option<String>,

        /// The article of that regulation, e.g. 6:162. Sent exactly as typed.
        #[arg(long)]
        article: Option<String>,

        /// One lid of that article. Sent exactly as typed.
        #[arg(long)]
        paragraph: Option<String>,

        /// The day the text applied, YYYY-MM-DD. Sent exactly as typed.
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_of: Option<String>,

        /// The day the register is read on (zicht). Sent exactly as typed.
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_known_on: Option<String>,

        /// The ruling it cites, by its ECLI. Sent exactly as typed.
        #[arg(long)]
        ecli: Option<String>,

        /// One rechtsoverweging of that ruling, e.g. 3.2.10. Sent exactly as
        /// typed.
        #[arg(long)]
        number: Option<String>,

        /// One paragraph of that ruling by its ordinal, as `case show` prints
        /// it. Digits are sent as a number, anything else as typed.
        #[arg(long, allow_negative_numbers = true)]
        ordinal: Option<String>,

        /// A JSON list of quotations, each an object as the service takes it:
        /// `text` with `regulation` and `article` (and `paragraph`, `as_of`,
        /// `as_known_on`), or with `ecli` (and `number` or `ordinal`). `-`
        /// reads stdin. Sent as the file holds it.
        #[arg(
            long,
            value_name = "FILE",
            conflicts_with_all = [
                "regulation", "article", "paragraph", "as_of", "as_known_on", "ecli", "number",
                "ordinal",
            ]
        )]
        file: Option<std::path::PathBuf>,
    },
}

/// A byte range as `start..end`, the first no later than the second.
fn byte_range(text: &str) -> Result<(usize, usize), String> {
    let (start, end) = text
        .split_once("..")
        .ok_or_else(|| format!("[{text}] is not a byte range as START..END"))?;
    let parse = |n: &str| {
        n.parse::<usize>()
            .map_err(|_| format!("[{text}] is not a byte range as START..END"))
    };
    let (start, end) = (parse(start)?, parse(end)?);
    if start > end {
        return Err(format!("[{text}] ends before it starts"));
    }
    Ok((start, end))
}

#[derive(Debug, Subcommand)]
pub enum Law {
    /// Searches by citeertitel. Returns candidates, not text.
    ///
    /// The register gives a regulation its names on a day, so a moment narrows
    /// which names count: a citeertitel is given and taken away like any other
    /// text.
    Search {
        query: Vec<String>,

        /// Only regulations of this type, as the register spells it: wet,
        /// AMvB, zbo.
        #[arg(long = "type", value_name = "TYPE")]
        kind: Option<String>,

        /// How many hits to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many rows to skip; untyped, none. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,

        #[command(flatten)]
        moment: Moment,
    },

    /// Shows a law's articles, or one of them.
    ///
    /// Articles only: `artikel`, or `enig-artikel` for a regulation's one
    /// article, and `article` in an act of the EU. A bijlage, the aanhef, the
    /// closing formula, a recital, an annex and every other unit a rendition
    /// prints beside its articles is `law parts`.
    Show {
        /// BWB identifier, e.g. BWBR0005289, or an act of the EU as celex: and
        /// its CELEX, e.g. celex:32016R0679.
        id: String,

        /// The number as printed, e.g. 162 or 162a, or 6:162 on a book of the
        /// BW.
        #[arg(long)]
        article: Option<String>,

        /// Only what this law publishes and does not yet apply.
        ///
        /// The one future the register holds. No version is waiting to take
        /// effect, but an article inside the current one can be. Each is
        /// answered with the text the register publishes for it, and names the
        /// day it applies where the register states one.
        #[arg(long)]
        not_commenced: bool,

        /// One lid, as the register numbers it: of --article, or of every
        /// article that prints one.
        ///
        /// The service answers the lid, so the span, the Juriconnect reference
        /// and the label are the lid's: an answer about lid 2 that cites the
        /// article's bytes is coarser than the claim it supports.
        #[arg(long)]
        paragraph: Option<String>,

        /// One item of that lid, as the register letters it: `a`, `b`, `2°`.
        ///
        /// A cut of what a person reads, and the one narrowing no route
        /// answers: an item has no span of its own in this corpus, so the
        /// answer stays the lid's rather than naming an offset nobody
        /// measured. With --json the envelope is the lid as the service sent
        /// it. Where a lid prints no such item, a person reads that it does
        /// not.
        #[arg(long, requires = "paragraph")]
        item: Option<String>,

        /// The text in this language: of a treaty as the register writes it,
        /// e.g. en or nl, where a number named without it answers the
        /// authentic text; of an act of the EU nl or en, nl where left out.
        /// The service says whether the language is one it prints.
        #[arg(long)]
        lang: Option<String>,

        /// Also carry the publisher's own bytes at every span.
        ///
        /// Without it an answer carries the offsets and the service's reading,
        /// and no markup travels. With it each span also carries the slice
        /// itself, on the service's word: it is what the service says stands
        /// at those offsets rather than a cut bron made from bytes it hashed,
        /// and fetching the blob and cutting it again is how that is checked.
        ///
        /// Only --json carries the slice. A person reads the service's reading
        /// either way, so without --json the bytes are fetched and nothing
        /// prints them.
        #[arg(long)]
        raw: bool,

        #[command(flatten)]
        moment: Moment,
    },

    /// The units of text a rendition prints, in document order: its articles,
    /// and beside them the aanhef, a bijlage, the closing formula.
    ///
    /// Each under the element the register printed it as, with the span and
    /// digest that place it. Every kind unless --kind names one, so the whole
    /// of what a rendition prints is one question.
    Parts {
        /// BWB identifier, e.g. BWBR0001941, or an act of the EU as celex: and
        /// its CELEX, e.g. celex:32005L0029.
        id: String,

        /// One kind of unit, as the register names the element: bijlage,
        /// aanhef, tekst, wetsluiting, artikel; of an act of the EU recital,
        /// article, annex.
        #[arg(long)]
        kind: Option<String>,

        /// The number the register prints for a unit of --kind, e.g. Ia.
        #[arg(long)]
        number: Option<String>,

        /// The place of one unit among the units of --kind, from 1: how a unit
        /// that prints no number is asked for. Sent to the service exactly as
        /// typed.
        #[arg(long, allow_negative_numbers = true)]
        position: Option<String>,

        /// One lid of that unit, as the register numbers it. The span, the
        /// text and the reference become the lid's.
        #[arg(long)]
        paragraph: Option<String>,

        /// The text in this language. See `law show --lang`.
        #[arg(long)]
        lang: Option<String>,

        /// Only the units this rendition publishes and does not yet apply, of
        /// every kind: a waiting bijlage beside the waiting articles. Each
        /// names the day it applies where the register states one. See
        /// `law show --not-commenced`.
        #[arg(long)]
        not_commenced: bool,

        /// Also carry the publisher's own bytes at every span. See `law show`.
        #[arg(long)]
        raw: bool,

        /// How many units to answer, or `none` for every one the rendition
        /// prints; untyped, the service's window.
        ///
        /// Sent to the service exactly as typed. The rendition bounds this
        /// route, as it bounds `law show`, so bron judges nothing about the
        /// word: `none`, a negative number, a decimal all reach the service
        /// and come back as its own `invalid_parameter` where they are wrong.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many units to skip; untyped, none. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,

        #[command(flatten)]
        moment: Moment,
    },

    /// The table of contents of a rendition: the parts the register heads,
    /// with their headings, and the articles and bijlagen under them.
    ///
    /// Read from the rendition's own bytes, of the rendition the moment
    /// resolves to.
    Contents {
        /// BWB identifier, e.g. BWBR0005289.
        id: String,

        #[command(flatten)]
        moment: Moment,
    },

    /// The texts the act that established this regulation produced elsewhere.
    ///
    /// An amending act is published as a regulation of its own, and what it
    /// did stands in the laws it amended. This answers those texts, from the
    /// regulations whose text the corpus holds.
    Amendments {
        /// BWB identifier, e.g. BWBR0046553.
        id: String,

        /// How many rows to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many rows to skip; untyped, none. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,
    },

    /// What the register states rests on a law, and what cites it.
    ///
    /// The WTI's own relations: what a law is the grondslag for, what refers
    /// to it, and from which article where the register states one.
    Relations {
        /// BWB identifier, e.g. BWBR0005289.
        id: String,

        /// One kind of relation, as the register groups them, e.g.
        /// grondslag_voor or verwijzing_door.
        #[arg(long)]
        kind: Option<String>,

        /// One article, as the service matches it. The register states a
        /// relation under the label it prints, `Artikel 162`.
        #[arg(long)]
        article: Option<String>,

        /// How many rows to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many rows to skip; untyped, none. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,

        #[command(flatten)]
        moment: Moment,
    },

    /// The rulings LiDO found that cite one article, or on an act of the EU
    /// the cases CELLAR states interpret or cite it, newest decided first.
    ///
    /// Each ruling names the rendition of the law in force on the day it was
    /// decided. That is not necessarily the text the court applied, which
    /// turns on the day of the facts and transitional law. A case of the EU
    /// names the places in the article it cites. How many LiDO or CELLAR
    /// misses is not measured, so an answer says how many were found and never
    /// that no ruling cites the article.
    Decisions {
        /// BWB identifier, e.g. BWBR0005289, or an act of the EU as celex: and
        /// its CELEX, e.g. celex:32016R0679.
        id: String,

        /// The number as printed, e.g. 162 or 162a, or 6:162 on a book of the
        /// BW.
        #[arg(long)]
        article: String,

        /// One court, as the answer's `courts` names it, e.g. "Hoge Raad der
        /// Nederlanden". Sent exactly as typed.
        #[arg(long)]
        court: Option<String>,

        /// Only rulings decided on this day or later, YYYY-MM-DD. Sent
        /// exactly as typed.
        #[arg(long)]
        from: Option<String>,

        /// Only rulings decided on this day or earlier, YYYY-MM-DD. Sent
        /// exactly as typed.
        #[arg(long)]
        to: Option<String>,

        /// How many rulings to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many rulings to skip; untyped, none. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,
    },

    /// What the index knows about one regulation.
    ///
    /// How much of it the corpus means to hold and how much it does, which is
    /// what says whether `law show` can answer about it; the body the register
    /// names for it; and the publications that created it.
    About {
        /// BWB identifier, e.g. BWBR0005289.
        id: String,
    },

    /// Every text one article, or one other unit, has had.
    ///
    /// Read from the renditions the corpus holds, so a window whose renditions
    /// it does not hold is a window whose text nobody here can state. Each
    /// text is named by the act that produced it and by the renditions that
    /// carry it.
    History {
        /// BWB identifier, e.g. BWBR0005289.
        id: String,

        /// The number as printed, e.g. 162 or 162a, or 6:162 on a book of the
        /// BW, or a bijlage's with --kind.
        #[arg(long)]
        article: String,

        /// The kind of unit the number is printed under, where it is no
        /// article: bijlage.
        #[arg(long)]
        kind: Option<String>,

        /// The day whose view of the register counts (zicht). Sent to the
        /// service exactly as typed.
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_known_on: Option<String>,
    },

    /// Every published version of a law, with its validity windows.
    Versions {
        /// BWB identifier, e.g. BWBR0005289, or an act of the EU as celex: and
        /// its CELEX, e.g. celex:32016R0679.
        id: String,
    },

    /// Which renditions the register published in a window, across every law.
    ///
    /// A question about a period that has passed, and reproducible: the same
    /// window answers the same renditions next year. It is the wrong command to
    /// poll with. The publisher publishes retroactively, so a rendition that
    /// arrives tomorrow can carry a date inside a window already read, and a
    /// poller that walks windows never sees it. `law feed` misses nothing.
    Published {
        /// The first day of the window on geldigheid: what took effect on or after it.
        /// Each of the four days is sent to the service exactly as typed.
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: Option<String>,

        /// The last day of that window.
        #[arg(long, value_name = "YYYY-MM-DD")]
        to: Option<String>,

        /// The first day of the window on zicht: what the register began showing on or after it.
        #[arg(long, value_name = "YYYY-MM-DD")]
        known_from: Option<String>,

        /// The last day of that window.
        #[arg(long, value_name = "YYYY-MM-DD")]
        known_to: Option<String>,

        /// Only laws of this type, as the register spells it: wet, AMvB, zbo.
        #[arg(long = "type", value_name = "TYPE")]
        kind: Option<String>,

        /// Only this law, by its BWB identifier.
        #[arg(long)]
        regulation: Option<String>,

        /// How many rows to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many rows to skip, to read past a list cut at its window;
        /// untyped, none. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,
    },

    /// What the register has done since a cursor, in the order bron saw it.
    ///
    /// Not a window and not re-readable as one: the order is the order bron saw
    /// things, which is not the order they took effect. It is the reading that
    /// misses nothing, which is what a poller needs.
    ///
    /// The answer states `next`, the cursor to come back to, and `origin`, the
    /// corpus that cursor belongs to. Keep both: an origin you do not recognise
    /// means the corpus was rebuilt and the number means something else now.
    Feed {
        /// The cursor to walk from. 0 for the whole log. Sent to the service
        /// exactly as typed, and the service refuses a walk without one.
        #[arg(long, allow_negative_numbers = true)]
        since: Option<String>,

        /// Only this law, by its BWB identifier.
        #[arg(long)]
        regulation: Option<String>,

        /// How many events to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,
    },

    /// Which regulations print provisions that do not yet apply, across every
    /// law.
    ///
    /// The register publishes no rendition ahead. What it holds of the future
    /// stands inside the rendition in force, as units it prints as not yet
    /// commenced. One row per regulation: the first day to come those units
    /// state, how many wait, the acts that produced them and the besluiten
    /// that commence them. Days to come first, earliest first; then the rows
    /// that state no day, the newest act first. Every day is one the register
    /// states, never an estimate.
    ///
    /// An act the Staatsblad published from 2020 that the BWB prints nowhere
    /// is a row of its own. Treaties are left out and counted. `law parts ID
    /// --not-commenced` reads one regulation's waiting units of every kind,
    /// and `law show ID --not-commenced` its waiting articles.
    Upcoming {
        /// Only regulations with a waiting unit that states a day on or after
        /// this one. Without --from and --to, regulations whose units state
        /// no day are listed too. Both days are sent to the service exactly as
        /// typed.
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: Option<String>,

        /// Only regulations with a waiting unit that states a day on or
        /// before this one.
        #[arg(long, value_name = "YYYY-MM-DD")]
        to: Option<String>,

        /// Only laws of this type, as the register spells it: wet, AMvB, zbo.
        #[arg(long = "type", value_name = "TYPE")]
        kind: Option<String>,

        /// How many rows to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many rows to skip, to read past a list cut at its window;
        /// untyped, none. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,

        #[command(flatten)]
        moment: Moment,
    },

    /// What changed in a law between two dates.
    ///
    /// The service's own comparison, unit by unit: the articles, and the
    /// bijlagen, aanhef and closing formula beside them. Units are matched on
    /// upstream's own identity, so a renumbered article reads as one amendment
    /// rather than a removal plus an addition. Both texts travel with the
    /// answer, each from its own frozen document.
    ///
    /// Whether a change reaches conduct from before it is transitional law, and
    /// not a question bron answers. The act that made each change is named.
    Changes {
        /// BWB identifier, e.g. BWBR0005289.
        id: String,

        /// The earlier date, resolved against geldigheid. Sent to the service
        /// exactly as typed, and the service refuses a comparison without one.
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: Option<String>,

        /// The later date. Sent exactly as typed; the service reads today
        /// where none is named.
        #[arg(long, value_name = "YYYY-MM-DD")]
        to: Option<String>,

        /// Also list units that did not change.
        #[arg(long)]
        all: bool,

        /// Only units of this kind, as the register names the element:
        /// artikel, bijlage, aanhef, wetsluiting.
        #[arg(long)]
        kind: Option<String>,

        /// Only units carrying this number as printed on either side, e.g.
        /// 162 or 162a, or 6:162 on a book of the BW, or a bijlage's with
        /// --kind.
        #[arg(long)]
        article: Option<String>,

        /// A treaty's text in this language, as the register writes it. See
        /// `law show --lang`.
        #[arg(long)]
        lang: Option<String>,

        /// The day the register is read on (zicht), which selects what it
        /// showed then. Both days of the comparison are resolved against it.
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_known_on: Option<String>,

        /// Also carry the publisher's own bytes at every span. See `law show`.
        #[arg(long)]
        raw: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum Case {
    /// One ruling or conclusie by its ECLI, a besluit of the ACM by
    /// acm:{node}, or a judgment or opinion of the EU's courts by its
    /// ECLI:EU, with its text.
    ///
    /// The service fetches a ruling from rechtspraak on the first read that
    /// asks for it and answers every later read from the bytes it froze. A
    /// ruling rechtspraak publishes as metadata alone answers with no text,
    /// and a tuchtcollege's names where its text is published. A ruling the
    /// service withholds answers its court and days and no text. A case of
    /// the EU, ECLI:EU:C:2019:773, is read from CELLAR, each paragraph under
    /// the number the Court prints for it.
    Show {
        ecli: String,

        /// Also carry the publisher's own bytes at every span. See `law show`.
        #[arg(long)]
        raw: bool,

        /// The language a case of the EU is read in, nl or en; nl where left
        /// out. Sent exactly as typed: rechtspraak publishes Dutch alone, and
        /// the service refuses en for its rulings.
        #[arg(long)]
        lang: Option<String>,

        /// How many units of its text to answer; untyped, the service's
        /// window. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many units to skip; untyped, none. With --number, counted from
        /// the rechtsoverweging's first paragraph. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,

        /// One rechtsoverweging of a Dutch ruling as the court prints it,
        /// e.g. 3.2.10, with the paragraphs that continue it and those
        /// numbered under it. Sent exactly as typed: the service refuses it
        /// for a case of the EU and a besluit of the ACM, and answers
        /// numbers_not_read for a ruling it has not read for its numbers,
        /// which --offset reads.
        #[arg(long)]
        number: Option<String>,
    },

    /// What the publisher states connects a ruling to others: its conclusie,
    /// and the instances before and after it.
    ///
    /// A relation is the publisher's statement and never a finding that an
    /// appeal was or was not lodged: one that settled, was withdrawn or ended
    /// in an unpublished ruling leaves no relation behind.
    Relations { ecli: String },

    /// The rulings the service holds, ranked against a question.
    ///
    /// The service ranks the rulings it has read, the ones someone asked for
    /// by their ECLI, and not the case law rechtspraak publishes: search over
    /// case law is still to come, and every answer says how many rulings the search covers.
    Search {
        query: Vec<String>,

        /// One court, as `case show` prints it, e.g. "Hoge Raad". Sent
        /// exactly as typed.
        #[arg(long)]
        court: Option<String>,

        /// Only rulings decided on this day or later, YYYY-MM-DD. Sent
        /// exactly as typed.
        #[arg(long)]
        from: Option<String>,

        /// Only rulings decided on this day or earlier, YYYY-MM-DD. Sent
        /// exactly as typed.
        #[arg(long)]
        to: Option<String>,

        /// Only rulings (`ruling`) or only conclusies (`opinion`). Sent
        /// exactly as typed.
        #[arg(long)]
        form: Option<String>,

        /// Only the decisions of one register: `rechtspraak`, `acm`, or
        /// `cellar` for the judgments and opinions of the EU's courts, which
        /// the service ranks only where asked. Sent exactly as typed.
        #[arg(long)]
        register: Option<String>,

        /// How many rulings to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// Also the hits past the service's line, each marked: they stand too
        /// far from the question to answer it, and are otherwise held back
        /// and counted.
        #[arg(long)]
        beyond_line: bool,
    },

    /// The newest rulings with a text, as rechtspraak's feed listed them,
    /// newest decided first.
    ///
    /// Not every ruling decided. The list holds the rulings with a text whose
    /// record rechtspraak's feed listed or changed from 30 days before bron
    /// first read it, so an older ruling can stand among them by its own day
    /// decided; it lists no conclusie and no ruling published as metadata
    /// alone, so a ruling missing here may still have been decided.
    Recent {
        /// One court, as the answer's `courts` names it, e.g. "Hoge Raad".
        /// Sent exactly as typed.
        #[arg(long)]
        court: Option<String>,

        /// Only rulings decided on this day or later, YYYY-MM-DD. Sent
        /// exactly as typed.
        #[arg(long)]
        from: Option<String>,

        /// Only rulings decided on this day or earlier, YYYY-MM-DD. Sent
        /// exactly as typed.
        #[arg(long)]
        to: Option<String>,

        /// How many rulings to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many rulings to skip; untyped, none. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,

        /// Only rulings the feed lists under civil law, `civiel`, or criminal
        /// law, `straf`. Sent exactly as typed.
        #[arg(long)]
        subject: Option<String>,

        /// Walk the feed in the order it stamped the rulings, after this
        /// moment with its offset, e.g. 2026-10-26T23:00:00Z, or the `next` of
        /// an earlier answer. A walk pages by `next` and never by --offset.
        /// Sent exactly as typed.
        #[arg(long)]
        since: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum Publication {
    /// The publisher's own ranking of a question over the officiële
    /// publicaties, asked live: bron keeps no index of them.
    ///
    /// At least one of the query, --collection and --dossier; the service
    /// refuses the absence of all three rather than bron.
    Search {
        query: Vec<String>,

        /// One collection, by the service's code: kst for kamerstukken, stb
        /// for the Staatsblad, stcrt for the Staatscourant, trb for the
        /// Tractatenblad, blg for bijlagen, h for the Handelingen. Sent
        /// exactly as typed, and the service says whether it is one.
        #[arg(long)]
        collection: Option<String>,

        /// One kamerstukdossier by its number, e.g. 36195.
        #[arg(long)]
        dossier: Option<String>,

        /// How many hits to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// Where to ask on: the `next` a previous answer stated. Walking it to
        /// its end reads every hit once.
        #[arg(long)]
        next: Option<String>,
    },

    /// One publication by identifier, e.g. stb-2026-56 or kst-36327-3.
    ///
    /// Held where the corpus keeps it, passed through where it was fetched on
    /// demand and not kept, and refused where bron keeps it out because it
    /// names private persons. The Aanhangsel van de Handelingen, the
    /// Kamervragen zonder antwoord, the Niet-dossierstukken and the Agenda
    /// are refused whole. A manifestation in XML answers the articles it
    /// prints, at their spans.
    Show {
        id: String,

        /// Also carry bron's rendering of each unit, beside the span and hash
        /// that place it. See `law show`.
        #[arg(long)]
        rendered: bool,

        /// Also carry the publisher's own bytes at every span. See `law show`.
        #[arg(long)]
        raw: bool,

        /// How many units to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many units to skip; untyped, none. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum DossierCmd {
    /// One kamerstukdossier by its number, e.g. 34608: the acts that name it
    /// and its papers oldest first, each with its kind and, on an amendement
    /// or a motie, what the Tweede Kamer decided.
    ///
    /// Read a paper itself with `publication show`. A dossier no held text's
    /// act names is not found (exit 2).
    Show {
        dossier: String,

        /// Only papers of this kind, as the service names it:
        /// memorie_van_toelichting, nota_van_wijziging, amendement, motie,
        /// memorie_van_antwoord and more. Sent exactly as typed.
        #[arg(long)]
        kind: Option<String>,

        /// How many papers to answer; untyped, the service's window. Sent
        /// exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        limit: Option<String>,

        /// How many papers to skip; untyped, none. Sent exactly as typed.
        #[arg(long, allow_negative_numbers = true)]
        offset: Option<String>,
    },
}

/// Which moment a question about legislation means.
///
/// Two windows, and a moment is a point in both. Geldigheid says what applied,
/// which is what you want to know about conduct in 2023. Zicht says what the
/// register showed, which is what you want to know about what someone could
/// have known then. They diverge the moment something is published with
/// retroactive effect, so a question may name a day in each.
///
/// Both are sent to the service exactly as typed, and each is left out where
/// its flag was: the service judges a date, reads `as_known_on` alone as that
/// day for both coordinates, reads a moment named by neither as today, and
/// states the moment it resolved in its answer.
#[derive(Debug, Args)]
pub struct Moment {
    /// The text as it applied on this date (geldigheid).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub as_of: Option<String>,

    /// The text as the register showed it on this date (zicht).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub as_known_on: Option<String>,
}

impl Moment {
    /// The two coordinates as typed, each absent where its flag was.
    pub fn coordinates(&self) -> (Option<&str>, Option<&str>) {
        (self.as_of.as_deref(), self.as_known_on.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    /// `law parts --limit` is a bare string, sent to the service exactly as
    /// typed: bron judges nothing the service judges, so `NONE`, `1.5` and
    /// `-1` all parse here and are the service's to refuse.
    #[test]
    fn law_parts_accepts_any_limit_text() {
        for limit in ["20", "none", "NONE", "1.5", "-1", "whole"] {
            let result =
                Cli::try_parse_from(["bron", "law", "parts", "BWBR0001941", "--limit", limit]);

            assert!(result.is_ok(), "[{limit}]: {result:?}");
        }
    }

    /// A negative number is the value of the flag before it, as two tokens,
    /// and a flag that is no number is never taken for one: `--json` after
    /// `--limit -1` is still `--json`.
    #[test]
    fn a_negative_number_is_a_value_and_a_flag_stays_a_flag() {
        let cli = Cli::try_parse_from([
            "bron", "law", "feed", "--since", "-1", "--limit", "-1", "--json",
        ])
        .unwrap();

        assert!(cli.json);
        let Command::Law(Law::Feed { since, limit, .. }) = cli.command else {
            panic!("law feed");
        };
        assert_eq!(since.as_deref(), Some("-1"));
        assert_eq!(limit.as_deref(), Some("-1"));

        let swallowed = Cli::try_parse_from(["bron", "law", "feed", "--since", "--json"]);
        assert!(swallowed.is_err(), "{swallowed:?}");
    }

    /// Each coordinate travels as typed and only where its flag was named. A
    /// zicht date alone stays alone: the service reads it as that day for
    /// both coordinates.
    #[test]
    fn a_moment_is_sent_as_typed() {
        let cli = Cli::try_parse_from([
            "bron",
            "law",
            "show",
            "BWBR0005289",
            "--as-known-on",
            "2026-02-30",
        ])
        .unwrap();
        let Command::Law(Law::Show { moment, .. }) = cli.command else {
            panic!("law show");
        };

        assert_eq!(moment.coordinates(), (None, Some("2026-02-30")));
    }

    #[test]
    fn the_two_moment_flags_combine() {
        let result = Cli::try_parse_from([
            "bron",
            "law",
            "show",
            "BWBR0005289",
            "--as-of",
            "2023-06-01",
            "--as-known-on",
            "2024-01-01",
        ]);
        assert!(result.is_ok(), "a moment is a point in both windows");
    }

    /// A date the service reads is the service's to judge.
    #[test]
    fn a_malformed_date_goes_to_the_service_as_typed() {
        let service =
            Cli::try_parse_from(["bron", "law", "show", "BWBR0005289", "--as-of", "gisteren"]);
        assert!(service.is_ok(), "{service:?}");
    }

    #[test]
    fn json_is_available_on_every_subcommand() {
        let cli =
            Cli::try_parse_from(["bron", "--json", "law", "versions", "BWBR0005289"]).unwrap();
        assert!(cli.json);
    }
}
