//! Officiële bekendmakingen: everything the state publishes about its own law.
//!
//! One SRU service and one query grammar cover kamerstukken (parliamentary
//! papers), Staatsblad, Staatscourant and Tractatenblad. They differ only by
//! `w.publicatienaam`, so they are one source here rather than four.

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use roxmltree::Document;

use super::xml::{self, Elem};
use super::{Client, FetchedPublication, Source};
use crate::ids::PublicationId;
use crate::model::PublicationHit;

const ENDPOINT: &str = "https://repository.overheid.nl/sru";

const NS_SRU: &str = "http://docs.oasis-open.org/ns/search-ws/sruResponse";
const NS_GZD: &str = "http://standaarden.overheid.nl/sru";
const NS_WETGEVING: &str = "http://standaarden.overheid.nl/wetgeving/";
const NS_DCTERMS: &str = "http://purl.org/dc/terms/";

/// Removes CQL's unescapable quote without joining the words around it.
///
/// CQL string literals have no escape syntax, so a quote in a term would
/// terminate it early and draw a parse diagnostic instead of results. Replaced
/// with a space and never removed: deleting it welds two words into a third
/// nobody asked for.
fn strip_quotes(text: &str) -> String {
    text.replace('"', " ")
}

/// Which collection to search. The values are upstream's own
/// `w.publicatienaam`, which is why they are Dutch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collection {
    Kamerstuk,
    Staatsblad,
    Staatscourant,
    Tractatenblad,
}

impl Collection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kamerstuk => "Kamerstuk",
            Self::Staatsblad => "Staatsblad",
            Self::Staatscourant => "Staatscourant",
            Self::Tractatenblad => "Tractatenblad",
        }
    }
}

/// What the index says about a document, and where its body is.
struct Record {
    hit: PublicationHit,
    url: Option<String>,
}

pub struct Query<'a> {
    pub terms: &'a str,
    pub collection: Collection,
    pub dossier: Option<&'a str>,
    pub limit: u32,
}

pub fn search(client: &Client, query: &Query) -> Result<Vec<PublicationHit>> {
    let mut cql: Vec<String> = query
        .terms
        .split_whitespace()
        .map(|term| format!("cql.textAndIndexes=\"{}\"", strip_quotes(term)))
        .collect();

    cql.push(format!(
        "w.publicatienaam=\"{}\"",
        query.collection.as_str()
    ));
    if let Some(dossier) = query.dossier {
        cql.push(format!("w.dossiernummer=\"{}\"", strip_quotes(dossier)));
    }

    Ok(fetch_records(client, &cql.join(" and "), query.limit)?
        .into_iter()
        .map(|r| r.hit)
        .collect())
}

/// Fetches a document by id, e.g. `kst-33669-6`, and hashes its body where the
/// register links one.
pub fn fetch(client: &Client, id: &PublicationId) -> Result<FetchedPublication> {
    let cql = format!("dt.identifier=\"{id}\"");
    let record = fetch_records(client, &cql, 1)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no publication with id {id}"))?;

    // Not every record links an XML body, and the answer says which it is.
    let body = match record.url.as_deref() {
        Some(url) if url.ends_with(".xml") => Some(
            client
                .read_url(url)
                .with_context(|| format!("cannot fetch the body of {id}"))?,
        ),
        _ => None,
    };

    Ok(FetchedPublication {
        id: record.hit.id,
        title: record.hit.title,
        dossier: record.hit.dossier,
        published_on: record.hit.published_on,
        provenance: body.as_ref().map(|f| Source::from(f).into()),
    })
}

fn fetch_records(client: &Client, cql: &str, limit: u32) -> Result<Vec<Record>> {
    let fetched = client.read(client.get(ENDPOINT).query(&[
        ("operation", "searchRetrieve"),
        ("version", "2.0"),
        ("maximumRecords", &limit.to_string()),
        ("query", cql),
    ]))?;

    parse_records(&fetched.text)
}

