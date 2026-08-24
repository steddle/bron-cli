//! Wetgeving, from the Basis Wetten Bestand.
//!
//! Two documents are involved. The per-law manifest lists every published
//! version with the dates it was in force and the digest of each. The versioned
//! XML behind each entry is the text itself, and is immutable.
//!
//! What is read here is what `update` needs to hold a law's current text and
//! hold the service's copy of it to the publisher's own digest. Answering a
//! question about a law goes through [`crate::service`], which resolves a date
//! against every manifest the register publishes rather than against the
//! handful this machine has fetched.

use anyhow::{Context, Result};
use chrono::NaiveDate;
use data_encoding::HEXLOWER;
use roxmltree::Document;
use sha2::{Digest, Sha512};

use super::xml::{self, Elem};
use super::{Client, FetchedArticle, FetchedLaw, FetchedLid, Source, VersionMeta};
use crate::ids::{BwbId, PublicationId};
use crate::model::{OPEN_ENDED, Span, Validity};

const REPOSITORY: &str = "https://repository.officiele-overheidspublicaties.nl/bwb";

/// Every published version of a law, oldest first.
///
/// One request, no text downloaded. This is what both `--as-of` and update
/// checking resolve against.
pub fn versions(client: &Client, law: &BwbId) -> Result<Vec<VersionMeta>> {
    let url = format!("{REPOSITORY}/{law}/manifest.xml");
    // Read and let go: a manifest is parsed into windows and digests, and no
    // ledger row or offset ever names the file it came out of. Only the version
    // XML below is kept, because `update` records it and stores spans into it.
    let fetched = client
        .read_url(&url)
        .with_context(|| format!("cannot read the manifest for {law}"))?;

    parse_manifest(law, &fetched.text)
}

pub fn parse_manifest(law: &BwbId, body: &str) -> Result<Vec<VersionMeta>> {
    let doc = Document::parse(body).with_context(|| format!("{law} has a malformed manifest"))?;

    let mut versions: Vec<VersionMeta> = xml::find_all_local(doc.root_element(), "expression")
        .filter_map(|expression| {
            let version = expression.attribute("label")?.to_string();

            let in_force = window(
                xml::find_local(expression, "datum_inwerkingtreding"),
                xml::find_local(expression, "einddatum"),
            )?;
            // Left absent when the manifest omits it. Falling back to in_force
            // would assert that the register showed this text exactly while it
            // applied, which is the claim the two windows exist to separate.
            let visible = window(
                xml::find_local(expression, "zichtdatum_start"),
                xml::find_local(expression, "zichtdatum_eind"),
            );

            let manifestation = xml::find_all_local(expression, "manifestation")
                .find(|m| m.attribute("label") == Some("xml"));

            Some(VersionMeta {
                version,
                in_force,
                visible,
                upstream_hash: manifestation
                    .and_then(|m| xml::find_local(m, "hashcode"))
                    .and_then(xml::inner_text),
                byte_len: manifestation
                    .and_then(|m| xml::find_local(m, "size"))
                    .and_then(xml::inner_text)
                    .and_then(|s| s.parse().ok()),
            })
        })
        .collect();

    anyhow::ensure!(!versions.is_empty(), "{law} has no published versions");

    versions.sort_by_key(|v| v.in_force.from);
    Ok(versions)
}

/// Reads a validity window, mapping the open-ended sentinel onto `None`.
fn window(from: Option<Elem>, until: Option<Elem>) -> Option<Validity> {
    let from = iso_date(&from.and_then(xml::inner_text)?)?;
    let until = match until.and_then(xml::inner_text) {
        Some(text) if text.trim() == OPEN_ENDED => None,
        Some(text) => iso_date(&text),
        None => None,
    };
    Some(Validity { from, until })
}

/// The version in force on a date, if one was.
pub fn version_on(versions: &[VersionMeta], date: NaiveDate) -> Option<&VersionMeta> {
    versions.iter().rev().find(|v| v.in_force.contains(date))
}

/// The version in force now, which is the one with no end date.
pub fn current_version(versions: &[VersionMeta]) -> Option<&VersionMeta> {
    versions
        .iter()
        .rev()
        .find(|v| v.in_force.is_open_ended())
        .or_else(|| versions.last())
}

