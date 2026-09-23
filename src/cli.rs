//! The command line.
//!
//! English throughout. Everything else a caller touches is already English
//! (flags, JSON keys, error kinds), the main consumer is code rather than a
//! person, and the sources will not stay Dutch: `wet` is the wrong word for an
//! EU directive.
//!
//! Dutch survives only where a term has no English equivalent and translating
//! it would lose precision: `citeertitel`, `geldigheid`, `zicht`, `lid`.

use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand, ValueEnum};

use bron::sources::publications::Collection;

#[derive(Debug, Parser)]
#[command(
    name = "bron",
    version,
    about = "Dutch primary legal sources, quotable to the byte",
    long_about = "Answers what Dutch law said on a date, and what the courts and the \
                  official publications say, naming the document behind every answer and \
                  the byte range inside it.\n\n\
                  Exit codes: 0 ok, 1 usage, 2 not found, 3 network, 4 upstream, 5 local."
)]
pub struct Cli {
    /// Machine-readable output. Implies --quiet.
    #[arg(long, global = true)]
    pub json: bool,

    /// No progress reporting on stderr.
    #[arg(long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Legislation from the Basis Wetten Bestand (BWB).
    #[command(subcommand)]
    Law(Law),

    /// Published decisions of the Dutch courts (Rechtspraak).
    #[command(subcommand)]
    Case(Case),

    /// Official publications (Officiële Bekendmakingen): kamerstukken,
    /// Staatsblad, Staatscourant, Tractatenblad.
    #[command(subcommand)]
    Publication(Publication),

    /// Answers which article, from a question or from a citation.
    ///
    /// Distinct from `law search`, which resolves a citeertitel to a BWB id and
    /// answers which law. This answers which article, across every corpus bron
    /// holds vectors for, and its answer says which of them it actually
    /// searched.
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

        #[arg(long, default_value_t = 20)]
        limit: u32,

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

        #[arg(long, default_value_t = 20)]
        limit: u32,

        /// How many rows to skip.
        #[arg(long, default_value_t = 0)]
        offset: u32,

