//! What each command does.
//!
//! Every arm returns an [`Answer`]. Nothing here prints: that happens once, at
//! the edge, so the two output formats cannot describe different things.

use std::io::{self, Write};

use chrono::Utc;
use serde::Serialize;

use bron::error::{Fault, Outcome, exit};
use bron::service::{self, Answered, Service};
use bron::transport::Client;
use bron::wire::{self, Envelope};

use crate::cli::{Case, Cli, Command, DossierCmd, Law, Moment, Publication, QuoteCmd};
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
    History(Box<Answered<service::History>>),
    About(Box<Answered<service::Regulation>>),
    Amendments(Box<Answered<service::Amendments>>),
    Relations {
        answered: Box<Answered<service::Relations>>,
        /// The article the question named, so an empty answer can say how the
        /// register states a relation for one.
        article: Option<String>,
    },
    Parts {
        answered: Box<Answered<service::Parts>>,
        /// See `Articles::service`.
        service: Service,
    },
    Contents(Box<Answered<service::Contents>>),
    Published(Answered<Vec<service::Expression>>),
    Feed(Answered<service::Feed>),
    Upcoming(Answered<Vec<service::Upcoming>>),
    Changes {
        answered: Box<Answered<service::Changes>>,
        /// Whether --kind or --article narrowed the list, which the service's
        /// summary does not follow.
        narrowed: bool,
        /// Where each side's document is served, for the source line a reader
        /// fetches. Absent where the service holds no text for it.
        from_url: Option<String>,
        to_url: Option<String>,
    },
    Articles {
        answered: Box<Answered<service::Articles>>,
        /// The service this run read, which serves the document behind each
        /// source line for a reader to fetch: the rendition's, or an annex an
        /// act of the EU publishes apart.
        service: Service,
        /// The article the question named, so an answer with none can say
        /// what was asked.
        number: Option<String>,
        /// The lid the question named, which the service narrowed the answer
        /// to. Carried so a heading says which lid where the service's label
        /// for the article is absent and cannot.
        paragraph: Option<String>,
        /// The item a person asked to read. See [`item`].
        item: Option<String>,
        /// Whether the question asked only for what is not yet applied.
        not_commenced: bool,
    },
    Decision {
        answered: Box<Answered<service::DecisionParagraphs>>,
        /// See `Articles::service`: of a case of the EU it serves each
        /// document of the package.
        service: Service,
        /// The offset the question named, so a cut page can say where the next
        /// one starts.
        offset: Option<String>,
        /// The rechtsoverweging the question named, which that offset counts
        /// within.
        number: Option<String>,
    },
    DecisionRelations(Box<Answered<service::DecisionRelations>>),
    DecisionHits(Answered<service::DecisionSearch>),
    ArticleDecisions {
        answered: Box<Answered<service::ArticleDecisions>>,
        /// The offset the question named, so a cut page can say where the next
        /// one starts.
        offset: Option<String>,
    },
    RecentDecisions {
        answered: Box<Answered<service::RecentDecisions>>,
        /// See `ArticleDecisions::offset`.
        offset: Option<String>,
    },
    PublicationHits(Answered<service::PublicationSearch>),
    Publication(Box<Answered<service::Publication>>),
    Dossier(Box<Answered<service::DossierPapers>>),
    Quotes(Box<Answered<service::QuoteVerdicts>>),
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
            Self::History(a) => sent(out, &a.envelope),
            Self::About(a) => sent(out, &a.envelope),
            Self::Amendments(a) => sent(out, &a.envelope),
            Self::Relations { answered, .. } => sent(out, &answered.envelope),
            Self::Parts { answered, .. } => sent(out, &answered.envelope),
            Self::Contents(a) => sent(out, &a.envelope),
            Self::Published(a) => sent(out, &a.envelope),
            Self::Feed(a) => sent(out, &a.envelope),
            Self::Upcoming(a) => sent(out, &a.envelope),
            Self::Changes { answered, .. } => sent(out, &answered.envelope),
            Self::Articles { answered, .. } => sent(out, &answered.envelope),
            Self::Decision { answered, .. } => sent(out, &answered.envelope),
            Self::DecisionRelations(a) => sent(out, &a.envelope),
            Self::DecisionHits(a) => sent(out, &a.envelope),
            Self::ArticleDecisions { answered, .. } => sent(out, &answered.envelope),
            Self::RecentDecisions { answered, .. } => sent(out, &answered.envelope),
            Self::PublicationHits(a) => sent(out, &a.envelope),
            Self::Publication(a) => sent(out, &a.envelope),
            Self::Dossier(a) => sent(out, &a.envelope),
            Self::Quotes(a) => sent(out, &a.envelope),
            Self::Blob { envelope, .. } => write(out, envelope),
        }
    }

    /// The exit code of an answer the service gave: 0, or for `quote check`
    /// 6 where a quotation is not found where it cites.
    pub fn exit(&self) -> u8 {
        match self {
            Self::Quotes(a) if a.data.quotes.iter().any(|q| q.verdict != "quote_found") => {
                exit::NOT_QUOTED
            }
            _ => exit::OK,
        }
    }

    pub fn render(&self, out: &mut impl Write, err: &mut impl Write) -> io::Result<()> {
        render::answer(self, out, err)
    }
}