/// Fetches one version of a law. The body is immutable once published.
pub fn fetch(client: &Client, law: &BwbId, meta: &VersionMeta) -> Result<FetchedLaw> {
    let version = &meta.version;
    let url = format!("{REPOSITORY}/{law}/{version}/xml/{law}_{version}.xml");
    let fetched = client
        .fetch_url(&url)
        .with_context(|| format!("cannot fetch {law} version {version}"))?;

    parse(Source::from(&fetched), law, meta)
}

/// Whether these bytes are the ones the publisher vouched for.
///
/// `None` when the manifest states no digest for this version: older entries
/// carry none, and there is then nothing to check against.
///
/// Deliberately not an error. The publisher's own manifest and the file it
/// serves do disagree in the wild: `BWBR0002638` version `2002-07-01_0` is
/// listed at 16.089 bytes and served at 125.937. Refusing that outright would
/// make bron unable to read a law the publisher is perfectly willing to hand
/// over, so the caller decides. On the publisher's own bytes a mismatch is a
/// caveat, because there is nowhere better to get them. On mirrored bytes it is
/// fatal, because the digest is the only thing making a mirror trustworthy.
pub fn digest_matches(bytes: &[u8], meta: &VersionMeta) -> Option<bool> {
    let expected = meta.upstream_hash.as_deref()?;
    let actual = HEXLOWER.encode(&Sha512::digest(bytes));

    Some(actual == expected.trim().to_ascii_lowercase())
}

pub fn parse(source: Source<'_>, law: &BwbId, meta: &VersionMeta) -> Result<FetchedLaw> {
    let doc = Document::parse(source.text).with_context(|| format!("{law} has malformed XML"))?;
    let root = doc.root_element();
    let wetgeving = xml::find_local(root, "wetgeving").unwrap_or(root);

    // citeertitel holds a nested <meta-data> block beside its text, so only the
    // element's own text nodes are the title.
    let title = xml::find_local(wetgeving, "citeertitel")
        .and_then(xml::own_text)
        .or_else(|| xml::find_local(wetgeving, "intitule").and_then(xml::own_text))
        .unwrap_or_else(|| law.to_string());

    let mut articles = Vec::new();
    let mut skipped_articles = 0;
    for artikel in xml::find_all_content(wetgeving, "artikel") {
        match parse_article(artikel) {
            Some(article) => articles.push(article),
            // An article whose body sits outside <al> would otherwise leave a
            // legal corpus with nothing recording that it did.
            None => skipped_articles += 1,
        }
    }

    Ok(FetchedLaw {
        id: law.clone(),
        title,
        law_type: wetgeving.attribute("soort").map(str::to_string),
        version: meta.version.clone(),
        in_force: meta.in_force,
        visible: meta.visible,
        upstream_hash: meta.upstream_hash.clone(),
        provenance: source.into(),
        articles,
        skipped_articles,
    })
}