        #[command(flatten)]
        moment: Moment,
    },

    /// Shows a law's articles, or one of them.
    ///
    /// Articles only: `artikel`, or `enig-artikel` for a regulation's one
    /// article. A bijlage, the aanhef, the closing formula and every other unit
    /// a rendition prints beside its articles is `law parts`.
    Show {
        /// BWB identifier, e.g. BWBR0005289.
        id: String,

        /// Article number as printed, e.g. 162 or 162a.
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

        /// One lid of that article, as the register numbers it.
        ///
        /// Needs --article: a lid number alone is not an address, since every
        /// article has a lid 1. The service answers the lid, so the span, the
        /// Juriconnect reference and the label are the lid's: an answer about
        /// lid 2 that cites the article's bytes is coarser than the claim it
        /// supports.
        #[arg(long, requires = "article")]
        paragraph: Option<String>,

        /// One item of that lid, as the register letters it: `a`, `b`, `2°`.
        ///
        /// A cut of what a person reads, and the one narrowing no route
        /// answers: an item has no span of its own in this corpus, so the
        /// answer stays the lid's rather than naming an offset nobody
        /// measured. With --json the envelope is the lid as the service sent
        /// it. An item nothing opens is refused either way.
        #[arg(long, requires = "paragraph")]
        item: Option<String>,

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
        /// BWB identifier, e.g. BWBR0001941.
        id: String,

        /// One kind of unit, as the register names the element: bijlage,
        /// aanhef, tekst, wetsluiting, artikel.
        #[arg(long)]
        kind: Option<String>,

        /// The number the register prints for a unit of --kind, e.g. Ia.
        #[arg(long, requires = "kind", conflicts_with = "position")]
        number: Option<String>,

        /// The place of one unit among the units of --kind, from 1: how a unit
        /// that prints no number is asked for.
        #[arg(long, requires = "kind", value_parser = clap::value_parser!(u32).range(1..))]
        position: Option<u32>,

        /// One lid of that unit, as the register numbers it. The span, the
        /// text and the reference become the lid's.
        #[arg(long)]
        paragraph: Option<String>,

        /// Only the units this rendition publishes and does not yet apply, of
        /// every kind: a waiting bijlage beside the waiting articles. Each
        /// names the day it applies where the register states one. See
        /// `law show --not-commenced`.
        #[arg(long)]
        not_commenced: bool,

        /// Also carry the publisher's own bytes at every span. See `law show`.
        #[arg(long)]
        raw: bool,

        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..))]
        limit: u32,

        /// How many units to skip.
        #[arg(long, default_value_t = 0)]
        offset: u32,

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

        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..))]
        limit: u32,

        /// How many rows to skip.
        #[arg(long, default_value_t = 0)]
        offset: u32,
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

        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..))]
        limit: u32,

        /// How many rows to skip.
        #[arg(long, default_value_t = 0)]
        offset: u32,

        #[command(flatten)]
        moment: Moment,
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

        /// The number as printed, e.g. 119a, or a bijlage's with --kind.
        #[arg(long)]
        article: String,

        /// The kind of unit the number is printed under, where it is no
        /// article: bijlage.
        #[arg(long)]
        kind: Option<String>,

        /// The day whose view of the register counts (zicht).
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_known_on: Option<NaiveDate>,
    },

    /// Every published version of a law, with its validity windows.
    Versions { id: String },

    /// Which renditions the register published in a window, across every law.
    ///
    /// A question about a period that has passed, and reproducible: the same
    /// window answers the same renditions next year. It is the wrong command to
    /// poll with. The publisher publishes retroactively, so a rendition that
    /// arrives tomorrow can carry a date inside a window already read, and a
    /// poller that walks windows never sees it. `law feed` misses nothing.
    Published {
        /// The first day of the window on geldigheid: what took effect on or after it.
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: Option<NaiveDate>,

        /// The last day of that window.
        #[arg(long, value_name = "YYYY-MM-DD")]
        to: Option<NaiveDate>,

        /// The first day of the window on zicht: what the register began showing on or after it.
        #[arg(long, value_name = "YYYY-MM-DD")]
        known_from: Option<NaiveDate>,

        /// The last day of that window.
        #[arg(long, value_name = "YYYY-MM-DD")]
        known_to: Option<NaiveDate>,

        /// Only laws of this type, as the register spells it: wet, AMvB, zbo.
        #[arg(long = "type", value_name = "TYPE")]
        kind: Option<String>,

        /// Only this law, by its BWB identifier.
        #[arg(long)]
        regulation: Option<String>,

        #[arg(long, default_value_t = 50)]
        limit: u32,

        /// How many rows to skip, to read past a list cut at its window.
        #[arg(long, default_value_t = 0)]
        offset: u32,
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
        /// The cursor to walk from. 0 for the whole log.
        #[arg(long)]
        since: u64,

        /// Only this law, by its BWB identifier.
        #[arg(long)]
        regulation: Option<String>,

        #[arg(long, default_value_t = 100)]
        limit: u32,
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
    /// Treaties are left out and counted, and an act the BWB does not print
    /// yet is not in the list. `law parts ID --not-commenced` reads one
    /// regulation's waiting units of every kind, and `law show ID
    /// --not-commenced` its waiting articles.
    Upcoming {
        /// Only regulations with a waiting unit that states a day on or after
        /// this one. Without --from and --to, regulations whose units state
        /// no day are listed too.
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: Option<NaiveDate>,

        /// Only regulations with a waiting unit that states a day on or
        /// before this one.
        #[arg(long, value_name = "YYYY-MM-DD")]
        to: Option<NaiveDate>,

        /// Only laws of this type, as the register spells it: wet, AMvB, zbo.
        #[arg(long = "type", value_name = "TYPE")]
        kind: Option<String>,

        /// How many rows to answer, 500 at most.
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=500))]
        limit: u32,

        /// How many rows to skip, to read past a list cut at its window.
        #[arg(long, default_value_t = 0)]
        offset: u32,

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
        id: String,

        /// The earlier date, resolved against geldigheid.
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: NaiveDate,

        /// The later date. Defaults to today.
        #[arg(long, value_name = "YYYY-MM-DD")]
        to: Option<NaiveDate>,

        /// Also list units that did not change.
        #[arg(long)]
        all: bool,

        /// Only units of this kind, as the register names the element:
        /// artikel, bijlage, aanhef, wetsluiting.
        #[arg(long)]
        kind: Option<String>,

        /// Only units carrying this number as printed on either side: an
        /// article's, or with --kind a bijlage's.
        #[arg(long)]
        article: Option<String>,

        /// The day the register is read on (zicht), which selects what it
        /// showed then. Both days of the comparison are resolved against it.
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_known_on: Option<NaiveDate>,

        /// Also carry the publisher's own bytes at every span. See `law show`.
        #[arg(long)]
        raw: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum Case {
    /// Searches decisions by text.
    ///
    /// Ordering comes from rechtspraak.nl's relevance score and is not
    /// reproducible over time. The answer says so itself.
    Search {
        query: Vec<String>,
        /// Only decisions of this court, by its whole name, e.g. 'Hoge Raad'.
        #[arg(long)]
        court: Option<String>,
        /// Only decisions from this date onwards.
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: Option<NaiveDate>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },

    /// Fetches one decision by ECLI.
    Show { ecli: String },

    /// What the decision's own relations say about appeal or cassation.
    ///
    /// Two answers and never a boolean: appealed, or no_published_successor
    /// dated to the moment bron read the decision. The second is a claim about
    /// what the register held then, not about whether an appeal was brought.
    Appeal { ecli: String },
}

