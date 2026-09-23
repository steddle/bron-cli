//! What the service answers with.
//!
//! Received shapes and nothing else: no rendering, no policy, no decisions.
//! Only the fields bron reads are named, so a field the service adds later does
//! not break a build here. A named field is required exactly where
//! `public/openapi.json` requires it: the service leaves a null out, so a field
//! the document does not require is absent on some row of some answer.

use chrono::NaiveDate;
use serde::Deserialize;

use crate::ids::BwbId;

/// A validity window, either of the two.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Window {
    pub from: NaiveDate,
    /// Absent when open-ended.
    pub until: Option<NaiveDate>,
}

/// One regulation, as the index lists it.
#[derive(Debug, Deserialize)]
pub struct Regulation {
    pub id: String,
    pub title: Option<String>,
    #[serde(rename = "type")]
    pub regulation_type: Option<String>,
    /// How much of the regulation the corpus means to hold: `manifest`,
    /// `current` or `history`, read as a string so a state added later crosses.
    pub coverage: String,
    /// The body the register names, and where it files the regulation.
    pub authority: Option<String>,
    #[serde(default)]
    pub legal_areas: Vec<String>,
    #[serde(default)]
    pub government_domains: Vec<String>,
    /// The publications that created it, as `stb-2019-130`. Empty where the
    /// register names none, absent where its WTI has not been read.
    #[serde(default)]
    pub established_by: Vec<String>,
    /// How many renditions the register publishes, and how many the corpus
    /// holds the text of.
    pub expression_count: Option<u32>,
    pub held_count: Option<u32>,
}

/// One rendition, as the manifest describes it and the corpus holds it.
#[derive(Debug, Deserialize)]
pub struct Expression {
    /// Stated on every rendition, so a row lifted out of a corpus-wide answer
    /// keeps saying which regulation it belongs to.
    pub regulation_id: String,
    pub label: String,
    /// What the register calls the regulation, stated on a list of renditions.
    pub regulation_title: Option<String>,
    pub in_force: Window,
    /// Absent where the manifest carried no zicht dates, and never filled in
    /// from `in_force`.
    pub as_known: Option<Window>,
    pub upstream_hash: Option<String>,
    /// What the publisher's hash says about the held bytes: `matches`,
    /// `differs`, `rewritten`, `absent` or `unchecked`, read as a string so a
    /// state added later crosses.
    pub upstream_hash_check: String,
    /// The size the manifest states, which is not the length of the document
    /// the spans point into: for a large share of renditions the manifest
    /// describes a gzip of the manifestation rather than what its own URL
    /// serves, and BWBR0008691 2016-07-01_0 is one, 49.606 bytes stated
    /// against 547.732 held.
    pub size: Option<u64>,
    /// The publisher withdrew it, and no date resolves to it.
    pub withdrawn: bool,
    /// True where the service holds this rendition's text.
    pub held: bool,
    /// Why held bytes were not parsed into articles: `too_large`,
    /// `unparseable` or `paragraph_drift`, read as a string so a reason added
    /// later crosses. Absent where they were, and where nothing is held.
    #[serde(default)]
    pub unread_because: Option<String>,
    pub blob: Option<String>,
    /// The images the rendition prints beside its text.
    #[serde(default)]
    pub images: Vec<Image>,
}

/// One image a rendition prints, named as the manifest lists it.
#[derive(Debug, Deserialize)]
pub struct Image {
    pub name: String,
    pub blob: String,
}

/// One thing the register did, in the order the corpus saw it.
///
/// `seq` is the cursor. There is no time here and there is not meant to be: the
/// order says which came first, and a date would say when the service looked.
#[derive(Debug, Deserialize)]
pub struct RegisterEvent {
    pub seq: u64,
    pub event: String,
    pub regulation_id: String,
    /// Absent on an event about a regulation rather than one of its renditions.
    pub label: Option<String>,
    pub in_force: Option<Window>,
    pub as_known: Option<Window>,
    pub withdrawn: bool,
}

/// A page of the walk, with the cursor to come back to.
#[derive(Debug, Deserialize)]
pub struct Feed {
    /// This corpus's own name for itself. Keep it beside the cursor: one you do
    /// not recognise means the corpus was rebuilt and the number means something
    /// else now.
    pub origin: String,
    /// The highest cursor the corpus holds.
    pub head: u64,
    /// Where to resume. The last row served where the page was cut, and the head
    /// where it was not, so a walk never skips what a cut left behind.
    pub next: u64,
    pub events: Vec<RegisterEvent>,
}

