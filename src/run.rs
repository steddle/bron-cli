//! What each command actually does.
//!
//! Every arm returns an [`Answer`], which carries a wire envelope. Nothing here
//! prints: that happens once, at the edge, so the two output formats cannot
//! describe different things.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use bron::diff;
use bron::error::{Fault, Outcome, from_local, from_source};
use bron::ids::{BwbId, Ecli, PublicationId};
use bron::service::{self, Service};
use bron::sources::{Client, publications, rechtspraak, wayback};
use bron::wire::{self, Caveat, Envelope, View};

use crate::cli::{Archive, Case, Cli, Command, Law, Publication};
use crate::render;

/// One command's result, ready to be written out either way.
pub enum Answer {
    LawHits(Envelope<Vec<wire::LawHit>>),
    Found(Envelope<wire::Found>),
    Versions(Envelope<Vec<wire::VersionSummary>>),
    Changes(Envelope<Box<wire::Changes>>),
    Articles(Envelope<Vec<wire::Article>>),
    CaseHits(Envelope<Vec<wire::CaseHit>>),
    Case(Envelope<Box<wire::Case>>),
    Appeal(Envelope<wire::AppealStatus>),
    PublicationHits(Envelope<Vec<wire::PublicationHit>>),
    Publication(Envelope<Box<wire::Publication>>),
    CaptureHits(Envelope<Vec<wire::CaptureHit>>),
    Capture(Box<Envelope<Box<wire::Capture>>>),
}

impl Answer {
    pub fn to_json(&self, out: &mut impl Write) -> io::Result<()> {
        fn write<T: Serialize>(out: &mut impl Write, value: &T) -> io::Result<()> {
            serde_json::to_writer(out, value).map_err(io::Error::other)
        }

        match self {
            Self::LawHits(e) => write(out, e),
            Self::Found(e) => write(out, e),
            Self::Versions(e) => write(out, e),
            Self::Changes(e) => write(out, e),
            Self::Articles(e) => write(out, e),
            Self::CaseHits(e) => write(out, e),
            Self::Case(e) => write(out, e),
            Self::Appeal(e) => write(out, e),
            Self::PublicationHits(e) => write(out, e),
            Self::Publication(e) => write(out, e),
            Self::CaptureHits(e) => write(out, e),
            Self::Capture(e) => write(out, e),
        }
    }

    pub fn render(&self, out: &mut impl Write) -> io::Result<()> {
        render::answer(self, out)
    }
}

pub fn execute(cli: &Cli, quiet: bool) -> Outcome<Answer> {
    // Read once. A run that reads the clock twice can straddle midnight.
    let today = Utc::now().date_naive();

    // Resolved once, so every fetch in a run takes the same route.
    let service = Service::resolve()?;

    match &cli.command {
        Command::Law(law) => run_law(law, today, quiet, &service),
        Command::Case(case) => run_case(case, quiet),
        Command::Publication(pub_) => run_publication(pub_, quiet),
        Command::Archive(archive) => run_archive(archive, today, quiet),
        Command::Find {
            query,
            limit,
            moment,
        } => run_find(&query.join(" "), *limit, moment, today, quiet, &service),
    }
}

/// Cuts an answer down to one lid, and one item of it.
///
/// A filter over stored text rather than a level of its own. The letter opens
/// the line because the parser puts it there, so asking for `sub b` costs no
/// schema and no second address space.
///
/// Narrowing to a lid narrows what the answer points at with it: the excerpt,
/// the Juriconnect reference and the label all become the lid's, because an
/// answer about lid 2 that cites the article's bytes is coarser than the claim
/// it supports.
///
/// An item is where that stops. It has no byte range in this corpus: offsets
/// are held per article and per lid, and inventing one for an item would be an
/// offset nobody measured. So `--item` leaves the excerpt at the lid's, which
/// is a concession the answer makes plainly rather than quietly.
fn narrow(
    articles: Vec<wire::Article>,
    paragraph: Option<&str>,
    item: Option<&str>,
) -> Outcome<Vec<wire::Article>> {
    let Some(number) = paragraph else {
        return Ok(articles);
    };

    let mut narrowed = Vec::new();

    for mut article in articles {
        let Some(mut lid) = article
            .leden
            .iter()
            .find(|l| l.number.as_deref() == Some(number))
            .cloned()
        else {
            continue;
        };

        if let Some(letter) = item {
            let wanted = letter.trim_end_matches('.');
            let line = lid
                .content
                .lines()
                .find(|line| {
                    line.split_whitespace()
                        .next()
                        .is_some_and(|first| first.trim_end_matches('.') == wanted)
                })
                .ok_or_else(|| {
                    Fault::not_found(
                        "item",
                        format!(
                            "{letter} of {} article {} lid {number}",
                            article.law_id, article.number
                        ),
                    )
                })?;

            lid.content = line.to_string();
        }

        // The answer is now about a lid, so everything that says what it points
        // at has to be the lid's. Dutch practice cites 'art. 6:162 lid 2 BW',
        // and an answer to that which reports the article's byte range is one
        // level coarser than the claim it is supporting. The offsets were
        // measured and stored per lid; only the reporting threw them away.
        article.content = lid.content.clone();
        article.excerpt = lid.excerpt.clone();
        article.jci = lid.jci.clone();
        article.label = article.label.map(|label| format!("{label} lid {number}"));
        article.leden = vec![lid];
        narrowed.push(article);
    }

    if narrowed.is_empty() {
        return Err(Fault::not_found("lid", number));
    }

    Ok(narrowed)
}

