//! Reading rulings through the bron service.
//!
//! The service fetches a ruling from rechtspraak on the first read that asks
//! for it, freezes the bytes and answers every later read from them, so a
//! ruling is one call like any article. Every register's ruling answers one
//! core; what only one publisher states stands in `native`, in its words, and
//! is carried here unread.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::error::Outcome;
use crate::transport::Client;

use super::client::Answered;
use super::model::Span;
use super::{Service, window};

/// A ruling or a conclusie under its ECLI, or a judgment or an opinion of the
/// EU's courts.
#[derive(Debug, Deserialize)]
pub struct Decision {
    /// The ECLI, in capitals.
    pub id: String,
    /// Who published it: `rechtspraak`, `cellar` for the EU's courts, `acm`.
    pub register: String,
    /// `ruling`, or `opinion` for the conclusie of an advocate-general.
    pub form: String,
    pub court: Option<String>,
    pub decided_on: Option<NaiveDate>,
    pub published_on: Option<NaiveDate>,
    /// The publisher's own stamp on the text held.
    pub known_since: Option<String>,
    /// Whether the publisher serves the text, or the metadata alone.
    pub has_body: bool,
    pub rendition: Option<DecisionRendition>,
    /// The digests of the texts the publisher replaced or withdrew before
    /// this one. The service serves none of them.
    #[serde(default)]
    pub superseded: Vec<String>,
    /// The publisher's own fields, whose shape is the register's.
    #[serde(default)]
    pub native: serde_json::Map<String, serde_json::Value>,
}

impl Decision {
    /// One of the publisher's own fields, every value it states joined, where
    /// it states any.
    pub fn native(&self, field: &str) -> Option<String> {
        let values: Vec<&str> = match self.native.get(field)? {
            serde_json::Value::String(value) => vec![value],
            serde_json::Value::Array(values) => values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect(),
            _ => return None,
        };

        (!values.is_empty()).then(|| values.join(", "))
    }
}

/// The document a ruling's spans point into, or of a case of the EU the
/// documents its package holds, each paragraph naming its own.
#[derive(Debug, Deserialize)]
pub struct DecisionRendition {
    pub language: Option<String>,
    pub blob: Option<String>,
    pub blob_size: Option<u64>,
    #[serde(default)]
    pub documents: Vec<DecisionDocument>,
}

/// One Formex document of a case of the EU.
#[derive(Debug, Deserialize)]
pub struct DecisionDocument {
    pub blob: String,
    pub blob_size: u64,
    /// `JUDGMENT`, `CONCLUSION`, or `REPORT.HEARING` for the report for the
    /// hearing an older case prints beside its judgment.
    pub root: String,
}

/// A ruling and the units of its text, each at its span in the bytes held.
#[derive(Debug, Deserialize)]
pub struct DecisionParagraphs {
    pub decision: Decision,
    pub paragraphs: Vec<DecisionParagraph>,
}

/// One unit of a ruling's text: the inhoudsindicatie, a section's title, a
/// paragraph of the body or of a footnote.
#[derive(Debug, Deserialize)]
pub struct DecisionParagraph {
    pub ordinal: u32,
    /// Of a case of the EU, the document the span points into.
    pub document: Option<String>,
    pub kind: String,
    /// Of a case of the EU, the number the Court prints, `45`, or `1)` in
    /// the operative part, which is how a paragraph is cited.
    pub number: Option<String>,
    /// How the court numbered the sections around it, `2/2.1`, or
    /// `voetnoot 3`. Absent where it stands in none.
    pub section_path: Option<String>,
    pub span: Span,
    pub content_hash: String,
    /// bron's rendering, present where `rendered` was asked.
    pub rendered: Option<String>,
    /// The publisher's bytes at the span, present where `raw` was asked.
    pub raw: Option<String>,
}

/// What documents state connects a ruling to something else.
#[derive(Debug, Deserialize)]
pub struct DecisionRelations {
    pub id: String,
    pub relations: Vec<DecisionRelation>,
}

/// One relation, in the words of whoever stated it.
#[derive(Debug, Deserialize)]
pub struct DecisionRelation {
    /// The stating document's own word for it: `conclusie`, `cassatie`.
    pub kind: String,
    /// The publisher's word for which way the other lies: `eerdereAanleg` or
    /// `latereAanleg`. Absent on a relation that states no instance.
    pub aanleg: Option<String>,
    pub to: Target,
    /// `publisher` where the document states it, `machine` where bron read it.
    pub asserted: String,
    pub stated_by: String,
    /// The digest of the document that states it.
    pub stated_in: Option<String>,
}

/// What a relation points at, under its own scheme.
#[derive(Debug, Deserialize)]
pub struct Target {
    pub scheme: String,
    pub id: String,
    /// A part of it, where the relation names one.
    pub part: Option<String>,
}

/// A question over the rulings the service holds. Every field the caller
/// typed is sent as typed and judged by the service; an untyped one is left
/// out.
#[derive(Debug, Serialize)]
pub struct DecisionQuery<'a> {
    pub q: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub court: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub register: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<&'a str>,
}

/// The rulings nearest a question, by identifier.
#[derive(Debug, Deserialize)]
pub struct DecisionSearch {
    pub hits: Vec<DecisionHit>,
}

/// One ruling a question ranked, named by the run of its units that ranked
/// first.
#[derive(Debug, Deserialize)]
pub struct DecisionHit {
    /// The ECLI, or `acm:` and the node of a besluit of the ACM, which states
    /// no ECLI.
    pub id: String,
    pub form: String,
    pub court: Option<String>,
    pub decided_on: Option<NaiveDate>,
    pub paragraphs: OrdinalRange,
    pub content_hashes: Vec<String>,
    pub blob: String,
    pub distance: f64,
}