/// One regulation whose rendition at the moment asked prints units it does
/// not yet apply.
#[derive(Debug, Deserialize)]
pub struct Upcoming {
    pub regulation_id: String,
    pub regulation_title: Option<String>,
    /// The rendition the moment resolved to, whose units these are.
    pub label: String,
    /// How many units of every kind the rendition prints as not yet in force.
    pub units: u32,
    /// How many of them state a day after the moment asked. The rest state
    /// none, or one on or before it.
    pub dated: u32,
    /// The earliest day to come a waiting unit states. Absent where none
    /// states one.
    pub first_day: Option<NaiveDate>,
    /// The publications that produced the waiting texts, as `stb-2025-385`.
    pub acts: Vec<String>,
    /// The besluiten the rendition names as commencing them, as
    /// `stb-2026-159`. Empty where it names none.
    pub decrees: Vec<String>,
}

/// Which moment to read the register's waiting units at, and how to narrow
/// the list.
#[derive(Debug)]
pub struct Waiting<'a> {
    pub as_of: NaiveDate,
    pub as_known_on: Option<NaiveDate>,
    /// Bounds on the days to come a waiting unit states. Where either is
    /// named, a regulation whose units state no day is not listed.
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    /// The register's own word for a kind of regulation: wet, AMvB, zbo.
    pub kind: Option<&'a str>,
    pub limit: u32,
    pub offset: u32,
}

/// What a rendition's articles resolved to.
#[derive(Debug, Deserialize)]
pub struct Articles {
    pub regulation_id: String,
    pub expression: Expression,
    pub articles: Vec<Article>,
}

/// What a rendition's units of text resolved to, of any kind and in document
/// order.
#[derive(Debug, Deserialize)]
pub struct Parts {
    pub regulation_id: String,
    pub expression: Expression,
    pub parts: Vec<Article>,
}

/// The texts the act that established a regulation produced elsewhere.
#[derive(Debug, Deserialize)]
pub struct Amendments {
    pub regulation_id: String,
    /// The publications the register names as having created the regulation.
    pub established_by: Vec<String>,
    /// One per unit of text the act produced, in whatever regulation.
    pub articles: Vec<Produced>,
}

/// One unit of text an act produced, where it stands now.
#[derive(Debug, Deserialize)]
pub struct Produced {
    pub regulation_id: String,
    pub kind: String,
    pub number: Option<String>,
    pub label: Option<String>,
    pub title: Option<String>,
    /// As the earliest rendition placing this text prints it.
    pub in_force_from: Option<NaiveDate>,
    pub effect: Option<String>,
    pub status: Option<String>,
}

/// What the register states relates a regulation, or one of its articles, to
/// another.
#[derive(Debug, Deserialize)]
pub struct Relations {
    pub regulation_id: String,
    pub relations: Vec<Relation>,
}

/// One relation the register states.
#[derive(Debug, Deserialize)]
pub struct Relation {
    /// The WTI's group name, e.g. `grondslag_voor` or `verwijzing_door`.
    pub kind: String,
    /// The register's label for the article it is stated for. Absent where it
    /// is stated for the regulation whole.
    pub article: Option<String>,
    pub related: Related,
    pub in_force: Option<Window>,
    pub as_known: Option<Window>,
}

/// The far end of a relation.
#[derive(Debug, Deserialize)]
pub struct Related {
    pub regulation_id: String,
    pub kind: Option<String>,
    pub label: Option<String>,
    pub title: Option<String>,
    pub jci: Option<String>,
}

/// The texts one unit has had, from held renditions only.
#[derive(Debug, Deserialize)]
pub struct History {
    pub regulation_id: String,
    /// The number asked for, as the register prints it.
    pub number: String,
    pub as_known_on: NaiveDate,
    /// One entry per unit the number names, which is more than one where the
    /// register prints it more than once.
    pub articles: Vec<Lineage>,
}

/// One unit the number names, and every text it has had.
#[derive(Debug, Deserialize)]
pub struct Lineage {
    pub kind: String,
    pub stam_id: String,
    pub number: Option<String>,
    pub label: Option<String>,
    pub path: Option<String>,
    pub entries: Vec<Had>,
}

/// One text the unit had, and the renditions that carry it.
#[derive(Debug, Deserialize)]
pub struct Had {
    /// `text` where the corpus holds it, `unknown` where the renditions are
    /// not held, `absent` where they are and do not carry it.
    pub state: String,
    pub in_force: Window,
    pub number: Option<String>,
    pub label: Option<String>,
    pub title: Option<String>,
    pub content_hash: Option<String>,
    pub amended_by: Option<String>,
    pub commenced_by: Option<String>,
    pub effect: Option<String>,
    pub status: Option<String>,
    /// The day this text applies from, which is not the window's first day
    /// where the register states it apart.
    pub text_in_force_from: Option<NaiveDate>,
    pub renditions: Carried,
}