/// Answers which article, from a question or from a citation, by identifier.
///
/// Two shapes come back and they stay apart. A question is ranked by a vector
/// and every hit carries the distance that ordered it; a citation is looked up
/// and carries none, because it arrived or it did not. Flattening them would
/// put a number beside a resolved citation that says the answer came about a
/// way it did not.
///
/// The one command with no publisher behind it. Every other route through the
/// service is an accelerant in front of a source of last resort; the shared
/// corpus has none, because it is the source. So no service is a refusal rather
/// than an empty answer: reporting nothing found would record an absence bron
/// never established, which is the distinction the whole contract rests on. A
/// 404 read as 'nothing matched' would record that same absence, and a citation
/// that resolves to nothing is exactly such a 404, which is why this reads the
/// service's own envelope like every other call here.
fn run_find(
    query: &str,
    limit: u32,
    moment: &crate::cli::Moment,
    today: NaiveDate,
    quiet: bool,
    service: &Service,
) -> Outcome<Answer> {
    if query.trim().is_empty() {
        return Err(Fault::usage("say what to search for"));
    }

    let (date, view) = moment.resolve(today);
    let (as_of, as_known_on) = match view {
        View::InForce => (Some(date.to_string()), None),
        View::AsKnown => (None, Some(date.to_string())),
    };

    let client = client(quiet)?;
    let answered = service.search(
        &client,
        &service::SearchRequest {
            q: query.to_string(),
            limit,
            as_of,
            as_known_on,
        },
    )?;

    let mut caveats = answered.caveats;

    let found = match answered.data {
        service::SearchData::Ranking { searched, hits } => wire::Found::Ranking(wire::Ranking {
            searched,
            hits: hits
                .into_iter()
                .map(|hit| wire::Hit {
                    source: hit.source,
                    unit: hit.unit,
                    law_id: hit.regulation_id,
                    version: hit.expression,
                    number: hit.number,
                    label: hit.label,
                    path: hit.path,
                    lid: hit.lid.and_then(|l| l.number),
                    content_hash: hit.content_hash,
                    distance: hit.distance,
                })
                .collect(),
        }),
        service::SearchData::Lookup { citation, resolved } => {
            // In the order they arrived. Every reading is answered, because
            // choosing between them is interpretation and bron does not do any.
            let read: Vec<_> = resolved.into_iter().map(cited).collect();

            caveats.extend(read.iter().filter_map(|(_, caveat)| caveat.clone()));

            wire::Found::Lookup(wire::Lookup {
                citation,
                resolved: read.into_iter().map(|(entry, _)| entry).collect(),
            })
        }
    };

    // Which lanes hold no vectors, and which readings came to nothing, are the
    // service's claims and travel as such. Working either out again here would
    // be a second place for the same fact to be stated.
    Ok(Answer::Found(
        Envelope::ok(found)
            .bound_to(date, view)
            .with_caveats(caveats),
    ))
}

/// One reading of a citation, in the shape a consumer parses, and the caveat it
/// earns where the service contradicted itself about it.
///
/// The service sends a flag and three fields that only belong with it set, so
/// the two can disagree in either direction. Both directions are said out loud
/// and neither is answered as though nothing were wrong. Both are answered as
/// not held, which is the half of the contradiction that claims least: a
/// missing hash cannot be invented, and one sent under `held: false` is not
/// something the service is willing to stand behind.
///
/// The caveat names the reading rather than the answer being dropped. A
/// citation reads more than one way and the other readings are still good; `8:4`
/// losing its Boek 8 answer because the Awb reading came back malformed would
/// be one broken row taking a sound one with it.
fn cited(entry: service::Cited) -> (wire::Cited, Option<Caveat>) {
    let (held, contradiction) = match (entry.held, entry.content_hash) {
        (true, Some(content_hash)) => (
            wire::Held::Text {
                label: entry.label,
                path: entry.path,
                content_hash,
            },
            None,
        ),
        (false, None) => (wire::Held::None, None),
        (true, None) => (
            wire::Held::None,
            Some(format!(
                "it holds {} rendition {} and did not say what artikel {} hashes to",
                entry.regulation_id, entry.expression, entry.number,
            )),
        ),
        (false, Some(_)) => (
            wire::Held::None,
            Some(format!(
                "it does not hold {} rendition {} and sent a hash for artikel {} anyway",
                entry.regulation_id, entry.expression, entry.number,
            )),
        ),
    };

    let caveat = contradiction.map(|stated| Caveat::ReadingContradicted {
        reading: entry.reading.clone(),
        stated,
    });

    (
        wire::Cited {
            reading: entry.reading,
            source: entry.source,
            law_id: entry.regulation_id,
            version: entry.expression,
            number: entry.number,
            held,
            caveats: entry.caveats.into_iter().map(Caveat::stated).collect(),
        },
        caveat,
    )
}

/// Where the document a rendition's spans point into came from.
///
/// One lookup per rendition, never one per article: a rendition names one blob
/// and every span in the answer points into those bytes.
fn provenance(
    client: &Client,
    service: &Service,
    law: &BwbId,
    expression: &service::Expression,
) -> Outcome<wire::Provenance> {
    // The rendition exists and its dates resolved; what is missing is its text.
    // Reporting that as no such rendition would have a caller stop looking, and
    // the publisher still serves it.
    let digest = expression.blob.as_deref().ok_or_else(|| {
        Fault::not_found(
            "text held for",
            format!("{law} rendition {}", expression.label),
        )
    })?;

    let blob = digest.parse().map_err(|e| Fault::Upstream {
        service: "bron",
        source: anyhow::anyhow!("{e}"),
    })?;
    let answered = service.blob_at(client, digest)?;

    Ok(wire::Provenance::served(
        blob,
        service.blob_url(digest),
        answered.data.size,
    ))
}

/// bron's reading of one article: its leden in order, each opened by the number
/// the register gives it.
///
/// Assembled here rather than taken from the article's own rendering, because
/// that string carries no lid numbers and the lid is the unit Dutch practice
/// cites: `art. 6:162 lid 2 BW`. It is rendering, so it lives where the answer
/// is built rather than on the shape the service sends.
fn content(article: &service::Article) -> String {
    leden(article).map(lid).collect::<Vec<_>>().join("\n")
}

/// The leden worth citing, which are the ones the register printed text in.
///
/// A repealed article is a single empty one. Keeping it would give every repeal
/// a lid that says nothing, and the article's own emptiness beside its
/// `vervallen` is the whole of what the register states.
fn leden(article: &service::Article) -> impl Iterator<Item = &service::Paragraph> {
    article
        .paragraphs
        .iter()
        .filter(|lid| !lid.text().is_empty())
}

/// One lid as bron prints it, opened by its number where it has one.
fn lid(lid: &service::Paragraph) -> String {
    match &lid.number {
        Some(number) => format!("{number}. {}", lid.text()),
        None => lid.text().to_string(),
    }
}

/// One article in the terms a comparison is made in.
fn compared_article(article: &service::Article) -> diff::Article {
    diff::Article {
        stam_id: article.stam_id.clone(),
        number: article.number.clone(),
        label: article.label.clone(),
        path: article.path.clone(),
        content: content(article),
        effect: article.effect.clone(),
    }
}

