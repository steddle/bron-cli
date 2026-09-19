//! What each command actually does.
//!
//! Every arm returns an [`Answer`]. Nothing here prints: that happens once, at
//! the edge, so the two output formats cannot describe different things.

use std::io::{self, Write};

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use bron::error::{Fault, Outcome, from_source};
use bron::ids::{BwbId, Ecli, PublicationId};
use bron::model::{AppealStatus, Direction, Relation};
use bron::service::{self, Answered, Service};
use bron::sources::{Client, FetchedCase, publications, rechtspraak};
use bron::wire::{self, Caveat, Envelope};

use crate::cli::{Case, Cli, Command, Law, Moment, Publication};
use crate::render;

/// One command's result, ready to be written out either way.
///
/// A command that is exactly one call to the service holds what the service
/// answered. Its JSON is the service's envelope as sent, and only the human
/// rendering reads anything out of it.
pub enum Answer {
    LawHits(Answered<Vec<service::Regulation>>),
    Found(Answered<service::SearchData>),
    Versions(Answered<Vec<service::Expression>>),
    Parts {
        answered: Box<Answered<service::Parts>>,
        /// See `Articles::blob_url`.
        blob_url: Option<String>,
    },
    Published(Answered<Vec<service::Expression>>),
    Feed(Answered<service::Feed>),
    Changes {
        answered: Box<Answered<service::Changes>>,
        /// Whether --kind or --article narrowed the list, which the service's
        /// summary does not follow.
        narrowed: bool,
        /// Where each side's document is served. See `Articles::blob_url`.
        from_url: Option<String>,
        to_url: Option<String>,
    },
    Articles {
        answered: Box<Answered<service::Articles>>,
        /// Where the rendition's document is served, for the source line a
        /// reader fetches. Absent where the service holds no text for it.
        blob_url: Option<String>,
        /// The lid the question named, which the service narrowed the answer
        /// to. Carried so a heading says which lid where the service's label
        /// for the article is absent and cannot.
        paragraph: Option<String>,
        /// The item a person asked to read. See [`item`].
        item: Option<String>,
    },
    CaseHits(Envelope<Vec<wire::CaseHit>>),
    Case(Envelope<Box<wire::Case>>),
    Appeal(Envelope<wire::AppealStatus>),
    PublicationHits(Envelope<Vec<wire::PublicationHit>>),
    Publication(Envelope<Box<wire::Publication>>),
    Blob {
        envelope: Envelope<Box<wire::Blob>>,
        /// The bytes handed over, the whole document or the span cut from
        /// it, exactly as they hashed.
        bytes: Vec<u8>,
    },
}

impl Answer {
    pub fn to_json(&self, out: &mut impl Write) -> io::Result<()> {
        fn write<T: Serialize>(out: &mut impl Write, value: &T) -> io::Result<()> {
            serde_json::to_writer(out, value).map_err(io::Error::other)
        }

        /// The caller ends every answer with one newline, so one the service
        /// already sent is left for it to write.
        fn sent(out: &mut impl Write, envelope: &str) -> io::Result<()> {
            out.write_all(envelope.strip_suffix('\n').unwrap_or(envelope).as_bytes())
        }

        match self {
            Self::LawHits(a) => sent(out, &a.envelope),
            Self::Found(a) => sent(out, &a.envelope),
            Self::Versions(a) => sent(out, &a.envelope),
            Self::Parts { answered, .. } => sent(out, &answered.envelope),
            Self::Published(a) => sent(out, &a.envelope),
            Self::Feed(a) => sent(out, &a.envelope),
            Self::Changes { answered, .. } => sent(out, &answered.envelope),
            Self::Articles { answered, .. } => sent(out, &answered.envelope),
            Self::CaseHits(e) => write(out, e),
            Self::Case(e) => write(out, e),
            Self::Appeal(e) => write(out, e),
            Self::PublicationHits(e) => write(out, e),
            Self::Publication(e) => write(out, e),
            Self::Blob { envelope, .. } => write(out, envelope),
        }
    }

    pub fn render(&self, out: &mut impl Write, err: &mut impl Write) -> io::Result<()> {
        render::answer(self, out, err)
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
        Command::Find {
            query,
            limit,
            moment,
        } => run_find(&query.join(" "), *limit, moment, today, quiet, &service),
        Command::Blob { digest, span } => run_blob(digest, *span, quiet, &service),
    }
}

/// The bytes a digest names, checked against it, and the span asked for.
fn run_blob(
    digest: &str,
    span: Option<(usize, usize)>,
    quiet: bool,
    service: &Service,
) -> Outcome<Answer> {
    let fetched = service.blob(&client(quiet)?, digest)?;
    let read_at = Utc::now();
    let length = fetched.bytes.len();

    let bytes = match span {
        Some((start, end)) if end > length => {
            return Err(Fault::usage(format!(
                "span {start}..{end} is outside the {length} bytes of {digest}"
            )));
        }
        Some((start, end)) => fetched.bytes[start..end].to_vec(),
        None => fetched.bytes.clone(),
    };

    let provenance = bron::sources::Provenance {
        blob: fetched.blob,
        url: fetched.url.clone(),
        byte_len: length,
    };

    Ok(Answer::Blob {
        envelope: Envelope::ok(Box::new(wire::Blob {
            provenance: wire::Provenance::frozen(&provenance, read_at),
            span: span.map(|(start, end)| wire::ByteRange { start, end }),
            text: String::from_utf8(bytes.clone()).ok(),
        })),
        bytes,
    })
}