fn parse_article(artikel: Elem) -> Option<FetchedArticle> {
    let kop = xml::children_local(artikel, "kop").next();
    let number = kop
        .and_then(|kop| xml::children_local(kop, "nr").next())
        .and_then(xml::inner_text)?;

    let effect = artikel.attribute("effect");
    let leden = parse_leden(artikel);

    // A repealed article carries a single empty <al> and no text. Dropping it
    // would lose the most interesting thing a diff can report, so it is kept
    // with empty content and its effect recorded.
    let repealed = effect == Some("vervallen");
    if leden.is_empty() && !repealed {
        return None;
    }

    let content = leden
        .iter()
        .map(|lid| lid.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    Some(FetchedArticle {
        stam_id: artikel.attribute("stam-id").map(str::to_string),
        number,
        label: artikel.attribute("label").map(str::to_string),
        title: kop
            .and_then(|kop| xml::children_local(kop, "titel").next())
            .and_then(xml::inner_text),
        path: artikel
            .attribute("bwb-ng-variabel-deel")
            .map(str::to_string),
        content,
        span: Span::new(artikel.range())?,
        leden,
        // `bron="Stb.2023-41"` names the act that produced this text. Keeping
        // it is what lets a caller reach the transitional provisions.
        amended_by: artikel
            .attribute("bron")
            .and_then(PublicationId::from_bwb_reference),
        effect: effect.map(str::to_string),
        // The day this article took the form it has here. wetten.nl renders it
        // beside a repealed provision, `[Vervallen per 01-01-2002]`, which is
        // the difference between knowing a provision is gone and knowing when
        // it went.
        in_force_from: artikel.attribute("inwerking").and_then(iso_date),
        tables: tables(artikel),
        illustrations: xml::find_all_content(artikel, "illustratie").count() as u32,
        versie_id: artikel.attribute("versie-id").map(str::to_string),
        status: artikel.attribute("status").map(str::to_string),
        jci: juriconnect(artikel),
    })
}

/// Splits an article into its citable units.
///
/// Leden are read as structural children, not descendants: a `<lid>` reached by
/// descending could sit inside another one, and its text would then be counted
/// twice. Articles with no lid structure yield a single unnumbered unit.
fn parse_leden(artikel: Elem) -> Vec<FetchedLid> {
    // Structural children first. Every content lid in BW6 sits directly under
    // its artikel, but other laws are not required to, so a deeper search backs
    // this up rather than letting the article collapse into one unnumbered
    // block and lose the unit practice actually cites.
    let mut leden: Vec<FetchedLid> = lid_elements(artikel)
        .into_iter()
        .filter_map(|lid| {
            let text = paragraphs(lid);
            if text.is_empty() {
                return None;
            }
            let number = xml::children_local(lid, "lidnr")
                .next()
                .and_then(xml::inner_text);
            // Joined by line, not by space, so a table row keeps its own line
            // and a reader can tell one row from the next. The service joins
            // the same units the same way; a lid whose text differs between the
            // two halves is the drift this project keeps having to close.
            //
            // Each line was tidied as it was collected. Tidying the join would
            // collapse the two spaces an empty table cell leaves, and a consumer
            // splitting on the separator would then read one cell where the
            // register printed two.
            let body = text.join("\n");
            Some(FetchedLid {
                content: match &number {
                    Some(n) => format!("{n}. {body}"),
                    None => body,
                },
                number,
                span: Span::new(lid.range())?,
                jci: juriconnect(lid),
            })
        })
        .collect();

    if leden.is_empty() {
        let text = paragraphs(artikel);
        if let Some(span) = Span::new(artikel.range())
            && !text.is_empty()
        {
            leden.push(FetchedLid {
                number: None,
                content: text.join("\n"),
                span,
                // An article printed without leden is one unit spanning its
                // whole text, and the article's own reference is the only one
                // the register states for it.
                jci: juriconnect(artikel),
            });
        }
    }

    leden
}

/// The lid elements of an article, preferring structural children.
///
/// Descending is only a fallback because a `<lid>` reached that way could sit
/// inside another one and have its text counted twice; when no lid is a direct
/// child, there is nothing to double count.
fn lid_elements<'a>(artikel: Elem<'a>) -> Vec<Elem<'a>> {
    let direct: Vec<Elem<'a>> = xml::children_local(artikel, "lid").collect();
    if direct.is_empty() {
        xml::find_all_content(artikel, "lid").collect()
    } else {
        direct
    }
}

/// What separates two cells of one table row inside stored text.
///
/// A pipe rather than a tab or a comma: statutory text is full of commas, and a
/// tab does not survive being read out of a terminal or pasted into a document.
/// Nothing else in the corpus emits this sequence, so a consumer can split on it
/// without a flag telling it when to.
///
/// The service writes the same separator from the same reasoning. It is a
/// contract between the two halves, not a rendering preference.
pub const CELL: &str = " | ";

/// The text under a node, as the paragraphs and table rows it is printed in.
///
/// Walked in document order rather than by collecting `<al>` alone, because a
/// table sits between prose lines and both have to keep their places. A row is
/// emitted whole, so its cells stay on one line; an `<al>` inside a cell is
/// skipped, because the row already carried it.
fn paragraphs(node: Elem) -> Vec<String> {
    let mut lines = Vec::new();

    for element in node.descendants() {
        if xml::in_metadata(element) {
            continue;
        }

        match element.tag_name().name() {
            "row" => lines.push(row(element)),
            "al" if in_row(element) => {}
            "al" => {
                if let Some(text) = xml::inner_text(element) {
                    let text = xml::tidy(&text);
                    lines.push(match item_letter(element) {
                        Some(letter) => format!("{letter} {text}"),
                        None => text,
                    });
                }
            }
            _ => {}
        }
    }

    lines
}

