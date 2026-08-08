//! Rechtspraak: published decisions of the Dutch courts.
//!
//! Two search endpoints exist and the choice between them is a determinism
//! decision. The open-data endpoint at data.rechtspraak.nl is documented,
//! needs no headers, and returns a stable ordering. The endpoint behind
//! uitspraken.rechtspraak.nl ranks by relevance, which is not reproducible
//! across time, and is used only where free text is unavoidable.
//!
//! Retrieval is always data.rechtspraak.nl. deeplink.rechtspraak.nl serves a
//! JavaScript shell with no content in the response body.

use anyhow::{Context, Result};
use chrono::NaiveDate;
use roxmltree::Document;
use serde::Deserialize;
use serde_json::json;

use super::xml;
use super::{Client, FetchedCase, Source};
use crate::ids::Ecli;
use crate::model::{CaseHit, Direction, Relation};

const SEARCH_ENDPOINT: &str = "https://uitspraken.rechtspraak.nl/api/zoek";
const CONTENT_ENDPOINT: &str = "https://data.rechtspraak.nl/uitspraken/content";

const NS_DCTERMS: &str = "http://purl.org/dc/terms/";
const NS_RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const NS_ECLI: &str = "https://e-justice.europa.eu/ecli";
const NS_PSI: &str = "http://psi.rechtspraak.nl/";
const NS_RECHTSPRAAK: &str = "http://www.rechtspraak.nl/schema/rechtspraak-1.0";

/// A fixed correlation id.
///
/// The endpoint only echoes this back for its own tracing. Generating a random
/// one per call would put a byte that changes between runs into an otherwise
/// identical request, which is exactly what reproducibility forbids.
const CORRELATION_ID: &str = "00000000-0000-0000-0000-000000000000";

/// Upper bound on the over-fetch used to satisfy client-side filters.
const MAX_PAGE_SIZE: u32 = 100;

/// Search results, with an explicit statement of whether the caller is seeing
/// everything that matched.
///
/// The court and date narrowings run client-side over a capped over-fetch, so a
/// short result set means either "nothing more matched" or "our window ran out"
/// and the caller cannot tell which. Saying so is cheaper than a dossier built
/// on a silently truncated list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResults {
    pub hits: Vec<CaseHit>,
    /// True when the over-fetch window was exhausted and matches may remain.
    pub truncated: bool,
    /// True when ordering came from upstream's relevance score, which is not
    /// reproducible across time.
    pub ordering_is_unstable: bool,
}

pub struct Query<'a> {
    pub terms: &'a str,
    /// Matched against the decision title, which carries the court name.
    pub court: Option<&'a str>,
    /// Decisions before this date are dropped.
    pub from: Option<NaiveDate>,
    pub limit: u32,
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default, rename = "Results")]
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    #[serde(rename = "TitelEmphasis")]
    ecli: Option<String>,
    #[serde(rename = "Titel")]
    title: Option<String>,
    #[serde(rename = "Tekstfragment")]
    fragment: Option<String>,
    #[serde(rename = "Uitspraakdatum")]
    date: Option<String>,
}

/// Ranked free-text search.
///
/// Ordering is upstream's relevance score and is not stable across time. Use
/// this to discover candidates, never to produce a result set something else
/// has to reproduce.
pub fn search(client: &Client, query: &Query) -> Result<SearchResults> {
    // Neither narrowing is available server-side in this payload shape, so when
    // one is set we ask for more rows than we need and drop the rest here.
    let page_size = if query.court.is_some() || query.from.is_some() {
        query.limit.saturating_mul(5).min(MAX_PAGE_SIZE)
    } else {
        query.limit
    };

    let payload = json!({
        "StartRow": 0,
        "PageSize": page_size,
        "ShouldReturnHighlights": false,
        "ShouldCountFacets": false,
        "SortOrder": "Relevance",
        "SearchTerms": [{ "Term": query.terms, "Field": "AlleVelden" }],
        "Contentsoorten": [],
        "Rechtsgebieden": [],
        "Instanties": [],
        "Proceduresoorten": [],
        "DatumPublicatie": [],
        "DatumUitspraak": [],
        "Advanced": { "PublicatieStatus": "AlleenGepubliceerd" },
        "CorrelationId": CORRELATION_ID,
    });

    let fetched = client.fetch(
        client
            .post(SEARCH_ENDPOINT)
            .header("Origin", "https://uitspraken.rechtspraak.nl")
            .header("Referer", "https://uitspraken.rechtspraak.nl/")
            .header("User-Agent", super::http::BROWSER_USER_AGENT)
            .json(&payload),
    )?;

    let response: SearchResponse = serde_json::from_str(&fetched.text)
        .context("rechtspraak search returned an unreadable body")?;

    let court = query.court.map(str::to_lowercase);
    let returned = response.results.len() as u32;

    let hits: Vec<CaseHit> = response
        .results
        .into_iter()
        .filter_map(|r| {
            let id = r.ecli?.parse::<Ecli>().ok()?;
            let decided_on = r.date.as_deref().and_then(dutch_date);

            if let Some(court) = &court
                && !r
                    .title
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(court)
            {
                return None;
            }
            if let (Some(from), Some(decided)) = (query.from, decided_on)
                && decided < from
            {
                return None;
            }

            Some(CaseHit {
                id,
                title: r.title,
                summary: r.fragment,
                decided_on,
            })
        })
        .take(query.limit as usize)
        .collect();

    // Upstream filled the page and the filters still left us short: there is no
    // way to distinguish exhausted from truncated, so report it as truncated.
    let truncated = hits.len() < query.limit as usize && returned >= page_size;

    Ok(SearchResults {
        hits,
        truncated,
        ordering_is_unstable: true,
    })
}

