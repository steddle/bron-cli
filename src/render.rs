//! Human output.
//!
//! Reads the same envelope the JSON does, so the two cannot describe different
//! things. Anything the envelope marks as a caveat is printed, because a limit
//! on what an answer supports is not a detail a reader may miss.

use std::io::{self, Write};

use bron::wire::{AppealStatus, Caveat, DigestCheck, Envelope, View, Window};

use crate::run::Answer;

pub fn answer(answer: &Answer, out: &mut impl Write) -> io::Result<()> {
    match answer {
        Answer::LawHits(e) => {
            for hit in data(e) {
                writeln!(out, "{}  {}", hit.id, hit.title.as_deref().unwrap_or(""))?;
            }
            caveats(e.caveats.as_slice(), out)
        }

        Answer::Ranking(e) => {
            bound(e, out)?;

            let Some(ranking) = &e.data else {
                return Ok(());
            };

            if ranking.hits.is_empty() {
                // Said in words. A blank screen reads as a failure, and this is
                // an answer: nothing in what was searched came close.
                writeln!(out, "nothing in the corpus came close to that")?;
            }

            for hit in &ranking.hits {
                let unit = match &hit.lid {
                    Some(number) => format!("lid {number}"),
                    None if hit.unit == "lid" => "lid".to_string(),
                    None => "article".to_string(),
                };

                writeln!(
                    out,
                    "{:.3}  {:<11} {:<8} {}  {}",
                    hit.distance,
                    hit.source,
                    unit,
                    hit.law_id,
                    hit.label.as_deref().unwrap_or(&hit.number),
                )?;
            }

            caveats(e.caveats.as_slice(), out)
        }

        Answer::Versions(e) => {
            for version in data(e) {
                let held = if version.held { "*" } else { " " };
                writeln!(
                    out,
                    "{held} {}  {}",
                    version.version,
                    window(&version.in_force)
                )?;
            }
            writeln!(out, "\n* = text held locally")?;
            caveats(e.caveats.as_slice(), out)
        }

        Answer::Articles(e) => {
            bound(e, out)?;
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
                if let Some(act) = &article.amended_by {
                    let effect = article.effect.as_deref().unwrap_or("wijziging");
                    writeln!(out, "in force through {act} ({effect})")?;
                }
                writeln!(out)?;
            }
            caveats(e.caveats.as_slice(), out)
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
                window(&changes.from.in_force),
                changes.to.version,
                window(&changes.to.in_force)
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

            caveats(e.caveats.as_slice(), out)
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
            caveats(e.caveats.as_slice(), out)
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
            caveats(e.caveats.as_slice(), out)
        }

        Answer::Appeal(e) => {
            let Some(status) = &e.data else { return Ok(()) };
            writeln!(out, "{}", appeal(status))?;
            caveats(e.caveats.as_slice(), out)
        }

        Answer::PublicationHits(e) => {
            for hit in data(e) {
                writeln!(out, "{}  {}", hit.id, hit.title.as_deref().unwrap_or(""))?;
                if let Some(dossier) = &hit.dossier {
                    writeln!(out, "  dossier {dossier}")?;
                }
            }
            caveats(e.caveats.as_slice(), out)
        }

        Answer::CaptureHits(e) => {
            let hits = data(e);
            if hits.is_empty() {
                // Said in words, because a blank screen reads as a failure and
                // this is an answer: the archive holds nothing here.
                writeln!(out, "the archive holds no captures for this address")?;
            }

            for hit in hits {
                // The status is the column that decides whether a capture is
                // worth citing, so it goes next to the date and not in a note.
                let status = match hit.archive_status {
                    Some(code) => code.to_string(),
                    None => "withheld".to_string(),
                };
                writeln!(
                    out,
                    "{}  {status:>8}  {}{}",
                    hit.captured_at.format("%Y-%m-%d %H:%M:%S"),
                    hit.url,
                    if hit.revisit {
                        "  (unchanged since an earlier capture)"
                    } else {
                        ""
                    },
                )?;
            }
            caveats(&e.caveats, out)
        }

        Answer::Capture(e) => {
            let Some(capture) = &e.data else {
                return Ok(());
            };

            // Never "as of today". The date an archive answer speaks for is the
            // date the capture was taken.
            writeln!(out, "{}", capture.original_url)?;
            writeln!(
                out,
                "as the archive recorded it on {}",
                capture.captured_at.format("%Y-%m-%d %H:%M:%S UTC")
            )?;
            writeln!(out, "cite: {}", capture.url)?;

            match capture.archive_status {
                Some(200) => {}
                Some(code) => writeln!(
                    out,
                    "the site answered the archive with HTTP {code}, so this capture holds \
                     that response and not the page"
                )?,
                None => {}
            }

            if capture.verified == Some(DigestCheck::Matches) {
                writeln!(out, "verified against the archive's own digest")?;
            }

            match &capture.content {
                Some(text) => {
                    writeln!(out)?;
                    writeln!(out, "{text}")?;
                }
                None => writeln!(
                    out,
                    "\nnot text; the capture is held as bytes and a digest{}",
                    capture
                        .mime_type
                        .as_deref()
                        .map(|m| format!(" ({m})"))
                        .unwrap_or_default()
                )?,
            }

            caveats(&e.caveats, out)
        }

        Answer::Updated(e) => {
            for law in data(e) {
                match law.outcome {
                    "current" => writeln!(out, "{}  current", law.id)?,
                    "updated" => writeln!(
                        out,
                        "{}  {} -> {}  ({} articles)",
                        law.id,
                        law.from.as_deref().unwrap_or("new"),
                        law.to.as_deref().unwrap_or("?"),
                        law.articles.unwrap_or(0)
                    )?,
                    _ => writeln!(
                        out,
                        "{}  FAILED  {}",
                        law.id,
                        law.error.as_deref().unwrap_or("unknown error")
                    )?,
                }
            }
            Ok(())
        }

        Answer::Verified(e) => {
            let Some(v) = &e.data else { return Ok(()) };
            writeln!(
                out,
                "{} of {} documents still hash to their own name",
                v.intact, v.documents
            )
        }

        Answer::Stats(e) => {
            let Some(s) = &e.data else { return Ok(()) };
            writeln!(out, "{}", s.path)?;
            writeln!(out, "  laws          {}", s.laws)?;
            writeln!(out, "  versions      {}", s.versions)?;
            writeln!(out, "  articles      {}", s.articles)?;
            writeln!(out, "  decisions     {}", s.cases)?;
            writeln!(out, "  publications  {}", s.publications)?;
            writeln!(out, "  documents     {}", s.documents)
        }

        Answer::Path(e) => match &e.data {
            Some(path) => writeln!(out, "{path}"),
            None => Ok(()),
        },
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

fn bound<T>(envelope: &Envelope<T>, out: &mut impl Write) -> io::Result<()> {
    let (Some(date), Some(view)) = (envelope.as_of, envelope.view) else {
        return Ok(());
    };
    let window = match view {
        View::InForce => "as in force on",
        View::AsKnown => "as the register showed on",
    };
    writeln!(out, "{window} {date}\n")
}

fn window(w: &Window) -> String {
    match w.until {
        Some(until) => format!("{} to {until}", w.from),
        None => format!("{} onwards", w.from),
    }
}

fn appeal(status: &AppealStatus) -> String {
    match status {
        AppealStatus::Appealed { successors } => {
            format!("a later decision exists: {}", successors.join(", "))
        }
        // Deliberately not 'not appealed'. This says what the corpus holds on
        // the day it looked, and nothing about whether an appeal was brought.
        AppealStatus::NoPublishedSuccessor { as_of } => {
            format!("no published successor known, as of {}", as_of.date_naive())
        }
        AppealStatus::Unknown => "unknown, this decision is not in the corpus".to_string(),
    }
}

fn caveats(caveats: &[Caveat], out: &mut impl Write) -> io::Result<()> {
    if caveats.is_empty() {
        return Ok(());
    }

    writeln!(out)?;
    for caveat in caveats {
        let line = match caveat {
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
            Caveat::UpstreamDigestDiffers { published, ours } => format!(
                "note: these bytes do not match the digest the publisher's manifest states. \
                 Published {}…, ours {}…. The register is inconsistent about what it hashes, \
                 so this is upstream disagreeing with itself rather than the text being wrong, \
                 and it cannot be settled from here.",
                &published[..published.len().min(16)],
                &ours[..ours.len().min(16)],
            ),
            Caveat::UpstreamDigestAbsent => {
                "note: the manifest published no hash for this rendition, so these bytes rest \
                 on bron's own capture alone"
                    .to_string()
            }
            Caveat::LanesUnsearched { lanes } => format!(
                "note: {} hold no vectors and were not searched, so this answer covers less \
                 than the question did",
                lanes.join(" and "),
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
            // Deliberately does not say which one to use. That is transitional
            // law, and naming a winner here would be a legal judgement wearing
            // a lookup's clothes.
            Caveat::WindowsDisagree { in_force, as_known } => format!(
                "note: the two windows disagree for this date. In force: {in_force}. \
                 As the register showed it: {as_known}. Which governs is a question of \
                 transitional law that bron does not answer."
            ),
            Caveat::NoZichtWindow => {
                "note: the manifest carries no zicht dates, so this question cannot be \
                 answered against that window"
                    .to_string()
            }
            Caveat::DigestUnverifiable => {
                "note: the index recorded no digest for this capture, so these bytes could \
                 not be checked against what the crawl stored"
                    .to_string()
            }
            Caveat::DecodedAs { encoding, declared } => {
                format!("note: read as {encoding}, {declared}")
            }
            Caveat::LossyDecoding => {
                "note: bytes that could not be decoded were replaced, so the text is not a \
                 faithful reading of the capture"
                    .to_string()
            }
            Caveat::Revisit => {
                "note: the archive stored no new bytes for this capture and replayed an \
                 earlier one, so the content is older than the capture date"
                    .to_string()
            }
            Caveat::ArchiveStatusUnknown => {
                "note: the archive does not say what the site answered, which is what it \
                 records for a crawl excluded by robots.txt. There is a capture; what it \
                 returned is withheld."
                    .to_string()
            }
            // Said out loud because a tool that can cause the evidence it then
            // cites should never leave the reader to work out which happened.
            Caveat::RequestedByBron => {
                "note: bron asked the archive for this capture; it is not one the archive \
                 already held"
                    .to_string()
            }
            Caveat::MoreCaptures => {
                "note: the archive held more captures than bron asked for, so this list is \
                 not everything"
                    .to_string()
            }
        };
        writeln!(out, "{line}")?;
    }
    Ok(())
}