/// One table row as one line, its cells separated.
///
/// Tables are CALS, so a rate table reaches here as `table > tgroup > tbody >
/// row > entry > al` and every cell is an ordinary paragraph. Collected as
/// paragraphs they arrive one per line with nothing marking where a row ended,
/// and Wet IB 2001 artikel 2.10 then reads as a column of numbers in which no
/// rate can be tied to the band it applies to. Every figure present and the
/// answer still wrong, which is the class of error this corpus exists to refuse.
///
/// An empty cell is kept as an empty string. Dropping it would shift every later
/// cell one column left, and a rate under the wrong heading is worse than a gap.
///
/// This is a linearisation and not a grid. [`Caveat::TableFlattened`] says so on
/// any answer carrying one.
fn row(row: Elem) -> String {
    xml::children_local(row, "entry")
        .map(|entry| {
            xml::find_all_content(entry, "al")
                .filter_map(xml::inner_text)
                .map(|text| xml::tidy(&text))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(CELL)
}

fn in_row(node: Elem) -> bool {
    node.ancestors().any(|a| a.tag_name().name() == "row")
}

/// The Juriconnect reference the register states for this element.
///
/// Juriconnect is the citation scheme Dutch practice already uses, and the
/// register hands one out per article and per lid inside the `<meta-data>` block
/// everything else here refuses to read. Constructing our own from the
/// structural path would be a second implementation of a string the source
/// already publishes, and ours would be the one that drifts.
///
/// Version 1.3 is taken and 1.0 ignored. Both occur, 1.3 on every element and
/// 1.0 on some as well, and they are the same reference in two grammars:
/// `jci1.3:c:BWBR…` against `1.0:c:BWBR…`, the older without the prefix. Taking
/// whichever came first would make the stored format depend on the element.
///
/// Read from the element's own `<meta-data>`, never a descendant's, or an
/// article takes the reference of its first lid: a string that still parses and
/// still resolves, at the wrong place.
///
/// The reference carries `z=` and `g=`, this rendition's own zicht and
/// geldigheid dates, so it describes a placement rather than a text. Here that
/// costs nothing because a row is already per version; on the service, where one
/// text row is shared between renditions, it is why the reference lives beside
/// the byte offsets.
fn juriconnect(element: Elem) -> Option<String> {
    xml::children_local(element, "meta-data")
        .flat_map(|meta| xml::find_all_local(meta, "jci"))
        .find(|jci| jci.attribute("versie") == Some("1.3"))
        .and_then(|jci| jci.attribute("verwijzing"))
        .map(str::to_string)
}

/// How many tables this element prints.
///
/// Counted so an answer can say its text is a linearisation, rather than leaving
/// a reader to infer it from a pipe. Metadata blocks are skipped like everywhere
/// else here; measured over BW6, the Awb and Wet IB 2001, no `<table>` sits
/// inside one, so this only ever counts printed tables.
fn tables(element: Elem) -> u32 {
    xml::find_all_content(element, "table").count() as u32
}

/// The `a.` of an enumerated item, for that item's opening paragraph only.
///
/// The letter sits in a `<li.nr>` beside the `<al>` rather than inside it, so
/// collecting paragraphs alone silently drops it and runs the items together
/// as one sentence.
///
/// It is part of the address, not decoration: `artikel 6:3 lid 2 sub b` is an
/// ordinary citation. Losing it also made this half render a provision
/// differently from the service, which reads the same `<li.nr>`. Two texts for
/// one identifier is worse than either text on its own when the whole claim is
/// that the same question twice gives the same answer.
///
/// Only the first paragraph of an item is prefixed. An item running to several
/// paragraphs is lettered once, the way it is printed.
fn item_letter(al: Elem) -> Option<String> {
    let item = al.ancestors().find(|a| a.tag_name().name() == "li")?;

    if xml::find_all_content(item, "al").next()? != al {
        return None;
    }

    xml::children_local(item, "li.nr")
        .next()
        .and_then(xml::inner_text)
}

/// Removes CQL's unescapable quote without joining the words around it.
pub fn strip_quotes(text: &str) -> String {
    text.replace('"', " ")
}

/// Upstream writes plain ISO dates here; anything else is not a date.
fn iso_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::BlobId;

    fn law_id() -> BwbId {
        "BWBR0005289".parse().unwrap()
    }

    // -- tables ------------------------------------------------------------

    /// Wet IB 2001 artikel 2.10 in miniature: a rate table between two prose
    /// lines, which is where the register actually puts them.
    fn rate_table() -> String {
        article_content(
            r#"<artikel label="Artikel 2.10">
                 <kop><nr>2.10</nr></kop>
                 <lid><lidnr>1</lidnr>
                   <al>De belasting wordt bepaald aan de hand van de volgende tabel.</al>
                   <table><tgroup cols="4">
                     <thead><row>
                       <entry><al>I</al></entry><entry><al>II</al></entry>
                       <entry><al>III</al></entry><entry><al>IV</al></entry>
                     </row></thead>
                     <tbody>
                       <row>
                         <entry><al>-</al></entry><entry><al>38.883</al></entry>
                         <entry/><entry><al>8,10%</al></entry>
                       </row>
                       <row>
                         <entry><al>38.883</al></entry><entry><al>78.426</al></entry>
                         <entry><al>3.149</al></entry><entry><al>37,56%</al></entry>
                       </row>
                     </tbody>
                   </tgroup></table>
                 </lid>
               </artikel>"#,
        )
    }

    #[test]
    fn a_table_row_stays_on_one_line_with_its_cells_separated() {
        // Collected as paragraphs the cells arrive one per line with nothing
        // marking where a row ended, and no rate can then be tied to the band
        // it applies to. Every figure present and the answer still wrong.
        let content = rate_table();
        let lines: Vec<&str> = content.lines().collect();

        assert_eq!(
            lines,
            [
                "1. De belasting wordt bepaald aan de hand van de volgende tabel.",
                "I | II | III | IV",
                "- | 38.883 |  | 8,10%",
                "38.883 | 78.426 | 3.149 | 37,56%",
            ]
        );
    }

    #[test]
    fn an_empty_cell_is_kept_so_the_columns_stay_aligned() {
        // Dropping it shifts every later cell one column left, and a rate under
        // the wrong heading is worse than a gap.
        assert!(rate_table().contains("- | 38.883 |  | 8,10%"));
    }

    #[test]
    fn the_cell_separator_is_the_one_the_service_writes() {
        assert_eq!(CELL, " | ");
        assert!(rate_table().contains(&format!("I{CELL}II")));
    }

    #[test]
    fn an_article_counts_the_tables_it_prints() {
        let doc = Document::parse(
            r#"<artikel label="Artikel 1"><kop><nr>1</nr></kop>
                 <lid><lidnr>1</lidnr><al>Gewone tekst.</al></lid>
               </artikel>"#,
        )
        .unwrap();

        assert_eq!(parse_article(doc.root_element()).unwrap().tables, 0);
    }

    // -- juriconnect ---------------------------------------------------------

    fn with_references(article_jci: &str, lid_jci: &str) -> Option<FetchedArticle> {
        let xml = format!(
            r#"<artikel label="Artikel 162" stam-id="1" versie-id="1" inwerking="1992-01-01">
                 <kop><nr>162</nr></kop>
                 <lid><lidnr>1</lidnr><al>Eerste lid.</al>{lid_jci}</lid>
                 {article_jci}
               </artikel>"#
        );
        let doc = Document::parse(&xml).unwrap();
        parse_article(doc.root_element())
    }

    fn jcis(refs: &[(&str, &str)]) -> String {
        let inner: String = refs
            .iter()
            .map(|(versie, verwijzing)| {
                let escaped = verwijzing.replace('&', "&amp;");
                format!(r#"<jci versie="{versie}" verwijzing="{escaped}"/>"#)
            })
            .collect();
        format!("<meta-data><jcis>{inner}</jcis></meta-data>")
    }

    #[test]
    fn the_register_states_the_reference_and_bron_takes_it() {
        // Constructing our own from the structural path would be a second
        // implementation of a string the source already publishes, and ours
        // would be the one that drifts.
        let article = with_references(
            &jcis(&[(
                "1.3",
                "jci1.3:c:BWBR0005289&artikel=162&z=2024-01-01&g=2024-01-01",
            )]),
            &jcis(&[(
                "1.3",
                "jci1.3:c:BWBR0005289&artikel=162&lid=1&z=2024-01-01&g=2024-01-01",
            )]),
        )
        .expect("an article");

        assert_eq!(
            article.jci.as_deref(),
            Some("jci1.3:c:BWBR0005289&artikel=162&z=2024-01-01&g=2024-01-01")
        );
        assert_eq!(
            article.leden[0].jci.as_deref(),
            Some("jci1.3:c:BWBR0005289&artikel=162&lid=1&z=2024-01-01&g=2024-01-01")
        );
    }

    #[test]
    fn version_1_3_wins_over_the_legacy_form_whatever_their_order() {
        // Both grammars occur in one document and say the same thing. Taking
        // whichever came first would make the stored format depend on the
        // element rather than on the corpus.
        let canonical = "jci1.3:c:BWBR0005289&artikel=162&z=2024-01-01&g=2024-01-01";
        let legacy = "1.0:c:BWBR0005289&artikel=162&g=2024-01-01";

        for order in [
            [("1.0", legacy), ("1.3", canonical)],
            [("1.3", canonical), ("1.0", legacy)],
        ] {
            let article = with_references(&jcis(&order), "").expect("an article");
            assert_eq!(article.jci.as_deref(), Some(canonical));
        }
    }

    #[test]
    fn a_legacy_only_reference_is_not_stored_in_the_other_grammar() {
        let article =
            with_references(&jcis(&[("1.0", "1.0:c:BWBR0005289&artikel=162")]), "").unwrap();

        assert_eq!(article.jci, None);
    }

    #[test]
    fn an_article_does_not_inherit_its_first_lids_reference() {
        // Reading a descendant's would give the article a string that still
        // parses and still resolves, at the wrong place.
        let article = with_references(
            "",
            &jcis(&[(
                "1.3",
                "jci1.3:c:BWBR0005289&artikel=162&lid=1&z=2024-01-01&g=2024-01-01",
            )]),
        )
        .expect("an article");

        assert_eq!(article.jci, None);
        assert!(article.leden[0].jci.is_some());
    }

    // -- the day a provision took this form ----------------------------------

    #[test]
    fn a_repealed_article_keeps_the_day_it_went() {
        // 'Vervallen' alone says a provision is gone. The date says when, which
        // is the part that decides whether it applied to the conduct in hand.
        let doc = Document::parse(
            r#"<artikel label="Artikel 113" stam-id="1" versie-id="1"
                        effect="vervallen" inwerking="2002-01-01">
                 <kop><nr>113</nr></kop><al></al>
               </artikel>"#,
        )
        .unwrap();

        let article = parse_article(doc.root_element()).expect("a repealed article");

        assert_eq!(article.effect.as_deref(), Some("vervallen"));
        assert_eq!(article.in_force_from, "2002-01-01".parse().ok());
    }

    // -- enumerated items -------------------------------------------------

    fn article_content(xml: &str) -> String {
        let doc = Document::parse(xml).unwrap();
        parse_article(doc.root_element())
            .expect("an article")
            .content
    }

    #[test]
    fn an_enumerated_item_keeps_the_letter_it_is_cited_by() {
        // 'artikel 6:3 lid 2 sub b' is an ordinary citation, and the letter
        // lives in <li.nr> beside the <al> rather than inside it. Without this
        // the items run together as one sentence, and this half renders the
        // provision differently from the service, which reads the same field.
        let content = article_content(
            r#"<artikel label="Artikel 3">
                 <kop><nr>3</nr></kop>
                 <lid><lidnr>2</lidnr>
                   <al>Een natuurlijke verbintenis bestaat:</al>
                   <lijst>
                     <li><li.nr>a.</li.nr><al>wanneer de wet het bepaalt;</al>
                       <meta-data><al>noise</al></meta-data></li>
                     <li><li.nr>b.</li.nr><al>wanneer een morele plicht bestaat.</al></li>
                   </lijst>
                 </lid>
               </artikel>"#,
        );

        // One line per printed unit, which is how the service stores the same
        // article. A lid whose text differs between the two halves is the drift
        // this project keeps having to close.
        assert_eq!(
            content,
            "2. Een natuurlijke verbintenis bestaat:\n\
             a. wanneer de wet het bepaalt;\n\
             b. wanneer een morele plicht bestaat."
        );
    }

    #[test]
    fn an_item_running_to_several_paragraphs_is_lettered_once() {
        let content = article_content(
            r#"<artikel label="Artikel 1">
                 <kop><nr>1</nr></kop>
                 <lid><lidnr>1</lidnr><lijst>
                   <li><li.nr>a.</li.nr><al>eerste alinea;</al><al>tweede alinea.</al></li>
                 </lijst></lid>
               </artikel>"#,
        );

        assert_eq!(content, "1. a. eerste alinea;\ntweede alinea.");
    }

    #[test]
    fn a_paragraph_outside_a_list_is_left_alone() {
        let content = article_content(
            r#"<artikel label="Artikel 2">
                 <kop><nr>2</nr></kop>
                 <lid><lidnr>1</lidnr><al>Gewone tekst.</al></lid>
               </artikel>"#,
        );

        assert_eq!(content, "1. Gewone tekst.");
    }

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn meta() -> VersionMeta {
        VersionMeta {
            version: "2026-07-16_0".into(),
            in_force: Validity {
                from: date("1992-01-01"),
                until: None,
            },
            visible: Some(Validity {
                from: date("1992-01-01"),
                until: None,
            }),
            upstream_hash: None,
            byte_len: None,
        }
    }

    // -- the publisher's own digest ---------------------------------------

    fn meta_hashed(hash: &str) -> VersionMeta {
        VersionMeta {
            upstream_hash: Some(hash.to_string()),
            ..meta()
        }
    }

    /// SHA-512 of `toestand`, the whole body of the fixture below.
    const TOESTAND_SHA512: &str = "9ab905fa7a78e3ce321d181e88f85139cfa7384c3b973c560c179d992fc9559d\
                                   513353206c8a92704ef5946c718c1ccde9ca8b5dccf12e1bfd3b72670a34b072";

    #[test]
    fn bytes_matching_the_published_digest_are_recognised() {
        // Not a digest bron computed and then checked against itself: this is
        // the value the manifest states, and matching it is what says the
        // document is the one the publisher published.
        assert_eq!(
            digest_matches(b"toestand", &meta_hashed(TOESTAND_SHA512)),
            Some(true)
        );
    }

    #[test]
    fn the_digest_is_read_case_insensitively() {
        let shouting = TOESTAND_SHA512.to_ascii_uppercase();
        assert_eq!(
            digest_matches(b"toestand", &meta_hashed(&shouting)),
            Some(true)
        );
    }

    #[test]
    fn bytes_the_publisher_did_not_vouch_for_are_reported() {
        assert_eq!(
            digest_matches(b"substituted", &meta_hashed(TOESTAND_SHA512)),
            Some(false)
        );
    }

    #[test]
    fn a_version_the_manifest_states_no_digest_for_cannot_be_checked() {
        // Distinct from a mismatch: nothing was stated, so nothing failed, and
        // a mirror is refused for exactly this case.
        assert_eq!(digest_matches(b"anything at all", &meta()), None);
    }

    const LAW: &str = r#"<toestand>
        <wetgeving soort="wet" inwerkingtredingsdatum="1992-01-01">
          <citeertitel>Burgerlijk Wetboek Boek 6<meta-data><al>junk</al></meta-data></citeertitel>
          <artikel label="Artikel 1" bwb-ng-variabel-deel="/Boek6/Artikel1"
                   bron="Stb.1991-600" effect="nieuwe-regeling">
            <kop><label>Artikel</label><nr>1</nr></kop>
            <al>Verbintenissen kunnen slechts ontstaan.</al>
            <meta-data><al>provenance noise</al></meta-data>
          </artikel>
          <artikel label="Artikel 2" bwb-ng-variabel-deel="/Boek6/Artikel2">
            <kop><label>Artikel</label><nr>2</nr><titel>Redelijkheid</titel></kop>
            <lid><lidnr>1</lidnr><al>Eerste lid.</al><meta-data><al>noise</al></meta-data></lid>
            <lid><lidnr>2</lidnr><al>Tweede lid.</al></lid>
          </artikel>
          <artikel label="Artikel 3"><kop><nr>3</nr></kop></artikel>
          <artikel label="Artikel 4" effect="vervallen" stam-id="99"><kop><nr>4</nr></kop><al></al></artikel>
        </wetgeving>
      </toestand>"#;

    fn parsed() -> FetchedLaw {
        parse(
            Source::new(
                BlobId::of(LAW.as_bytes()),
                LAW,
                "https://test.invalid/law.xml",
            ),
            &law_id(),
            &meta(),
        )
        .unwrap()
    }

    #[test]
    fn reads_the_title_without_the_nested_metadata() {
        assert_eq!(parsed().title, "Burgerlijk Wetboek Boek 6");
    }

    #[test]
    fn excludes_metadata_paragraphs_from_article_text() {
        let law = parsed();
        assert_eq!(
            law.articles[0].content,
            "Verbintenissen kunnen slechts ontstaan."
        );
        assert!(!law.articles[0].content.contains("provenance"));
        assert!(!law.articles[1].content.contains("noise"));
    }

    #[test]
    fn splits_articles_into_citable_leden() {
        let law = parsed();
        let leden = &law.articles[1].leden;
        assert_eq!(leden.len(), 2);
        assert_eq!(leden[0].number.as_deref(), Some("1"));
        assert_eq!(leden[0].content, "1. Eerste lid.");
        assert_eq!(leden[1].content, "2. Tweede lid.");
    }

    #[test]
    fn spans_point_at_the_source_text_they_came_from() {
        let law = parsed();

        let article = &law.articles[0];
        let quoted = article.span.slice(LAW).unwrap();
        assert!(quoted.starts_with("<artikel label=\"Artikel 1\""));
        assert!(quoted.ends_with("</artikel>"));

        let lid = &law.articles[1].leden[1];
        assert_eq!(
            lid.span.slice(LAW),
            Some("<lid><lidnr>2</lidnr><al>Tweede lid.</al></lid>")
        );
    }

    #[test]
    fn keeps_the_act_that_produced_each_article() {
        let law = parsed();
        let article = &law.articles[0];
        assert_eq!(
            article.amended_by.as_ref().map(|p| p.as_str()),
            Some("stb-1991-600")
        );
        assert_eq!(article.effect.as_deref(), Some("nieuwe-regeling"));
    }

    #[test]
    fn skips_articles_that_carry_no_text_but_keeps_repealed_ones() {
        let law = parsed();
        // Artikel 3 has no text at all and is dropped; artikel 4 is repealed,
        // which is itself the fact worth keeping.
        assert_eq!(law.articles.len(), 3);
        assert_eq!(law.skipped_articles, 1);

        let repealed = law.articles.iter().find(|a| a.number == "4").unwrap();
        assert_eq!(repealed.effect.as_deref(), Some("vervallen"));
        assert!(repealed.content.is_empty());
        assert_eq!(repealed.stam_id.as_deref(), Some("99"));
    }

    const MANIFEST: &str = r#"<work label="BWBR0005289" _latestItem="2026-07-16_0/xml/x.xml">
        <expression label="2002-04-12_0"><metadata>
          <datum_inwerkingtreding>2002-04-12</datum_inwerkingtreding>
          <einddatum>2002-11-30</einddatum>
          <zichtdatum_start>2002-04-12</zichtdatum_start>
          <zichtdatum_eind>9999-12-31</zichtdatum_eind>
        </metadata><manifestation label="xml"><metadata>
          <hashcode>abc123</hashcode><size>913394</size>
        </metadata></manifestation></expression>
        <expression label="2026-07-16_0"><metadata>
          <datum_inwerkingtreding>2026-07-16</datum_inwerkingtreding>
          <einddatum>9999-12-31</einddatum>
          <zichtdatum_start>2026-07-16</zichtdatum_start>
          <zichtdatum_eind>9999-12-31</zichtdatum_eind>
        </metadata><manifestation label="xml"><metadata>
          <size>1425926</size><hashcode>def456</hashcode>
        </metadata></manifestation></expression>
      </work>"#;

    #[test]
    fn reads_every_version_from_the_manifest() {
        let versions = parse_manifest(&law_id(), MANIFEST).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, "2002-04-12_0");
        assert_eq!(versions[0].upstream_hash.as_deref(), Some("abc123"));
        assert_eq!(versions[0].byte_len, Some(913_394));
    }

    #[test]
    fn maps_the_open_ended_sentinel_onto_no_upper_bound() {
        let versions = parse_manifest(&law_id(), MANIFEST).unwrap();
        assert_eq!(versions[0].in_force.until, Some(date("2002-11-30")));
        assert_eq!(versions[1].in_force.until, None);
        assert!(versions[1].in_force.is_open_ended());
    }

    #[test]
    fn resolves_the_version_in_force_on_a_date() {
        let versions = parse_manifest(&law_id(), MANIFEST).unwrap();

        let during = version_on(&versions, date("2002-06-01")).unwrap();
        assert_eq!(during.version, "2002-04-12_0");

        let after = version_on(&versions, date("2026-08-06")).unwrap();
        assert_eq!(after.version, "2026-07-16_0");

        // The gap between 2002-11-30 and 2026-07-16 is not covered by either.
        assert!(version_on(&versions, date("2010-01-01")).is_none());
        assert!(version_on(&versions, date("1990-01-01")).is_none());
    }

    #[test]
    fn current_version_is_the_open_ended_one() {
        let versions = parse_manifest(&law_id(), MANIFEST).unwrap();
        assert_eq!(current_version(&versions).unwrap().version, "2026-07-16_0");
    }
}
