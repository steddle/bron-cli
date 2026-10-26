//! Human output.
//!
//! Reads the same envelope the JSON does, so the two cannot describe different
//! things. Anything the envelope marks as a caveat is printed, because a limit
//! on what an answer supports is not a detail a reader may miss.

// Writing into a `String` cannot fail, which is why every `write!` into one
// here discards its result. A buffer that becomes an `io::Write` must not.
use std::fmt::Write as _;
use std::io::{self, Write};

use chrono::NaiveDate;

use bron::service::{
    Article, CitingDecision, Consolidated, ContentsPart, CourtCount, Dossier, Expression, Had,
    Lineage, LinkedDecree, ListedDecision, Pinpoint, Place, Produced, Regulation, Relation, Remark,
    SearchData, SearchLid, Service, Upcoming, name,
};
use bron::wire::Caveat;

use crate::run::Answer;

/// Column widths for a citation lookup, named once so the line and everything
/// written under it cannot drift apart.
///
/// `NOT_HELD` is a column and not a suffix: what it says is true of the
/// rendition, not of the label it would otherwise trail.
const READING: usize = 8;
const SOURCE: usize = 11;
const NOT_HELD: &str = "not held";

/// Column widths for a list of renditions: the longest window, `from` and
/// `until`; the longest hash state, `unchecked`; and an image count.
const WINDOW: usize = 24;
const HASH_CHECK: usize = 9;
const IMAGES: usize = 10;

/// The widest a hit's `found_by` reads, `vector+text`.
const FOUND_BY: usize = 11;

/// A waiting row that states no day to come, as wide as a date.
const NO_DAY: &str = "no day yet";

