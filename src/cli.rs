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
use bron::sources::wayback;
use bron::wire::{Grade, View};

#[derive(Debug, Parser)]
#[command(
    name = "bron",
    version,
    about = "Dutch primary legal sources on your own disk",
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

    /// The web archive: pages as a third party recorded them.
    ///
    /// What this answers is a different claim from the rest of bron. A statute
    /// or a ruling speaks with its own authority; a capture is the Internet
    /// Archive's record that a page said something on a day. Every answer here
    /// is graded `archive` and dated to its capture, never to today.
    #[command(subcommand)]
    Archive(Archive),

    /// Fetches newer versions of laws already held.
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

        #[arg(long, default_value_t = 20)]
        limit: u32,

        #[command(flatten)]
        moment: Moment,
    },
}

impl Command {
    /// What an answer from this command rests on.
    ///
    /// Known from the command alone, which is what lets a failure carry the
    /// right grade: a lookup that could not be answered is still a lookup
    /// against the archive, not against an authority.
    pub fn grade(&self) -> Grade {
        match self {
            Self::Archive(_) => Grade::Archive,
            _ => Grade::Authority,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Archive {
    /// Lists what the archive holds for an address, without fetching any of it.
    ///
    /// A row whose status is withheld is still listed: the archive records that
    /// for a crawl excluded by robots.txt, and there being a capture whose
    /// result is unknown is a different fact from there being no capture.
    Captures {
        /// The address as captured, e.g. https://example.com/pricing.
        url: String,

        /// Only captures taken on or after this date.
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: Option<NaiveDate>,

        /// Only captures taken on or before this date, inclusive of the day.
        #[arg(long, value_name = "YYYY-MM-DD")]
        to: Option<NaiveDate>,

        #[arg(long, default_value_t = 100)]
        limit: u32,

        /// How the address matches. A domain-wide enumeration of a large site
        /// runs to millions of rows; prefix is the usable form.
        #[arg(long, value_enum, default_value_t = MatchArg::Exact)]
        match_type: MatchArg,
    },

    /// Shows a page as it stood on a date.
    ///
    /// Resolves the newest capture taken at or before `--as-of`, fetches its
    /// bytes unmodified, and checks them against the digest the index recorded.
    Show {
        url: String,

        /// The date to answer for. Without it, the newest capture there is.
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_of: Option<NaiveDate>,
    },

    /// Asks the archive to capture a page that it may not hold yet.
    ///
    /// The only command in bron that writes to a third party, and the only one
    /// that needs credentials: BRON_IA_ACCESS_KEY and BRON_IA_SECRET_KEY from
    /// archive.org/account/s3.php. Reading the archive needs neither.
    ///
    /// Nothing is reported as captured until an index row postdating the
    /// submission confirms it. Submitting a URL again returns the job of the
    /// capture already held, so a job id never establishes that anything new
    /// was made.
    Capture {
        url: String,

        /// Do not make a new capture when one exists inside this many days.
        #[arg(long, value_name = "DAYS")]
        if_not_archived_within: Option<u32>,

        /// Also capture a rendered screenshot. Worth having when the claim is
        /// in the layout of a pricing table rather than in prose.
        #[arg(long)]
        screenshot: bool,

        /// How long to wait for the archive before giving up on it.
        #[arg(long, value_name = "SECONDS", default_value_t = 300)]
        timeout: u64,
    },
}

/// How a CDX query matches the address given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum MatchArg {
    Exact,
    Prefix,
    Host,
    Domain,
}

impl From<MatchArg> for wayback::MatchType {
    fn from(arg: MatchArg) -> Self {
        match arg {
            MatchArg::Exact => Self::Exact,
            MatchArg::Prefix => Self::Prefix,
            MatchArg::Host => Self::Host,
            MatchArg::Domain => Self::Domain,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Law {
    /// Searches by citeertitel. Returns candidates, not text.
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },

    /// Shows a law, or one article from it.
    Show {
        /// BWB identifier, e.g. BWBR0005289.
        id: String,

        /// Article number as printed, e.g. 162 or 162a.
        #[arg(long)]
        article: Option<String>,

        /// One lid of that article, as the register numbers it.
        ///
        /// Needs --article: a lid number alone is not an address, since every
        /// article has a lid 1.
        #[arg(long, requires = "article")]
        paragraph: Option<String>,

        /// One item of that lid, as the register letters it: `a`, `b`, `2°`.
        ///
        /// The span reported stays the lid's. An item has no span of its own
        /// in this corpus, and naming the lid's is the honest reading rather
        /// than an offset nobody measured.
        #[arg(long, requires = "paragraph")]
        item: Option<String>,

        /// Also carry the publisher's own bytes at every span.
        ///
        /// Without it an answer carries the offsets and bron's reading, and no
        /// markup travels. With it each excerpt also carries the slice itself,
        /// on the service's word: checking it means fetching the blob it was
        /// cut from, and not having to is the reason it is offered at all.
        #[arg(long)]
        raw: bool,

        #[command(flatten)]
        moment: Moment,
    },

    /// Every published version of a law, with its validity windows.
    Versions { id: String },

    /// What changed in a law between two dates.
    ///
    /// Articles are matched on upstream's own identity, so a renumbered article
    /// reads as one amendment rather than a removal plus an addition. Both
    /// texts travel with the answer, each from its own frozen document.
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

        /// Also list articles that did not change.
        #[arg(long)]
        all: bool,

        /// Only this article number.
        #[arg(long)]
        article: Option<String>,

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
        /// Filters on the court, e.g. 'Hoge Raad'.
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
/// Two windows, two questions, and one flag cannot choose between them.
/// Geldigheid says what applied, which is what you want to know about conduct
/// in 2023. Zicht says what the register showed, which is what you want to know
/// about what someone could have known then. They diverge the moment something
/// is published with retroactive effect.
#[derive(Debug, Args)]
#[group(multiple = false)]
pub struct Moment {
    /// The text as it applied on this date (geldigheid).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub as_of: Option<NaiveDate>,

    /// The text as the register showed it on this date (zicht).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub as_known_on: Option<NaiveDate>,
}

impl Moment {
    /// Resolves once, at startup, so a run cannot straddle midnight by reading
    /// the clock twice.
    pub fn resolve(&self, today: NaiveDate) -> (NaiveDate, View) {
        match (self.as_of, self.as_known_on) {
            (_, Some(date)) => (date, View::AsKnown),
            (Some(date), _) => (date, View::InForce),
            // Unstated means now, and now means what is in force.
            (None, None) => (today, View::InForce),
        }
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

    #[test]
    fn an_unstated_moment_means_what_is_in_force_today() {
        let moment = Moment {
            as_of: None,
            as_known_on: None,
        };
        assert_eq!(
            moment.resolve(date("2026-08-06")),
            (date("2026-08-06"), View::InForce)
        );
    }

    #[test]
    fn as_of_and_as_known_on_select_different_windows() {
        let in_force = Moment {
            as_of: Some(date("2023-06-01")),
            as_known_on: None,
        };
        assert_eq!(
            in_force.resolve(date("2026-08-06")),
            (date("2023-06-01"), View::InForce)
        );

        let as_known = Moment {
            as_of: None,
            as_known_on: Some(date("2023-06-01")),
        };
        assert_eq!(
            as_known.resolve(date("2026-08-06")),
            (date("2023-06-01"), View::AsKnown)
        );
    }

    #[test]
    fn the_two_moment_flags_cannot_be_combined() {
        let result = Cli::try_parse_from([
            "bron",
            "law",
            "show",
            "BWBR0005289",
            "--as-of",
            "2023-06-01",
            "--as-known-on",
            "2023-06-01",
        ]);
        assert!(result.is_err(), "one date cannot mean two windows at once");
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