pub fn execute(cli: &Cli) -> Outcome<Answer> {
    // Resolved once, so every fetch in a run takes the same route.
    let service = Service::resolve()?;

    match &cli.command {
        Command::Law(law) => run_law(law, &service),
        Command::Case(case) => run_case(case, &service),
        Command::Publication(pub_) => run_publication(pub_, &service),
        Command::Dossier(dossier) => run_dossier(dossier, &service),
        Command::Quote(quote) => run_quote(quote, &service),
        Command::Find {
            query,
            regulation,
            jurisdiction,
            limit,
            moment,
        } => run_find(
            &query.join(" "),
            regulation,
            jurisdiction.as_deref(),
            limit.as_deref(),
            moment,
            &service,
        ),
        Command::Blob { digest, span } => run_blob(digest, *span, &service),
    }
}

/// The bytes a digest names, checked against it, and the span asked for.
fn run_blob(digest: &str, span: Option<(usize, usize)>, service: &Service) -> Outcome<Answer> {
    let fetched = service.blob(&client()?, digest)?;
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

    Ok(Answer::Blob {
        envelope: Envelope::ok(Box::new(wire::Blob {
            provenance: wire::Provenance::frozen(
                fetched.blob,
                fetched.url.clone(),
                length,
                read_at,
            ),
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
    // The BWB letters an item `a.`, an act of the EU `a)` in Dutch and `(a)`
    // in English, and a point inserted later in two words, `e bis)`.
    fn bare(label: &str) -> &str {
        label.trim_start_matches('(').trim_end_matches(['.', ')'])
    }
    let words: Vec<&str> = bare(letter.trim()).split_whitespace().collect();
    let wanted = words.join(" ");

    text.lines().find(|line| {
        let label: Vec<&str> = line.split_whitespace().take(words.len()).collect();
        !words.is_empty() && bare(&label.join(" ")) == wanted
    })
}

/// Answers which article, from a question or from a citation, by identifier.
///
/// No service is a refusal rather than an empty answer: reporting nothing
/// found would record an absence bron never established, which is the
/// distinction the whole contract rests on. A 404 read as 'nothing matched'
/// would record that same absence, and a citation that resolves to nothing is
/// exactly such a 404, which is why this reads the service's own envelope like
/// every other call here.
fn run_find(
    query: &str,
    regulation: &[String],
    jurisdiction: Option<&str>,
    limit: Option<&str>,
    moment: &Moment,
    service: &Service,
) -> Outcome<Answer> {
    let (as_of, as_known_on) = moment.coordinates();

    service
        .search(
            &client()?,
            &service::SearchRequest {
                q: query,
                regulation: (!regulation.is_empty()).then(|| regulation.join(",")),
                jurisdiction,
                limit,
                as_of,
                as_known_on,
            },
        )
        .map(Answer::Found)
}

// -- wetgeving -----------------------------------------------------------

fn run_law(law_cmd: &Law, service: &Service) -> Outcome<Answer> {
    let client = client()?;

    match law_cmd {
        Law::Search {
            query,
            kind,
            limit,
            offset,
            moment,
        } => service
            .regulations(
                &client,
                &service::Named {
                    query: &query.join(" "),
                    kind: kind.as_deref(),
                    as_of: moment.as_of.as_deref(),
                    as_known_on: moment.as_known_on.as_deref(),
                    limit: limit.as_deref(),
                    offset: offset.as_deref(),
                },
            )
            .map(Answer::LawHits),

        Law::Amendments { id, limit, offset } => service
            .amendments(&client, id, limit.as_deref(), offset.as_deref())
            .map(|answered| Answer::Amendments(Box::new(answered))),

        Law::Relations {
            id,
            kind,
            article,
            limit,
            offset,
            moment,
        } => {
            let (as_of, as_known_on) = moment.coordinates();

            service
                .relations(
                    &client,
                    id,
                    &service::Relating {
                        as_of,
                        as_known_on,
                        kind: kind.as_deref(),
                        article: article.as_deref(),
                        limit: limit.as_deref(),
                        offset: offset.as_deref(),
                    },
                )
                .map(|answered| Answer::Relations {
                    answered: Box::new(answered),
                    article: article.clone(),
                })
        }

        Law::Decisions {
            id,
            article,
            court,
            from,
            to,
            limit,
            offset,
        } => service
            .article_decisions(
                &client,
                id,
                article,
                &service::Citing {
                    court: court.as_deref(),
                    from: from.as_deref(),
                    to: to.as_deref(),
                    limit: limit.as_deref(),
                    offset: offset.as_deref(),
                },
            )
            .map(|answered| Answer::ArticleDecisions {
                answered: Box::new(answered),
                offset: offset.clone(),
            }),

        Law::About { id } => service
            .regulation(&client, id)
            .map(|answered| Answer::About(Box::new(answered))),

        Law::History {
            id,
            article,
            kind,
            as_known_on,
        } => service
            .history(
                &client,
                id,
                article,
                kind.as_deref(),
                as_known_on.as_deref(),
            )
            .map(|answered| Answer::History(Box::new(answered))),

        Law::Versions { id } => service.expressions(&client, id).map(Answer::Versions),

        Law::Parts {
            id,
            kind,
            number,
            position,
            paragraph,
            lang,
            raw,
            not_commenced,
            limit,
            offset,
            moment,
        } => {
            let (as_of, as_known_on) = moment.coordinates();

            service
                .parts(
                    &client,
                    id,
                    &service::Units {
                        as_of,
                        as_known_on,
                        kind: kind.as_deref(),
                        number: number.as_deref(),
                        paragraph: paragraph.as_deref(),
                        position: position.as_deref(),
                        lang: lang.as_deref(),
                        raw: *raw,
                        not_commenced: *not_commenced,
                        limit: limit.as_deref(),
                        offset: offset.as_deref(),
                    },
                )
                .map(|answered| Answer::Parts {
                    answered: Box::new(answered),
                    service: service.clone(),
                })
        }

        Law::Contents { id, moment } => {
            let (as_of, as_known_on) = moment.coordinates();

            service
                .contents(&client, id, as_of, as_known_on)
                .map(|answered| Answer::Contents(Box::new(answered)))
        }

        Law::Published {
            from,
            to,
            known_from,
            known_to,
            kind,
            regulation,
            limit,
            offset,
        } => service
            .published(
                &client,
                &service::Published {
                    from: from.as_deref(),
                    to: to.as_deref(),
                    known_from: known_from.as_deref(),
                    known_to: known_to.as_deref(),
                    kind: kind.as_deref(),
                    regulation: regulation.as_deref(),
                    limit: limit.as_deref(),
                    offset: offset.as_deref(),
                },
            )
            .map(Answer::Published),

        Law::Feed {
            since,
            regulation,
            limit,
        } => service
            .feed(
                &client,
                since.as_deref(),
                limit.as_deref(),
                regulation.as_deref(),
            )
            .map(Answer::Feed),

        Law::Upcoming {
            from,
            to,
            kind,
            limit,
            offset,
            moment,
        } => {
            let (as_of, as_known_on) = moment.coordinates();

            service
                .upcoming(
                    &client,
                    &service::Waiting {
                        as_of,
                        as_known_on,
                        from: from.as_deref(),
                        to: to.as_deref(),
                        kind: kind.as_deref(),
                        limit: limit.as_deref(),
                        offset: offset.as_deref(),
                    },
                )
                .map(Answer::Upcoming)
        }

        Law::Changes {
            id,
            from,
            to,
            all,
            kind,
            article,
            lang,
            as_known_on,
            raw,
        } => service
            .changes(
                &client,
                id,
                &service::Compare {
                    from: from.as_deref(),
                    to: to.as_deref(),
                    as_known_on: as_known_on.as_deref(),
                    kind: kind.as_deref(),
                    article: article.as_deref(),
                    lang: lang.as_deref(),
                    all: *all,
                    raw: *raw,
                },
            )
            .map(|answered| Answer::Changes {
                from_url: served(service, &answered.data.from),
                to_url: served(service, &answered.data.to),
                answered: Box::new(answered),
                narrowed: kind.is_some() || article.is_some(),
            }),

        Law::Show {
            id,
            article: wanted,
            paragraph,
            item: letter,
            lang,
            moment,
            raw,
            not_commenced,
        } => {
            let (as_of, as_known_on) = moment.coordinates();

            let answered = service.articles(
                &client,
                id,
                &service::Ask {
                    as_of,
                    as_known_on,
                    number: wanted.as_deref(),
                    paragraph: paragraph.as_deref(),
                    lang: lang.as_deref(),
                    raw: *raw,
                    not_commenced: *not_commenced,
                },
            )?;

            Ok(Answer::Articles {
                answered: Box::new(answered),
                service: service.clone(),
                number: wanted.clone(),
                paragraph: paragraph.clone(),
                item: letter.clone(),
                not_commenced: *not_commenced,
            })
        }
    }
}

// -- rechtspraak ---------------------------------------------------------

fn run_case(case_cmd: &Case, service: &Service) -> Outcome<Answer> {
    let client = client()?;

    match case_cmd {
        Case::Show {
            ecli,
            raw,
            lang,
            limit,
            offset,
            number,
        } => service
            .decision(
                &client,
                ecli,
                &service::RulingText {
                    raw: *raw,
                    lang: lang.as_deref(),
                    number: number.as_deref(),
                    limit: limit.as_deref(),
                    offset: offset.as_deref(),
                },
            )
            .map(|answered| Answer::Decision {
                answered: Box::new(answered),
                service: service.clone(),
                offset: offset.clone(),
                number: number.clone(),
            }),

        Case::Relations { ecli } => service
            .decision_relations(&client, ecli)
            .map(|answered| Answer::DecisionRelations(Box::new(answered))),

        Case::Recent {
            court,
            from,
            to,
            limit,
            offset,
            subject,
            since,
        } => service
            .recent_decisions(
                &client,
                &service::Listing {
                    citing: service::Citing {
                        court: court.as_deref(),
                        from: from.as_deref(),
                        to: to.as_deref(),
                        limit: limit.as_deref(),
                        offset: offset.as_deref(),
                    },
                    subject: subject.as_deref(),
                    since: since.as_deref(),
                },
            )
            .map(|answered| Answer::RecentDecisions {
                answered: Box::new(answered),
                offset: offset.clone(),
            }),

        Case::Search {
            query,
            court,
            from,
            to,
            form,
            register,
            limit,
            beyond_line,
        } => service
            .decision_search(
                &client,
                &service::DecisionQuery {
                    q: &query.join(" "),
                    court: court.as_deref(),
                    from: from.as_deref(),
                    to: to.as_deref(),
                    form: form.as_deref(),
                    register: register.as_deref(),
                    limit: limit.as_deref(),
                    beyond_line: *beyond_line,
                },
            )
            .map(Answer::DecisionHits),
    }
}

// -- officiële publicaties -----------------------------------------------

fn run_publication(cmd: &Publication, service: &Service) -> Outcome<Answer> {
    let client = client()?;

    match cmd {
        Publication::Search {
            query,
            collection,
            dossier,
            limit,
            next,
        } => {
            let q = query.join(" ");

            service
                .publication_search(
                    &client,
                    &service::PublicationQuery {
                        q: (!q.is_empty()).then_some(q.as_str()),
                        collection: collection.as_deref(),
                        dossier: dossier.as_deref(),
                        limit: limit.as_deref(),
                        next: next.as_deref(),
                    },
                )
                .map(Answer::PublicationHits)
        }

        Publication::Show {
            id,
            rendered,
            raw,
            limit,
            offset,
        } => service
            .publication(
                &client,
                id,
                limit.as_deref(),
                offset.as_deref(),
                *rendered,
                *raw,
            )
            .map(|answered| Answer::Publication(Box::new(answered))),
    }
}

// -- kamerstukdossiers ---------------------------------------------------

fn run_dossier(cmd: &DossierCmd, service: &Service) -> Outcome<Answer> {
    let client = client()?;

    match cmd {
        DossierCmd::Show {
            dossier,
            kind,
            limit,
            offset,
        } => service
            .dossier(
                &client,
                dossier,
                kind.as_deref(),
                limit.as_deref(),
                offset.as_deref(),
            )
            .map(|answered| Answer::Dossier(Box::new(answered))),
    }
}

// -- quotations ----------------------------------------------------------

fn run_quote(cmd: &QuoteCmd, service: &Service) -> Outcome<Answer> {
    let QuoteCmd::Check {
        text,
        regulation,
        article,
        paragraph,
        as_of,
        as_known_on,
        ecli,
        number,
        ordinal,
        file,
    } = cmd;

    let quotes = match (file, text) {
        (Some(file), _) => quotations(file)?,
        (None, Some(text)) => {
            let mut quote = serde_json::Map::new();
            quote.insert("text".into(), text.as_str().into());
            for (name, value) in [
                ("regulation", regulation),
                ("article", article),
                ("paragraph", paragraph),
                ("as_of", as_of),
                ("as_known_on", as_known_on),
                ("ecli", ecli),
                ("number", number),
            ] {
                if let Some(value) = value {
                    quote.insert(name.into(), value.as_str().into());
                }
            }
            // The service reads an ordinal as a JSON number, and a string of
            // digits is not one; what is no integer goes as typed, for the
            // service to refuse.
            if let Some(ordinal) = ordinal {
                let sent = ordinal
                    .parse::<i64>()
                    .map_or_else(|_| ordinal.as_str().into(), serde_json::Value::from);
                quote.insert("ordinal".into(), sent);
            }
            serde_json::Value::Array(vec![quote.into()])
        }
        // clap requires a text where no file is named.
        (None, None) => return Err(Fault::usage("quote check takes a text or --file")),
    };

    service
        .quotes(&client()?, &quotes)
        .map(|answered| Answer::Quotes(Box::new(answered)))
}

/// The quotations a file holds, as it holds them: bron reads the JSON to carry
/// it and judges nothing in it.
fn quotations(file: &std::path::Path) -> Outcome<serde_json::Value> {
    let read = if file == std::path::Path::new("-") {
        io::read_to_string(io::stdin())
    } else {
        std::fs::read_to_string(file)
    }
    .map_err(|e| Fault::usage(format!("cannot read {}: {e}", file.display())))?;

    serde_json::from_str(&read)
        .map_err(|e| Fault::usage(format!("{} holds no JSON: {e}", file.display())))
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

fn client() -> Outcome<Client> {
    Client::new().map_err(Fault::Local)
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

    #[test]
    fn an_item_of_an_act_of_the_eu_is_the_line_its_letter_opens() {
        let lid = "In het geval van een uitnodiging tot aankoop:\n\
                   a) de voornaamste kenmerken van het product;\n\
                   b) het geografische adres van de handelaar;";

        for spelling in ["b", "b)"] {
            assert_eq!(
                item(lid, spelling),
                Some("b) het geografische adres van de handelaar;")
            );
        }

        let english = "Processing shall be lawful only if:\n\
                       (a) the data subject has given consent;\n\
                       (b) processing is necessary for a contract;";
        for spelling in ["b", "(b)"] {
            assert_eq!(
                item(english, spelling),
                Some("(b) processing is necessary for a contract;")
            );
        }
    }

    #[test]
    fn a_point_inserted_later_is_the_line_its_two_words_open() {
        let lid = "e) de totale prijs;\n\
                   e bis) indien van toepassing, dat de prijs is gepersonaliseerd;\n\
                   f) de kosten voor communicatie op afstand;";

        for spelling in ["e bis", "e bis)", " e  bis "] {
            assert_eq!(
                item(lid, spelling),
                Some("e bis) indien van toepassing, dat de prijs is gepersonaliseerd;")
            );
        }
        assert_eq!(item(lid, "e"), Some("e) de totale prijs;"));
        assert_eq!(item(lid, ""), None);
    }
}