/// Cuts the text of a lid down to the one item a letter opens.
///
/// A filter over the text the service answered, for a person reading a
/// terminal. No route answers an item: offsets are held per article and per
/// lid, so an item's would be an offset nobody measured, and the answer stays
/// the lid's. `--json` prints the envelope as sent for the same reason.
///
/// The letter opens the line because the parser puts it there, so asking for
/// `sub b` costs no schema and no second address space.
pub fn item<'a>(text: &'a str, letter: &str) -> Option<&'a str> {
    let wanted = letter.trim_end_matches('.');

    text.lines().find(|line| {
        line.split_whitespace()
            .next()
            .is_some_and(|first| first.trim_end_matches('.') == wanted)
    })
}

/// Answers which article, from a question or from a citation, by identifier.
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
    moment: &Moment,
    today: NaiveDate,
    quiet: bool,
    service: &Service,
) -> Outcome<Answer> {
    if query.trim().is_empty() {
        return Err(Fault::usage("say what to search for"));
    }

    let (as_of, as_known_on) = moment.coordinates(today);

    service
        .search(
            &client(quiet)?,
            &service::SearchRequest {
                q: query.to_string(),
                limit,
                as_of,
                as_known_on,
            },
        )
        .map(Answer::Found)
}

// -- wetgeving -----------------------------------------------------------

fn run_law(law_cmd: &Law, today: NaiveDate, quiet: bool, service: &Service) -> Outcome<Answer> {
    let client = client(quiet)?;

    match law_cmd {
        Law::Search { query, limit } => service
            .regulations(&client, &query.join(" "), *limit)
            .map(Answer::LawHits),

        Law::Versions { id } => service
            .expressions(&client, &parse_id(id)?)
            .map(Answer::Versions),

        Law::Parts {
            id,
            kind,
            number,
            position,
            raw,
            limit,
            offset,
            moment,
        } => {
            let (as_of, as_known_on) = moment.coordinates(today);

            service
                .parts(
                    &client,
                    &parse_id(id)?,
                    &service::Units {
                        as_of,
                        as_known_on,
                        kind: kind.as_deref(),
                        number: number.as_deref(),
                        position: *position,
                        raw: *raw,
                        limit: *limit,
                        offset: *offset,
                    },
                )
                .map(|answered| Answer::Parts {
                    blob_url: served(service, &answered.data.expression),
                    answered: Box::new(answered),
                })
        }

        Law::Published {
            from,
            to,
            known_from,
            known_to,
            kind,
            regulation,
            limit,
        } => {
            // Checked here rather than by clap, which can require a flag but
            // cannot say that one of four is enough.
            if from.is_none() && to.is_none() && known_from.is_none() && known_to.is_none() {
                return Err(Fault::usage(
                    "name a window: --from, --to, --known-from or --known-to".to_string(),
                ));
            }

            ordered(*from, *to, "--from", "--to")?;
            ordered(*known_from, *known_to, "--known-from", "--known-to")?;

            let law = regulation.as_deref().map(parse_id).transpose()?;

            service
                .published(
                    &client,
                    &service::Published {
                        from: *from,
                        to: *to,
                        known_from: *known_from,
                        known_to: *known_to,
                        kind: kind.as_deref(),
                        regulation: law.as_ref(),
                        limit: *limit,
                    },
                )
                .map(Answer::Published)
        }

        Law::Feed {
            since,
            regulation,
            limit,
        } => {
            let law = regulation.as_deref().map(parse_id).transpose()?;

            service
                .feed(&client, *since, *limit, law.as_ref())
                .map(Answer::Feed)
        }

        Law::Changes {
            id,
            from,
            to,
            all,
            kind,
            article,
            raw,
        } => {
            let law: BwbId = parse_id(id)?;
            let to = to.unwrap_or(today);
            if *from > to {
                return Err(Fault::usage(format!("--from {from} is after --to {to}")));
            }

            service
                .changes(
                    &client,
                    &law,
                    &service::Compare {
                        from: *from,
                        to,
                        kind: kind.as_deref(),
                        article: article.as_deref(),
                        all: *all,
                        raw: *raw,
                    },
                )
                .map(|answered| Answer::Changes {
                    from_url: served(service, &answered.data.from),
                    to_url: served(service, &answered.data.to),
                    answered: Box::new(answered),
                    narrowed: kind.is_some() || article.is_some(),
                })
        }

        Law::Show {
            id,
            article: wanted,
            paragraph,
            item: letter,
            moment,
            raw,
            not_commenced,
        } => {
            let law: BwbId = parse_id(id)?;
            let (as_of, as_known_on) = moment.coordinates(today);

            let answered = service.articles(
                &client,
                &law,
                &service::Ask {
                    as_of,
                    as_known_on,
                    number: wanted.as_deref(),
                    paragraph: paragraph.as_deref(),
                    raw: *raw,
                    not_commenced: *not_commenced,
                },
            )?;

            // A rendition that resolved and answered nothing. The service
            // raises no fault for text it does not hold or did not parse, and
            // a bare 'no articles' would have a caller stop looking: the
            // rendition is named, so it can still be fetched from the
            // publisher, and why it answered nothing is said where the service
            // said it.
            let expression = &answered.data.expression;
            if answered.data.articles.is_empty() {
                let rendition = format!("{law} rendition {}", expression.label);

                return Err(match (&expression.blob, &expression.unread_because) {
                    (None, _) => Fault::not_found("text held for", rendition),
                    (Some(_), Some(because)) => {
                        Fault::not_found("articles parsed from", format!("{rendition} ({because})"))
                    }
                    (Some(_), None) => {
                        let what = match (wanted, not_commenced) {
                            (Some(number), _) => format!("article {number} of {rendition}"),
                            (None, true) => {
                                format!("provisions of {rendition} published and not in force")
                            }
                            (None, false) => format!("articles of {rendition}"),
                        };
                        Fault::not_found("stored", what)
                    }
                });
            }

            // Refused here whichever way the answer prints, so an item nobody
            // prints is one exit code rather than two.
            if let Some(letter) = letter
                && let Some(article) = answered
                    .data
                    .articles
                    .iter()
                    .find(|article| item(&article.text(), letter).is_none())
            {
                return Err(Fault::not_found(
                    "item",
                    format!("{letter} of {law} {}", article.name()),
                ));
            }

            Ok(Answer::Articles {
                blob_url: served(service, &answered.data.expression),
                answered: Box::new(answered),
                paragraph: paragraph.clone(),
                item: letter.clone(),
            })
        }
    }
}