fn parse_records(body: &str) -> Result<Vec<Record>> {
    let doc = Document::parse(body).context("the SRU search returned malformed XML")?;

    let mut records = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for record in xml::find_all(doc.root_element(), NS_SRU, "record") {
        let Some(kern) = xml::find(record, NS_WETGEVING, "owmskern") else {
            continue;
        };
        let Some(id) = xml::text_of(kern, NS_DCTERMS, "identifier") else {
            continue;
        };
        let Ok(id) = id.parse::<PublicationId>() else {
            continue;
        };

        if !seen.insert(id.clone()) {
            continue;
        }

        let mantel = xml::find(record, NS_WETGEVING, "owmsmantel");
        let tpmeta = xml::find(record, NS_WETGEVING, "tpmeta");

        records.push(Record {
            hit: PublicationHit {
                id,
                title: xml::text_of(kern, NS_DCTERMS, "title"),
                dossier: field(tpmeta, NS_WETGEVING, "dossiernummer"),
                // The publication date is on the mantel. dcterms:modified in
                // the kern is when the record was last touched, often years
                // later; sorting a corpus by it silently reorders history.
                published_on: field(mantel, NS_DCTERMS, "date")
                    .or_else(|| xml::text_of(kern, NS_DCTERMS, "modified"))
                    .as_deref()
                    .and_then(iso_date),
            },
            url: xml::text_of(record, NS_GZD, "url"),
        });
    }

    Ok(records)
}

fn field(node: Option<Elem>, namespace: &'static str, name: &'static str) -> Option<String> {
    node.and_then(|n| xml::text_of(n, namespace, name))
}

fn iso_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &str = r#"
      <sru:searchRetrieveResponse
          xmlns:sru="http://docs.oasis-open.org/ns/search-ws/sruResponse"
          xmlns:gzd="http://standaarden.overheid.nl/sru"
          xmlns:ow="http://standaarden.overheid.nl/wetgeving/"
          xmlns:dcterms="http://purl.org/dc/terms/">
        <sru:records><sru:record><sru:recordData><gzd:gzd>
          <gzd:originalData><ow:meta>
            <ow:owmskern>
              <dcterms:identifier>kst-33669-6</dcterms:identifier>
              <dcterms:title>Programmatische aanpak stikstof</dcterms:title>
              <dcterms:type>Kamerstuk</dcterms:type>
              <dcterms:modified>2014-06-24</dcterms:modified>
            </ow:owmskern>
            <ow:owmsmantel><dcterms:date>2013-10-01</dcterms:date></ow:owmsmantel>
            <ow:tpmeta>
              <ow:publicatienaam>Kamerstuk</ow:publicatienaam>
              <ow:vergaderjaar>2013-2014</ow:vergaderjaar>
              <ow:dossiernummer>33669</ow:dossiernummer>
            </ow:tpmeta>
          </ow:meta></gzd:originalData>
          <gzd:enrichedData><gzd:url>https://example.invalid/kst-33669-6.xml</gzd:url></gzd:enrichedData>
        </gzd:gzd></sru:recordData></sru:record></sru:records>
      </sru:searchRetrieveResponse>"#;

    #[test]
    fn reads_the_record_metadata() {
        let records = parse_records(RESPONSE).unwrap();
        assert_eq!(records.len(), 1);

        let record = &records[0];
        assert_eq!(record.hit.id.as_str(), "kst-33669-6");
        assert_eq!(record.hit.id.collection(), "kst");
        assert_eq!(record.hit.dossier.as_deref(), Some("33669"));
        assert_eq!(
            record.url.as_deref(),
            Some("https://example.invalid/kst-33669-6.xml")
        );
    }

    #[test]
    fn prefers_the_publication_date_over_the_record_mtime() {
        let records = parse_records(RESPONSE).unwrap();
        assert_eq!(
            records[0].hit.published_on,
            Some("2013-10-01".parse().unwrap())
        );
    }

    #[test]
    fn every_collection_maps_to_its_upstream_name() {
        assert_eq!(Collection::Staatsblad.as_str(), "Staatsblad");
        assert_eq!(Collection::Staatscourant.as_str(), "Staatscourant");
        assert_eq!(Collection::Kamerstuk.as_str(), "Kamerstuk");
    }
}