/// Writes one answer for a person.
///
/// Where stdout carries rows, one per thing listed, every note on them goes to
/// `err`: a caveat, the cursor, a legend. A shell script reading line by line
/// then never reads a sentence as a row. An answer that is a document rather
/// than a list keeps its notes beneath it on `out`.
pub fn answer(answer: &Answer, out: &mut impl Write, err: &mut impl Write) -> io::Result<()> {
    match answer {
        Answer::LawHits(a) => {
            let held = |hit: &Regulation| match (hit.held_count, hit.expression_count) {
                (Some(held), Some(count)) => format!("{held} of {count} held"),
                _ => String::new(),
            };
            let kind = |hit: &Regulation| hit.regulation_type.clone().unwrap_or_default();
            // Widths read off the rows in this answer, counted in characters as
            // `{:<w$}` pads, so the title starts in one place on every row. A
            // column no row fills is left out rather than printed as a gap.
            let width = |column: &dyn Fn(&Regulation) -> String| {
                a.data
                    .iter()
                    .map(|hit| column(hit).chars().count())
                    .max()
                    .unwrap_or(0)
            };
            let kind_width = width(&kind);
            let coverage_width = width(&|hit| hit.coverage.clone());
            let held_width = width(&held);

            for hit in &a.data {
                let mut row = hit.id.clone();
                for (value, width) in [
                    (kind(hit), kind_width),
                    (hit.coverage.clone(), coverage_width),
                    (held(hit), held_width),
                ] {
                    if width > 0 {
                        let _ = write!(row, "  {value:<width$}");
                    }
                }
                if let Some(title) = &hit.title {
                    let _ = write!(row, "  {title}");
                }
                writeln!(out, "{}", row.trim_end())?;
            }
            caveats(&a.caveats, err)
        }

        Answer::Found(a) => {
            if let Some(line) = bound(a.as_of, a.as_known_on) {
                writeln!(out, "{line}\n")?;
            }

            match &a.data {
                SearchData::Ranking { hits, .. } => {
                    if hits.is_empty() {
                        // Said in words. A blank screen reads as a failure, and
                        // this is an answer: nothing in what was searched came
                        // close.
                        writeln!(out, "nothing in the corpus came close to that")?;
                    }

                    for hit in hits {
                        let unit = unit_of(&hit.unit, hit.lid.as_ref());

                        let distance = hit
                            .distance
                            .map_or_else(|| "    -".to_string(), |d| format!("{d:.3}"));

                        let found_by = hit.found_by.join("+");
                        let hit_name =
                            name(hit.label.as_deref(), hit.number.as_deref(), &hit.kind, None);
                        let row = HitRow {
                            distance: &distance,
                            source: &hit.source,
                            unit: &unit,
                            regulation: &hit.regulation_id,
                            expression: &hit.expression,
                            found_by: &found_by,
                            name: &hit_name,
                        };
                        writeln!(out, "{}", row.line())?;

                        // The same words in other places, each named as the
                        // hit is, so a text sitting in two regulations is not
                        // read as found once.
                        write_places(&hit.also, out)?;
                        let unnamed = (hit.places as usize).saturating_sub(1 + hit.also.len());
                        if unnamed > 0 {
                            writeln!(out, "{:>5}  and {unnamed} more place(s)", "")?;
                        }

                        // The unit's other leden that ranked, in rank order,
                        // under the unit rather than as hits of their own.
                        for lid in &hit.leden {
                            let lid_unit = lid
                                .number
                                .as_ref()
                                .map_or_else(|| "lid".to_string(), |n| format!("lid {n}"));
                            let lid_distance = lid
                                .distance
                                .map_or_else(|| "    -".to_string(), |d| format!("{d:.3}"));
                            let lid_found_by = lid.found_by.join("+");
                            let row = HitRow {
                                distance: &lid_distance,
                                source: "same unit",
                                unit: &lid_unit,
                                regulation: &hit.regulation_id,
                                expression: &hit.expression,
                                found_by: &lid_found_by,
                                name: &hit_name,
                            };
                            writeln!(out, "{}", row.line())?;
                            write_places(&lid.also, out)?;
                        }
                    }
                }

                // No distance, and the column it would sit in is not left
                // empty: a lookup is arithmetic and prints what it resolved.
                // The reading leads every line, because with more than one it
                // is the answer to which of them the reader meant.
                SearchData::Lookup {
                    citation,
                    paragraph,
                    name: law,
                    resolved,
                } => {
                    let mut read = citation.clone();
                    if let Some(lid) = paragraph {
                        let _ = write!(read, " lid {lid}");
                    }
                    if let Some(law) = law {
                        let _ = write!(read, " {law}");
                    }
                    writeln!(out, "{read}\n")?;

                    for entry in resolved {
                        // Holding is its own column, so the name after it
                        // starts in the same place on every line rather than
                        // being pushed along by however long the one above was.
                        let holding = if entry.held { "" } else { NOT_HELD };
                        let name = entry.label.as_deref().unwrap_or(&entry.number);

                        writeln!(
                            out,
                            "{:<reading$} {:<source$} {}  {}  {holding:<width$} {name}",
                            entry.reading,
                            entry.source,
                            entry.regulation_id,
                            entry.expression,
                            reading = READING,
                            source = SOURCE,
                            width = NOT_HELD.len(),
                        )?;
                        // The service reads a lid and does not look it up, so
                        // the answer above is the article's. Where the text is
                        // held, the command that answers the lid is named.
                        if let Some(lid) = paragraph
                            && entry.held
                        {
                            writeln!(
                                out,
                                "{:indent$}the lid is not looked up here: law show {} --article {} \
                                 --paragraph {lid} answers it",
                                "",
                                entry.regulation_id,
                                entry.number,
                                indent = READING + 1,
                            )?;
                        }
                        // A limit on one reading is not a limit on the answer,
                        // and the reader quoting that line is the one who needs
                        // it. Indented to the column the line starts in, read
                        // off that column rather than counted out again here.
                        for caveat in &entry.caveats {
                            writeln!(
                                out,
                                "{:indent$}{}",
                                "",
                                caveat_line(&Caveat::stated(caveat)),
                                indent = READING + 1,
                            )?;
                        }
                    }
                }
            }

            caveats(&a.caveats, out)
        }

        Answer::Published(a) => {
            if a.data.is_empty() {
                writeln!(out, "The register published nothing in that window.")?;
                return caveats(&a.caveats, out);
            }

            for rendition in &a.data {
                writeln!(
                    out,
                    "{}",
                    rendition_row(rendition, Some(&rendition.regulation_id))
                )?;
            }
            rendition_legend(&a.data, err)?;
            caveats(&a.caveats, err)
        }

        Answer::Feed(a) => {
            for event in &a.data.events {
                let mut row = format!(
                    "{:>8}  {:<12}{}",
                    event.seq, event.event, event.regulation_id
                );
                // A regulation-level event names no rendition, so the row ends
                // at the identifier rather than on the padding of an empty one.
                if let Some(label) = &event.label {
                    let _ = write!(
                        row,
                        "  {label}  {:<WINDOW$}  {:<WINDOW$}",
                        stated(event.in_force),
                        stated(event.as_known),
                    );
                }
                if event.withdrawn {
                    let _ = write!(row, "  withdrawn");
                }
                writeln!(out, "{}", row.trim_end())?;
            }

            if a.data.events.is_empty() {
                writeln!(out, "The register has done nothing since {}.", a.data.next)?;
            }

            // A shell script takes its next cursor from --json, where it is a
            // field rather than a sentence.
            writeln!(
                err,
                "next {}  head {}  origin {}",
                a.data.next, a.data.head, a.data.origin
            )?;

            caveats(&a.caveats, err)
        }

        // Rows, one per regulation or published act, in the service's order;
        // what the columns mean is a note on them and goes to stderr.
        Answer::Upcoming(a) => {
            let rows = &a.data;
            if rows.is_empty() {
                writeln!(
                    out,
                    "No regulation prints a unit waiting to apply, as asked."
                )?;
                return caveats(&a.caveats, out);
            }

            // The lists last and named, because one regulation naming three
            // acts would otherwise push every row's title out by the width of
            // all three.
            let listed = |named: &[String]| {
                if named.is_empty() {
                    "-".to_string()
                } else {
                    named.join(", ")
                }
            };
            let linked = |named: &[LinkedDecree]| {
                if named.is_empty() {
                    "-".to_string()
                } else {
                    named
                        .iter()
                        .map(|d| d.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            };
            let cited = |named: &[Dossier]| {
                if named.is_empty() {
                    "-".to_string()
                } else {
                    named
                        .iter()
                        .map(|d| d.dossier.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            };

            // Column widths read off the consolidated rows alone: a published
            // row has no label or unit count to align.
            let consolidated: Vec<&Consolidated> = rows
                .iter()
                .filter_map(|row| match row {
                    Upcoming::Consolidated(row) => Some(row),
                    Upcoming::Published(_) | Upcoming::Unknown(_) => None,
                })
                .collect();
            // A row where some units state a day to come and some do not says
            // how many do, so the first day is not read as every unit's. Where
            // none does, `no day yet` already says so.
            let partly_dated = |row: &Consolidated| 0 < row.dated && row.dated < row.units;
            let units_width = consolidated
                .iter()
                .map(|row| digits(row.units))
                .max()
                .unwrap_or(1);
            let dated_width = consolidated
                .iter()
                .filter(|row| partly_dated(row))
                .map(|row| ", ".len() + digits(row.dated) + " dated".len())
                .max()
                .unwrap_or(0);
            let label_width = consolidated
                .iter()
                .map(|row| row.label.chars().count())
                .max()
                .unwrap_or(0);

            for row in rows {
                match row {
                    Upcoming::Consolidated(row) => {
                        let day = row
                            .first_day
                            .map_or_else(|| NO_DAY.to_string(), |day| day.to_string());
                        let dated = if partly_dated(row) {
                            format!(", {} dated", row.dated)
                        } else {
                            String::new()
                        };
                        let mut line = format!(
                            "{day}  {}  {:<label_width$}  {:>units_width$} \
                             waiting{dated:<dated_width$}",
                            row.regulation_id, row.label, row.units
                        );
                        if let Some(title) = &row.regulation_title {
                            let _ = write!(line, "  {title}");
                        }
                        let _ = write!(
                            line,
                            "  acts {}  decrees {}  linked decrees {}",
                            listed(&row.acts),
                            listed(&row.decrees),
                            linked(&row.linked_decrees),
                        );
                        writeln!(out, "{line}")?;
                    }
                    // Stated no day, in every held answer: the register keeps
                    // no future rendition, and neither does the Staatsblad.
                    Upcoming::Published(row) => {
                        let mut line = format!(
                            "{NO_DAY}  {}  {}, published in the Staatsblad on {}, not \
                             consolidated in the BWB",
                            row.publication, row.kind, row.published_on
                        );
                        if let Some(title) = &row.title {
                            let _ = write!(line, "  {title}");
                        }
                        let _ = write!(
                            line,
                            "  dossiers {}  linked decrees {}",
                            cited(&row.dossiers),
                            linked(&row.linked_decrees),
                        );
                        writeln!(out, "{line}")?;
                    }
                    // A stage this build has no name for does not sink the
                    // rows beside it. --json still carries it as the service
                    // sent it. No day column: this build does not know the
                    // row's shape well enough to say it has none.
                    Upcoming::Unknown(value) => {
                        let stage = value.get("stage").and_then(|s| s.as_str()).unwrap_or("?");
                        writeln!(
                            out,
                            "stage {stage}, which this build has no sentence for: {value}"
                        )?;
                    }
                }
            }

            writeln!(
                err,
                "first column = the earliest day to come a waiting unit states, {NO_DAY} = none \
                 states one, or the row is a published act that states no day"
            )?;
            writeln!(
                err,
                "a consolidated row: the rendition read, how many units it prints as waiting, \
                 and how many of them state a day to come where some do and some do not"
            )?;
            writeln!(
                err,
                "acts = the publications that produced the waiting text, decrees = the besluiten \
                 the rendition names as commencing it, - = it names none"
            )?;
            writeln!(
                err,
                "linked decrees = besluiten bron holds that name the row's act, whether or not \
                 the BWB has taken them in yet, - = it holds none; a published row's dossiers \
                 are the kamerstukdossiers the act states as its own"
            )?;
            caveats(&a.caveats, err)
        }

        // Rows, one per text the act produced; the act itself is a note on
        // them and goes to stderr.
        Answer::Amendments(a) => {
            let produced = &a.data.articles;
            let named = |unit: &Produced| {
                name(
                    unit.label.as_deref(),
                    unit.number.as_deref(),
                    &unit.kind,
                    None,
                )
            };
            let width = |of: &dyn Fn(&Produced) -> String| {
                produced
                    .iter()
                    .map(|u| of(u).chars().count())
                    .max()
                    .unwrap_or(0)
            };
            let effect = |unit: &Produced| unit.effect.clone().unwrap_or_default();
            let (name_width, effect_width) = (width(&named), width(&effect));

            for unit in produced {
                let mut row = format!(
                    "{}  {:<name_width$}  {:<effect_width$}",
                    unit.regulation_id,
                    named(unit),
                    effect(unit),
                );
                if let Some(day) = unit.in_force_from {
                    let _ = write!(row, "  {day}");
                }
                writeln!(out, "{}", row.trim_end())?;
            }

            if produced.is_empty() {
                writeln!(
                    out,
                    "The act that established it produced no text held here."
                )?;
            }
            if !a.data.established_by.is_empty() {
                writeln!(err, "established by {}", a.data.established_by.join(", "))?;
            }

            caveats(&a.caveats, err)
        }

        // Rows, one per relation the register states.
        Answer::Relations {
            answered: a,
            article,
        } => {
            let relations = &a.data.relations;
            if relations.is_empty() {
                writeln!(out, "The register states no relation for that.")?;
                // Measured on production: the service matches an article by
                // the label the register prints, so a bare number answers
                // nothing and would read as an article nothing rests on.
                if let Some(article) = article {
                    writeln!(
                        out,
                        "note: the register states a relation under the label it prints, so \
                         --article 'Artikel {article}' is what that reads as here."
                    )?;
                }
                return caveats(&a.caveats, out);
            }

            let width = |of: &dyn Fn(&Relation) -> String| {
                relations
                    .iter()
                    .map(|r| of(r).chars().count())
                    .max()
                    .unwrap_or(0)
            };
            let article = |relation: &Relation| relation.article.clone().unwrap_or_default();
            let (kind_width, article_width) = (width(&|r| r.kind.clone()), width(&article));

            for relation in relations {
                // The article column is blank where the register states the
                // relation for the regulation whole, which is a fact about
                // that relation rather than a missing value.
                writeln!(
                    out,
                    "{:<kind_width$}  {:<article_width$}  {}  {:<WINDOW$}  {}",
                    relation.kind,
                    article(relation),
                    relation.related.regulation_id,
                    stated(relation.in_force),
                    relation.related.title.as_deref().unwrap_or(""),
                )?;
            }

            caveats(&a.caveats, err)
        }

        // A document rather than a list: one regulation, and what the index
        // states about it.
        Answer::About(a) => {
            let about = &a.data;
            writeln!(
                out,
                "{}  {}",
                about.id,
                about.title.as_deref().unwrap_or("")
            )?;

            let mut line = Vec::new();
            if let Some(kind) = &about.regulation_type {
                line.push(kind.clone());
            }
            line.push(about.coverage.clone());
            if let (Some(held), Some(count)) = (about.held_count, about.expression_count) {
                line.push(format!("{held} of {count} renditions held"));
            }
            writeln!(out, "{}", line.join(", "))?;

            for (what, named) in [
                ("authority", about.authority.clone().into_iter().collect()),
                ("legal areas", about.legal_areas.clone()),
                ("domains", about.government_domains.clone()),
                ("established by", about.established_by.clone()),
            ] {
                if !named.is_empty() {
                    writeln!(out, "{what}: {}", named.join(", "))?;
                }
            }

            caveats(&a.caveats, out)
        }

        // One row per text, because a history is a list: what the unit said,
        // for how long, on whose act, and in which renditions.
        Answer::History(a) => {
            let units = &a.data.articles;
            let rows: Vec<(&Lineage, &Had)> = units
                .iter()
                .flat_map(|unit| unit.entries.iter().map(move |entry| (unit, entry)))
                .collect();

            let width = |of: &dyn Fn(&(&Lineage, &Had)) -> String| {
                rows.iter()
                    .map(|row| of(row).chars().count())
                    .max()
                    .unwrap_or(0)
            };
            // A number can name more than one unit, so every row says which.
            let named = |(unit, entry): &(&Lineage, &Had)| {
                name(
                    entry.label.as_deref().or(unit.label.as_deref()),
                    entry.number.as_deref().or(unit.number.as_deref()),
                    &unit.kind,
                    None,
                )
            };
            let act = |(_, entry): &(&Lineage, &Had)| match (&entry.amended_by, &entry.effect) {
                (Some(act), Some(effect)) => format!("{act} ({effect})"),
                (Some(act), None) => act.clone(),
                (None, _) => String::new(),
            };
            let (name_width, act_width) = (width(&named), width(&act));

            for row in &rows {
                let (_, entry) = row;
                // `text` is the corpus holding the words; the other two are
                // the register's renditions not held, or held and carrying no
                // such unit. Silence would read as a window it said nothing in.
                let state = match entry.state.as_str() {
                    _ if entry.withdrawn.is_some() => "withdrawn",
                    "text" => "",
                    "unknown" => "not held",
                    "absent" => "not carried",
                    other => other,
                };
                writeln!(
                    out,
                    "{:<name_width$}  {:<WINDOW$}  {:<11} {:<act_width$}  {} {}, {} to {}",
                    named(row),
                    window(entry.in_force.from, entry.in_force.until),
                    state,
                    act(row),
                    entry.renditions.count,
                    if entry.renditions.count == 1 {
                        "rendition"
                    } else {
                        "renditions"
                    },
                    entry.renditions.first,
                    entry.renditions.last,
                )?;
            }

            // Notes on the rows, so on stderr: the register's own remarks,
            // at their range in the entry's first rendition.
            for row @ (_, entry) in &rows {
                if entry.withdrawn.is_some() {
                    writeln!(
                        err,
                        "{}, {}: {WITHDRAWN}",
                        named(row),
                        window(entry.in_force.from, entry.in_force.until)
                    )?;
                }
                if let Some(remarks) = &entry.remarks {
                    write_remarks(
                        remarks,
                        &format!("{} in {}: ", named(row), entry.renditions.first),
                        err,
                    )?;
                }
            }

            caveats(&a.caveats, err)
        }

        Answer::Versions(a) => {
            for version in &a.data {
                writeln!(out, "{}", rendition_row(version, None))?;
            }
            if !a.data.is_empty() {
                rendition_legend(&a.data, err)?;
            }
            caveats(&a.caveats, err)
        }

        // A document rather than a list: the text of each unit, and the notes
        // on the answer beneath it.
        Answer::Parts {
            answered: a,
            service,
        } => {
            answered_by(
                header(&a.data.parts, a.as_of, a.as_known_on).as_deref(),
                &a.data.expression,
                out,
            )?;

            for part in &a.data.parts {
                write_unit(part, &a.data.expression, service, Narrowed::default(), out)?;
            }

            caveats(&a.caveats, out)
        }

        // The outline alone on stdout, so it pipes; which rendition answered
        // and every note on stderr.
        Answer::Contents(a) => {
            let rendition = &a.data.expression;
            answered_by(bound(a.as_of, a.as_known_on).as_deref(), rendition, err)?;

            if a.data.contents.is_empty() {
                writeln!(
                    err,
                    "The service read no contents from {} rendition {}.",
                    a.data.regulation_id, rendition.label
                )?;
            }

            let mut starred = false;
            outline(&a.data.contents, 0, out, &mut starred)?;
            if starred {
                writeln!(err, "* = repealed (vervallen)")?;
            }
            caveats(&a.caveats, err)
        }

        Answer::Articles {
            answered: a,
            service,
            number,
            paragraph,
            item,
            not_commenced,
        } => {
            let articles = &a.data.articles;

            // An answer the service gave with no article in it, in one line:
            // why there is none, said where the service said it, and where a
            // rendition that holds text prints its other units.
            if articles.is_empty() {
                let law = &a.data.regulation_id;
                let rendition = &a.data.expression;
                let named = format!("{law} rendition {}", rendition.label);
                let line = match (&rendition.blob, &rendition.unread_because) {
                    (None, _) => {
                        format!("The service holds no text of {named}, so it answers no articles.")
                    }
                    (Some(_), Some(because)) => format!(
                        "{named} is held and was not parsed ({because}), so it answers no articles."
                    ),
                    (Some(_), None) => {
                        let what = match (number, paragraph, not_commenced) {
                            (Some(number), _, _) => format!("no article {number}"),
                            (None, Some(lid), _) => format!("no article with a lid {lid}"),
                            (None, None, true) => "no article it does not yet apply".to_string(),
                            (None, None, false) => "no articles".to_string(),
                        };
                        let parts = if *not_commenced {
                            format!(
                                "`law parts {law} --not-commenced` answers its waiting units of \
                                 every kind"
                            )
                        } else {
                            format!("`law parts {law}` answers the other units it prints")
                        };
                        format!("{named} prints {what}; {parts}.")
                    }
                };
                writeln!(out, "{line}")?;
                return caveats(&a.caveats, out);
            }

            answered_by(
                header(articles, a.as_of, a.as_known_on).as_deref(),
                &a.data.expression,
                out,
            )?;

            for article in articles {
                write_unit(
                    article,
                    &a.data.expression,
                    service,
                    Narrowed {
                        lid: paragraph.as_deref(),
                        item: item.as_deref(),
                    },
                    out,
                )?;
            }
            caveats(&a.caveats, out)
        }

        Answer::Changes {
            answered: a,
            narrowed,
            from_url,
            to_url,
        } => {
            let changes = &a.data;

            writeln!(out, "{}", changes.regulation_id)?;
            for (side, rendition, url) in [
                ("from", &changes.from, from_url),
                ("to", &changes.to, to_url),
            ] {
                writeln!(
                    out,
                    "{side:<4}  {}  {}, hash {}",
                    rendition.label,
                    window(rendition.in_force.from, rendition.in_force.until),
                    rendition.upstream_hash_check,
                )?;
                if let Some(url) = url {
                    writeln!(out, "      {url}")?;
                }
            }
            writeln!(out)?;

            // The service counts every unit whatever narrowed the list, so
            // above a narrowed list the counts say whose they are. Kept rather
            // than left out: how much changed around the unit asked about is
            // still an answer, as long as it is not read as that unit's.
            if *narrowed {
                write!(out, "in the whole regulation: ")?;
            }
            let s = &changes.summary;
            writeln!(
                out,
                "{} amended, {} commenced, {} added, {} repealed, {} withdrawn, {} unchanged\n",
                s.amended, s.commenced, s.added, s.repealed, s.withdrawn, s.unchanged
            )?;

            for change in &changes.articles {
                let side = change.to.as_ref().or(change.from.as_ref());
                let name = name(
                    change.label.as_deref(),
                    change.number.as_deref(),
                    &change.kind,
                    side.and_then(|side| side.position),
                );
                writeln!(out, "{:<10} {name}", change.change.to_uppercase())?;

                if let Some(path) = &change.path {
                    writeln!(out, "           {path}")?;
                }

                if let Some(to) = &change.to
                    && let Some(act) = &to.amended_by
                {
                    let mut line = format!("           through {act}");
                    if let Some(effect) = &to.effect {
                        let _ = write!(line, " ({effect})");
                    }
                    // The act the earlier text stood through, where it is not
                    // the same one: an amendment names what it replaced.
                    if let Some(earlier) = change.from.as_ref().and_then(|f| f.amended_by.as_ref())
                        && earlier != act
                    {
                        let _ = write!(line, "; was through {earlier}");
                    }
                    writeln!(out, "{line}")?;
                }

                if change.change == "commenced"
                    && let Some(day) = change.to.as_ref().and_then(|to| to.in_force_from)
                {
                    writeln!(out, "           commenced on {day}")?;
                }
                if change.to.as_ref().is_some_and(|to| to.withdrawn.is_some()) {
                    writeln!(out, "           {WITHDRAWN}")?;
                }

                // Where the unit stands in each side's document, which the
                // lines above name, so either text can be cut again.
                match (&change.from, &change.to) {
                    (Some(from), Some(to)) => {
                        writeln!(out, "           spans {} -> {}", span(from), span(to))?
                    }
                    (Some(from), None) => writeln!(out, "           span {} (from)", span(from))?,
                    (None, Some(to)) => writeln!(out, "           span {} (to)", span(to))?,
                    (None, None) => {}
                }
                // A lid the later side prints under another number with the
                // same words, which the service matched on the text.
                for lid in change.to.iter().flat_map(|to| &to.paragraphs) {
                    if let (Some(number), Some(was)) = (&lid.number, &lid.renumbered_from) {
                        writeln!(out, "           lid {number} was lid {was}, word for word")?;
                    }
                }
                // Each side's remarks stand in that side's document.
                for (side, unit) in [("from", &change.from), ("to", &change.to)] {
                    if let Some(remarks) = unit.as_ref().and_then(|unit| unit.remarks.as_deref()) {
                        write_remarks(remarks, &format!("           {side} "), out)?;
                    }
                }

                match (&change.from, &change.to) {
                    (Some(from), Some(to)) if change.change == "amended" => {
                        for line in word_diff(&from.text(), &to.text()) {
                            writeln!(out, "  {line}")?;
                        }
                    }
                    (None, Some(to)) => writeln!(out, "  + {}", first_line(&to.text()))?,
                    (Some(from), None) => writeln!(out, "  - {}", first_line(&from.text()))?,
                    _ => {}
                }
                writeln!(out)?;
            }

            caveats(&a.caveats, out)
        }

        Answer::Decision {
            answered: a,
            service,
            offset,
        } => {
            let decision = &a.data.decision;

            let mut head = decision.id.clone();
            if decision.form.as_deref() == Some("opinion") {
                head.push_str("  conclusie");
            }
            writeln!(out, "{head}")?;
            if let Some(court) = &decision.court {
                writeln!(out, "{court}")?;
            }
            let dates: Vec<String> = [
                decision.decided_on.map(|day| format!("decided {day}")),
                decision.published_on.map(|day| format!("published {day}")),
            ]
            .into_iter()
            .flatten()
            .collect();
            if !dates.is_empty() {
                writeln!(out, "{}", dates.join(", "))?;
            }
            if let Some(procedure) = decision.native("procedure") {
                writeln!(out, "{procedure}")?;
            }
            if let Some(numbers) = decision.native("zaaknummer") {
                writeln!(out, "zaaknummer {numbers}")?;
            }
            if let Some(celex) = decision.native("celex") {
                writeln!(out, "celex {celex}")?;
            }
            let documents = decision
                .rendition
                .as_ref()
                .map_or(&[][..], |rendition| &rendition.documents);
            if let Some(rendition) = &decision.rendition {
                if let Some(blob) = &rendition.blob {
                    writeln!(out, "source: {blob}")?;
                    if let Some(url) = service.blob_url(blob) {
                        writeln!(out, "{url}")?;
                    }
                }
                for document in documents {
                    write!(out, "source: {} ({}", document.blob, document.root)?;
                    if let Some(language) = &rendition.language {
                        write!(out, ", {language}")?;
                    }
                    writeln!(out, ", {} bytes)", document.blob_size)?;
                    if let Some(url) = service.blob_url(&document.blob) {
                        writeln!(out, "{url}")?;
                    }
                }
            }
            if let Some(stamp) = &decision.known_since {
                writeln!(out, "as the publisher served it since {stamp}")?;
            }
            // Never served, so named and nothing more: a text the court took
            // back stays out of what a reader is handed.
            if !decision.superseded.is_empty() {
                writeln!(
                    out,
                    "replaces {} earlier text(s) the publisher withdrew, which bron does not serve",
                    decision.superseded.len()
                )?;
            }

            // That bron withholds it and never why, as the site says it.
            match (&decision.withheld_because, decision.register.as_str()) {
                (Some(_), "acm") => writeln!(out, "\nbron does not show this besluit.")?,
                (Some(_), _) => writeln!(out, "\nbron does not show this ruling.")?,
                (None, _) if decision.has_body => {}
                (None, "rechtspraak") => writeln!(
                    out,
                    "\nrechtspraak publishes {} as metadata alone, so it has no text.",
                    decision.id
                )?,
                (None, _) => writeln!(out, "\nThe service answers no text of {}.", decision.id)?,
            }

            // Where the package holds more than one document, a span is a range
            // of the document named above it.
            let mut reading = None;
            for paragraph in &a.data.paragraphs {
                if documents.len() > 1
                    && let Some(document) = paragraph.document.as_deref()
                    && reading != Some(document)
                {
                    let root = documents
                        .iter()
                        .find(|held| held.blob == document)
                        .map_or("document", |held| held.root.as_str());
                    writeln!(out, "\n{root}: {document}")?;
                    reading = Some(document);
                }

                let mut name = match (paragraph.kind.as_str(), &paragraph.section_path) {
                    ("inhoudsindicatie", _) => "inhoudsindicatie".to_string(),
                    ("keywords", _) => "keywords".to_string(),
                    ("title", Some(path)) => format!("section {path}"),
                    ("title", None) => "section".to_string(),
                    ("footnote", Some(path)) => path.clone(),
                    (_, Some(path)) => format!("{path}, paragraph {}", paragraph.ordinal),
                    _ => format!("paragraph {}", paragraph.ordinal),
                };
                // The Court's number is how its paragraph is cited; the ordinal
                // stays beside it, because --offset counts ordinals.
                if let Some(number) = &paragraph.number {
                    name = format!("point {number}, {name}");
                }
                writeln!(
                    out,
                    "\n{name}  ({}..{})",
                    paragraph.span.start, paragraph.span.end
                )?;
                if let Some(text) = paragraph.rendered.as_deref().or(paragraph.raw.as_deref()) {
                    writeln!(out, "{text}")?;
                }
            }

            // A cut ruling is a page of one document and not a filtered list,
            // so `truncated` says where the next page starts.
            let (cut, rest): (Vec<Caveat>, Vec<Caveat>) = a
                .caveats
                .iter()
                .cloned()
                .partition(|caveat| matches!(caveat, Caveat::Truncated { .. }));
            if let Some(Caveat::Truncated { window }) = cut.first() {
                // The service accepted the offset, so it is a whole number.
                let from = offset
                    .as_deref()
                    .and_then(|typed| typed.parse::<u64>().ok())
                    .unwrap_or(0);
                let shown = a.data.paragraphs.len() as u64;
                writeln!(
                    out,
                    "\nnote: units {} to {} shown; read on with --offset {}, or the whole ruling with --limit none",
                    from + 1,
                    from + shown,
                    from + u64::from(*window)
                )?;
            }
            caveats(&rest, out)
        }

        Answer::DecisionRelations(a) => {
            let ruling = &a.data;

            if ruling.relations.is_empty() {
                writeln!(
                    out,
                    "No document bron holds states a relation for {}.",
                    ruling.id
                )?;
            }
            for relation in &ruling.relations {
                let mut row = match &relation.aanleg {
                    Some(aanleg) => format!("{aanleg}  {}", relation.kind),
                    None => relation.kind.clone(),
                };
                // Under `cited_by` the place is in this ruling, not in the
                // other, so it stands before the other's id.
                if relation.direction.as_deref() == Some("cited_by") {
                    row.push_str("  ");
                    if let Some(place) = pinpoint(relation.place.as_ref(), relation.part.as_deref())
                    {
                        let _ = write!(row, "{place} ");
                    }
                    let _ = write!(row, "cited by {}", relation.to.id);
                } else {
                    let _ = write!(row, "  {}", relation.to.id);
                    if let Some(place) = relation
                        .label
                        .clone()
                        .or_else(|| pinpoint(relation.place.as_ref(), relation.to.part.as_deref()))
                    {
                        let _ = write!(row, " {place}");
                    }
                    if let Some(title) = &relation.title {
                        let _ = write!(row, "  {title}");
                    }
                }
                if relation.asserted == "machine" {
                    row.push_str("  (bron's reading)");
                }
                if let Some(digest) = &relation.stated_in {
                    let _ = write!(row, "  stated in {digest}");
                }
                writeln!(out, "{row}")?;
            }
            // The one conclusion a reader is tempted to draw, and the one these
            // rows cannot support.
            writeln!(
                err,
                "A relation is the publisher's statement. None says whether an appeal was lodged."
            )?;

            let notes: Vec<String> = a
                .caveats
                .iter()
                .map(|caveat| match caveat {
                    Caveat::MachineReading { .. } => "note: the articles and rulings marked \
                         (bron's reading) are the references LiDO's computer recognised in the \
                         ruling's text, which no publisher states. A reference it recognised can \
                         name another regulation than the one the court meant, so read each \
                         beside the ruling."
                        .to_string(),
                    other => caveat_line(other),
                })
                .collect();
            if !notes.is_empty() {
                writeln!(err, "\n{}", notes.join("\n"))?;
            }
            Ok(())
        }

        Answer::Publication(a) => {
            let doc = &a.data;

            write!(out, "{}", doc.id)?;
            if let Some(kind) = &doc.publication_type {
                write!(out, "  {kind}")?;
            }
            writeln!(out, "  {}", if doc.held { "held" } else { NOT_HELD })?;
            if let Some(title) = &doc.title {
                writeln!(out, "{title}")?;
            }
            if let Some(dossier) = &doc.dossier {
                writeln!(out, "dossier {dossier}")?;
            }
            let dates: Vec<String> = [
                doc.signed_on.map(|d| format!("signed {d}")),
                doc.published_on.map(|d| format!("published {d}")),
            ]
            .into_iter()
            .flatten()
            .collect();
            if !dates.is_empty() {
                writeln!(out, "{}", dates.join(", "))?;
            }

            writeln!(out)?;
            for manifestation in &doc.manifestations {
                let mut row = manifestation.format.clone();
                if let Some(blob) = &manifestation.blob {
                    let _ = write!(row, "  {blob}");
                }
                writeln!(out, "{row}")?;
            }

            if !doc.dossiers.is_empty() {
                writeln!(out, "\ndossiers")?;
                for dossier in &doc.dossiers {
                    writeln!(
                        out,
                        "  {}  stated in {}",
                        dossier.dossier, dossier.stated_in
                    )?;
                }
            }

            // The span points into the XML manifestation, so a reader who
            // wants to check a unit fetches that document and none other.
            let xml = doc
                .manifestations
                .iter()
                .find(|m| m.format == "xml")
                .and_then(|m| m.blob.as_deref());

            for unit in doc.units.iter().flatten() {
                writeln!(out, "\n{}", unit.name())?;
                match xml {
                    Some(blob) => writeln!(
                        out,
                        "source: {blob} ({}..{})",
                        unit.span.start, unit.span.end
                    )?,
                    None => writeln!(out, "span {}..{}", unit.span.start, unit.span.end)?,
                }
                writeln!(out, "hash {}", unit.content_hash)?;
                if let Some(text) = unit.rendered.as_deref().or(unit.raw.as_deref()) {
                    writeln!(out, "\n{text}")?;
                }
            }

            caveats(&a.caveats, out)
        }

        // The bytes on stdout exactly as they hashed, and nothing else there:
        // anything written beside them would change what a pipe hashes. What
        // was checked goes to stderr.
        Answer::Blob { envelope, bytes } => {
            out.write_all(bytes)?;
            let Some(blob) = &envelope.data else {
                return Ok(());
            };
            let checked = &blob.provenance;
            write!(err, "{} bytes hash to {}", checked.byte_len, checked.blob)?;
            if let Some(span) = blob.span {
                write!(err, "; cut at {}..{}", span.start, span.end)?;
            }
            writeln!(err, "\n{}", checked.url)
        }

        Answer::DecisionHits(a) => {
            for hit in &a.data.hits {
                let mut row = format!("{}  {}", hit.id, hit.form);
                if let Some(court) = &hit.court {
                    let _ = write!(row, "  {court}");
                }
                if let Some(day) = hit.decided_on {
                    let _ = write!(row, "  {day}");
                }
                writeln!(out, "{row}")?;

                let units = if hit.paragraphs.from == hit.paragraphs.to {
                    format!("unit {}", hit.paragraphs.from)
                } else {
                    format!("units {} to {}", hit.paragraphs.from, hit.paragraphs.to)
                };
                writeln!(
                    out,
                    "  {units}  --offset {}  distance {:.4}",
                    hit.paragraphs.from.saturating_sub(1),
                    hit.distance
                )?;
            }

            let (cut, rest): (Vec<Caveat>, Vec<Caveat>) = a
                .caveats
                .iter()
                .cloned()
                .partition(|caveat| matches!(caveat, Caveat::Truncated { .. }));
            let mut notes: Vec<String> = cut
                .iter()
                .map(|caveat| match caveat {
                    Caveat::Truncated { window } => format!(
                        "note: the {window} nearest rulings shown; more rank below them, which a higher --limit answers"
                    ),
                    other => caveat_line(other),
                })
                .collect();
            notes.extend(rest.iter().map(caveat_line));
            if !notes.is_empty() {
                writeln!(err, "\n{}", notes.join("\n"))?;
            }
            Ok(())
        }

        Answer::ArticleDecisions {
            answered: a,
            offset,
        } => {
            let found = &a.data;
            let cellar = found.register == "cellar";

            writeln!(
                err,
                "{} {} found for article {} of {}",
                found.count,
                if cellar { "case(s)" } else { "ruling(s)" },
                found.number,
                found.regulation_id
            )?;
            court_counts(err, &found.courts)?;

            let court = |ruling: &CitingDecision| ruling.court.clone().unwrap_or_default();
            let court_width = found
                .decisions
                .iter()
                .map(|ruling| court(ruling).chars().count())
                .max()
                .unwrap_or(0);

            for ruling in &found.decisions {
                let day = ruling
                    .decided_on
                    .map(|day| day.to_string())
                    .unwrap_or_default();
                let mut row = format!(
                    "{day:<10}  {:<court_width$}  {}",
                    court(ruling),
                    ruling.ecli
                );
                if !ruling.held {
                    let _ = write!(row, "  {NOT_HELD}");
                }
                if let Some(rendition) = &ruling.rendition {
                    let _ = write!(row, "  version {rendition}");
                }
                let mut places: Vec<String> = Vec::new();
                for link in &ruling.links {
                    if let Some(place) = pinpoint(link.place.as_ref(), link.part.as_deref())
                        && !places.contains(&place)
                    {
                        places.push(place);
                    }
                }
                if !places.is_empty() {
                    let _ = write!(row, "  {}", places.join(", "));
                }
                writeln!(out, "{row}")?;
            }

            // `truncated` on a paged list says where the next page starts, and
            // `rulings_found` on an act of the EU names CELLAR rather than LiDO.
            let notes: Vec<String> = a
                .caveats
                .iter()
                .map(|caveat| match caveat {
                    Caveat::Truncated { window } => read_on(
                        if cellar { "cases" } else { "rulings" },
                        offset.as_deref(),
                        found.decisions.len(),
                        found.count,
                        *window,
                    ),
                    Caveat::RulingsFound if cellar => "note: these are the cases CELLAR states \
                         interpret or cite the article, as the Publications Office documents \
                         them. How many it misses is not measured, so a case missing here may \
                         still cite the article."
                        .to_string(),
                    other => caveat_line(other),
                })
                .collect();
            if !notes.is_empty() {
                writeln!(err, "\n{}", notes.join("\n"))?;
            }
            Ok(())
        }

        Answer::RecentDecisions {
            answered: a,
            offset,
        } => {
            let listed = &a.data;

            writeln!(
                err,
                "{} ruling(s) listed by rechtspraak's feed",
                listed.count
            )?;
            court_counts(err, &listed.courts)?;

            let court = |ruling: &ListedDecision| ruling.court.clone().unwrap_or_default();
            let court_width = listed
                .decisions
                .iter()
                .map(|ruling| court(ruling).chars().count())
                .max()
                .unwrap_or(0);

            for ruling in &listed.decisions {
                let day = ruling
                    .decided_on
                    .map(|day| day.to_string())
                    .unwrap_or_default();
                let mut row = format!(
                    "{day:<10}  {:<court_width$}  {}",
                    court(ruling),
                    ruling.ecli
                );
                if let Some(numbers) = &ruling.case_numbers {
                    let _ = write!(row, "  {numbers}");
                }
                if ruling.summary_left_out.as_deref() == Some("criminal_law") {
                    let _ = write!(row, "  criminal law");
                }
                // A withheld ruling is never fetched, and `not held` reads as
                // not yet.
                if ruling.summary_left_out.as_deref() == Some("privacy") {
                    let _ = write!(row, "  withheld");
                } else if !ruling.held {
                    let _ = write!(row, "  {NOT_HELD}");
                }
                writeln!(out, "{row}")?;
                if let Some(summary) = &ruling.summary {
                    writeln!(out, "    {summary}")?;
                }
            }

            let notes: Vec<String> = a
                .caveats
                .iter()
                .map(|caveat| match caveat {
                    Caveat::Truncated { window } => read_on(
                        "rulings",
                        offset.as_deref(),
                        listed.decisions.len(),
                        listed.count,
                        *window,
                    ),
                    other => caveat_line(other),
                })
                .collect();
            if !notes.is_empty() {
                writeln!(err, "\n{}", notes.join("\n"))?;
            }
            Ok(())
        }

        Answer::PublicationHits(a) => {
            for hit in &a.data.hits {
                let mut row = hit.id.clone();
                if !hit.held {
                    let _ = write!(row, "  {NOT_HELD}");
                }
                if let Some(day) = hit.published_on {
                    let _ = write!(row, "  {day}");
                }
                writeln!(out, "{row}")?;
                if let Some(title) = &hit.title {
                    writeln!(out, "  {title}")?;
                }
            }

            write!(err, "source_total {}", a.data.source_total)?;
            if let Some(next) = &a.data.next {
                write!(err, "  next {next}")?;
            }
            writeln!(err)?;

            caveats(&a.caveats, err)
        }

        Answer::Dossier(a) => {
            let dossier = &a.data;

            write!(err, "Kamerstukken {}", dossier.dossier)?;
            if let Some(title) = &dossier.title {
                write!(err, "  {title}")?;
            }
            writeln!(err)?;
            for act in &dossier.acts {
                let regulations: Vec<String> = act
                    .regulations
                    .iter()
                    .map(|r| match &r.title {
                        Some(title) => format!("{} {title}", r.regulation_id),
                        None => r.regulation_id.clone(),
                    })
                    .collect();
                writeln!(err, "  {}  {}", act.act, regulations.join(", "))?;
            }

            if dossier.papers.is_empty() {
                writeln!(
                    out,
                    "No paper of dossier {} answers as asked.",
                    dossier.dossier
                )?;
                return caveats(&a.caveats, out);
            }

            for paper in &dossier.papers {
                let mut row = paper.id.clone();
                if !paper.held {
                    let _ = write!(row, "  {NOT_HELD}");
                }
                if let Some(day) = paper.published_on {
                    let _ = write!(row, "  {day}");
                }
                let _ = write!(row, "  {}", paper.kind.as_deref().unwrap_or("-"));
                // Absent on a kind nobody votes on and where bron has not
                // asked the Tweede Kamer; empty where it states no besluit,
                // which is not a vote against.
                match paper.fates.as_deref() {
                    Some([]) => {
                        let _ = write!(row, "  no besluit stated");
                    }
                    Some(fates) => {
                        let decisions: Vec<&str> =
                            fates.iter().map(|f| f.decision.as_str()).collect();
                        let _ = write!(row, "  {}", decisions.join("; "));
                    }
                    None => {}
                }
                if let Some(later) = &paper.replaced_by {
                    let _ = write!(row, "  replaced by {later}");
                }
                writeln!(out, "{row}")?;
                if let Some(title) = &paper.title {
                    writeln!(out, "  {title}")?;
                }
                if let Some(citation) = &paper.citation {
                    writeln!(out, "  {citation}")?;
                }
            }

            caveats(&a.caveats, err)
        }
    }
}

/// What a question narrowed one unit to, named so a call site cannot pass
/// the lid where the item goes: both are text, and swapping them prints the
/// wrong cut of what is cited.
#[derive(Default)]
struct Narrowed<'a> {
    /// The lid the question named, so a heading says which lid where the
    /// service's label for the unit is absent and cannot.
    lid: Option<&'a str>,
    /// The item a letter opens; see [`crate::run::item`].
    item: Option<&'a str>,
}

/// One unit of text, as `law show` and `law parts` both print it: what it is,
/// its words, where those words stand in which document, and whether it
/// applies.
fn write_unit(
    unit: &Article,
    rendition: &Expression,
    service: &Service,
    narrowed: Narrowed<'_>,
    out: &mut impl Write,
) -> io::Result<()> {
    // The lawyer's minimum, ahead of the name and the words: a reader should
    // not have to reach the bottom of the block to learn the text above it
    // does not apply.
    if unit.withdrawn.is_some() {
        writeln!(out, "{WITHDRAWN}")?;
    } else if not_commenced(unit) {
        match unit.in_force_from {
            Some(day) => writeln!(out, "not yet applied, applies from {day}")?,
            None => writeln!(out, "not yet applied")?,
        }
    }
    match (&unit.label, narrowed.lid) {
        (None, Some(lid)) => write!(out, "{} lid {lid}", unit.name())?,
        _ => write!(out, "{}", unit.name())?,
    }
    if let Some(title) = &unit.title {
        write!(out, "  {title}")?;
    }
    // A treaty can print one number in several languages, and without this
    // two of its units read as the same heading.
    match (&unit.lang, &unit.text_status) {
        (Some(lang), Some(status)) => write!(out, "  ({lang}, {status})")?,
        (Some(lang), None) => write!(out, "  ({lang})")?,
        _ => {}
    }
    writeln!(out)?;
    if let Some(path) = &unit.path {
        writeln!(out, "{path}")?;
    }

    let text = unit.text();
    let text = match narrowed.item {
        Some(letter) => crate::run::item(&text, letter).map_or_else(
            || format!("This lid prints no item {letter}."),
            str::to_string,
        ),
        None => text,
    };

    // A repealed provision reads as one word, so the day it went is the only
    // thing on the line that answers anything. wetten.nl prints the same date
    // for the same reason.
    match (unit.effect.as_deref(), unit.in_force_from) {
        (Some("vervallen"), Some(day)) => writeln!(out, "\n{text} per {day}\n")?,
        _ => writeln!(out, "\n{text}\n")?,
    }

    if let Some(blob) = unit.blob.as_ref().or(rendition.blob.as_ref()) {
        writeln!(
            out,
            "source: {blob} ({}..{})",
            unit.span.start, unit.span.end
        )?;
        if let Some(url) = service.blob_url(blob) {
            writeln!(out, "{url}")?;
        }
    }
    // The register's own citation string for exactly what is printed above,
    // carrying this rendition's zicht and geldigheid dates. It is what Dutch
    // practice already cites with, so a reader needs no mapping from ours.
    if let Some(jci) = &unit.jci {
        writeln!(out, "{jci}")?;
    }
    if !unit.images.is_empty() {
        writeln!(out, "images: {}", unit.images.join(", "))?;
    }
    write_remarks(unit.remarks.as_deref().unwrap_or_default(), "", out)?;
    if let Some(act) = &unit.amended_by {
        let effect = unit.effect.as_deref().unwrap_or("wijziging");
        write!(out, "published by {act} ({effect})")?;
        if let Some(dossiers) = &unit.dossiers
            && !dossiers.is_empty()
        {
            let numbers = dossiers
                .iter()
                .map(|dossier| dossier.dossier.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            write!(out, ", dossier {numbers}")?;
        }
        writeln!(out)?;
    }
    // A withdrawn change was still published by its act; it never put
    // anything in force.
    if let Some(besluit) = &unit.commenced_by
        && !not_commenced(unit)
        && unit.withdrawn.is_none()
    {
        writeln!(out, "in force through {besluit}")?;
    }
    // bron's reading and not the publisher's, so each line says so and
    // carries the digest and span a reader checks it against.
    for passage in unit.passages.as_deref().unwrap_or_default() {
        let paper = passage.citation.as_deref().unwrap_or(&passage.paper);
        writeln!(
            out,
            "explained in {paper} (bron's reading): {} ({}..{})",
            passage.stated_in, passage.span.start, passage.span.end
        )?;
        if let Some(through) = &passage.through {
            writeln!(
                out,
                "  through the bill {}: {} ({}..{})",
                through.paper, through.stated_in, through.span.start, through.span.end
            )?;
        }
        if let Some(earlier) = &passage.earlier_text {
            writeln!(
                out,
                "  explains the bill's words, which {} prints otherwise",
                earlier.publication
            )?;
        }
    }
    for change in unit.amendments.as_deref().unwrap_or_default() {
        let paper = change.citation.as_deref().unwrap_or(&change.paper);
        writeln!(
            out,
            "changed by amendement {paper} (bron's reading): {} ({}..{})",
            change.stated_in, change.span.start, change.span.end
        )?;
        if let Some(through) = &change.through {
            writeln!(
                out,
                "  through the bill {}: {} ({}..{})",
                through.paper, through.stated_in, through.span.start, through.span.end
            )?;
        }
        match change.in_act {
            Some(true) => writeln!(out, "  the act prints every word it puts in")?,
            Some(false) => writeln!(out, "  the act does not print every word it puts in")?,
            None => {}
        }
        let decided = change.decided_on.as_deref().unwrap_or(&change.paper);
        match change.fates.as_deref() {
            Some([]) => writeln!(out, "  no besluit stated on {decided}")?,
            Some(fates) => writeln!(
                out,
                "  besluit on {decided}: {}",
                fates
                    .iter()
                    .map(|fate| fate.decision.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )?,
            None => {}
        }
    }
    writeln!(out)
}

/// The line an answer about units of text opens with.
///
/// An answer holding only provisions not yet applied is bound to the rendition
/// that publishes them, and 'as in force on' would say they apply.
fn header(
    units: &[Article],
    as_of: Option<NaiveDate>,
    as_known_on: Option<NaiveDate>,
) -> Option<String> {
    if !units.is_empty() && units.iter().all(not_commenced) {
        return Some(match as_of {
            Some(day) => {
                format!("published in the rendition current on {day}, and not yet applied")
            }
            None => "published and not yet applied".to_string(),
        });
    }

    bound(as_of, as_known_on)
}

/// The line an answer about a rendition's text opens with, and under it which
/// rendition that was: a date resolves to one, and a reader told only the date
/// is not told which text answered.
fn answered_by(
    header: Option<&str>,
    rendition: &Expression,
    out: &mut impl Write,
) -> io::Result<()> {
    if let Some(header) = header {
        writeln!(out, "{header}")?;
    }
    let mut named = rendition.label.clone();
    if let Some(language) = &rendition.language {
        let _ = write!(named, " ({language})");
    }
    writeln!(
        out,
        "from rendition {named}, in force {}\n",
        window(rendition.in_force.from, rendition.in_force.until)
    )
}

/// Writes parts as an outline, two spaces a level. A run of articles is one
/// `art.` line of their numbers, because a code of 1000 articles on a line
/// each buries its structure; a part between two articles ends the run, so
/// document order holds. Sets `starred` where a repealed article was marked.
fn outline(
    parts: &[ContentsPart],
    depth: usize,
    out: &mut impl Write,
    starred: &mut bool,
) -> io::Result<()> {
    let indent = "  ".repeat(depth);
    let mut run = Vec::new();

    for part in parts {
        if let ("artikel" | "enig-artikel", Some(number)) = (part.kind.as_str(), &part.number) {
            *starred |= part.repealed;
            run.push(if part.repealed {
                format!("{number}*")
            } else {
                number.clone()
            });
            continue;
        }

        article_run(&std::mem::take(&mut run), &indent, out)?;

        let mut line = part_heading(part);
        if part.repealed {
            line.push_str(" (repealed)");
        }
        writeln!(out, "{indent}{line}")?;
        outline(&part.children, depth + 1, out, starred)?;
    }

    article_run(&run, &indent, out)
}

/// `label. title`, or what of it the register prints.
fn part_heading(part: &ContentsPart) -> String {
    let name = match (&part.label, &part.number) {
        (Some(label), _) => Some(label.clone()),
        (None, Some(number)) => Some(format!("{} {number}", part.kind)),
        (None, None) => None,
    };

    match (name, &part.title) {
        (Some(name), Some(title)) => format!("{name}. {title}"),
        (Some(name), None) => name,
        (None, Some(title)) => title.clone(),
        (None, None) => part.kind.clone(),
    }
}

/// The width an `art.` line wraps at. Fixed rather than read off the terminal,
/// so the same answer prints the same lines wherever it is piped.
const OUTLINE_WIDTH: usize = 80;

/// One `art.` line, wrapped at [`OUTLINE_WIDTH`] with every continuation
/// starting under the first number.
fn article_run(numbers: &[String], indent: &str, out: &mut impl Write) -> io::Result<()> {
    if numbers.is_empty() {
        return Ok(());
    }

    let hang = indent.len() + "art.".len();
    let mut line = format!("{indent}art.");
    for number in numbers {
        let width = line.chars().count();
        if width > hang && width + 1 + number.chars().count() > OUTLINE_WIDTH {
            writeln!(out, "{line}")?;
            line = " ".repeat(hang);
        }
        line.push(' ');
        line.push_str(number);
    }
    writeln!(out, "{line}")
}

/// One rendition as a row, the same whichever command lists it. `law published`
/// lists renditions of many regulations and names each; `law versions` lists
/// one regulation's and does not.
///
/// Both windows, because they diverge wherever something was published with
/// retroactive effect; `-` where the register states no zicht dates.
fn rendition_row(rendition: &Expression, regulation: Option<&str>) -> String {
    let held = if rendition.held { "*" } else { " " };
    // The service states images for held text only, so of a rendition it does
    // not hold the count is unknown, not zero.
    let images = if rendition.held {
        format!("{} images", rendition.images.len())
    } else {
        String::new()
    };

    let mut row = format!("{held} ");
    if let Some(regulation) = regulation {
        let _ = write!(row, "{regulation}  ");
    }
    let _ = write!(
        row,
        "{}  {:<WINDOW$}  {:<WINDOW$}  {:<HASH_CHECK$}  {images:<IMAGES$}",
        rendition.label,
        window(rendition.in_force.from, rendition.in_force.until),
        stated(rendition.as_known),
        rendition.upstream_hash_check,
    );
    if rendition.withdrawn {
        let _ = write!(row, "  withdrawn");
    }
    if let Some(because) = &rendition.unread_because {
        let _ = write!(row, "  unread: {because}");
    }
    if let Some(title) = rendition
        .regulation_title
        .as_ref()
        .or(rendition.title.as_ref())
    {
        let _ = write!(row, "  {title}");
    }
    row.trim_end().to_string()
}

/// What the columns of [`rendition_row`] mean, on stderr beside the rows.
fn rendition_legend(renditions: &[Expression], err: &mut impl Write) -> io::Result<()> {
    writeln!(err, "* = the service holds this text")?;
    writeln!(
        err,
        "first window = in force (geldigheid), second = shown by the register (zicht), \
         - = the register states none"
    )?;
    writeln!(
        err,
        "matches, differs, rewritten, absent, unchecked = what the publisher's hash says \
         of the held bytes"
    )?;
    if renditions.iter().any(|r| r.withdrawn) {
        writeln!(
            err,
            "withdrawn = the publisher withdrew it, and no date resolves to it"
        )?;
    }
    if renditions.iter().any(|r| r.unread_because.is_some()) {
        writeln!(
            err,
            "unread = held and not parsed into articles, so it has none to answer"
        )?;
    }
    Ok(())
}

/// Which unit a text was found as: a numbered lid, a lid the register numbers
/// not, or the whole article.
/// The other places the text above sits, each named as a hit is.
///
/// No distance and no finder: neither is stated of another place, only of
/// the text that ranked.
fn write_places(places: &[Place], out: &mut impl Write) -> io::Result<()> {
    for place in places {
        let place_unit = unit_of(&place.unit, place.lid.as_ref());
        let place_name = name(
            place.label.as_deref(),
            place.number.as_deref(),
            &place.kind,
            None,
        );
        let row = HitRow {
            source: "also",
            unit: &place_unit,
            regulation: &place.regulation_id,
            expression: &place.expression,
            name: &place_name,
            ..HitRow::default()
        };
        writeln!(out, "{}", row.line())?;
    }
    Ok(())
}

fn unit_of(unit: &str, lid: Option<&SearchLid>) -> String {
    match lid.and_then(|lid| lid.number.as_ref()) {
        Some(number) => format!("lid {number}"),
        None if unit == "lid" => "lid".to_string(),
        None => "article".to_string(),
    }
}

/// A window, or `-` where none is stated.
fn stated(bounds: Option<bron::service::Window>) -> String {
    bounds.map_or_else(|| "-".to_string(), |w| window(w.from, w.until))
}

/// Where a unit stands in its rendition's document.
fn span(side: &Article) -> String {
    format!("{}..{}", side.span.start, side.span.end)
}

/// One row of a ranking: a hit, or another place its words sit, in one column
/// order so a reader comparing the two compares like with like. Named fields,
/// because two of them read as identifiers and a swap would still look
/// plausible, only in the wrong columns.
#[derive(Default)]
struct HitRow<'a> {
    distance: &'a str,
    source: &'a str,
    unit: &'a str,
    regulation: &'a str,
    expression: &'a str,
    found_by: &'a str,
    name: &'a str,
}

impl HitRow<'_> {
    fn line(&self) -> String {
        let Self {
            distance,
            source,
            unit,
            regulation,
            expression,
            found_by,
            name,
        } = self;
        format!(
            "{distance:>5}  {source:<11} {unit:<8} {regulation}  {expression}  {found_by:<FOUND_BY$} {name}"
        )
        .trim_end()
        .to_string()
    }
}

/// Shows what changed inside an amended article.
///
/// The form follows the size of the change, because one form cannot serve both.
/// A few words altered in a long sentence is best read in place, marked
/// `[-was-]` and `{+is+}`. An article that was rewritten is not: marking it up
/// word by word produces noise that hides rather than shows, so both texts are
/// given whole and the reader compares them.
///
/// Rendering only. The JSON carries both full texts either way, so a consumer
/// runs its own comparison and never has to trust this one.
fn word_diff(before: &str, after: &str) -> Vec<String> {
    use similar::{ChangeTag, TextDiff};

    /// Below this similarity the article is treated as rewritten.
    const REWRITTEN: f32 = 0.6;
    /// Words of unchanged text kept either side of a change.
    const CONTEXT: usize = 8;

    let diff = TextDiff::from_words(before, after);

    if diff.ratio() < REWRITTEN {
        return vec![
            "rewritten, both texts follow".to_string(),
            format!("- {}", collapse(before)),
            format!("+ {}", collapse(after)),
        ];
    }

    // similar emits whitespace as its own token, so consecutive changes are
    // gathered into runs before anything is marked up. Marking each token
    // separately yields a wall of empty {++} markers.
    let mut runs: Vec<(ChangeTag, String)> = Vec::new();
    for change in diff.iter_all_changes() {
        match runs.last_mut() {
            Some((tag, text)) if *tag == change.tag() => text.push_str(change.value()),
            _ => runs.push((change.tag(), change.value().to_string())),
        }
    }

    let mut out = String::new();
    for (i, (tag, text)) in runs.iter().enumerate() {
        match tag {
            ChangeTag::Delete => out.push_str(&format!(" [-{}-] ", text.trim())),
            ChangeTag::Insert => out.push_str(&format!(" {{+{}+}} ", text.trim())),
            ChangeTag::Equal => {
                let words: Vec<&str> = text.split_whitespace().collect();
                let first = i == 0;
                let last = i == runs.len() - 1;

                if words.len() <= CONTEXT * 2 {
                    out.push_str(text);
                } else if first {
                    out.push_str(&format!(
                        "[...] {}",
                        words[words.len() - CONTEXT..].join(" ")
                    ));
                } else if last {
                    out.push_str(&format!("{} [...]", words[..CONTEXT].join(" ")));
                } else {
                    out.push_str(&format!(
                        "{} [...] {}",
                        words[..CONTEXT].join(" "),
                        words[words.len() - CONTEXT..].join(" ")
                    ));
                }
            }
        }
    }

    let rendered = collapse(&out);
    if rendered.is_empty() {
        vec!["(whitespace only)".to_string()]
    } else {
        vec![rendered]
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    if line.chars().count() > 100 {
        format!("{}...", line.chars().take(100).collect::<String>())
    } else {
        line.to_string()
    }
}

/// Whether the register publishes this provision and does not yet apply it.
///
/// Not where its own remark says the change will not take effect: the
/// register prints it `nogniet` all the same, and 'not yet' would promise a
/// day it has cancelled.
fn not_commenced(article: &Article) -> bool {
    article.status.as_deref() == Some("nogniet") && article.withdrawn.is_none()
}

/// What a person reads for a unit whose remark says it will not take effect,
/// in place of when it applies.
const WITHDRAWN: &str = "the register states this change will not take effect";

/// Each remark the register prints on a unit, verbatim, at its byte range in
/// the document `source:` names.
fn write_remarks(remarks: &[Remark], indent: &str, out: &mut impl Write) -> io::Result<()> {
    for remark in remarks {
        writeln!(
            out,
            "{indent}register's remark ({}..{}): {}",
            remark.span.start, remark.span.end, remark.text
        )?;
    }
    Ok(())
}

/// The moment an answer is bound to, where it is bound to one.
fn bound(as_of: Option<NaiveDate>, as_known_on: Option<NaiveDate>) -> Option<String> {
    match (as_of, as_known_on) {
        (Some(day), Some(read_on)) => Some(format!(
            "as in force on {day}, as the register showed on {read_on}"
        )),
        (Some(day), None) => Some(format!("as in force on {day}")),
        (None, Some(read_on)) => Some(format!("as the register showed on {read_on}")),
        (None, None) => None,
    }
}

/// How many decimal digits `n` prints as.
fn digits(n: u32) -> usize {
    n.checked_ilog10().map_or(1, |log| log as usize + 1)
}

fn window(from: NaiveDate, until: Option<NaiveDate>) -> String {
    match until {
        Some(until) => format!("{from} to {until}"),
        None => format!("{from} onwards"),
    }
}

fn caveats(caveats: &[Caveat], out: &mut impl Write) -> io::Result<()> {
    if caveats.is_empty() {
        return Ok(());
    }

    writeln!(out)?;
    for caveat in caveats {
        writeln!(out, "{}", caveat_line(caveat))?;
    }
    Ok(())
}

fn court_counts(err: &mut impl Write, courts: &[CourtCount]) -> io::Result<()> {
    let width = courts
        .iter()
        .map(|court| court.count.to_string().len())
        .max()
        .unwrap_or(0);
    for court in courts {
        writeln!(err, "  {:>width$}  {}", court.count, court.court)?;
    }
    Ok(())
}

/// A place as a reader cites it, `art. 8(2)(e)`, `recital 26` or `point 81`,
/// else CELLAR's fragment as it writes it.
fn pinpoint(place: Option<&Pinpoint>, part: Option<&str>) -> Option<String> {
    let read = place.and_then(|place| {
        if let Some(article) = &place.article {
            let mut cited = format!("art. {article}");
            for sub in [&place.paragraph, &place.point].into_iter().flatten() {
                let _ = write!(cited, "({sub})");
            }
            Some(cited)
        } else if let Some(recital) = &place.recital {
            Some(format!("recital {recital}"))
        } else {
            place.point.as_ref().map(|point| format!("point {point}"))
        }
    });

    read.or_else(|| part.map(str::to_string))
}

/// Where the next page of a list of rulings starts, from the `truncated`
/// caveat's window.
fn read_on(noun: &str, offset: Option<&str>, shown: usize, count: u64, window: u32) -> String {
    // The service accepted the offset, so it is a whole number.
    let from = offset
        .and_then(|typed| typed.parse::<u64>().ok())
        .unwrap_or(0);
    format!(
        "note: {noun} {} to {} of {count} shown; read on with --offset {}",
        from + 1,
        from + shown as u64,
        from + u64::from(window)
    )
}

/// One limit, in words.
///
/// Separate from the loop above because a limit is not always the answer's:
/// a citation read two ways carries one per reading, and those belong under
/// the line they qualify rather than in a block at the end.
fn caveat_line(caveat: &Caveat) -> String {
    match caveat {
        Caveat::Truncated { window } => {
            format!("note: filtered over a window of {window}, matches may be missing")
        }
        Caveat::SkippedArticles { count } => {
            format!("note: {count} article(s) yielded no text and were skipped")
        }
        Caveat::AmbiguousArticle { number, paths } => format!(
            "note: {} articles in this law are numbered {number}; their paths tell them \
                 apart:\n      {}",
            paths.len(),
            paths.join("\n      ")
        ),
        Caveat::TableFlattened {
            articles,
            cell_separator,
        } => format!(
            "note: {} print(s) a table. Its rows are one line each with cells separated by \
                 [{}], so a value belongs to the heading in the same position on the row above \
                 rather than to the line it follows.",
            articles.join(", "),
            cell_separator.trim(),
        ),
        Caveat::CursorAhead { asked, head } => format!(
            "note: cursor {asked} is past everything the service holds, which ends at {head}. \
                 That is not a quiet register: the cursor belongs to another deployment, or to \
                 this one before it was rebuilt. Start again from the head."
        ),
        Caveat::UpstreamHashDiffers => {
            "note: the bytes behind this answer do not hash to the SHA-512 the publisher's \
                 manifest states for them. The register is inconsistent about what it hashes, \
                 so this is upstream disagreeing with itself rather than the text being wrong, \
                 and it cannot be settled from here."
                .to_string()
        }
        Caveat::UpstreamHashRewritten => {
            "note: the register rewrote this rendition after bron read it, and this is the \
                 earlier text. It stays the answer until bron reads the rendition again."
                .to_string()
        }
        Caveat::UpstreamHashAbsent => {
            "note: the manifest published no hash for this rendition, so these bytes rest \
                 on bron's own capture alone"
                .to_string()
        }
        Caveat::SeveralReadings {
            readings,
            regulations,
        } => {
            let mut line = format!(
                "note: this citation reads more than one way and every reading is answered: {}.",
                readings.join(", "),
            );
            if !regulations.is_empty() {
                let _ = write!(
                    line,
                    " The register gives the name to more than one regulation: {}.",
                    regulations.join(", "),
                );
            }
            line.push_str(" Which was meant is yours to say.");
            line
        }
        Caveat::ReadingUnresolved {
            reading,
            because,
            regulation_id,
            renditions,
        } => {
            let mut line = format!("note: the reading {reading} came to nothing ({because})");
            if let Some(id) = regulation_id {
                line.push_str(&format!(", for {id}"));
            }
            if !renditions.is_empty() {
                line.push_str(&format!(": {}", renditions.join(", ")));
            }
            line.push_str(
                ". The other reading is answered above; this one was looked for and is not.",
            );
            line
        }
        Caveat::HeldTextOnly {
            held_regulations,
            known_regulations,
        } => format!(
            "note: this is drawn from the {held_regulations} regulation(s) whose text the \
                 corpus holds, out of {known_regulations} it knows of. What the other \
                 {} would have said is not in this answer.",
            known_regulations.saturating_sub(*held_regulations),
        ),
        Caveat::LanesUnsearched { lanes } => format!(
            "note: {} hold no vectors and were not searched, so this answer covers less \
                 than the question did",
            lanes.join(" and "),
        ),
        Caveat::UnrankableTexts { scope, count } => format!(
            "note: {count} text(s) in the {scope} carry no vector, because the model will not \
             take them in one input, so no ranking reaches them. They are answered by \
             identifier.",
        ),
        // The words rather than a count: which words went unsearched is what
        // says whether to ask again in other words.
        Caveat::WordsUnsearched { words } => format!(
            "note: full text did not search {}: too common, or past the words it takes. The \
             ranking read the whole question, so a hit found only by words may be missing.",
            words.join(", "),
        ),
        Caveat::PlacesRead {
            bound,
            content_hashes,
        } => format!(
            "note: places for text(s) {} were read up to {bound} in the article or lid list. \
             Their places counts are lower bounds; the places named are first among those \
             read, not necessarily first across all places.",
            content_hashes.join(", "),
        ),
        Caveat::IndexBehind { scope, count } => format!(
            "note: the search index is behind on {count} rendition(s) for the {scope} asked. \
             Their texts rank as the index last held them, or not at all."
        ),
        // The text answered is the authentic one, which is the text that binds.
        // The translation is named rather than silently passed over.
        Caveat::TranslationLeftOut {
            number,
            lang,
            paths,
        } => format!(
            "note: the treaty prints {number} in its authentic text and in a translation ({}). \
             The authentic text is answered; the translation stands at {}.",
            lang.join(", "),
            paths.join(", "),
        ),
        Caveat::RankingShort { asked, reached } => format!(
            "note: the ranking reached {reached} of the {asked} nearest texts it asked for, and \
             may have stopped at a bound before finding more, so texts that would rank may be \
             missing."
        ),
        Caveat::CountsNotTaken { caveats } => {
            // A phrase for the kinds the service names today; a kind it adds
            // later is named by its own key rather than dropped.
            const KNOWN: [(&str, &str); 2] = [
                ("unrankable_texts", "texts no ranking reaches"),
                ("texts_unindexed", "texts too long to index in full text"),
            ];
            let named: Vec<&str> = caveats
                .iter()
                .map(|key| {
                    KNOWN
                        .iter()
                        .find(|(known, _)| known == key)
                        .map_or(key.as_str(), |(_, phrase)| phrase)
                })
                .collect();
            format!(
                "note: this search did not count the {}, so none being reported says nothing \
                 about whether the corpus holds any.",
                named.join(" or "),
            )
        }
        Caveat::TextsUnindexed { scope, count } => format!(
            "note: {count} text(s) in the {scope} are too long to index for full text, so no \
             word in them finds them. They are answered by identifier.",
        ),
        Caveat::NotCommenced { articles } => format!(
            "note: article(s) {} have not commenced and do not apply yet. Each is shown as \
                 the rendition prints it, the stub or the settled text, with the day it applies \
                 where the register states one.",
            articles.join(", "),
        ),
        Caveat::IllustrationOmitted { articles } => format!(
            "note: {} print(s) an image, which the text does not place. Where the image is a \
                 formula, the text states what its variables mean and not the formula itself. \
                 The rendition names the digest each image is held under: law versions --json.",
            articles.join(", "),
        ),
        Caveat::Retroactive {
            in_force_from,
            first_visible,
        } => format!(
            "note: this text applies from {in_force_from} but was only visible from \
                 {first_visible}"
        ),
        // The reason this lane goes through the service. A withdrawn
        // rendition keeps its dates, so it wins an ordering by date and is
        // still not what the register says applies.
        Caveat::WithdrawnRenditions { excluded } => format!(
            "note: the publisher has withdrawn {}, which cover(s) this date and were \
                 passed over. This answer is the newest rendition the register still \
                 publishes for it.",
            excluded.join(", "),
        ),
        // Deliberately does not say which one to use. That is transitional
        // law, and naming a winner here would be a legal judgement wearing
        // a lookup's clothes.
        Caveat::AmbiguousRendition { chosen, discarded } => format!(
            "note: more than one published rendition covers this date. This is {chosen}, \
                 which commenced last; {} also cover(s) it. Which governs is a question of \
                 transitional law that bron does not answer.",
            discarded.join(", "),
        ),
        Caveat::SupersededKnowledge { visible_until } => format!(
            "note: the register has since revised what it shows for this date; this text \
                 was what it showed until {visible_until}"
        ),
        // Said because the counts above cannot say it. A unit left out of a
        // comparison is left out of its totals too, so silence here reads as
        // 'nothing changed'.
        Caveat::ArticlesNotCompared { repeated } => format!(
            "note: not every unit could be compared. The register used stam-id {} more than \
                 once inside one rendition, and picking one of those would be a guess. They are \
                 outside the counts above.",
            repeated.join(", "),
        ),
        Caveat::CitationNotRead {
            number,
            name,
            because,
        } => format!(
            "note: {number} {name} reads as a citation, and the register gives no regulation \
                 the name {name} ({because}). These are the nearest texts to it, not the article \
                 cited."
        ),
        Caveat::RenditionsNotHeld { count, renditions } => format!(
            "note: the service does not hold the text of {count} of the {renditions} \
                 rendition(s) listed"
        ),
        Caveat::RenditionsUnread { count, renditions } => format!(
            "note: {count} of the {renditions} rendition(s) listed are held and were not parsed \
                 into articles, so they have no article to answer"
        ),
        Caveat::ReadForArticles { renditions } => format!(
            "note: rendition(s) {} were parsed for articles alone. This answer covers their \
             articles and no other units.",
            renditions.join(", "),
        ),
        Caveat::TreatiesLeftOut { count } => format!(
            "note: {count} treaty(ies) print units as not yet in force and are not in this list. \
             What a treaty prints so is no announcement: some treaties never enter into force."
        ),
        Caveat::NotInBwb => "note: this list holds what the BWB prints, and the acts the \
                             Staatsblad published that the BWB does not print, as far as bron \
                             holds the Staatsblad. An act published before that and never printed \
                             in the BWB, the kinds of act counted as left out, and EU regulations \
                             are not in it."
            .to_string(),
        Caveat::ProvenanceOfArticle { articles, fields } => format!(
            "note: these fields describe article(s) {}, not the lid: {}. The register records \
             provenance per article.",
            articles.join(", "),
            fields.join(", "),
        ),
        Caveat::MachineReading { articles, readings } => format!(
            "note: the passages and amendementen shown for article(s) {} are bron's reading of \
             the parliamentary papers' headings, the amendementen's places and the bill's \
             formulas (reading {}), which no publisher states; a heading may number an article \
             as the bill did, which the act can have given to another. Each is quoted by digest \
             and span, so it can be checked against the paper.",
            articles.join(", "),
            readings
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        Caveat::PassageNotRead { articles } => format!(
            "note: a toelichting of the dossier behind article(s) {} was read and gives it no \
             passage of its own, which is not that the legislator said nothing about it.",
            articles.join(", "),
        ),
        Caveat::ExplainsEarlierText { articles } => format!(
            "note: a passage for article(s) {} explains the bill's words at a place the act \
             prints otherwise, so it explains a text that is not the one in force.",
            articles.join(", "),
        ),
        Caveat::UnitsWithdrawnByRemark { count } => format!(
            "note: {count} unit(s) the rendition prints as not yet in force are left out, because \
             the register's own remark on each says the change will not take effect."
        ),
        // Every element, the aanhef too: the count is the service's, and naming
        // only some of it would be bron deciding which text matters.
        Caveat::TextOutsideArticles { expression, units } => format!(
            "note: rendition {expression} prints words that no unit holds: {}. No answer \
                 covers them and no comparison compared them.",
            units
                .iter()
                .map(|(unit, count)| format!("{unit} ({count})"))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        Caveat::ActsLeftOut { counts } => {
            // Fixed order and phrase for the kinds the service names today;
            // a kind it adds later is named by its own key rather than
            // dropped.
            const KNOWN: [(&str, &str, &str); 3] = [
                ("budget", "budget act", "budget acts"),
                ("treaty_approval", "treaty approval", "treaty approvals"),
                (
                    "constitution_first_reading",
                    "first reading of a constitutional amendment",
                    "first readings of a constitutional amendment",
                ),
            ];
            let mut named: Vec<String> = KNOWN
                .iter()
                .filter_map(|(key, one, many)| {
                    counts.get(*key).map(|count| {
                        let word = if *count == 1 { one } else { many };
                        format!("{count} {word}")
                    })
                })
                .collect();
            for (key, count) in counts {
                if !KNOWN.iter().any(|(known, _, _)| known == key) {
                    named.push(format!("{count} {key}"));
                }
            }
            format!("note: left out of the published acts: {}", named.join(", "))
        }
        Caveat::FoundingActsUnread { regulations } => format!(
            "note: {regulations} regulation(s) have not had their founding act read, so an act \
             that founded one of them may be listed as published although the BWB prints it",
        ),
        Caveat::DecreesUnmatched { unlinked, decrees } => {
            let mut note = format!(
                "note: {unlinked} held besluit(en) name no act by reference or by chain, so bron \
                 cannot say what they commence."
            );
            if !decrees.is_empty() {
                let _ = write!(
                    note,
                    " {} of them published in the last 15 days: {}.",
                    decrees.len(),
                    decrees
                        .iter()
                        .map(|d| format!("{} ({})", d.id, d.published_on))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            note
        }
        Caveat::CommencementPending { decrees } => format!(
            "note: a published besluit names the act behind a waiting unit, and no rendition \
             here names it as commenced yet. Whether its day has come stands in its own words, \
             which bron does not read: {}.",
            decrees
                .iter()
                .map(|d| format!(
                    "{} commences {}, published {}",
                    d.decree, d.act, d.published_on
                ))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        Caveat::PublicationNotHeld => "note: this publication was passed on rather than kept; \
             its digest is of the bytes bron read, and not one /blobs can serve again"
            .to_string(),
        Caveat::SourceTotalEstimated => "note: the publisher's own total is an estimate, so \
             walking this search to its end can read a few hits more or fewer than it stated"
            .to_string(),
        Caveat::PapersBeforeDigital => "note: every act naming this dossier is from before \
             1995, so its papers are scans in Staten-Generaal Digitaal, which bron does not read"
            .to_string(),
        Caveat::FateUnknown { papers } => format!(
            "note: {papers} amendement(en) or motie(s) here have no besluit stated in the Tweede \
             Kamer's open data: not put to a vote, withdrawn, from before 2008 which it holds \
             nothing about, or a paper the open data files under more than one dossier, which \
             bron does not read. No besluit is not 'verworpen'."
        ),
        Caveat::FatesNotRead { papers } => format!(
            "note: bron has not yet asked the Tweede Kamer about {papers} amendement(en) or \
             motie(s) here, so no besluit is shown for them."
        ),
        Caveat::PapersNotHeld { count, papers } => format!(
            "note: {count} of the {papers} paper(s) the publisher lists for this dossier are not \
             held and are left out while they are read."
        ),
        Caveat::PublicationsLeftOut { count } => format!(
            "note: {count} hit(s) on this page are of a kind bron does not open and are left \
             out; the publisher's next page may still hold more."
        ),
        Caveat::SearchUpcoming { count } => format!(
            "note: search over case law is still to come. This ranked the {count} ruling(s) \
             bron has read, not all case law: a ruling missing here may still exist."
        ),
        Caveat::LinksNotRead => "note: LiDO was not asked yet or did not answer what this ruling \
             cites, so the articles and rulings it cites are missing, and their absence says nothing"
            .to_string(),
        Caveat::LinksUnresolved { count } => format!(
            "note: LiDO named {count} article(s) this ruling cites that bron cannot place in any \
             text it holds, so they are left out"
        ),
        Caveat::RulingsFound => "note: these are the rulings LiDO found, by its computer's \
             reading and the publisher's own Wetsverwijzingen. How many it misses or names \
             wrongly is not measured, so a ruling missing here may still cite the article."
            .to_string(),
        Caveat::RulingsListed => "note: these are the rulings with a text whose record \
             rechtspraak's feed listed or changed from 30 days before bron first read it. A \
             conclusie, a ruling published as metadata alone and one the feed last listed before \
             then are not listed, so a ruling missing here may still have been decided."
            .to_string(),
        // The site's sentences, from the service's lang/en/search.php, whose
        // wording goes past the lawyer project before it changes.
        Caveat::ConsolidationNotAuthentic { act } => format!(
            "note: this is a consolidated text by the Publications Office of the European Union, \
             for information only and not legally binding. The authentic text is that of {act} in \
             the Official Journal, with the acts that amended it and the corrigenda."
        ),
        Caveat::WindowEndInferred { next } => format!(
            "note: the EU states no end to this text. bron runs it to the day before {next} takes \
             effect. An amendment the Publications Office has not yet consolidated, bron does not \
             see: check that the acts amending the text are in it."
        ),
        Caveat::WindowStartFromMetadata { day } => format!(
            "note: that this text entered into force on {day} is stated as a date in the EU's \
             metadata; the act itself gives the rule. An act can apply later than it enters into \
             force: read its final provisions."
        ),
        Caveat::StructureReadFromHtml { .. } => "note: CELLAR delivers this old text as a web \
             page alone; the Official Journal is the authentic text. bron read its articles, \
             paragraphs and annexes from the headings on that page, so check in the Official \
             Journal where an article ends."
            .to_string(),
        Caveat::ConsolidationsUnread { consolidations } => format!(
            "note: after this text the Publications Office made a further consolidated version \
             that already reflects the day asked, which bron cannot read: CELLAR does not deliver \
             it in Dutch or English as Formex or HTML. {}. The text that day can read otherwise; \
             read that version in EUR-Lex.",
            consolidations.join(", "),
        ),
        Caveat::NotYetInForce { in_force_from } => format!(
            "note: on the day asked this text was adopted and not yet in force: it entered into \
             force on {in_force_from}."
        ),
        Caveat::DirectiveBindsStates => "note: a directive binds the Member States: conduct in \
             the Netherlands is governed by the law that transposes it."
            .to_string(),
        Caveat::RelationsNotRead => "note: CELLAR states which provisions a case interprets and \
             which rulings it cites. bron has not read that for this case yet, so an empty list \
             does not mean there are none."
            .to_string(),
        // Never `because`, as on the withheld line of `case show`.
        Caveat::TextWithheld { .. } => "note: bron keeps the digests of what it held of this \
             text and none of its bytes, and answers none of it."
            .to_string(),
        Caveat::Unrecognised { stated } => {
            format!("note: the service stated a limit this bron has no sentence for: {stated}")
        }
        Caveat::NoZichtWindow => {
            "note: the register states no zicht dates for this text, so this question cannot be \
                 answered against that window"
                .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bron::service::{Answered, Cited};

    use crate::run::Answer;

    fn cited(
        reading: &str,
        held: bool,
        label: Option<&str>,
        caveats: Vec<serde_json::Value>,
    ) -> Cited {
        Cited {
            reading: reading.into(),
            source: "wetgeving".into(),
            regulation_id: "BWBR0005291".into(),
            expression: "2025-07-01_0".into(),
            held,
            number: "305a".into(),
            label: label.map(str::to_string),
            path: None,
            content_hash: held.then(|| "8fc1e1eb".into()),
            caveats,
        }
    }

    fn rendered(resolved: Vec<Cited>) -> String {
        let answer = Answer::Found(Answered {
            data: SearchData::Lookup {
                citation: "3:305a".into(),
                paragraph: None,
                name: None,
                resolved,
            },
            caveats: Vec::new(),
            as_of: None,
            as_known_on: None,
            envelope: String::new(),
        });

        let mut out = Vec::new();
        answer.render(&mut out, &mut io::sink()).unwrap();
        String::from_utf8(out).unwrap()
    }

    /// A limit on one reading is written under that reading, and it stays under
    /// it. Counting the indent out by hand would leave two numbers that have to
    /// agree and nothing making them.
    #[test]
    fn a_caveat_starts_where_the_line_it_qualifies_starts() {
        let text = rendered(vec![cited(
            "book",
            true,
            Some("Artikel 305a"),
            vec![serde_json::json!({"kind": "upstream_hash_absent"})],
        )]);

        let entry = text.lines().find(|l| l.starts_with("book")).unwrap();
        let note = text
            .lines()
            .find(|l| l.trim_start().starts_with("note:"))
            .unwrap();

        let indent = note.len() - note.trim_start().len();
        let second = entry.find("wetgeving").unwrap();

        assert_eq!(indent, second, "{text}");
    }

    /// The note is about the law and never about the answer. It is written
    /// from the caveat alone, and `--paragraph` can leave out an article whose
    /// path it names, so a word on what is shown would be false of that answer.
    #[test]
    fn the_note_on_a_repeated_number_claims_nothing_about_the_answer() {
        let note = caveat_line(&Caveat::AmbiguousArticle {
            number: "1".into(),
            paths: vec!["/Bijlage2/Artikel1".into(), "/Bijlage3/Artikel1".into()],
        });

        assert_eq!(
            note,
            "note: 2 articles in this law are numbered 1; their paths tell them apart:\n      \
             /Bijlage2/Artikel1\n      /Bijlage3/Artikel1"
        );
    }

    /// Every unit counted is named, the aanhef with the rest: the caveat is
    /// raised for the others, and the reader is owed the whole count.
    #[test]
    fn text_outside_articles_names_the_rendition_and_every_unit() {
        let note = caveat_line(&Caveat::TextOutsideArticles {
            expression: "2026-09-12_0".into(),
            units: [("aanhef".to_string(), 1), ("bijlage".to_string(), 10)].into(),
        });

        assert!(note.contains("2026-09-12_0"), "{note}");
        assert!(note.contains("bijlage (10)"), "{note}");
        assert!(note.contains("aanhef (1)"), "{note}");
    }

    /// The hits under a citation that was not read are the nearest texts to
    /// an identifier, and the note says so and quotes the name as typed.
    #[test]
    fn a_citation_not_read_says_the_hits_are_not_the_article_cited() {
        let note = caveat_line(&Caveat::CitationNotRead {
            number: "3:305a".into(),
            name: "WAMCA".into(),
            because: "name_not_stated".into(),
        });

        assert!(note.contains("WAMCA"), "{note}");
        assert!(note.contains("3:305a"), "{note}");
        assert!(note.contains("name_not_stated"), "{note}");
        assert!(note.contains("not the article"), "{note}");
    }

    /// A rendition prints a provision not yet applied as a stub or as its
    /// settled text, and the note claims neither: it says the text is the
    /// rendition's, whichever it printed.
    #[test]
    fn the_note_on_a_provision_not_yet_applied_claims_no_one_kind_of_text() {
        let note = caveat_line(&Caveat::NotCommenced {
            articles: vec!["119b".into()],
        });

        assert!(note.contains("119b"), "{note}");
        assert!(note.contains("do not apply yet"), "{note}");
        assert!(note.contains("as the rendition prints"), "{note}");
        assert!(!note.contains("will have"), "{note}");
    }

    /// The words are named, because which of them went unsearched is what
    /// tells a reader whether to ask again in other words.
    #[test]
    fn the_words_full_text_did_not_search_are_named() {
        let note = caveat_line(&Caveat::WordsUnsearched {
            words: vec!["van".into(), "de".into()],
        });

        assert!(note.contains("van, de"), "{note}");
        assert!(note.contains("ranking read the whole question"), "{note}");
    }

    #[test]
    fn bounded_places_name_the_texts_and_limit_what_the_count_claims() {
        let hashes = ["a".repeat(64), "b".repeat(64)];
        let note = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "places_read", "bound": 100, "content_hashes": hashes,
        })));

        for hash in hashes {
            assert!(note.contains(&hash), "{note}");
        }
        assert!(note.contains("up to 100"), "{note}");
        assert!(note.contains("lower bounds"), "{note}");
        assert!(note.contains("first among those read"), "{note}");
        assert!(
            note.contains("not necessarily first across all places"),
            "{note}"
        );
    }

    #[test]
    fn an_index_behind_names_the_moment_count_and_the_ranking_limit() {
        let note = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "index_behind", "scope": "moment", "count": 7,
        })));

        assert!(
            note.contains("7 rendition(s) for the moment asked"),
            "{note}"
        );
        assert!(
            note.contains("as the index last held them, or not at all"),
            "{note}"
        );
    }

    #[test]
    fn renditions_read_for_articles_name_what_the_answer_covers() {
        let note = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "read_for_articles", "renditions": ["2025-01-01_0", "2026-01-01_0"],
        })));

        assert!(note.contains("2025-01-01_0, 2026-01-01_0"), "{note}");
        assert!(note.contains("parsed for articles alone"), "{note}");
        assert!(
            note.contains("covers their articles and no other units"),
            "{note}"
        );
    }

    /// A treaty's waiting text is left out of a list of what waits, and the
    /// note counts them without calling what a treaty prints a date to come.
    #[test]
    fn treaties_left_out_are_counted_and_not_announced() {
        let note = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "treaties_left_out", "count": 268,
        })));

        assert!(note.contains("268 treaty(ies)"), "{note}");
        assert!(note.contains("not in this list"), "{note}");
        assert!(note.contains("no announcement"), "{note}");
    }

    /// What the list cannot hold is said in words, because nothing in the
    /// rows can say it.
    #[test]
    fn not_in_bwb_says_what_the_list_cannot_hold() {
        let note = caveat_line(&Caveat::stated(&serde_json::json!({"kind": "not_in_bwb"})));

        assert!(note.contains("what the BWB prints"), "{note}");
        assert!(note.contains("Staatsblad"), "{note}");
        assert!(note.contains("EU regulations"), "{note}");
    }

    /// The text answered is the authentic one, and the note says a translation
    /// stands beside it rather than leaving a reader to find out.
    #[test]
    fn a_translation_left_out_names_the_number_and_the_languages() {
        let note = caveat_line(&Caveat::TranslationLeftOut {
            number: "3".into(),
            lang: vec!["nl".into()],
            paths: vec!["/Verdragtekst2/Artikel3".into()],
        });

        assert!(note.contains("3"), "{note}");
        assert!(note.contains("nl"), "{note}");
        assert!(note.contains("authentic"), "{note}");
        assert!(note.contains("/Verdragtekst2/Artikel3"), "{note}");
    }

    /// Texts no word finds are named apart from texts no vector ranks: a reader
    /// told one would look for the other by the wrong route.
    #[test]
    fn texts_no_word_finds_say_they_are_found_by_identifier() {
        let note = caveat_line(&Caveat::TextsUnindexed {
            scope: "corpus".into(),
            count: 3,
        });

        assert!(note.contains("3 text(s) in the corpus"), "{note}");
        assert!(note.contains("no word"), "{note}");
        assert!(note.contains("identifier"), "{note}");
    }

    /// The regulations sharing the name are named where the service names them.
    #[test]
    fn several_readings_name_the_regulations_that_share_a_name() {
        let note = caveat_line(&Caveat::SeveralReadings {
            readings: vec!["literal".into()],
            regulations: vec!["BWBR0005537".into(), "BWBR0005538".into()],
        });

        assert!(note.contains("BWBR0005537, BWBR0005538"), "{note}");

        let without = caveat_line(&Caveat::SeveralReadings {
            readings: vec!["book".into(), "literal".into()],
            regulations: Vec::new(),
        });
        assert!(!without.contains("regulations"), "{without}");
    }

    #[test]
    fn renditions_not_held_says_how_many_of_how_many() {
        let note = caveat_line(&Caveat::RenditionsNotHeld {
            count: 30,
            renditions: 41,
        });

        assert!(note.contains("30 of the 41"), "{note}");
    }

    #[test]
    fn renditions_unread_says_how_many_of_how_many() {
        let note = caveat_line(&Caveat::RenditionsUnread {
            count: 2,
            renditions: 41,
        });

        assert!(note.contains("2 of the 41"), "{note}");
    }

    /// The kinds print in a fixed order and each with its own phrase, plural
    /// where its count reads more than one, whatever order the map holds
    /// them in and whichever kinds it leaves out.
    #[test]
    fn acts_left_out_names_each_kind_in_order_with_its_own_phrase() {
        let note = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "acts_left_out",
            "counts": {"treaty_approval": 1, "budget": 2},
        })));

        assert_eq!(
            note,
            "note: left out of the published acts: 2 budget acts, 1 treaty approval"
        );
    }

    /// A kind this build has no phrase for is named by its own key rather
    /// than dropped.
    #[test]
    fn acts_left_out_names_a_kind_it_has_no_phrase_for_by_its_own_key() {
        let note = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "acts_left_out",
            "counts": {"budget": 45, "a_kind_this_bron_predates": 3},
        })));

        assert!(note.contains("45 budget acts"), "{note}");
        assert!(note.contains("3 a_kind_this_bron_predates"), "{note}");
    }

    /// A regulation whose WTI is unread is why a published row can list an
    /// act the BWB does print, under a founding regulation bron has not read
    /// yet: the note says so rather than the row silently overclaiming.
    #[test]
    fn founding_acts_unread_names_how_many_regulations() {
        let note = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "founding_acts_unread", "regulations": 2,
        })));

        assert!(
            note.contains("2 regulation(s) have not had their founding act read"),
            "{note}"
        );
        assert!(
            note.contains("listed as published although the BWB prints it"),
            "{note}"
        );
    }

    /// The count of every unlinked besluit, and the recent ones by name:
    /// which ones bron could not link is what tells a reader whether to look
    /// one up by hand.
    #[test]
    fn decrees_unmatched_counts_every_besluit_and_names_the_recent_ones() {
        let note = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "decrees_unmatched",
            "unlinked": 262,
            "decrees": [{"id": "stb-2026-280", "published_on": "2026-09-20"}],
        })));

        assert!(note.contains("262 held besluit(en) name no act"), "{note}");
        assert!(note.contains("cannot say what they commence"), "{note}");
        assert!(
            note.contains("1 of them published in the last 15 days: stb-2026-280 (2026-09-20)."),
            "{note}"
        );
    }

    /// The caveat stands wherever any besluit is unlinked, and with none of
    /// them recent it states the count and lists nothing.
    #[test]
    fn decrees_unmatched_with_no_recent_besluit_states_the_count_alone() {
        let note = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "decrees_unmatched",
            "unlinked": 262,
            "decrees": [],
        })));

        assert!(note.contains("262 held besluit(en) name no act"), "{note}");
        assert!(!note.contains("last 15 days"), "{note}");
        assert!(!note.contains(": ."), "{note}");
    }

    /// Both search limits the service states read in words, each with what
    /// it carried, and neither as a limit this build has no sentence for.
    #[test]
    fn a_short_ranking_and_counts_not_taken_have_sentences() {
        let short = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "ranking_short", "asked": 80, "reached": 23,
        })));
        assert!(
            short.contains("reached 23 of the 80 nearest texts"),
            "{short}"
        );

        let untaken = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "counts_not_taken", "caveats": ["unrankable_texts", "texts_unindexed"],
        })));
        assert!(
            untaken.contains(
                "did not count the texts no ranking reaches or texts too long to index in full text"
            ),
            "{untaken}"
        );

        let later = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "counts_not_taken", "caveats": ["something_later"],
        })));
        assert!(
            later.contains("did not count the something_later"),
            "{later}"
        );
    }

    /// The besluit, the act it names and when it was published: enough to
    /// look the besluit up without bron reading its day for the reader.
    #[test]
    fn commencement_pending_names_the_besluit_and_the_act_it_commences() {
        let note = caveat_line(&Caveat::stated(&serde_json::json!({
            "kind": "commencement_pending",
            "decrees": [{"decree": "stb-2026-8", "published_on": "2026-01-21", "act": "stb-2025-62"}],
        })));

        assert!(note.contains("stb-2026-8 commences stb-2025-62"), "{note}");
        assert!(note.contains("published 2026-01-21"), "{note}");
        assert!(note.contains("bron does not read"), "{note}");
    }

    #[test]
    fn a_publication_not_held_says_its_digest_cannot_be_fetched_again() {
        let note = caveat_line(&Caveat::PublicationNotHeld);

        assert!(note.contains("passed on rather than kept"), "{note}");
    }

    #[test]
    fn a_source_total_estimated_says_the_publisher_stated_it_as_one() {
        let note = caveat_line(&Caveat::SourceTotalEstimated);

        assert!(note.contains("estimate"), "{note}");
    }

    fn printed(answer: &Answer) -> String {
        let mut out = Vec::new();
        super::answer(answer, &mut out, &mut io::sink()).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn answered<T>(data: T) -> bron::service::Answered<T> {
        bron::service::Answered {
            data,
            caveats: Vec::new(),
            as_of: None,
            as_known_on: None,
            envelope: String::new(),
        }
    }

    /// A ruling as the service answers it: the court and the days, the
    /// publisher's procedure and case numbers, the digest every span points
    /// into, then each unit under the section the court numbered it in.
    #[test]
    fn case_show_prints_the_ruling_and_where_each_unit_stands() {
        let ruling: bron::service::DecisionParagraphs = serde_json::from_str(
            r#"{"decision":{"id":"ECLI:NL:HR:2024:1780","register":"rechtspraak","jurisdiction":"nl",
                "form":"ruling","court":"Hoge Raad","decided_on":"2024-11-29","published_on":"2024-11-29",
                "known_since":"2026-01-16T10:06:33","has_body":true,
                "rendition":{"language":"nl","blob":"de3e3678909a1d2e51c4afc4ffe19139f514ca4d357083576e49fbb3eadbe1ff","blob_size":63030},
                "superseded":[],
                "native":{"type":"Uitspraak","procedure":["Prejudiciële beslissing"],"zaaknummer":["24/00169","24/00170"],"subject":[],"vindplaatsen":[]}},
              "paragraphs":[
                {"ordinal":1,"kind":"inhoudsindicatie","span":{"start":10,"end":20},"content_hash":"a","rendered":"Huurrecht."},
                {"ordinal":2,"kind":"title","section_path":"2","span":{"start":30,"end":40},"content_hash":"b","rendered":"2 Uitgangspunten"},
                {"ordinal":3,"kind":"para","section_path":"2/2.1","span":{"start":50,"end":60},"content_hash":"c","rendered":"2.1 De huurder."}]}"#,
        )
        .unwrap();

        let text = printed(&Answer::Decision {
            answered: Box::new(answered(ruling)),
            service: Service::at("https://bron.steddle.com"),
            offset: None,
        });

        assert!(
            text.starts_with("ECLI:NL:HR:2024:1780\nHoge Raad\n"),
            "{text}"
        );
        assert!(
            text.contains("decided 2024-11-29, published 2024-11-29"),
            "{text}"
        );
        assert!(text.contains("Prejudiciële beslissing"), "{text}");
        assert!(text.contains("zaaknummer 24/00169, 24/00170"), "{text}");
        assert!(text.contains("source: de3e3678"), "{text}");
        assert!(
            text.contains("inhoudsindicatie  (10..20)\nHuurrecht."),
            "{text}"
        );
        assert!(
            text.contains("2/2.1, paragraph 3  (50..60)\n2.1 De huurder."),
            "{text}"
        );
    }

    /// A ruling cut at its window is a page of one document, not a filtered
    /// list: the note says which units were shown and how to read on.
    #[test]
    fn a_cut_ruling_says_where_to_read_on() {
        let ruling: bron::service::DecisionParagraphs = serde_json::from_str(
            r#"{"decision":{"id":"ECLI:NL:HR:2024:1780","register":"rechtspraak","form":"ruling","has_body":true,"superseded":[],"native":{}},
              "paragraphs":[
                {"ordinal":11,"kind":"para","span":{"start":1,"end":2},"content_hash":"a","rendered":"Een."},
                {"ordinal":12,"kind":"para","span":{"start":3,"end":4},"content_hash":"b","rendered":"Twee."}]}"#,
        )
        .unwrap();
        let mut answered = answered(ruling);
        answered.caveats = vec![Caveat::Truncated { window: 2 }];

        let text = printed(&Answer::Decision {
            answered: Box::new(answered),
            service: Service::at("https://bron.test"),
            offset: Some("10".into()),
        });

        assert!(
            text.contains(
                "note: units 11 to 12 shown; read on with --offset 12, or the whole ruling with --limit none"
            ),
            "{text}"
        );
        assert!(!text.contains("filtered over a window"), "{text}");
    }

    /// A ruling a question ranked, by identifier and the run of units that
    /// ranked, with where `case show` reads them; on stderr how few rulings
    /// the ranking covered and that a cut list has more below it.
    #[test]
    fn case_search_names_each_ruling_and_says_what_it_searched() {
        let hits: bron::service::DecisionSearch = serde_json::from_str(
            r#"{"hits":[{"id":"ECLI:NL:HR:2024:1780","ecli":"ECLI:NL:HR:2024:1780","register":"rechtspraak","jurisdiction":"nl","form":"ruling",
              "court":"Hoge Raad","decided_on":"2024-11-29","paragraphs":{"from":40,"to":42},
              "content_hashes":["a","b","c"],"blob":"d","distance":0.2134}]}"#,
        )
        .unwrap();
        let mut answered = answered(hits);
        answered.caveats = vec![
            Caveat::Truncated { window: 1 },
            Caveat::SearchUpcoming { count: 7 },
        ];

        let (mut out, mut err) = (Vec::new(), Vec::new());
        super::answer(&Answer::DecisionHits(answered), &mut out, &mut err).unwrap();
        let (out, err) = (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        );

        assert!(
            out.contains("ECLI:NL:HR:2024:1780  ruling  Hoge Raad  2024-11-29"),
            "{out}"
        );
        assert!(
            out.contains("units 40 to 42  --offset 39  distance 0.2134"),
            "{out}"
        );
        assert!(err.contains("the 1 nearest rulings shown"), "{err}");
        assert!(
            err.contains("ranked the 7 ruling(s) bron has read"),
            "{err}"
        );
        assert!(!err.contains("filtered over a window"), "{err}");
    }

    /// The publisher's relations on stdout, one a row, and on stderr the one
    /// conclusion they cannot carry.
    #[test]
    fn case_relations_says_no_relation_settles_an_appeal() {
        let relations: bron::service::DecisionRelations = serde_json::from_str(
            r#"{"id":"ECLI:NL:HR:2024:1780","register":"rechtspraak","jurisdiction":"nl","relations":[
                {"kind":"conclusie","aanleg":"eerdereAanleg","to":{"scheme":"ecli","id":"ECLI:NL:PHR:2024:771"},
                 "asserted":"publisher","stated_by":"rechtspraak","stated_in":"de3e"}]}"#,
        )
        .unwrap();

        let (mut out, mut err) = (Vec::new(), Vec::new());
        super::answer(
            &Answer::DecisionRelations(Box::new(answered(relations))),
            &mut out,
            &mut err,
        )
        .unwrap();
        let (out, err) = (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        );

        assert_eq!(
            out,
            "eerdereAanleg  conclusie  ECLI:NL:PHR:2024:771  stated in de3e\n"
        );
        assert!(
            err.contains("None says whether an appeal was lodged"),
            "{err}"
        );
    }

    /// A limit on what LiDO answered names what is missing and why.
    #[test]
    fn the_links_caveats_say_what_is_missing() {
        let read = super::caveat_line(&Caveat::stated(
            &serde_json::json!({"kind": "links_not_read"}),
        ));
        let unresolved = super::caveat_line(&Caveat::stated(
            &serde_json::json!({"kind": "links_unresolved", "count": 3}),
        ));

        assert!(!read.contains("no sentence for"), "{read}");
        assert!(read.contains("not asked yet or did not answer"), "{read}");
        assert!(!unresolved.contains("no sentence for"), "{unresolved}");
        assert!(unresolved.contains("3 article(s)"), "{unresolved}");
    }

    /// A place reads as a reader cites it, and a fragment the service reads no
    /// grammar for stands as CELLAR writes it.
    #[test]
    fn a_place_reads_as_it_is_cited() {
        let read = |place: serde_json::Value, part: Option<&str>| {
            let place: Option<Pinpoint> = serde_json::from_value(place).unwrap();
            super::pinpoint(place.as_ref(), part)
        };

        assert_eq!(
            read(
                serde_json::json!({"article": "8", "paragraph": "2", "point": "e"}),
                Some("A08P2LE")
            )
            .as_deref(),
            Some("art. 8(2)(e)")
        );
        assert_eq!(
            read(
                serde_json::json!({"article": "2", "point": "b"}),
                Some("A02LB")
            )
            .as_deref(),
            Some("art. 2(b)")
        );
        assert_eq!(
            read(serde_json::json!({"recital": "26"}), Some("C26")).as_deref(),
            Some("recital 26")
        );
        assert_eq!(
            read(serde_json::json!({"point": "81"}), Some("N81")).as_deref(),
            Some("point 81")
        );
        assert_eq!(
            read(serde_json::Value::Null, Some("A08P5L1")).as_deref(),
            Some("A08P5L1")
        );
        assert_eq!(read(serde_json::Value::Null, None), None);
    }

    /// A case of the EU names each place in the article it cites once,
    /// however CELLAR spells the fragment.
    #[test]
    fn law_decisions_on_an_act_of_the_eu_names_each_place_once() {
        let found: bron::service::ArticleDecisions = serde_json::from_str(
            r#"{"regulation_id":"celex:31995L0046","register":"cellar","jurisdiction":"eu","number":"8","count":1,
              "courts":[{"court":"Hof van Justitie","count":1}],
              "decisions":[{"ecli":"ECLI:EU:C:2019:773","register":"cellar","jurisdiction":"eu","court":"Hof van Justitie",
                "decided_on":"2019-09-24","held":true,"links":[
                 {"kind":"work_cites_work","asserted":"publisher","part":"A08P1","place":{"article":"8","paragraph":"1"},"stated_by":"cellar","stated_in":"ab"},
                 {"kind":"work_cites_work","asserted":"publisher","part":"A8P1","place":{"article":"8","paragraph":"1"},"stated_by":"cellar","stated_in":"ab"},
                 {"kind":"work_cites_work","asserted":"publisher","part":"A08P5L1","stated_by":"cellar","stated_in":"ab"}]}]}"#,
        )
        .unwrap();

        let (mut out, mut err) = (Vec::new(), Vec::new());
        super::answer(
            &Answer::ArticleDecisions {
                answered: Box::new(answered(found)),
                offset: None,
            },
            &mut out,
            &mut err,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "2019-09-24  Hof van Justitie  ECLI:EU:C:2019:773  art. 8(1), A08P5L1\n"
        );
        assert!(
            String::from_utf8(err)
                .unwrap()
                .starts_with("1 case(s) found for article 8 of celex:31995L0046\n")
        );
    }

    /// The rulings under an article, in the shape the service's
    /// ArticleDecisionController answers: a ruling LiDO names no court, day
    /// or rendition for carries none of those keys.
    #[test]
    fn law_decisions_counts_the_rulings_found_and_says_what_the_list_cannot() {
        let envelope: serde_json::Value = serde_json::from_str(
            r#"{"bron":1,"grade":"authority","data":{"regulation_id":"BWBR0005289","register":"bwb","jurisdiction":"nl",
              "number":"162","count":12274,"courts":[{"court":"Rechtbank Rotterdam","count":1122}],
              "decisions":[
               {"ecli":"ECLI:NL:GHAMS:2026:2678","register":"rechtspraak","jurisdiction":"nl","court":"Gerechtshof Amsterdam",
                "decided_on":"2026-09-22","held":false,"rendition":"2026-07-16_0",
                "links":[{"kind":"rvr-referentie","asserted":"publisher","stated_by":"lido","stated_in":"ab"}]},
               {"ecli":"ECLI:NL:XX:2015:1","register":"rechtspraak","jurisdiction":"nl","held":true,
                "links":[{"kind":"rvr-referentie","asserted":"publisher","stated_by":"lido"}]}]},
              "caveats":[{"kind":"rulings_found"},{"kind":"truncated","window":2}]}"#,
        )
        .unwrap();
        let found: bron::service::ArticleDecisions =
            serde_json::from_value(envelope["data"].clone()).unwrap();
        let mut answered = answered(found);
        answered.caveats = envelope["caveats"]
            .as_array()
            .unwrap()
            .iter()
            .map(Caveat::stated)
            .collect();

        let (mut out, mut err) = (Vec::new(), Vec::new());
        super::answer(
            &Answer::ArticleDecisions {
                answered: Box::new(answered),
                offset: Some("40".into()),
            },
            &mut out,
            &mut err,
        )
        .unwrap();
        let (out, err) = (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        );

        assert_eq!(
            out,
            "2026-09-22  Gerechtshof Amsterdam  ECLI:NL:GHAMS:2026:2678  not held  version 2026-07-16_0\n\
             \x20                                  ECLI:NL:XX:2015:1\n"
        );
        assert!(!err.contains("no sentence for"), "{err}");
        assert!(
            err.starts_with(
                "12274 ruling(s) found for article 162 of BWBR0005289\n  \
                 1122  Rechtbank Rotterdam\n"
            ),
            "{err}"
        );
        assert!(err.contains("the rulings LiDO found"), "{err}");
        assert!(err.contains("is not measured"), "{err}");
        assert!(
            err.contains("rulings 41 to 42 of 12274 shown; read on with --offset 42"),
            "{err}"
        );
    }

    /// The rulings rechtspraak's feed listed, in the shape the service's
    /// DecisionController answers: a ruling whose title names no court or day
    /// carries neither key.
    #[test]
    fn case_recent_counts_the_rulings_listed_per_court_and_marks_what_is_not_held() {
        let envelope: serde_json::Value = serde_json::from_str(
            r#"{"bron":1,"grade":"authority","data":{"count":4,
              "courts":[{"court":"Rechtbank Amsterdam","count":2},{"court":"Hoge Raad","count":1}],
              "decisions":[
               {"ecli":"ECLI:NL:HR:2026:1552","register":"rechtspraak","jurisdiction":"nl","court":"Hoge Raad",
                "decided_on":"2026-09-25","case_numbers":"24/04589","summary":"Huurrecht. Opzegging wegens dringend eigen gebruik.","form":"ruling","held":true,"stated_by":"rechtspraak","stated_in":"ab"},
               {"ecli":"ECLI:NL:XX:2026:1","register":"rechtspraak","jurisdiction":"nl","form":"ruling","summary_left_out":"criminal_law",
                "held":false,"stated_by":"rechtspraak","stated_in":"ab"}]},
              "caveats":[{"kind":"rulings_listed"},{"kind":"truncated","window":2}]}"#,
        )
        .unwrap();
        let listed: bron::service::RecentDecisions =
            serde_json::from_value(envelope["data"].clone()).unwrap();
        let mut answered = answered(listed);
        answered.caveats = envelope["caveats"]
            .as_array()
            .unwrap()
            .iter()
            .map(Caveat::stated)
            .collect();

        let (mut out, mut err) = (Vec::new(), Vec::new());
        super::answer(
            &Answer::RecentDecisions {
                answered: Box::new(answered),
                offset: None,
            },
            &mut out,
            &mut err,
        )
        .unwrap();
        let (out, err) = (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        );

        assert_eq!(
            out,
            "2026-09-25  Hoge Raad  ECLI:NL:HR:2026:1552  24/04589\n\
             \x20   Huurrecht. Opzegging wegens dringend eigen gebruik.\n\
             \x20                      ECLI:NL:XX:2026:1  criminal law  not held\n"
        );
        assert!(!err.contains("no sentence for"), "{err}");
        assert!(
            err.starts_with(
                "4 ruling(s) listed by rechtspraak's feed\n  \
                 2  Rechtbank Amsterdam\n  \
                 1  Hoge Raad\n"
            ),
            "{err}"
        );
        assert!(err.contains("may still have been decided"), "{err}");
        assert!(
            err.contains("rulings 1 to 2 of 4 shown; read on with --offset 2"),
            "{err}"
        );
    }

    /// A rendition's contents rendered for a person: stdout and stderr apart.
    fn contents_rendered(
        data: serde_json::Value,
        caveats: &[serde_json::Value],
    ) -> (String, String) {
        let mut answered = answered(
            serde_json::from_value::<bron::service::Contents>(serde_json::json!({
                "regulation_id": "BWBR0005289",
                "register": "bwb",
                "jurisdiction": "nl",
                "expression": {
                    "regulation_id": "BWBR0005289", "label": "2026-01-01_0",
                    "in_force": {"from": "2026-01-01"}, "withdrawn": false, "held": true,
                    "upstream_hash_check": "matches", "blob": "a".repeat(64),
                },
                "contents": data,
            }))
            .unwrap(),
        );
        answered.as_of = NaiveDate::from_ymd_opt(2026, 3, 1);
        answered.as_known_on = NaiveDate::from_ymd_opt(2026, 3, 2);
        answered.caveats = caveats.iter().map(Caveat::stated).collect();

        let (mut out, mut err) = (Vec::new(), Vec::new());
        super::answer(&Answer::Contents(Box::new(answered)), &mut out, &mut err).unwrap();
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    /// Parts nest two spaces a level, the articles of a part are one `art.`
    /// line in document order, a part between them ends the run, and a
    /// repealed article is starred with the legend on stderr.
    #[test]
    fn law_contents_prints_an_outline_with_the_articles_of_a_part_on_one_line() {
        let (out, err) = contents_rendered(
            serde_json::json!([
                {"kind": "boek", "label": "Boek 6", "title": "Algemeen gedeelte van het verbintenissenrecht", "path": "/Boek6", "children": [
                    {"kind": "titeldeel", "label": "Titel 3", "title": "Onrechtmatige daad", "children": [
                        {"kind": "artikel", "label": "Artikel 161", "number": "161"},
                        {"kind": "afdeling", "label": "Afdeling 1", "title": "Algemene bepalingen", "later_field": 1, "children": [
                            {"kind": "artikel", "label": "Artikel 162", "number": "162"},
                            {"kind": "artikel", "label": "Artikel 163", "number": "163", "repealed": true},
                            {"kind": "artikel", "label": "Artikel 164", "number": "164"},
                        ]},
                        {"kind": "afdeling", "label": "Afdeling 2", "repealed": true, "children": []},
                    ]},
                ]},
                {"kind": "enig-artikel", "label": "Enig artikel"},
                {"kind": "bijlage", "label": "Bijlage 1", "number": "1"},
            ]),
            &[serde_json::json!({"kind": "no_zicht_window"})],
        );

        assert_eq!(
            out,
            "Boek 6. Algemeen gedeelte van het verbintenissenrecht\n\
             \x20 Titel 3. Onrechtmatige daad\n\
             \x20   art. 161\n\
             \x20   Afdeling 1. Algemene bepalingen\n\
             \x20     art. 162 163* 164\n\
             \x20   Afdeling 2 (repealed)\n\
             Enig artikel\n\
             Bijlage 1\n"
        );
        assert!(
            err.starts_with(
                "as in force on 2026-03-01, as the register showed on 2026-03-02\n\
                 from rendition 2026-01-01_0, in force 2026-01-01 onwards\n"
            ),
            "{err}"
        );
        assert!(err.contains("* = repealed (vervallen)"), "{err}");
        assert!(!err.contains("no sentence for"), "{err}");
        assert!(err.contains("states no zicht dates"), "{err}");
    }

    /// No article marked, no legend; and a run too long for one line wraps
    /// with every continuation under the first number.
    #[test]
    fn law_contents_wraps_a_long_run_and_states_no_legend_it_does_not_need() {
        let articles: Vec<serde_json::Value> = (1..=30)
            .map(|n| serde_json::json!({"kind": "artikel", "number": format!("{n}")}))
            .collect();
        let (out, err) = contents_rendered(
            serde_json::json!([{"kind": "hoofdstuk", "label": "Hoofdstuk 1", "children": articles}]),
            &[],
        );

        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "Hoofdstuk 1");
        assert!(lines[1].starts_with("  art. 1 2 3 "), "{out}");
        assert!(lines[2].starts_with("       "), "{out}");
        assert!(lines.iter().all(|line| line.chars().count() <= 80), "{out}");
        assert_eq!(
            lines[1..]
                .iter()
                .flat_map(|line| line.split_whitespace())
                .filter(|word| *word != "art.")
                .count(),
            30
        );
        assert!(!err.contains("repealed"), "{err}");
    }

    /// A rendition the service read no contents from says so, rather than
    /// printing nothing on either stream.
    #[test]
    fn law_contents_says_so_where_the_service_read_none() {
        let (out, err) = contents_rendered(serde_json::json!([]), &[]);

        assert_eq!(out, "");
        assert!(
            err.contains("The service read no contents from BWBR0005289 rendition 2026-01-01_0."),
            "{err}"
        );
    }

    /// A search hit built for a test: every field a caller does not care about
    /// held to a sensible default, so a test states only what it means to.
    fn publication_hit(
        id: &str,
        held: bool,
        title: Option<&str>,
        published_on: Option<&str>,
    ) -> bron::service::PublicationHit {
        bron::service::PublicationHit {
            id: id.into(),
            held,
            publication_type: Some("Kamerstuk".into()),
            title: title.map(str::to_string),
            published_on: published_on.map(|d| d.parse().unwrap()),
        }
    }

    fn publication_search(
        hits: Vec<bron::service::PublicationHit>,
        caveats: Vec<Caveat>,
    ) -> Answer {
        Answer::PublicationHits(Answered {
            data: bron::service::PublicationSearch {
                hits,
                source_total: 1,
                next: None,
            },
            caveats,
            as_of: None,
            as_known_on: None,
            envelope: String::new(),
        })
    }

    /// When a publication appeared is on its row and on the record.
    #[test]
    fn a_publication_says_when_it_was_published() {
        use bron::service::Publication;

        let hits = publication_search(
            vec![publication_hit(
                "kst-33669-6",
                true,
                Some("Programmatische aanpak stikstof"),
                Some("2013-10-01"),
            )],
            Vec::new(),
        );
        let record = Answer::Publication(Box::new(Answered {
            data: Publication {
                id: "kst-33669-6".into(),
                publication_type: Some("Kamerstuk".into()),
                title: Some("Programmatische aanpak stikstof".into()),
                published_on: Some("2013-10-01".parse().unwrap()),
                signed_on: None,
                dossier: Some("33669".into()),
                held: true,
                manifestations: Vec::new(),
                dossiers: Vec::new(),
                units: None,
            },
            caveats: Vec::new(),
            as_of: None,
            as_known_on: None,
            envelope: String::new(),
        }));

        assert!(
            printed(&hits)
                .lines()
                .any(|l| l == "kst-33669-6  2013-10-01"),
            "{}",
            printed(&hits)
        );
        assert!(
            printed(&record).contains("published 2013-10-01"),
            "{}",
            printed(&record)
        );
    }

    /// Rows on stdout and every note on stderr, so a shell script reading
    /// line by line never reads a sentence as a row.
    #[test]
    fn a_list_of_publications_keeps_its_notes_off_stdout() {
        let publications = publication_search(
            vec![publication_hit(
                "kst-33669-6",
                true,
                Some("Programmatische aanpak stikstof"),
                None,
            )],
            vec![Caveat::Truncated { window: 20 }],
        );

        let (mut out, mut err) = (Vec::new(), Vec::new());
        super::answer(&publications, &mut out, &mut err).unwrap();
        let (out, err) = (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        );

        assert!(out.contains("kst-33669-6"), "{out}");
        assert!(!out.contains("note:"), "{out}");
        assert!(err.contains("note: filtered over a window of 20"), "{err}");
    }

    /// Three facts about the publisher's hash, and a reader told one must not
    /// take it for another: a rewritten rendition is the earlier text and says
    /// so, where the other two say nothing about which publication this is.
    #[test]
    fn each_hash_limit_prints_a_sentence_of_its_own() {
        let absent = caveat_line(&Caveat::UpstreamHashAbsent);
        let differs = caveat_line(&Caveat::UpstreamHashDiffers);
        let rewritten = caveat_line(&Caveat::UpstreamHashRewritten);

        assert!(absent.contains("published no hash"), "{absent}");
        assert!(differs.contains("do not hash to"), "{differs}");
        assert!(rewritten.contains("rewrote this rendition"), "{rewritten}");
        assert!(rewritten.contains("earlier text"), "{rewritten}");
    }

    #[test]
    fn a_text_of_the_eu_read_from_html_or_past_an_unread_consolidation_says_so() {
        let html = caveat_line(&Caveat::StructureReadFromHtml {
            format: "xhtml".into(),
        });
        let unread = caveat_line(&Caveat::ConsolidationsUnread {
            consolidations: vec!["02005L0029-20050612".into(), "02005L0029-20080101".into()],
        });

        assert!(
            html.contains("the Official Journal is the authentic text"),
            "{html}"
        );
        assert!(
            html.contains("check in the Official Journal where an article ends"),
            "{html}"
        );
        assert!(
            unread
                .contains("as Formex or HTML. 02005L0029-20050612, 02005L0029-20080101. The text"),
            "{unread}"
        );
    }

    /// What a rendition holds is true of the rendition, so it has a column of
    /// its own. Trailing the label would move it by however long the label
    /// above happened to be.
    #[test]
    fn every_label_starts_in_the_same_place_whether_or_not_the_text_is_held() {
        let text = rendered(vec![
            cited("book", false, None, Vec::new()),
            cited("literal", true, Some("Artikel 3:305a"), Vec::new()),
        ]);

        let unheld = text.lines().find(|l| l.starts_with("book")).unwrap();
        let held = text.lines().find(|l| l.starts_with("literal")).unwrap();

        assert!(unheld.contains(NOT_HELD), "{text}");
        assert_eq!(
            unheld.find("305a").unwrap(),
            held.find("Artikel 3:305a").unwrap(),
            "{text}"
        );
    }
}