#[derive(Debug, Subcommand)]
pub enum Publication {
    /// Searches official publications.
    Search {
        query: Vec<String>,
        #[arg(long, value_enum, default_value_t = CollectionArg::Kamerstuk)]
        collection: CollectionArg,
        #[arg(long)]
        dossier: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },

    /// Fetches one publication, e.g. kst-33669-6.
    Show { id: String },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CollectionArg {
    Kamerstuk,
    Staatsblad,
    Staatscourant,
    Tractatenblad,
}

impl From<CollectionArg> for Collection {
    fn from(arg: CollectionArg) -> Self {
        match arg {
            CollectionArg::Kamerstuk => Self::Kamerstuk,
            CollectionArg::Staatsblad => Self::Staatsblad,
            CollectionArg::Staatscourant => Self::Staatscourant,
            CollectionArg::Tractatenblad => Self::Tractatenblad,
        }
    }
}

/// Which moment a question about legislation means.
///
/// Two windows, and a moment is a point in both. Geldigheid says what applied,
/// which is what you want to know about conduct in 2023. Zicht says what the
/// register showed, which is what you want to know about what someone could
/// have known then. They diverge the moment something is published with
/// retroactive effect, so a question may name a day in each.
#[derive(Debug, Args)]
pub struct Moment {
    /// The text as it applied on this date (geldigheid).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub as_of: Option<NaiveDate>,

    /// The text as the register showed it on this date (zicht).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub as_known_on: Option<NaiveDate>,
}

impl Moment {
    /// The two coordinates as the service is asked for them: the day the text
    /// is for, and the day the register is read on where one was named.
    ///
    /// A zicht date alone names both: the text for a day, as the register had
    /// it that day. Sent without the first, the service reads that coordinate
    /// as today and resolves a rendition in force now, which for anything
    /// amended since is none at all.
    ///
    /// `today` is read once, at startup, so a run cannot straddle midnight by
    /// reading the clock twice.
    pub fn coordinates(&self, today: NaiveDate) -> (NaiveDate, Option<NaiveDate>) {
        let as_of = self.as_of.or(self.as_known_on).unwrap_or(today);

        (as_of, self.as_known_on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    fn moment(as_of: Option<&str>, as_known_on: Option<&str>) -> Moment {
        Moment {
            as_of: as_of.map(date),
            as_known_on: as_known_on.map(date),
        }
    }

    #[test]
    fn an_unstated_moment_means_what_is_in_force_today() {
        assert_eq!(
            moment(None, None).coordinates(date("2026-08-06")),
            (date("2026-08-06"), None)
        );
    }

    #[test]
    fn a_moment_is_a_point_in_both_windows() {
        let today = date("2026-08-06");

        assert_eq!(
            moment(Some("2023-06-01"), None).coordinates(today),
            (date("2023-06-01"), None)
        );
        assert_eq!(
            moment(None, Some("2023-06-01")).coordinates(today),
            (date("2023-06-01"), Some(date("2023-06-01")))
        );
        assert_eq!(
            moment(Some("2023-06-01"), Some("2024-01-01")).coordinates(today),
            (date("2023-06-01"), Some(date("2024-01-01")))
        );
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

    #[test]
    fn a_malformed_date_is_refused_by_the_parser() {
        let result =
            Cli::try_parse_from(["bron", "law", "show", "BWBR0005289", "--as-of", "gisteren"]);
        assert!(result.is_err());
    }

    #[test]
    fn json_and_quiet_are_available_on_every_subcommand() {
        let cli =
            Cli::try_parse_from(["bron", "--json", "law", "versions", "BWBR0005289"]).unwrap();
        assert!(cli.json);
    }
}