// -- rechtspraak ---------------------------------------------------------

/// One decision, and the moment bron read it.
///
/// Fetched every time. A decision is immutable once published, so the only
/// thing a stored copy would save is a request, and what it would cost is a
/// store that has to be kept, verified and migrated.
///
/// The moment is bron's own and is stamped once, because it is what an answer
/// about the absence of a successor is dated to and two reads of the clock
/// would date two halves of one answer differently.
fn decision(ecli: &Ecli, quiet: bool) -> Outcome<(FetchedCase, DateTime<Utc>)> {
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
fn appeal(relations: &[Relation], read_at: DateTime<Utc>) -> AppealStatus {
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

/// Where a rendition's document is served, which is the address a reader
/// fetches to check a span. Built here because the service this run reads is
/// resolved here and nowhere below.
fn served(service: &Service, expression: &service::Expression) -> Option<String> {
    expression
        .blob
        .as_deref()
        .and_then(|digest| service.blob_url(digest))
}

/// A window that runs backwards is a request nobody meant to make.
fn ordered(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    from_flag: &str,
    to_flag: &str,
) -> Outcome<()> {
    match (from, to) {
        (Some(from), Some(to)) if from > to => Err(Fault::usage(format!(
            "{from_flag} {from} is after {to_flag} {to}"
        ))),
        _ => Ok(()),
    }
}

fn parse_id<T: std::str::FromStr>(text: &str) -> Outcome<T>
where
    T::Err: std::fmt::Display,
{
    text.parse().map_err(|e| Fault::usage(format!("{e}")))
}

fn client(quiet: bool) -> Outcome<Client> {
    let client = Client::new().map_err(Fault::Local)?;
    if quiet {
        return Ok(client);
    }

    Ok(client.on_wait(|host, waited| {
        // A sweep that pauses must say so rather than look hung.
        eprintln!("bron: waiting {:.1}s for {host}", waited.as_secs_f32());
    }))
}

#[cfg(test)]
mod item_tests {
    use super::item;

    const LID: &str = "2. Een natuurlijke verbintenis bestaat:\n\
                       a. wanneer de wet dat bepaalt;\n\
                       b. wanneer een morele plicht bestaat.";

    #[test]
    fn an_item_is_the_line_its_letter_opens() {
        assert_eq!(
            item(LID, "b"),
            Some("b. wanneer een morele plicht bestaat.")
        );
    }

    /// The register writes `a.` and practice cites `sub a`, so both spellings
    /// name one item and neither may be the only one that works.
    #[test]
    fn the_letter_is_matched_with_or_without_its_full_stop() {
        for spelling in ["a", "a."] {
            assert_eq!(item(LID, spelling), Some("a. wanneer de wet dat bepaalt;"));
        }
    }

    #[test]
    fn an_item_nothing_opens_is_not_there() {
        assert_eq!(item(LID, "z"), None);
    }
}

#[cfg(test)]
mod appeal_tests {
    use super::appeal;
    use bron::model::{AppealStatus, Direction, Relation};
    use chrono::{DateTime, Utc};

    fn read_at() -> DateTime<Utc> {
        "2026-08-24T09:00:00Z".parse().unwrap()
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