/// Fetches one decision by ECLI.
pub fn fetch(client: &Client, ecli: &Ecli) -> Result<FetchedCase> {
    let fetched = client
        .fetch(client.get(CONTENT_ENDPOINT).query(&[("id", ecli.as_str())]))
        .with_context(|| format!("cannot fetch {ecli}"))?;

    parse(Source::from(&fetched), ecli)
}

pub fn parse(source: Source<'_>, ecli: &Ecli) -> Result<FetchedCase> {
    let doc = Document::parse(source.text).with_context(|| format!("{ecli} has malformed XML"))?;
    let root = doc.root_element();

    let mut court = None;
    let mut decided_on = None;
    let mut published_on = None;
    let mut procedure_type = None;
    let mut summary = None;

    for description in xml::find_all(root, NS_RDF, "Description") {
        court = court.or_else(|| xml::text_of(description, NS_DCTERMS, "creator"));
        decided_on = decided_on.or_else(|| {
            xml::text_of(description, NS_DCTERMS, "date")
                .as_deref()
                .and_then(iso_date)
        });
        published_on = published_on.or_else(|| {
            xml::text_of(description, NS_DCTERMS, "issued")
                .as_deref()
                .and_then(iso_date)
        });
        procedure_type = procedure_type.or_else(|| xml::text_of(description, NS_DCTERMS, "type"));
        summary = summary.or_else(|| xml::text_of(description, NS_DCTERMS, "abstract"));
    }

    let summary = summary
        .or_else(|| xml::text_of(root, NS_RECHTSPRAAK, "inhoudsindicatie"))
        .map(|text| xml::tidy(&text))
        .filter(|text| !text.is_empty());

    // A decision's body is <uitspraak>; an advocate-general's opinion is
    // <conclusie>. Reading only the first leaves every PHR document empty, and
    // there are 75.397 of them: the reasoned analysis behind the judgment, and
    // the most cited material after the judgments themselves.
    let full_text = ["uitspraak", "conclusie"]
        .into_iter()
        .flat_map(|element| xml::find_all(root, NS_RECHTSPRAAK, element))
        .filter_map(xml::inner_text)
        .map(|section| xml::tidy(&section))
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok(FetchedCase {
        id: ecli.clone(),
        court,
        decided_on,
        published_on,
        procedure_type,
        summary,
        full_text: Some(full_text).filter(|s| !s.is_empty()),
        provenance: source.into(),
        relations: parse_relations(root),
    })
}

/// Formal relations to other decisions, in the direction upstream records them.
///
/// `psi:aanleg` names the direction: `eerdereAanleg` points back at a decision
/// that came before, `latereAanleg` forward at one that came after. Forward
/// links are what answer the appeal question, and their absence is a weaker
/// claim than their presence.
fn parse_relations(root: xml::Elem) -> Vec<Relation> {
    xml::find_all(root, NS_DCTERMS, "relation")
        .filter_map(|relation| {
            let target = relation
                .attribute((NS_ECLI, "resourceIdentifier"))?
                .parse::<Ecli>()
                .ok()?;

            let direction = match last_segment(relation.attribute((NS_PSI, "aanleg"))?) {
                "latereAanleg" => Direction::Later,
                "eerdereAanleg" => Direction::Earlier,
                _ => return None,
            };

            Some(Relation {
                target,
                kind: relation
                    .attribute((NS_PSI, "type"))
                    .map(|t| last_segment(t).to_string()),
                direction,
            })
        })
        .collect()
}

/// `http://psi.rechtspraak.nl/cassatie` -> `cassatie`.
fn last_segment(uri: &str) -> &str {
    uri.rsplit('/').next().unwrap_or(uri)
}

/// `17-09-2025`, the format the search endpoint returns.
fn dutch_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%d-%m-%Y")
        .or_else(|_| NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d"))
        .ok()
}