/// One article, in the shape a consumer parses.
fn article(
    article: &service::Article,
    law: &BwbId,
    version: &str,
    provenance: &wire::Provenance,
) -> wire::Article {
    wire::Article {
        law_id: law.to_string(),
        version: version.to_string(),
        number: article.number.clone(),
        label: article.label.clone(),
        title: article.title.clone(),
        path: article.path.clone(),
        content: content(article),
        excerpt: excerpt(article.span, article.raw.as_deref()),
        leden: leden(article)
            .map(|paragraph| wire::Lid {
                number: paragraph.number.clone(),
                content: lid(paragraph),
                excerpt: excerpt(paragraph.span, paragraph.raw.as_deref()),
                jci: paragraph.jci.clone(),
            })
            .collect(),
        amended_by: article.amended_by.clone(),
        effect: article.effect.clone(),
        status: article.status.clone(),
        in_force_from: article.in_force_from,
        jci: article.jci.clone(),
        provenance: provenance.clone(),
    }
}

fn excerpt(span: service::Span, raw: Option<&str>) -> wire::Excerpt {
    wire::Excerpt {
        start: span.start,
        end: span.end,
        text: raw.map(str::to_string),
    }
}

// -- wetgeving -----------------------------------------------------------

fn run_law(law_cmd: &Law, today: NaiveDate, quiet: bool, service: &Service) -> Outcome<Answer> {
    let client = client(quiet)?;

    match law_cmd {
        Law::Search { query, limit } => {
            let answered = service.regulations(&client, &query.join(" "), *limit)?;

            Ok(Answer::LawHits(
                Envelope::ok(
                    answered
                        .data
                        .into_iter()
                        .map(|hit| wire::LawHit {
                            id: hit.id,
                            title: hit.title,
                            law_type: hit.regulation_type,
                        })
                        .collect(),
                )
                .with_caveats(answered.caveats),
            ))
        }

        Law::Versions { id } => {
            let law: BwbId = parse_id(id)?;
            let answered = service.expressions(&client, &law)?;

            Ok(Answer::Versions(
                Envelope::ok(
                    answered
                        .data
                        .into_iter()
                        .map(|rendition| wire::VersionSummary {
                            version: rendition.label,
                            in_force: rendition.in_force.into(),
                            as_known: rendition.as_known.map(Into::into),
                            upstream_hash: rendition.upstream_hash,
                            byte_len: rendition.size,
                            held: rendition.held,
                        })
                        .collect(),
                )
                .with_caveats(answered.caveats),
            ))
        }

        Law::Changes {
            id,
            from,
            to,
            all,
            article: wanted,
            raw,
        } => {
            let law: BwbId = parse_id(id)?;
            let to = to.unwrap_or(today);
            if *from > to {
                return Err(Fault::usage(format!("--from {from} is after --to {to}")));
            }

            let ask = |on: NaiveDate| service::Ask {
                as_of: on,
                as_known_on: None,
                number: None,
                raw: *raw,
            };

            let earlier = service.articles(&client, &law, &ask(*from))?;
            let later = service.articles(&client, &law, &ask(to))?;

            let mut caveats = earlier.caveats;
            caveats.extend(later.caveats);

            let before = earlier.data;
            let after = later.data;
            let before_provenance = provenance(&client, service, &law, &before.expression)?;
            let after_provenance = provenance(&client, service, &law, &after.expression)?;

            let earlier_compared: Vec<diff::Article> =
                before.articles.iter().map(compared_article).collect();
            let later_compared: Vec<diff::Article> =
                after.articles.iter().map(compared_article).collect();
            let comparison = diff::compare(&earlier_compared, &later_compared);

            if !comparison.is_whole() {
                caveats.push(Caveat::ArticlesNotCompared {
                    unidentified_from: comparison.unidentified.before,
                    unidentified_to: comparison.unidentified.after,
                    repeated: comparison.repeated,
                });
            }

            let mut summary = wire::ChangeSummary::default();
            let mut articles = Vec::new();

            for change in comparison.changes {
                match change.change {
                    diff::Change::Unchanged => summary.unchanged += 1,
                    diff::Change::Amended => summary.amended += 1,
                    diff::Change::Added => summary.added += 1,
                    diff::Change::Repealed => summary.repealed += 1,
                    diff::Change::Withdrawn => summary.withdrawn += 1,
                }

                if !*all && !change.change.is_change() {
                    continue;
                }
                if wanted
                    .as_deref()
                    .is_some_and(|number| change.number != number)
                {
                    continue;
                }

                articles.push(wire::ArticleChange {
                    change: change.change.as_str(),
                    number: change.number,
                    label: change.label,
                    path: change.path,
                    before: change.before.map(|at| {
                        side(
                            &before.articles[at],
                            &before.expression.label,
                            &before_provenance,
                        )
                    }),
                    after: change.after.map(|at| {
                        side(
                            &after.articles[at],
                            &after.expression.label,
                            &after_provenance,
                        )
                    }),
                });
            }

            // Read off what the answer carries rather than off the flag that
            // asked for it. `--all` decides which articles are in the list at
            // all, and a caveat about the request cannot see that.
            if articles.iter().any(|change| {
                change
                    .before
                    .as_ref()
                    .is_some_and(wire::Side::reports_a_slice)
                    || change
                        .after
                        .as_ref()
                        .is_some_and(wire::Side::reports_a_slice)
            }) {
                caveats.push(Caveat::UnverifiedSlice);
            }

            Ok(Answer::Changes(
                Envelope::ok(Box::new(wire::Changes {
                    law_id: law.into_string(),
                    from: compared(&before.expression, before_provenance.blob),
                    to: compared(&after.expression, after_provenance.blob),
                    articles,
                    summary,
                }))
                .with_caveats(caveats),
            ))
        }

        Law::Show {
            id,
            article: wanted,
            paragraph,
            item,
            moment,
            raw,
        } => {
            let law: BwbId = parse_id(id)?;
            let (date, view) = moment.resolve(today);

            let answered = service.articles(
                &client,
                &law,
                &service::Ask {
                    as_of: date,
                    // One date names both coordinates of a zicht question: the
                    // text for a day, as the register had it that day. Sending
                    // only the second resolves against a rendition in force
                    // now, which for anything amended since is none at all.
                    as_known_on: (view == View::AsKnown).then_some(date),
                    number: wanted.as_deref(),
                    raw: *raw,
                },
            )?;

            let mut caveats = answered.caveats;
            let found = answered.data;
            let provenance = provenance(&client, service, &law, &found.expression)?;

            let articles: Vec<wire::Article> = found
                .articles
                .iter()
                .map(|a| article(a, &law, &found.expression.label, &provenance))
                .collect();

            if articles.is_empty() {
                let what = match wanted {
                    Some(number) => format!("article {number} of {law}"),
                    None => format!("articles of {law}"),
                };
                return Err(Fault::not_found("stored", what));
            }

            // Asking for one number and getting several means the number named
            // several. Both are answered, because picking would be bron
            // deciding which was meant, but the reader has to be told: two
            // headings reading "1" are otherwise indistinguishable from a
            // duplicated row.
            if let Some(number) = wanted.as_deref()
                && articles.len() > 1
            {
                caveats.push(Caveat::AmbiguousArticle {
                    number: number.to_string(),
                    paths: articles
                        .iter()
                        .map(|a| a.path.clone().unwrap_or_else(|| a.number.clone()))
                        .collect(),
                });
            }

            let articles = narrow(articles, paragraph.as_deref(), item.as_deref())?;

            // Read off what the answer carries rather than off the flag that
            // asked for it. Narrowing to a lid changes which excerpts travel,
            // and a caveat about the request cannot see that.
            if articles.iter().any(wire::Article::reports_a_slice) {
                caveats.push(Caveat::UnverifiedSlice);
            }

            Ok(Answer::Articles(
                Envelope::ok(articles)
                    .bound_to(date, view)
                    .with_caveats(caveats),
            ))
        }
    }
}