/// Which renditions carry one text, and how many.
#[derive(Debug, Deserialize)]
pub struct Carried {
    pub first: String,
    pub last: String,
    pub count: u32,
}

/// A regulation compared between two days, unit by unit on the publisher's
/// own identity.
#[derive(Debug, Deserialize)]
pub struct Changes {
    pub regulation_id: String,
    pub from: Expression,
    pub to: Expression,
    /// Counted over every unit of both renditions, whatever `kind`, `article`
    /// and `all` narrowed the list to.
    pub summary: Summary,
    /// Every kind of unit, under the name the service gives the list.
    pub articles: Vec<Change>,
}

#[derive(Debug, Deserialize)]
pub struct Summary {
    pub unchanged: u32,
    pub amended: u32,
    pub added: u32,
    pub commenced: u32,
    pub repealed: u32,
    pub withdrawn: u32,
}

/// What happened to one unit, named as the side that still exists names it.
#[derive(Debug, Deserialize)]
pub struct Change {
    /// `unchanged`, `amended`, `added`, `commenced`, `repealed` or `withdrawn`,
    /// read as a string so one added later crosses.
    pub change: String,
    pub kind: String,
    pub number: Option<String>,
    pub label: Option<String>,
    pub path: Option<String>,
    pub from: Option<Article>,
    pub to: Option<Article>,
}

/// One unit of text, where it sits in the frozen bytes, and what stands there.
///
/// On `/articles` an article; on `/parts` and `/changes` a unit of any kind.
#[derive(Debug, Clone, Deserialize)]
pub struct Article {
    /// The element the text was read from: `artikel`, `bijlage`, `aanhef`.
    pub kind: String,
    /// Its place among the units of its kind in the rendition, from 1.
    pub position: Option<u32>,
    /// Absent where the register prints none, as on an aanhef and on some
    /// articles: BWBR0020368 on 2015-01-01 holds one.
    pub number: Option<String>,
    pub label: Option<String>,
    pub title: Option<String>,
    pub path: Option<String>,
    /// The publisher's stable identity for this article across renditions,
    /// which is what makes a comparison an identity match rather than a guess.
    pub stam_id: String,
    pub amended_by: Option<String>,
    pub effect: Option<String>,
    pub status: Option<String>,
    /// The language of the treaty text the unit stands in, and whether that
    /// text is `authentiek` or a `vertaling`. Both absent outside a treaty.
    pub lang: Option<String>,
    pub text_status: Option<String>,
    /// The day this article took the form it has here, as the register states
    /// it. A date and not a moment, and absent where the register named no day.
    pub in_force_from: Option<NaiveDate>,
    pub jci: Option<String>,
    pub span: Span,
    /// The service's reading of the whole unit, present only where
    /// `rendered=1` asked for it. See [`Article::text`].
    pub rendered: Option<String>,
    /// The publisher's own bytes at `span`, present only where `raw=1` asked
    /// for them. On the service's word: checking it means fetching the blob it
    /// was cut from, and not having to is why it is here.
    pub raw: Option<String>,
    /// The file each image the unit prints is named by, in the order printed.
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub paragraphs: Vec<Paragraph>,
}

impl Article {
    /// What a reader calls this unit: its label, else its number, else its
    /// kind and place, so a unit that prints neither is still named.
    pub fn name(&self) -> String {
        name(
            self.label.as_deref(),
            self.number.as_deref(),
            &self.kind,
            self.position,
        )
    }

    /// bron's reading of the unit.
    ///
    /// An article reads as its leden in order, each opened by the number the
    /// register gives it: the service's rendering of an article carries no lid
    /// numbers, and the lid is the unit Dutch practice cites, `art. 6:162 lid 2
    /// BW`. Any other unit reads as the service's rendering of it, because a
    /// bijlage prints words beside its leden that no lid holds, and its leden
    /// are pieces the parser cut rather than units anyone cites. Either falls
    /// back to the other where the service sent nothing to read.
    pub fn text(&self) -> String {
        let leden = self
            .leden()
            .map(Paragraph::cited)
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = self.rendered.clone().unwrap_or_default();

        let is_article = matches!(self.kind.as_str(), "artikel" | "enig-artikel");
        let (first, second) = if is_article {
            (leden, rendered)
        } else {
            (rendered, leden)
        };

        if first.is_empty() { second } else { first }
    }