/// A run of a ruling's units, by their ordinals.
#[derive(Debug, Deserialize)]
pub struct OrdinalRange {
    pub from: u32,
    pub to: u32,
}

/// Which of the rulings that cite an article, or that rechtspraak's feed
/// listed, to answer. Every field the caller typed is sent as typed and judged
/// by the service; an untyped one is left out.
#[derive(Debug)]
pub struct Citing<'a> {
    pub court: Option<&'a str>,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub limit: Option<&'a str>,
    pub offset: Option<&'a str>,
}

impl Citing<'_> {
    fn query(&self) -> Vec<(&'static str, &str)> {
        let mut query = Vec::new();
        window(&mut query, self.limit, self.offset);

        if let Some(court) = self.court {
            query.push(("court", court));
        }
        if let Some(day) = self.from {
            query.push(("from", day));
        }
        if let Some(day) = self.to {
            query.push(("to", day));
        }

        query
    }
}

/// The rulings LiDO found that cite one article, newest decided first.
#[derive(Debug, Deserialize)]
pub struct ArticleDecisions {
    pub regulation_id: String,
    pub number: String,
    /// How many rulings the question matches, of which `decisions` is a page.
    pub count: u64,
    /// Every court with a ruling found under the dates asked, most first,
    /// whatever `court` asked.
    pub courts: Vec<CourtCount>,
    pub decisions: Vec<CitingDecision>,
}

#[derive(Debug, Deserialize)]
pub struct CitingDecision {
    pub ecli: String,
    /// Absent where LiDO names no court for the ruling.
    pub court: Option<String>,
    pub decided_on: Option<NaiveDate>,
    /// Whether the service holds the ruling.
    pub held: bool,
    /// The label of the regulation's rendition in force on `decided_on`, by
    /// the service's arithmetic. Absent where the ruling states no day or no
    /// rendition was in force on it.
    pub rendition: Option<String>,
}

/// The rulings with a text rechtspraak's feed listed, newest decided first.
#[derive(Debug, Deserialize)]
pub struct RecentDecisions {
    /// How many rulings the question matches, of which `decisions` is a page.
    pub count: u64,
    /// Every court with a ruling listed under the dates asked, most first,
    /// whatever `court` asked.
    pub courts: Vec<CourtCount>,
    pub decisions: Vec<ListedDecision>,
}

#[derive(Debug, Deserialize)]
pub struct CourtCount {
    pub court: String,
    pub count: u64,
}

#[derive(Debug, Deserialize)]
pub struct ListedDecision {
    pub ecli: String,
    /// Absent where the feed's title does not name one.
    pub court: Option<String>,
    /// Absent where the feed's title does not state one.
    pub decided_on: Option<NaiveDate>,
    /// The zaaknummer or zaaknummers as the feed's title prints them. Absent
    /// where it prints none.
    pub case_numbers: Option<String>,
    /// The inhoudsindicatie as the feed prints it, cut short by the publisher.
    /// Absent where it prints none, for a criminal case, and while the service
    /// has not read whether the ruling is criminal law.
    pub summary: Option<String>,
    /// `criminal_law` where the service leaves a criminal case's summary out.
    pub summary_left_out: Option<String>,
    /// Whether the service holds the ruling. One it does not is fetched on the
    /// first read that asks for it.
    pub held: bool,
}

impl Service {
    pub fn article_decisions(
        &self,
        client: &Client,
        law: &str,
        number: &str,
        ask: &Citing<'_>,
    ) -> Outcome<Answered<ArticleDecisions>> {
        let url = self.route(&["regulations", law, "articles", number, "decisions"])?;

        self.read(client, client.get(&url).query(&ask.query()))
    }

    /// The rulings with a text rechtspraak's feed listed, newest decided first.
    pub fn recent_decisions(
        &self,
        client: &Client,
        ask: &Citing<'_>,
    ) -> Outcome<Answered<RecentDecisions>> {
        let url = self.route(&["decisions"])?;

        self.read(client, client.get(&url).query(&ask.query()))
    }

    /// The rulings the service holds, ranked against a question.
    pub fn decision_search(
        &self,
        client: &Client,
        ask: &DecisionQuery<'_>,
    ) -> Outcome<Answered<DecisionSearch>> {
        let url = self.route(&["decisions", "search"])?;

        self.read(client, client.post(&url).json(ask))
    }

    /// A ruling and its text, rendered. `limit` and `offset` page its units,
    /// and `lang` names the language a case of the EU is read in.
    pub fn decision(
        &self,
        client: &Client,
        ecli: &str,
        raw: bool,
        lang: Option<&str>,
        limit: Option<&str>,
        offset: Option<&str>,
    ) -> Outcome<Answered<DecisionParagraphs>> {
        let url = self.route(&["decisions", ecli, "paragraphs"])?;

        let mut query = vec![("rendered", "1")];
        window(&mut query, limit, offset);
        if raw {
            query.push(("raw", "1"));
        }
        if let Some(lang) = lang {
            query.push(("lang", lang));
        }

        self.read(client, client.get(&url).query(&query))
    }

    /// What documents state connects a ruling to others.
    pub fn decision_relations(
        &self,
        client: &Client,
        ecli: &str,
    ) -> Outcome<Answered<DecisionRelations>> {
        let url = self.route(&["decisions", ecli, "relations"])?;

        self.read(client, client.get(&url))
    }
}
