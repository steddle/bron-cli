//! Human output.
//!
//! Reads the same envelope the JSON does, so the two cannot describe different
//! things. Anything the envelope marks as a caveat is printed, because a limit
//! on what an answer supports is not a detail a reader may miss.

use std::io::{self, Write};

use chrono::NaiveDate;

use bron::service::SearchData;
use bron::wire::{AppealStatus, Body, Caveat, Envelope};

use crate::run::Answer;

/// Column widths for a citation lookup, named once so the line and everything
/// written under it cannot drift apart.
///
/// `NOT_HELD` is a column and not a suffix: what it says is true of the
/// rendition, not of the label it would otherwise trail.
const READING: usize = 8;
const SOURCE: usize = 11;
const NOT_HELD: &str = "not held";

pub fn answer(answer: &Answer, out: &mut impl Write) -> io::Result<()> {
    match answer {
        Answer::LawHits(a) => {
            for hit in &a.data {
                writeln!(out, "{}  {}", hit.id, hit.title.as_deref().unwrap_or(""))?;
            }
            caveats(&a.caveats, out)
        }

        Answer::Found(a) => {
            bound(a.as_of, a.as_known_on, out)?;

            match &a.data {
                SearchData::Ranking { hits, .. } => {
                    if hits.is_empty() {
                        // Said in words. A blank screen reads as a failure, and
                        // this is an answer: nothing in what was searched came
                        // close.
                        writeln!(out, "nothing in the corpus came close to that")?;
                    }

                    for hit in hits {
                        let unit = match hit.lid.as_ref().and_then(|lid| lid.number.as_ref()) {
                            Some(number) => format!("lid {number}"),
                            None if hit.unit == "lid" => "lid".to_string(),
                            None => "article".to_string(),
                        };

                        let distance = hit
                            .distance
                            .map_or_else(|| "    -".to_string(), |d| format!("{d:.3}"));

                        writeln!(
                            out,
                            "{}  {:<11} {:<8} {}  {}",
                            distance,
                            hit.source,
                            unit,
                            hit.regulation_id,
                            hit.label.as_deref().unwrap_or(&hit.number),
                        )?;
                    }
                }

                // No distance, and the column it would sit in is not left
                // empty: a lookup is arithmetic and prints what it resolved.
                // The reading leads every line, because with more than one it
                // is the answer to which of them the reader meant.
                SearchData::Lookup { citation, resolved } => {
                    writeln!(out, "{citation}\n")?;

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
                let held = if rendition.held { "*" } else { " " };
                writeln!(
                    out,
                    "{held} {}  {}  {}",
                    rendition.regulation_id.as_deref().unwrap_or(""),
                    rendition.label,
                    window(rendition.in_force.from, rendition.in_force.until)
                )?;
            }

            writeln!(out, "\n* = the service holds this text")?;
            caveats(&a.caveats, out)
        }

        Answer::Feed(a) => {
            for event in &a.data.events {
                writeln!(
                    out,
                    "{:>8}  {:<12}{}  {}",
                    event.seq,
                    event.event,
                    event.regulation_id,
                    event.label.as_deref().unwrap_or("")
                )?;
            }

            if a.data.events.is_empty() {
                writeln!(out, "The register has done nothing since {}.", a.data.next)?;
            }

            // The cursor and the corpus it belongs to go to stderr, so a shell
            // script reads events on stdout and takes its next cursor from
            // --json, where both are fields rather than a sentence.
            eprintln!("next {}  head {}  origin {}", a.data.next, a.data.head, a.data.origin);

            caveats(&a.caveats, out)
        }

        Answer::Versions(a) => {
            for version in &a.data {
                let held = if version.held { "*" } else { " " };
                writeln!(
                    out,
                    "{held} {}  {}",
                    version.label,
                    window(version.in_force.from, version.in_force.until)
                )?;
            }
            writeln!(out, "\n* = the service holds this text")?;
            caveats(&a.caveats, out)
        }

        Answer::Articles(e) => {
            bound(e.as_of, e.as_known_on, out)?;
            for article in data(e) {
                let label = article.label.as_deref().unwrap_or(&article.number);
                write!(out, "{label}")?;
                if let Some(title) = &article.title {
                    write!(out, "  {title}")?;
                }
                writeln!(out)?;
                if let Some(path) = &article.path {
                    writeln!(out, "{path}")?;
                }
                // A repealed provision reads as one word, so the day it went is
                // the only thing on the line that answers anything. wetten.nl
                // prints the same date for the same reason.
                match (article.effect.as_deref(), article.in_force_from) {
                    (Some("vervallen"), Some(day)) => {
                        writeln!(out, "\n{} per {day}\n", article.content)?
                    }
                    _ => writeln!(out, "\n{}\n", article.content)?,
                }
                writeln!(
                    out,
                    "source: {} ({}..{})",
                    article.provenance.blob, article.excerpt.start, article.excerpt.end
                )?;
                writeln!(out, "{}", article.provenance.url)?;
                // The register's own citation string for exactly what is
                // printed above, carrying this rendition's zicht and geldigheid
                // dates. It is what Dutch practice already cites with, so a
                // reader needs no mapping from ours, and it was being stored
                // and thrown away at the last step.
                if let Some(jci) = &article.jci {
                    writeln!(out, "{jci}")?;
                }
                if let Some(act) = &article.amended_by {
                    let effect = article.effect.as_deref().unwrap_or("wijziging");
                    writeln!(out, "in force through {act} ({effect})")?;
                }
                writeln!(out)?;
            }
            caveats(&e.caveats, out)
        }

        Answer::Changes(e) => {
            let Some(changes) = &e.data else {
                return Ok(());
            };

            writeln!(out, "{}", changes.law_id)?;
            writeln!(
                out,
                "{} ({}) -> {} ({})\n",
                changes.from.version,
                window(changes.from.in_force.from, changes.from.in_force.until),
                changes.to.version,
                window(changes.to.in_force.from, changes.to.in_force.until)
            )?;

            let s = &changes.summary;
            writeln!(
                out,
                "{} amended, {} added, {} repealed, {} withdrawn, {} unchanged\n",
                s.amended, s.added, s.repealed, s.withdrawn, s.unchanged
            )?;

            for article in &changes.articles {
                let label = article.label.as_deref().unwrap_or(&article.number);
                writeln!(out, "{:<10} {label}", article.change.to_uppercase())?;

                if let Some(after) = &article.after
                    && let Some(act) = &after.amended_by
                {
                    writeln!(out, "           through {act}")?;
                }

                match (&article.before, &article.after) {
                    (Some(before), Some(after)) if article.change == "amended" => {
                        for line in word_diff(&before.content, &after.content) {
                            writeln!(out, "  {line}")?;
                        }
                    }
                    (None, Some(after)) => writeln!(out, "  + {}", first_line(&after.content))?,
                    (Some(before), None) => writeln!(out, "  - {}", first_line(&before.content))?,
                    _ => {}
                }
                writeln!(out)?;
            }

            caveats(&e.caveats, out)
        }

        Answer::CaseHits(e) => {
            for hit in data(e) {
                writeln!(out, "{}", hit.ecli)?;
                if let Some(title) = &hit.title {
                    writeln!(out, "  {title}")?;
                }
                if let Some(summary) = &hit.summary {
                    writeln!(out, "  {summary}")?;
                }
            }
            caveats(&e.caveats, out)
        }

        Answer::Case(e) => {
            let Some(case) = &e.data else { return Ok(()) };
            writeln!(out, "{}", case.ecli)?;
            if let Some(court) = &case.court {
                writeln!(out, "{court}")?;
            }
            if let Some(decided) = case.decided_on {
                writeln!(out, "decided {decided}")?;
            }
            if let Some(summary) = &case.summary {
                writeln!(out, "\n{summary}")?;
            }
            if !case.relations.is_empty() {
                writeln!(out, "\nrelations")?;
                for relation in &case.relations {
                    let kind = relation.kind.as_deref().unwrap_or("relation");
                    writeln!(out, "  {} {kind} {}", relation.direction, relation.target)?;
                }
            }
            writeln!(out, "\nappeal: {}", appeal(&case.appeal))?;
            writeln!(out, "source: {}", case.provenance.blob)?;
            caveats(&e.caveats, out)
        }

        Answer::Appeal(e) => {
            let Some(status) = &e.data else { return Ok(()) };
            writeln!(out, "{}", appeal(status))?;
            caveats(&e.caveats, out)
        }

        Answer::Publication(e) => {
            let Some(doc) = &e.data else { return Ok(()) };

            writeln!(out, "{}  {}", doc.id, doc.title.as_deref().unwrap_or(""))?;
            if let Some(dossier) = &doc.dossier {
                writeln!(out, "  dossier {dossier}")?;
            }

            match &doc.body {
                Body::Linked { provenance } => {
                    writeln!(out, "source: {}", provenance.blob)?;
                    writeln!(out, "{}", provenance.url)?;
                }
                // Said in words. Silence here reads as a digest that went
                // missing rather than one the register never had.
                Body::Absent => writeln!(
                    out,
                    "the register links no document for this record, so there is nothing to hash"
                )?,
            }

            caveats(&e.caveats, out)
        }

        Answer::PublicationHits(e) => {
            for hit in data(e) {
                writeln!(out, "{}  {}", hit.id, hit.title.as_deref().unwrap_or(""))?;
                if let Some(dossier) = &hit.dossier {
                    writeln!(out, "  dossier {dossier}")?;
                }
            }
            caveats(&e.caveats, out)
        }
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

fn data<T>(envelope: &Envelope<Vec<T>>) -> &[T] {
    envelope.data.as_deref().unwrap_or_default()
}

/// The moment an answer is bound to, where it is bound to one.
fn bound(
    as_of: Option<NaiveDate>,
    as_known_on: Option<NaiveDate>,
    out: &mut impl Write,
) -> io::Result<()> {
    match (as_of, as_known_on) {
        (Some(day), Some(read_on)) => writeln!(
            out,
            "as in force on {day}, as the register showed on {read_on}\n"
        ),
        (Some(day), None) => writeln!(out, "as in force on {day}\n"),
        (None, Some(read_on)) => writeln!(out, "as the register showed on {read_on}\n"),
        (None, None) => Ok(()),
    }
}

fn window(from: NaiveDate, until: Option<NaiveDate>) -> String {
    match until {
        Some(until) => format!("{from} to {until}"),
        None => format!("{from} onwards"),
    }
}

fn appeal(status: &AppealStatus) -> String {
    match status {
        AppealStatus::Appealed { successors } => {
            format!("a later decision exists: {}", successors.join(", "))
        }
        // Deliberately not 'not appealed'. This says what the register held on
        // the day it was read, and nothing about whether an appeal was brought.
        AppealStatus::NoPublishedSuccessor { as_of } => {
            format!("no published successor known, as of {}", as_of.date_naive())
        }
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

/// One limit, in words.
///
/// Separate from the loop above because a limit is not always the answer's:
/// a citation read two ways carries one per reading, and those belong under
/// the line they qualify rather than in a block at the end.
fn caveat_line(caveat: &Caveat) -> String {
    match caveat {
        Caveat::UnstableOrdering => {
            "note: ordering comes from a relevance score and is not reproducible".to_string()
        }
        Caveat::Truncated { window } => {
            format!("note: filtered over a window of {window}, matches may be missing")
        }
        Caveat::SkippedArticles { count } => {
            format!("note: {count} article(s) yielded no text and were skipped")
        }
        Caveat::AmbiguousArticle { number, paths } => format!(
            "note: {} articles are numbered {number} in this law; all are shown, \
                 and their paths are what tell them apart:\n      {}",
            paths.len(),
            paths.join("\n      ")
        ),
        Caveat::TableFlattened {
            articles,
            cell_separator,
        } => format!(
            "note: article(s) {} print a table. Its rows are one line each with cells \
                 separated by [{}], so a value belongs to the heading in the same position \
                 on the row above rather than to the line it follows.",
            articles.join(", "),
            cell_separator.trim(),
        ),
        Caveat::CursorAhead { asked, head } => format!(
            "note: cursor {asked} is past everything the service holds, which ends at {head}. \
                 That is not a quiet register: the cursor belongs to another deployment, or to \
                 this one before it was rebuilt. Start again from the head."
        ),
        Caveat::UpstreamDigestDiffers => {
            "note: the bytes behind this answer do not match the digest the publisher's \
                 manifest states for them. The register is inconsistent about what it hashes, \
                 so this is upstream disagreeing with itself rather than the text being wrong, \
                 and it cannot be settled from here."
                .to_string()
        }
        Caveat::UpstreamDigestAbsent => {
            "note: the manifest published no hash for this rendition, so these bytes rest \
                 on bron's own capture alone"
                .to_string()
        }
        Caveat::SeveralReadings { readings } => format!(
            "note: this citation reads more than one way and every reading is answered: {}. \
                 Which was meant is yours to say.",
            readings.join(", "),
        ),
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
        Caveat::NotCommenced { articles } => format!(
            "note: article(s) {} have not commenced. The register publishes the text they \
                 will have, and it does not apply yet.",
            articles.join(", "),
        ),
        Caveat::IllustrationOmitted { articles } => format!(
            "note: article(s) {} print an image, which bron does not hold. Where the image \
                 is a formula, the text states what its variables mean and not the formula \
                 itself.",
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
        Caveat::UnverifiedSlice => {
            "note: the text beside each span is the service's word for what stands there. \
                 bron did not cut it out of bytes it hashed, so it is a report and not a \
                 proof; fetch the blob and slice it at those offsets to check it."
                .to_string()
        }
        // Said because the counts above cannot say it. An article left out
        // of a comparison is left out of its totals too, so silence here
        // reads as 'nothing changed'.
        Caveat::ArticlesNotCompared {
            unidentified_from,
            unidentified_to,
            repeated,
        } => {
            let mut line = String::from("note: not every article could be compared.");
            if unidentified_from + unidentified_to > 0 {
                // Counted per rendition, and said that way. A missing
                // stam-id is the register declining to say whether the one
                // here is the one there, so one total would read as two
                // articles where there may be one.
                line.push_str(&format!(
                    " {unidentified_from} article(s) in the earlier rendition and \
                         {unidentified_to} in the later carry no stam-id, so nothing says \
                         whether one on either side is the same article."
                ));
            }
            if !repeated.is_empty() {
                line.push_str(&format!(
                    " The register used stam-id {} more than once inside one rendition, and \
                         picking one of those would be a guess.",
                    repeated.join(", "),
                ));
            }
            line.push_str(" They are outside the counts above.");
            line
        }
        Caveat::Unrecognised { stated } => {
            format!("note: the service stated a limit this bron has no sentence for: {stated}")
        }
        Caveat::NoZichtWindow => {
            "note: the manifest carries no zicht dates, so this question cannot be \
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
                resolved,
            },
            caveats: Vec::new(),
            as_of: None,
            as_known_on: None,
            envelope: String::new(),
        });

        let mut out = Vec::new();
        answer.render(&mut out).unwrap();
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
            vec![serde_json::json!({"kind": "unverified"})],
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