// -- rechtspraak ---------------------------------------------------------

/// One decision, and the moment bron read it.
///
/// Fetched every time. There is no corpus in front of this any more, and there
/// was never much point to one: a decision is immutable once published, so the
/// only thing a stored copy saved was a request, and the only thing it cost was
/// a store that had to be kept, verified and migrated.
///
/// The moment is bron's own and is stamped once, because it is what an answer
/// about the absence of a successor is dated to and two reads of the clock
/// would date two halves of one answer differently.
fn decision(ecli: &Ecli, quiet: bool) -> Outcome<(bron::sources::FetchedCase, DateTime<Utc>)> {
    let client = client(quiet)?;
    let fetched = rechtspraak::fetch(&client, ecli).map_err(|e| from_source("rechtspraak", e))?;

    Ok((fetched, Utc::now()))
}

/// What the decision's own relations can say about appeal or cassation.
///
/// Read off the document rather than out of a corpus, because that is where
/// upstream states them. Dated to when bron looked, because
/// `no_published_successor` is a claim about what the register held at that
/// moment and it began going stale as it was made.
fn appeal(
    relations: &[bron::model::Relation],
    read_at: DateTime<Utc>,
) -> bron::model::AppealStatus {
    use bron::model::{AppealStatus, Direction};

    let successors: Vec<Ecli> = relations
        .iter()
        .filter(|relation| relation.direction == Direction::Later)
        .map(|relation| relation.target.clone())
        .collect();

    if successors.is_empty() {
        AppealStatus::NoPublishedSuccessor { as_of: read_at }
    } else {
        AppealStatus::Appealed { successors }
    }
}

fn run_case(case_cmd: &Case, quiet: bool) -> Outcome<Answer> {
    match case_cmd {
        Case::Search {
            query,
            court,
            from,
            limit,
        } => {
            let client = client(quiet)?;
            let results = rechtspraak::search(
                &client,
                &rechtspraak::Query {
                    terms: &query.join(" "),
                    court: court.as_deref(),
                    from: *from,
                    limit: *limit,
                },
            )
            .map_err(|e| from_source("rechtspraak", e))?;

            let mut caveats = vec![Caveat::UnstableOrdering];
            if results.truncated {
                caveats.push(Caveat::Truncated { window: *limit });
            }

            Ok(Answer::CaseHits(
                Envelope::ok(results.hits.into_iter().map(Into::into).collect())
                    .with_caveats(caveats),
            ))
        }

        Case::Show { ecli } => {
            let ecli: Ecli = parse_id(ecli)?;
            let (decision, read_at) = decision(&ecli, quiet)?;

            Ok(Answer::Case(Envelope::ok(Box::new(wire::Case {
                ecli: decision.id.into_string(),
                court: decision.court,
                decided_on: decision.decided_on,
                published_on: decision.published_on,
                procedure_type: decision.procedure_type,
                summary: decision.summary,
                full_text: decision.full_text,
                appeal: appeal(&decision.relations, read_at).into(),
                relations: decision.relations.into_iter().map(Into::into).collect(),
                provenance: wire::Provenance::frozen(&decision.provenance, read_at),
            }))))
        }

        Case::Appeal { ecli } => {
            let ecli: Ecli = parse_id(ecli)?;
            let (decision, read_at) = decision(&ecli, quiet)?;

            Ok(Answer::Appeal(Envelope::ok(
                appeal(&decision.relations, read_at).into(),
            )))
        }
    }
}

// -- officiële publicaties -----------------------------------------------

fn run_publication(cmd: &Publication, quiet: bool) -> Outcome<Answer> {
    match cmd {
        Publication::Search {
            query,
            collection,
            dossier,
            limit,
        } => {
            let client = client(quiet)?;
            let hits = publications::search(
                &client,
                &publications::Query {
                    terms: &query.join(" "),
                    collection: (*collection).into(),
                    dossier: dossier.as_deref(),
                    limit: *limit,
                },
            )
            .map_err(|e| from_source("officiële bekendmakingen", e))?;

            Ok(Answer::PublicationHits(Envelope::ok(
                hits.into_iter().map(Into::into).collect(),
            )))
        }

        Publication::Show { id } => {
            let id: PublicationId = parse_id(id)?;
            let client = client(quiet)?;
            let doc = publications::fetch(&client, &id)
                .map_err(|e| from_source("officiële bekendmakingen", e))?;
            let read_at = Utc::now();

            // Not every record links a body, and the answer says which it is.
            // A digest is the whole of how this lane can be checked, so its
            // absence is a fact to state rather than a key to leave out.
            let body = match &doc.provenance {
                Some(source) => wire::Body::Linked {
                    provenance: wire::Provenance::frozen(source, read_at),
                },
                None => wire::Body::Absent,
            };

            Ok(Answer::Publication(Envelope::ok(Box::new(
                wire::Publication {
                    id: doc.id.into_string(),
                    title: doc.title,
                    dossier: doc.dossier,
                    published_on: doc.published_on,
                    body,
                },
            ))))
        }
    }
}

// -- shared --------------------------------------------------------------

/// One side of a comparison, with the span that places it.
fn side(article: &service::Article, version: &str, provenance: &wire::Provenance) -> wire::Side {
    wire::Side {
        version: version.to_string(),
        excerpt: excerpt(article.span, article.raw.as_deref()),
        content: content(article),
        amended_by: article.amended_by.clone(),
        effect: article.effect.clone(),
        provenance: provenance.clone(),
    }
}