    /// The leden worth citing, which are the ones the register printed text in.
    ///
    /// A repealed article is a single empty one. Keeping it would give every
    /// repeal a lid that says nothing, and the article's own emptiness beside
    /// its `vervallen` is the whole of what the register states.
    pub fn leden(&self) -> impl Iterator<Item = &Paragraph> {
        self.paragraphs.iter().filter(|lid| !lid.text().is_empty())
    }
}

/// See [`Article::name`].
pub fn name(
    label: Option<&str>,
    number: Option<&str>,
    kind: &str,
    position: Option<u32>,
) -> String {
    match (label, number, position) {
        (Some(label), _, _) => label.to_string(),
        (None, Some(number), _) if matches!(kind, "artikel" | "enig-artikel") => number.to_string(),
        (None, Some(number), _) => format!("{kind} {number}"),
        (None, None, Some(position)) => format!("{kind} {position}"),
        (None, None, None) => kind.to_string(),
    }
}

/// One lid, as the register numbers it.
#[derive(Debug, Clone, Deserialize)]
pub struct Paragraph {
    pub number: Option<String>,
    /// The service's reading of the bytes at `span`, present only where
    /// `rendered=1` asked for it.
    pub rendered: Option<String>,
    pub raw: Option<String>,
    pub jci: Option<String>,
    pub span: Span,
}

impl Paragraph {
    /// The service's reading, or the empty string where it sent none.
    pub fn text(&self) -> &str {
        self.rendered.as_deref().unwrap_or_default()
    }

    /// One lid as bron prints it, opened by its number where it has one.
    pub fn cited(&self) -> String {
        match &self.number {
            Some(number) => format!("{number}. {}", self.text()),
            None => self.text().to_string(),
        }
    }
}

/// A byte range into the rendition's document, absolute.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

/// Which window of the register to ask about, and how to narrow it.
///
/// A window on either axis, or both. Geldigheid is what applied to conduct and
/// zicht is what the register showed, so a rendition published with retroactive
/// effect falls in one and not the other, and asking one is not asking both.
#[derive(Debug)]
pub struct Published<'a> {
    /// How many rows to skip, so a list cut at its window can be read past.
    pub offset: u32,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub known_from: Option<NaiveDate>,
    pub known_to: Option<NaiveDate>,
    /// The register's own word for a kind of regulation: wet, AMvB, zbo.
    pub kind: Option<&'a str>,
    pub regulation: Option<&'a BwbId>,
    pub limit: u32,
}

/// Which rendition a question is about, and how much of its text to send.
///
/// Both coordinates travel, because the service resolves a point in both
/// windows and defaults the one it is not given to today. Asking what the
/// register showed on a past date without saying which date's text is meant
/// resolves against a rendition in force now, which for anything amended since
/// is no rendition at all.
#[derive(Debug)]
pub struct Ask<'a> {
    pub as_of: NaiveDate,
    /// Stated only where the caller asked what the register showed. One date
    /// names both coordinates of that point: the text for a day, as the
    /// register had it that day.
    pub as_known_on: Option<NaiveDate>,
    pub number: Option<&'a str>,
    /// One lid number as printed. The service narrows the span, the text, the
    /// label and the Juriconnect reference to that lid.
    pub paragraph: Option<&'a str>,
    /// Ask for the publisher's own bytes at every span. Off by default, so no
    /// markup travels for an answer nobody will read it in.
    pub raw: bool,
    /// Narrow to what this rendition publishes and does not yet apply.
    pub not_commenced: bool,
}

/// Which units of a rendition to ask for. See [`Ask`] for the two coordinates.
#[derive(Debug)]
pub struct Units<'a> {
    pub as_of: NaiveDate,
    pub as_known_on: Option<NaiveDate>,
    /// One lid number as printed, which narrows the span, the text and the
    /// reference to that lid.
    pub paragraph: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub number: Option<&'a str>,
    pub position: Option<u32>,
    pub raw: bool,
    /// Narrow to what this rendition publishes and does not yet apply.
    pub not_commenced: bool,
    pub limit: u32,
    pub offset: u32,
}

/// Which regulations to look for by name, and how to narrow the answer.
#[derive(Debug)]
pub struct Named<'a> {
    pub query: &'a str,
    /// The register's own word for a kind of regulation: wet, AMvB, zbo.
    pub kind: Option<&'a str>,
    /// The day a name is asked to stand on, on either axis.
    pub as_of: Option<NaiveDate>,
    pub as_known_on: Option<NaiveDate>,
    pub limit: u32,
    pub offset: u32,
}

