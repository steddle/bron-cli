//! Human output.
//!
//! Reads the same envelope the JSON does, so the two cannot describe different
//! things. Anything the envelope marks as a caveat is printed, because a limit
//! on what an answer supports is not a detail a reader may miss.

use std::io::{self, Write};

use chrono::NaiveDate;

use bron::service::SearchData;
use bron::wire::{AppealStatus, Article, Body, Caveat, Envelope};

use crate::run::Answer;

/// Column widths for a citation lookup, named once so the line and everything
/// written under it cannot drift apart.
///
/// `NOT_HELD` is a column and not a suffix: what it says is true of the
/// rendition, not of the label it would otherwise trail.
const READING: usize = 8;
const SOURCE: usize = 11;
const NOT_HELD: &str = "not held";

/// Writes one answer for a person.
///
/// Where stdout carries rows, one per thing listed, every note on them goes to
/// `err`: a caveat, the cursor, a legend. A shell script reading line by line
/// then never reads a sentence as a row. An answer that is a document rather
/// than a list keeps its notes beneath it on `out`.
pub fn answer(answer: &Answer, out: &mut impl Write, err: &mut impl Write) -> io::Result<()> {
    match answer {
        Answer::LawHits(a) => {
            for hit in &a.data {
                writeln!(out, "{}  {}", hit.id, hit.title.as_deref().unwrap_or(""))?;
            }
            caveats(&a.caveats, err)
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
                            bron::service::name(
                                hit.label.as_deref(),
                                hit.number.as_deref(),
                                &hit.kind,
                                None,
                            ),
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
                    rendition.regulation_id,
                    rendition.label,
                    window(rendition.in_force.from, rendition.in_force.until)
                )?;
            }

            writeln!(err, "* = the service holds this text")?;
            caveats(&a.caveats, err)
        }

        Answer::Feed(a) => {
            for event in &a.data.events {
                // A regulation-level event names no rendition, so the line ends
                // at the identifier rather than on the padding of an empty one.
                let line = format!(
                    "{:>8}  {:<12}{}",
                    event.seq, event.event, event.regulation_id
                );

                match &event.label {
                    Some(label) => writeln!(out, "{line}  {label}")?,
                    None => writeln!(out, "{line}")?,
                }
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

        Answer::Versions(a) => {
            for version in &a.data {
                let held = if version.held { "*" } else { " " };
                write!(
                    out,
                    "{held} {}  {}",
                    version.label,
                    window(version.in_force.from, version.in_force.until)
                )?;
                match &version.unread_because {
                    Some(because) => writeln!(out, "  unread: {because}")?,
                    None => writeln!(out)?,
                }
            }
            writeln!(err, "* = the service holds this text")?;
            if a.data
                .iter()
                .any(|version| version.unread_because.is_some())
            {
                writeln!(
                    err,
                    "unread = held and not parsed into articles, so it has none to answer"
                )?;
            }
            caveats(&a.caveats, err)
        }

        Answer::Articles(e) => {
            let articles = data(e);

            // An answer holding only provisions not yet applied is bound to
            // the rendition that publishes them, and 'as in force on' would
            // say they apply.
            if !articles.is_empty() && articles.iter().all(not_commenced) {
                match e.as_of {
                    Some(day) => writeln!(
                        out,
                        "published in the rendition current on {day}, and not yet applied\n"
                    )?,
                    None => writeln!(out, "published and not yet applied\n")?,
                }
            } else {
                bound(e.as_of, e.as_known_on, out)?;
            }

            for article in articles {
                write!(out, "{}", article.name())?;
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
                if not_commenced(article) {
                    match article.in_force_from {
                        Some(day) => writeln!(out, "not yet applied, applies from {day}")?,
                        None => writeln!(out, "not yet applied")?,
                    }
                }
                if let Some(act) = &article.amended_by {
                    let effect = article.effect.as_deref().unwrap_or("wijziging");
                    if not_commenced(article) {
                        writeln!(out, "published by {act} ({effect})")?;
                    } else {
                        writeln!(out, "in force through {act} ({effect})")?;
                    }
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
                "{} amended, {} commenced, {} added, {} repealed, {} withdrawn, {} unchanged\n",
                s.amended, s.commenced, s.added, s.repealed, s.withdrawn, s.unchanged
            )?;

            for article in &changes.articles {
                let label = article
                    .label
                    .as_deref()
                    .or(article.number.as_deref())
                    .unwrap_or_default();
                writeln!(out, "{:<10} {label}", article.change.to_uppercase())?;

                if let Some(after) = &article.after
                    && let Some(act) = &after.amended_by
                {
                    writeln!(out, "           through {act}")?;
                }

                if article.change == "commenced"
                    && let Some(day) = article.after.as_ref().and_then(|after| after.in_force_from)
                {
                    writeln!(out, "           commenced on {day}")?;
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
            caveats(&e.caveats, err)
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
            caveats(&e.caveats, err)
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

/// Whether the register publishes this provision and does not yet apply it.
fn not_commenced(article: &Article) -> bool {
    article.status.as_deref() == Some("nogniet")
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
            "note: {} articles in this law are numbered {number}; their paths tell them \
                 apart:\n      {}",
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
            "note: article(s) {} have not commenced and do not apply yet. Each is shown as \
                 the rendition prints it, the stub or the settled text, with the day it applies \
                 where the register states one.",
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
        // Every unit, the aanhef too: the count is the service's, and naming
        // only some of it would be bron deciding which text matters.
        Caveat::TextOutsideArticles { expression, units } => format!(
            "note: rendition {expression} prints text outside its articles, which bron does \
                 not read: {}. This answer is true of the articles alone.",
            units
                .iter()
                .map(|(unit, count)| format!("{unit} ({count})"))
                .collect::<Vec<_>>()
                .join(", "),
        ),
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

    /// Rows on stdout and every note on stderr, for the two lists read from a
    /// publisher, whose endpoints no test can stand in for.
    #[test]
    fn a_list_read_from_a_publisher_keeps_its_notes_off_stdout() {
        use bron::wire::{CaseHit, PublicationHit};

        let cases = Answer::CaseHits(
            Envelope::ok(vec![CaseHit {
                ecli: "ECLI:NL:HR:2025:1888".into(),
                court: Some("Hoge Raad".into()),
                title: Some("Hoge Raad, 12-12-2025, 25/01812".into()),
                summary: None,
                decided_on: None,
            }])
            .with_caveats(vec![
                Caveat::UnstableOrdering,
                Caveat::Truncated { window: 20 },
            ]),
        );
        let publications = Answer::PublicationHits(
            Envelope::ok(vec![PublicationHit {
                id: "kst-33669-6".into(),
                title: Some("Programmatische aanpak stikstof".into()),
                dossier: Some("33669".into()),
                published_on: None,
            }])
            .with_caveats(vec![Caveat::Truncated { window: 20 }]),
        );

        for (answer, row) in [
            (cases, "ECLI:NL:HR:2025:1888"),
            (publications, "kst-33669-6"),
        ] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            super::answer(&answer, &mut out, &mut err).unwrap();
            let (out, err) = (
                String::from_utf8(out).unwrap(),
                String::from_utf8(err).unwrap(),
            );

            assert!(out.contains(row), "{out}");
            assert!(!out.contains("note:"), "{out}");
            assert!(err.contains("note: filtered over a window of 20"), "{err}");
        }
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