fn compared(expression: &service::Expression, blob: bron::blob::BlobId) -> wire::ComparedVersion {
    wire::ComparedVersion {
        version: expression.label.clone(),
        in_force: expression.in_force.into(),
        blob,
    }
}

// -- the web archive -----------------------------------------------------

/// The service name every archive fault is attributed to.
const ARCHIVE: &str = "web.archive.org";

fn run_archive(command: &Archive, today: NaiveDate, quiet: bool) -> Outcome<Answer> {
    match command {
        Archive::Captures {
            url,
            from,
            to,
            limit,
            match_type,
        } => {
            let client = client(quiet)?;
            let query = wayback::IndexQuery {
                from: *from,
                to: *to,
                limit: Some(*limit),
                match_type: (*match_type).into(),
            };

            let found =
                wayback::captures(&client, url, &query).map_err(|e| from_source(ARCHIVE, e))?;

            let mut caveats = Vec::new();
            if found.truncated {
                caveats.push(Caveat::MoreCaptures);
            }
            if found
                .rows
                .iter()
                .any(|row| row.status == wayback::ArchiveStatus::Unknown)
            {
                caveats.push(Caveat::ArchiveStatusUnknown);
            }

            // An empty list here is an answer: the archive holds nothing for
            // this address. Only a query that could not be run is a fault.
            let hits: Vec<wire::CaptureHit> = found.rows.iter().map(Into::into).collect();

            Ok(Answer::CaptureHits(
                Envelope::archive(hits).with_caveats(caveats),
            ))
        }

        Archive::Show { url, as_of } => {
            let on = as_of.unwrap_or(today);
            let client = client(quiet)?;

            // The index decides which capture answers the question, and only
            // then are its bytes read. Asking a store first would answer "the
            // page in 2025" with whatever happens to be held, which for one
            // 2016 capture is a nine-year-old page returned in ten
            // milliseconds with nothing to say so.
            let row = wayback::capture_on(&client, url, on)
                .map_err(|e| from_source(ARCHIVE, e))?
                .ok_or_else(|| Fault::not_found("capture", format!("{url} on or before {on}")))?;

            let fetched =
                wayback::fetch_capture(&client, &row).map_err(|e| from_source(ARCHIVE, e))?;

            // Not one bron caused. This command reads the index and fetches
            // what it names; it never asks the archive to make anything.
            let capture = read_capture(fetched, false, Utc::now());
            let caveats = capture_caveats(&capture);
            let bound = capture.captured_at.date_naive();

            Ok(Answer::Capture(Box::new(
                Envelope::archive(Box::new(capture.into()))
                    // The date the archive took the capture, which is the only
                    // date this answer can speak for.
                    .dated(bound)
                    .with_caveats(caveats),
            )))
        }

        Archive::Capture {
            url,
            if_not_archived_within,
            screenshot,
            timeout,
        } => {
            let credentials =
                wayback::Credentials::from_env().map_err(|e| Fault::usage(e.to_string()))?;
            let client = client(quiet)?;

            // What the archive held before asking. A job id proves nothing, so
            // the capture reported afterwards has to postdate this to be one
            // bron caused.
            //
            // The newest row, not the history: a paged read of every capture can
            // stop short of the end, and a row missing because the read
            // truncated reads exactly like a row that was never there.
            let newest_before =
                wayback::newest_capture(&client, url, None).map_err(|e| from_source(ARCHIVE, e))?;

            let request = wayback::SaveRequest {
                url: url.clone(),
                if_not_archived_within_days: *if_not_archived_within,
                screenshot: *screenshot,
            };

            let job = wayback::submit(&client, &credentials, &request)
                .map_err(|e| from_source(ARCHIVE, e))?;

            if !quiet {
                eprintln!("bron: the archive is working on {job}");
            }

            let status = poll(
                &client,
                &credentials,
                &job,
                Duration::from_secs(*timeout),
                quiet,
            )?;

            let Some(stamp) = status.timestamp.clone() else {
                return Err(Fault::not_found("capture", url));
            };

            // Confirmation, not the job. The index is what establishes that a
            // capture exists, and re-reading it is also what makes a resubmit
            // that returned an old job visible as exactly that.
            let query = wayback::IndexQuery {
                from: Some(stamp.date()),
                ..Default::default()
            };
            let after =
                wayback::captures(&client, url, &query).map_err(|e| from_source(ARCHIVE, e))?;

            let row = after
                .rows
                .iter()
                .find(|row| *row.id.timestamp() == stamp)
                .ok_or_else(|| {
                    Fault::not_found(
                        "capture",
                        format!(
                            "{url}: the archive reported {stamp} but the index does not \
                                 carry it yet"
                        ),
                    )
                })?;

            // Whether bron caused this one, decided against what the archive
            // held before it was asked. A job id proves nothing: the archive
            // answers a resubmit with the job of the capture it already had.
            let fresh = wayback::caused_by_bron(newest_before.as_ref(), &stamp);
            let fetched =
                wayback::fetch_capture(&client, row).map_err(|e| from_source(ARCHIVE, e))?;

            let capture = read_capture(fetched, fresh, Utc::now());
            let caveats = capture_caveats(&capture);
            let bound = capture.captured_at.date_naive();

            Ok(Answer::Capture(Box::new(
                Envelope::archive(Box::new(capture.into()))
                    .dated(bound)
                    .with_caveats(caveats),
            )))
        }
    }
}

/// Waits on a job, reporting rather than looking hung.
///
/// The decision of what to do next is [`wayback::next`], which is pure. This is
/// only the part that sleeps, and it is the only part that cannot be tested.
fn poll(
    client: &Client,
    credentials: &wayback::Credentials,
    job: &str,
    budget: Duration,
    quiet: bool,
) -> Outcome<wayback::SaveStatus> {
    let started = Instant::now();

    for attempt in 1.. {
        let status =
            wayback::job_status(client, credentials, job).map_err(|e| from_source(ARCHIVE, e))?;

        match wayback::next(&status, attempt, started.elapsed(), budget) {
            wayback::Poll::Done(finished) => return Ok(*finished),
            wayback::Poll::GaveUp(reason) => {
                return Err(from_source(ARCHIVE, anyhow::anyhow!("{reason}")));
            }
            wayback::Poll::Wait(wait) => {
                if !quiet {
                    eprintln!(
                        "bron: still capturing, {}s elapsed",
                        started.elapsed().as_secs()
                    );
                }
                std::thread::sleep(wait);
            }
        }
    }

    unreachable!("next() bounds both attempts and elapsed time")
}