/// Which relations to ask for, and how to narrow them.
#[derive(Debug)]
pub struct Relating<'a> {
    pub as_of: NaiveDate,
    pub as_known_on: Option<NaiveDate>,
    /// One kind of relation, as the WTI groups them.
    pub kind: Option<&'a str>,
    /// One article, as the service matches it. Passed on as typed: what the
    /// service matches it against is the service's rule, not one this end
    /// guesses at.
    pub article: Option<&'a str>,
    pub limit: u32,
    pub offset: u32,
}

/// Which two days to compare, and how to narrow the comparison.
#[derive(Debug)]
pub struct Compare<'a> {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// The day the register is read on, which selects what it showed then.
    pub as_known_on: Option<NaiveDate>,
    pub kind: Option<&'a str>,
    pub article: Option<&'a str>,
    pub all: bool,
    pub raw: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(number: Option<&str>, text: &str) -> Paragraph {
        Paragraph {
            number: number.map(str::to_string),
            rendered: Some(text.to_string()),
            raw: None,
            jci: None,
            span: Span { start: 0, end: 1 },
        }
    }

    fn unit(kind: &str, rendered: Option<&str>, leden: Vec<Paragraph>) -> Article {
        Article {
            kind: kind.into(),
            position: None,
            number: Some("162".into()),
            label: None,
            title: None,
            path: None,
            stam_id: "1".into(),
            amended_by: None,
            effect: None,
            status: None,
            lang: None,
            text_status: None,
            in_force_from: None,
            jci: None,
            span: Span { start: 0, end: 1 },
            rendered: rendered.map(str::to_string),
            raw: None,
            images: Vec::new(),
            paragraphs: leden,
        }
    }

    fn article(leden: Vec<Paragraph>) -> Article {
        unit("artikel", None, leden)
    }

    /// The lid is the unit Dutch practice cites, and the service's own
    /// rendering of an article carries no lid numbers at all.
    #[test]
    fn an_article_reads_as_its_leden_each_opened_by_its_number() {
        let article = unit(
            "artikel",
            Some("Eerste lid.\nTweede lid."),
            vec![
                paragraph(Some("1"), "Eerste lid."),
                paragraph(Some("2"), "Tweede lid."),
            ],
        );

        assert_eq!(article.text(), "1. Eerste lid.\n2. Tweede lid.");
    }

    /// An article printed without leden is one unnumbered unit, and opening it
    /// with a number the register never printed would be bron writing text.
    #[test]
    fn an_article_without_leden_reads_as_itself() {
        assert_eq!(
            article(vec![paragraph(None, "Eén geheel.")]).text(),
            "Eén geheel."
        );
    }

    /// A repealed article is a single empty lid. Keeping it would give every
    /// repeal a citable unit that says nothing.
    #[test]
    fn an_empty_lid_is_not_a_citable_unit() {
        let repealed = article(vec![paragraph(None, "")]);

        assert_eq!(repealed.leden().count(), 0);
        assert_eq!(repealed.text(), "");
    }

    /// A bijlage prints words beside its leden that no lid holds, and its
    /// own rendering is the only place they are.
    #[test]
    fn a_unit_that_is_no_article_reads_as_its_own_rendering() {
        let bijlage = unit(
            "bijlage",
            Some("Toelichting.\n1 Stofgroep"),
            vec![paragraph(None, "1 Stofgroep")],
        );

        assert_eq!(bijlage.text(), "Toelichting.\n1 Stofgroep");
        assert_eq!(
            unit("bijlage", None, vec![paragraph(None, "1 Stofgroep")]).text(),
            "1 Stofgroep"
        );
    }

    /// A number alone names an article; on any other unit it needs its kind,
    /// or bijlage Ia and an article Ia read the same.
    #[test]
    fn a_number_names_a_unit_with_its_kind_unless_it_is_an_article() {
        assert_eq!(name(None, Some("162"), "artikel", Some(3)), "162");
        assert_eq!(name(None, Some("1"), "enig-artikel", None), "1");
        assert_eq!(name(None, Some("Ia"), "bijlage", Some(2)), "bijlage Ia");
        assert_eq!(name(None, None, "aanhef", Some(1)), "aanhef 1");
        assert_eq!(
            name(Some("Lijst Ia"), Some("Ia"), "bijlage", None),
            "Lijst Ia"
        );
    }
}