/// `2019-07-19`, the format the content endpoint returns. Timestamps carry a
/// time part that is not part of the date being recorded.
fn iso_date(text: &str) -> Option<NaiveDate> {
    let text = text.trim();
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .or_else(|| {
            text.get(..10)
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::BlobId;

    const CASE: &str = r#"
      <open-rechtspraak xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
                        xmlns:dcterms="http://purl.org/dc/terms/"
                        xmlns:ecli="https://e-justice.europa.eu/ecli"
                        xmlns:psi="http://psi.rechtspraak.nl/"
                        xmlns:rs="http://www.rechtspraak.nl/schema/rechtspraak-1.0">
        <rdf:RDF>
          <rdf:Description>
            <dcterms:creator>Hoge Raad</dcterms:creator>
            <dcterms:date>2019-07-19</dcterms:date>
            <dcterms:issued>2019-07-18</dcterms:issued>
            <dcterms:type>Uitspraak</dcterms:type>
            <dcterms:relation ecli:resourceIdentifier="ECLI:NL:PHR:2019:496"
                              psi:type="http://psi.rechtspraak.nl/conclusie"
                              psi:aanleg="http://psi.rechtspraak.nl/eerdereAanleg">Conclusie</dcterms:relation>
            <dcterms:relation ecli:resourceIdentifier="ECLI:NL:HR:2020:1"
                              psi:type="http://psi.rechtspraak.nl/cassatie"
                              psi:aanleg="http://psi.rechtspraak.nl/latereAanleg">Cassatie</dcterms:relation>
          </rdf:Description>
        </rdf:RDF>
        <rs:inhoudsindicatie>Onrechtmatige   daad.</rs:inhoudsindicatie>
        <rs:uitspraak>De   Hoge Raad   verwerpt het beroep.</rs:uitspraak>
      </open-rechtspraak>"#;

    fn parsed() -> FetchedCase {
        let ecli: Ecli = "ECLI:NL:HR:2019:1278".parse().unwrap();
        parse(
            Source::new(
                BlobId::of(CASE.as_bytes()),
                CASE,
                "https://test.invalid/c.xml",
            ),
            &ecli,
        )
        .unwrap()
    }

    #[test]
    fn reads_the_decision_metadata() {
        let case = parsed();
        assert_eq!(case.court.as_deref(), Some("Hoge Raad"));
        assert_eq!(case.decided_on, Some("2019-07-19".parse().unwrap()));
        assert_eq!(case.published_on, Some("2019-07-18".parse().unwrap()));
        assert_eq!(case.summary.as_deref(), Some("Onrechtmatige daad."));
        assert_eq!(
            case.full_text.as_deref(),
            Some("De Hoge Raad verwerpt het beroep.")
        );
    }

    #[test]
    fn reads_relations_with_their_direction() {
        let relations = parsed().relations;
        assert_eq!(relations.len(), 2);

        let earlier = relations
            .iter()
            .find(|r| r.direction == Direction::Earlier)
            .unwrap();
        assert_eq!(earlier.target.as_str(), "ECLI:NL:PHR:2019:496");
        assert_eq!(earlier.kind.as_deref(), Some("conclusie"));

        let later = relations
            .iter()
            .find(|r| r.direction == Direction::Later)
            .unwrap();
        assert_eq!(later.target.as_str(), "ECLI:NL:HR:2020:1");
        assert_eq!(later.kind.as_deref(), Some("cassatie"));
    }

    /// The Parket's opinions carry their text in <conclusie>. Reading only
    /// <uitspraak> silently returned nothing for all 75.397 of them, which read
    /// as 'this decision has no body' rather than as a bug.
    #[test]
    fn reads_an_advocate_generals_opinion() {
        let body = r#"
          <open-rechtspraak xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
                            xmlns:dcterms="http://purl.org/dc/terms/"
                            xmlns:rs="http://www.rechtspraak.nl/schema/rechtspraak-1.0">
            <rdf:RDF><rdf:Description>
              <dcterms:creator>Parket bij de Hoge Raad</dcterms:creator>
            </rdf:Description></rdf:RDF>
            <rs:conclusie><rs:section><rs:title>1 Inleiding</rs:title>
              <rs:para>De prejudiciele vraag stelt aan de orde of.</rs:para>
            </rs:section></rs:conclusie>
          </open-rechtspraak>"#;

        let ecli: Ecli = "ECLI:NL:PHR:2019:496".parse().unwrap();
        let case = parse(
            Source::new(BlobId::of(body.as_bytes()), body, "https://x.invalid"),
            &ecli,
        )
        .unwrap();

        let text = case.full_text.expect("an opinion has a body");
        assert!(text.contains("prejudiciele vraag"), "{text}");
    }

    #[test]
    fn parses_both_date_formats_and_rejects_the_rest() {
        assert_eq!(
            dutch_date("17-09-2025"),
            Some("2025-09-17".parse().unwrap())
        );
        assert_eq!(
            dutch_date("2025-09-17"),
            Some("2025-09-17".parse().unwrap())
        );
        assert_eq!(dutch_date("nonsense"), None);
        assert_eq!(dutch_date("32-01-2025"), None);
        assert_eq!(
            iso_date("2025-04-15T17:06:59"),
            Some("2025-04-15".parse().unwrap())
        );
        assert_eq!(iso_date(""), None);
    }
}