/// One capture, as bron read it.
///
/// `requested` is the caller's own knowledge and cannot be read off the
/// capture: only the command that asked the archive to make one knows that it
/// did, and it knows by comparing what the index held before against what came
/// back. See [`wayback::caused_by_bron`].
fn read_capture(
    fetched: bron::sources::FetchedCapture,
    requested: bool,
    read_at: DateTime<Utc>,
) -> bron::model::Capture {
    use bron::model::DigestCheck;
    use wayback::{Reading, Verified};

    let (content, encoding, encoding_from) = match fetched.reading {
        Reading::Text {
            text,
            encoding,
            declared,
            ..
        } => (
            Some(text),
            Some(encoding.to_string()),
            Some(format!("{declared:?}").to_lowercase()),
        ),
        // A capture of a PDF is evidence as bytes and a digest; running a text
        // extractor over it would invent a document.
        Reading::Binary => (None, None, None),
    };

    bron::model::Capture {
        captured_at: fetched.id.captured_at(),
        id: fetched.id,
        original_url: fetched.original,
        archive_status: fetched.status.code(),
        index_digest: fetched.digest,
        verified: Some(match fetched.verified {
            Verified::Matches => DigestCheck::Matches,
            Verified::Differs => DigestCheck::Differs,
            Verified::Absent => DigestCheck::Absent,
        }),
        is_revisit: fetched.is_revisit,
        mime_type: fetched.mime_type,
        encoding,
        encoding_from,
        requested,
        content,
        blob: Some(fetched.provenance.blob),
        fetched_at: read_at,
    }
}

/// What limits what may be concluded from one capture.
fn capture_caveats(capture: &bron::model::Capture) -> Vec<Caveat> {
    let mut caveats = Vec::new();

    if capture.archive_status.is_none() {
        caveats.push(Caveat::ArchiveStatusUnknown);
    }
    // Said whenever the replay could not be shown to be byte-identical to the
    // crawl, which covers both a digest that differs and one that is absent.
    // Neither is a failure and neither is a clean bill of health.
    if capture.verified != Some(bron::model::DigestCheck::Matches) {
        caveats.push(Caveat::DigestUnverifiable);
    }
    if capture.is_revisit {
        caveats.push(Caveat::Revisit);
    }
    if capture.requested {
        caveats.push(Caveat::RequestedByBron);
    }
    if let (Some(encoding), Some(declared)) = (&capture.encoding, &capture.encoding_from) {
        // Only worth saying when nothing authoritative declared it; a stated
        // charset that was honoured is not a limit on anything.
        if declared == "assumed" {
            caveats.push(Caveat::DecodedAs {
                encoding: encoding.clone(),
                declared: "nothing declared an encoding, so the web's historical default \
                           was assumed"
                    .to_string(),
            });
        }
    }

    caveats
}

fn parse_id<T: std::str::FromStr>(text: &str) -> Outcome<T>
where
    T::Err: std::fmt::Display,
{
    text.parse().map_err(|e| Fault::usage(format!("{e}")))
}

fn client(quiet: bool) -> Outcome<Client> {
    let client = Client::new().map_err(from_local)?;
    if quiet {
        return Ok(client);
    }

    Ok(client.on_wait(|host, waited| {
        // A sweep that pauses must say so rather than look hung.
        eprintln!("bron: waiting {:.1}s for {host}", waited.as_secs_f32());
    }))
}

#[cfg(test)]
mod reading_tests {
    use super::{compared_article, content, leden};
    use bron::service::{Article, Paragraph, Span};

    fn paragraph(number: Option<&str>, text: &str) -> Paragraph {
        Paragraph {
            number: number.map(str::to_string),
            rendered: Some(text.to_string()),
            raw: None,
            jci: None,
            span: Span { start: 0, end: 1 },
        }
    }

    fn article(leden: Vec<Paragraph>) -> Article {
        Article {
            number: "162".into(),
            label: None,
            title: None,
            path: None,
            stam_id: Some("1".into()),
            amended_by: None,
            effect: None,
            status: None,
            in_force_from: None,
            jci: None,
            span: Span { start: 0, end: 1 },
            raw: None,
            paragraphs: leden,
        }
    }

    /// The lid is the unit Dutch practice cites, and the service's own
    /// rendering of an article carries no lid numbers at all.
    #[test]
    fn an_article_reads_as_its_leden_each_opened_by_its_number() {
        let article = article(vec![
            paragraph(Some("1"), "Eerste lid."),
            paragraph(Some("2"), "Tweede lid."),
        ]);

        assert_eq!(content(&article), "1. Eerste lid.\n2. Tweede lid.");
    }

    /// An article printed without leden is one unnumbered unit, and opening it
    /// with a number the register never printed would be bron writing text.
    #[test]
    fn an_article_without_leden_reads_as_itself() {
        assert_eq!(
            content(&article(vec![paragraph(None, "Eén geheel.")])),
            "Eén geheel."
        );
    }

    /// A repealed article is a single empty lid. Keeping it would give every
    /// repeal a citable unit that says nothing.
    #[test]
    fn an_empty_lid_is_not_a_citable_unit() {
        let repealed = article(vec![paragraph(None, "")]);

        assert_eq!(leden(&repealed).count(), 0);
        assert_eq!(content(&repealed), "");
    }

    /// A comparison is made over what bron prints, and what bron prints opens
    /// each lid with its number. Two renditions that renumber a lid say
    /// different things about which rule is which, and comparing the service's
    /// unnumbered text alone would call that unchanged.
    #[test]
    fn a_comparison_reads_the_text_bron_prints() {
        let numbered = |number: &str| article(vec![paragraph(Some(number), "Dezelfde woorden.")]);

        assert_ne!(
            compared_article(&numbered("1")).content,
            compared_article(&numbered("2")).content
        );
    }
}

#[cfg(test)]
mod narrow_tests {
    use super::narrow;
    use bron::wire::{Article, Excerpt, Lid, Provenance};

    fn article(leden: &[(&str, &str)]) -> Article {
        Article {
            law_id: "BWBR0005289".into(),
            version: "2024-01-01_0".into(),
            number: "3".into(),
            label: Some("Artikel 3".into()),
            title: None,
            path: None,
            content: leden.iter().map(|(_, c)| *c).collect::<Vec<_>>().join("\n"),
            excerpt: Excerpt {
                start: 1000,
                end: 2000,
                text: None,
            },
            // Distinct spans per lid, because the whole point of narrowing is
            // that the answer stops pointing at the article's range.
            leden: leden
                .iter()
                .enumerate()
                .map(|(i, (number, content))| Lid {
                    number: Some((*number).into()),
                    content: (*content).to_string(),
                    excerpt: Excerpt {
                        start: 1100 + i as u32 * 100,
                        end: 1150 + i as u32 * 100,
                        text: None,
                    },
                    jci: Some(format!("jci1.3:c:BWBR0005289&artikel=3&lid={number}")),
                })
                .collect(),
            amended_by: None,
            effect: None,
            in_force_from: None,
            jci: None,
            status: None,
            provenance: Provenance::served(
                bron::blob::BlobId::of(b"<toestand/>"),
                "https://example.invalid".into(),
                1,
            ),
        }
    }

    fn items() -> Article {
        article(&[
            ("1", "1. Een natuurlijke verbintenis is niet afdwingbaar."),
            (
                "2",
                "2. Een natuurlijke verbintenis bestaat:\na. wanneer de wet dat bepaalt;\nb. wanneer een morele plicht bestaat.",
            ),
        ])
    }

    /// Dutch practice cites 'art. 6:162 lid 2 BW'. An answer to that which
    /// reports the article's byte range is one level coarser than the claim it
    /// supports, and the lid's range was measured and stored all along.
    #[test]
    fn a_lid_answer_points_at_the_lid_and_not_at_the_article() {
        let narrowed = narrow(vec![items()], Some("2"), None).unwrap();
        let answer = &narrowed[0];

        assert_eq!(answer.excerpt.start, 1200);
        assert_eq!(answer.excerpt.end, 1250);
        assert_eq!(
            answer.jci.as_deref(),
            Some("jci1.3:c:BWBR0005289&artikel=3&lid=2")
        );
        assert_eq!(answer.label.as_deref(), Some("Artikel 3 lid 2"));
    }

    /// An item has no byte range in this corpus, so the answer stays at the
    /// lid's rather than inventing one nobody measured.
    #[test]
    fn an_item_answer_points_at_the_lid_that_holds_it() {
        let narrowed = narrow(vec![items()], Some("2"), Some("b")).unwrap();

        assert_eq!(narrowed[0].excerpt.start, 1200);
        assert_eq!(narrowed[0].excerpt.end, 1250);
    }

    #[test]
    fn an_article_answered_whole_still_points_at_the_article() {
        let narrowed = narrow(vec![items()], None, None).unwrap();

        assert_eq!(narrowed[0].excerpt.start, 1000);
        assert_eq!(narrowed[0].label.as_deref(), Some("Artikel 3"));
    }

    #[test]
    fn without_a_lid_the_article_is_answered_whole() {
        assert_eq!(narrow(vec![items()], None, None).unwrap()[0].leden.len(), 2);
    }

    #[test]
    fn a_lid_narrows_to_that_lid_alone() {
        let narrowed = narrow(vec![items()], Some("1"), None).unwrap();

        assert_eq!(narrowed[0].leden.len(), 1);
        assert!(narrowed[0].content.starts_with("1. Een natuurlijke"));
    }

    #[test]
    fn an_item_narrows_to_the_line_it_letters() {
        let narrowed = narrow(vec![items()], Some("2"), Some("b")).unwrap();

        assert_eq!(narrowed[0].content, "b. wanneer een morele plicht bestaat.");
    }

    /// The register writes `a.` and practice cites `sub a`, so both spellings
    /// name one item and neither may be the only one that works.
    #[test]
    fn the_letter_is_matched_with_or_without_its_full_stop() {
        for spelling in ["a", "a."] {
            let narrowed = narrow(vec![items()], Some("2"), Some(spelling)).unwrap();
            assert_eq!(narrowed[0].content, "a. wanneer de wet dat bepaalt;");
        }
    }

    /// A lid or an item that is not there is a failure rather than an empty
    /// success: an empty answer cannot say whether nothing matched or no such
    /// thing was ever asked for.
    #[test]
    fn a_lid_that_does_not_exist_is_not_found() {
        assert!(narrow(vec![items()], Some("9"), None).is_err());
    }

    #[test]
    fn an_item_that_does_not_exist_is_not_found() {
        assert!(narrow(vec![items()], Some("2"), Some("z")).is_err());
    }

    /// The span stays the lid's. An item has no byte range in this corpus, and
    /// reporting a narrower one would be an offset nobody measured.
    #[test]
    fn narrowing_to_an_item_does_not_narrow_the_span() {
        let before = items().leden[1].excerpt.end;
        let narrowed = narrow(vec![items()], Some("2"), Some("b")).unwrap();

        assert_eq!(narrowed[0].leden[0].excerpt.end, before);
    }
}

#[cfg(test)]
mod archive_tests {
    use super::{appeal, capture_caveats, read_capture};
    use bron::blob::BlobId;
    use bron::ids::CaptureId;
    use bron::model::{AppealStatus, Captured, DigestCheck, Direction, Relation};
    use bron::sources::{FetchedCapture, Provenance, wayback};
    use bron::wire::Caveat;
    use chrono::{DateTime, Utc};

    fn read_at() -> DateTime<Utc> {
        "2026-08-24T09:00:00Z".parse().unwrap()
    }

    fn fetched(timestamp: &str, status: wayback::ArchiveStatus, body: &[u8]) -> FetchedCapture {
        // The key is the archive's, taken from the index row, so a fixture
        // writes one out rather than deriving one and calling it the same thing.
        let id = CaptureId::new(
            timestamp.parse().unwrap(),
            "com,steddle)/pricing".to_string(),
        );

        FetchedCapture {
            id,
            original: "https://steddle.com/pricing".to_string(),
            status,
            digest: Some(wayback::digest_of(body)),
            verified: wayback::Verified::Matches,
            is_revisit: false,
            mime_type: Some("text/html".to_string()),
            content_type: Some("text/html; charset=utf-8".to_string()),
            reading: wayback::Reading::Text {
                decompressed: false,
                text: String::from_utf8_lossy(body).into_owned(),
                encoding: "UTF-8",
                declared: wayback::Declared::Header,
                lossy: false,
            },
            provenance: Provenance {
                blob: BlobId::of(body),
                url: "https://web.archive.org/web/x/y".to_string(),
                byte_len: body.len(),
            },
        }
    }

    /// The capture is from January and it was read in August. An answer bound
    /// to the read date would date a 2024 page to today.
    #[test]
    fn a_capture_keeps_the_date_it_was_taken_apart_from_the_date_it_was_read() {
        let capture = read_capture(
            fetched(
                "20240101120000",
                wayback::ArchiveStatus::Answered(200),
                b"old",
            ),
            false,
            read_at(),
        );

        assert_eq!(
            capture.captured_at.date_naive(),
            "2024-01-01".parse::<chrono::NaiveDate>().unwrap()
        );
        assert_eq!(capture.fetched_at, read_at());
    }

    /// Not `No`: there is a capture and the archive will not say what it got.
    #[test]
    fn a_withheld_status_reads_as_unknown_rather_than_as_no() {
        let capture = read_capture(
            fetched("20240101120000", wayback::ArchiveStatus::Unknown, b"body"),
            false,
            read_at(),
        );

        assert_eq!(capture.archive_status, None);
        assert_eq!(capture.captured_the_page(), Captured::Unknown);
        assert!(capture_caveats(&capture).contains(&Caveat::ArchiveStatusUnknown));
    }

    /// A capture of a 404 is a faithful capture of an error page, and citing it
    /// as the page is the mistake this distinction exists to prevent.
    #[test]
    fn a_captured_error_page_reads_as_not_the_page() {
        let capture = read_capture(
            fetched(
                "20240101120000",
                wayback::ArchiveStatus::Answered(404),
                b"404",
            ),
            false,
            read_at(),
        );

        assert_eq!(capture.archive_status, Some(404));
        assert_eq!(capture.captured_the_page(), Captured::No);
    }

    /// Two different answers, kept apart. Only a match is positive evidence,
    /// and an index that recorded no digest is not a clean bill of health.
    #[test]
    fn a_capture_with_no_index_digest_is_not_read_as_a_verified_one() {
        let mut old = fetched(
            "20040101120000",
            wayback::ArchiveStatus::Answered(200),
            b"ancient",
        );
        old.digest = None;
        old.verified = wayback::Verified::Absent;

        let capture = read_capture(old, false, read_at());

        assert_eq!(capture.verified, Some(DigestCheck::Absent));
        assert_ne!(capture.verified, Some(DigestCheck::Matches));
        assert!(capture_caveats(&capture).contains(&Caveat::DigestUnverifiable));
    }

    /// A tool that can cause the evidence it then cites says which happened.
    /// Only the command that asked the archive to make a capture knows that it
    /// did, so this is the caller's own knowledge and not the capture's.
    #[test]
    fn a_capture_bron_asked_for_says_so() {
        let row = || {
            fetched(
                "20240101120000",
                wayback::ArchiveStatus::Answered(200),
                b"x",
            )
        };

        let found = read_capture(row(), false, read_at());
        assert!(!found.requested);
        assert!(!capture_caveats(&found).contains(&Caveat::RequestedByBron));

        let caused = read_capture(row(), true, read_at());
        assert!(caused.requested);
        assert!(capture_caveats(&caused).contains(&Caveat::RequestedByBron));
    }

    /// Not text at all. A capture of a PDF is evidence as bytes and a digest;
    /// running a text extractor over it would invent a document.
    #[test]
    fn a_capture_that_is_not_text_carries_bytes_and_a_digest_and_no_content() {
        let mut binary = fetched(
            "20240101120000",
            wayback::ArchiveStatus::Answered(200),
            b"%PDF",
        );
        binary.reading = wayback::Reading::Binary;
        binary.mime_type = Some("application/pdf".into());

        let capture = read_capture(binary, false, read_at());

        assert!(capture.content.is_none());
        assert!(capture.encoding.is_none());
        assert!(capture.blob.is_some());
    }

    fn relation(target: &str, direction: Direction) -> Relation {
        Relation {
            target: target.parse().unwrap(),
            kind: Some("cassatie".into()),
            direction,
        }
    }

    /// A forward link is the appeal. Read off the decision's own relations,
    /// which is where upstream states them.
    #[test]
    fn a_forward_relation_is_an_appeal() {
        let status = appeal(
            &[relation("ECLI:NL:HR:2016:2326", Direction::Later)],
            read_at(),
        );

        assert_eq!(
            status,
            AppealStatus::Appealed {
                successors: vec!["ECLI:NL:HR:2016:2326".parse().unwrap()],
            }
        );
    }

    /// A cassation decision points back at the hof. That says something was
    /// appealed *to* it, not that it was itself appealed.
    #[test]
    fn a_backward_relation_alone_is_not_an_appeal() {
        let status = appeal(
            &[relation("ECLI:NL:GHARL:2015:10068", Direction::Earlier)],
            read_at(),
        );

        assert!(matches!(status, AppealStatus::NoPublishedSuccessor { .. }));
    }

    /// The strongest true claim available, and deliberately not "was not
    /// appealed". It is dated to when bron looked, because it began going stale
    /// as it was made.
    #[test]
    fn no_forward_relation_is_dated_to_the_moment_bron_looked() {
        assert_eq!(
            appeal(&[], read_at()),
            AppealStatus::NoPublishedSuccessor { as_of: read_at() }
        );
    }
}

#[cfg(test)]
mod citation_tests {
    use super::cited;
    use bron::service;
    use bron::wire::{Caveat, Held};

    fn entry(held: bool, content_hash: Option<&str>) -> service::Cited {
        service::Cited {
            reading: "literal".into(),
            source: "wetgeving".into(),
            regulation_id: "BWBR0005537".into(),
            expression: "2026-08-15_0".into(),
            held,
            number: "8:4".into(),
            label: Some("Artikel 8:4".into()),
            path: None,
            content_hash: content_hash.map(str::to_string),
            caveats: Vec::new(),
        }
    }

    #[test]
    fn a_reading_the_service_holds_carries_what_resolves_it() {
        let (read, caveat) = cited(entry(true, Some("4ba2172e")));

        assert!(caveat.is_none());
        assert!(matches!(
            read.held,
            Held::Text { ref content_hash, .. } if content_hash == "4ba2172e"
        ));
    }

    #[test]
    fn a_reading_the_service_does_not_hold_carries_nothing_to_resolve() {
        let (read, caveat) = cited(entry(false, None));

        assert!(caveat.is_none());
        assert!(matches!(read.held, Held::None));
    }

    /// Both directions of the same disagreement, and both are said. Answering
    /// either as though nothing were wrong puts a reader one step from a
    /// citation nobody can check.
    #[test]
    fn either_half_of_a_contradiction_is_reported_and_claims_the_less() {
        for broken in [entry(true, None), entry(false, Some("4ba2172e"))] {
            let held = broken.held;
            let (read, caveat) = cited(broken);

            assert!(matches!(read.held, Held::None), "held was {held}");
            assert!(
                matches!(caveat, Some(Caveat::ReadingContradicted { ref reading, .. }) if reading == "literal"),
                "held was {held}, got {caveat:?}"
            );
        }
    }
}
