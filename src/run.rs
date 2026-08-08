//! What each command actually does.
//!
//! Every arm returns an [`Answer`], which carries a wire envelope. Nothing here
//! prints: that happens once, at the edge, so the two output formats cannot
//! describe different things.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use chrono::{NaiveDate, Utc};
use data_encoding::HEXLOWER;
use serde::Serialize;
use sha2::{Digest, Sha512};

use bron::blob::BlobStore;
use bron::db::{self, Db};
use bron::diff;
use bron::error::{Fault, Outcome, from_corpus, from_source};
use bron::ids::{BwbId, Ecli, PublicationId};
use bron::mirror::{self, Mirror, ResolvedVia};
use bron::paths;
use bron::sources::{
    Client, FetchedLaw, Source, VersionMeta, bwb, publications, rechtspraak, wayback,
};
use bron::wire::{self, Caveat, Envelope, View};

use crate::cli::{Archive, Case, Cli, Command, DbCommand, Law, Publication, UpdateArgs};
use crate::render;

/// One command's result, ready to be written out either way.
pub enum Answer {
    LawHits(Envelope<Vec<wire::LawHit>>),
    Ranking(Envelope<wire::Ranking>),
    Versions(Envelope<Vec<wire::VersionSummary>>),
    Changes(Envelope<Box<wire::Changes>>),
    Articles(Envelope<Vec<wire::Article>>),
    CaseHits(Envelope<Vec<wire::CaseHit>>),
    Case(Envelope<Box<wire::Case>>),
    Appeal(Envelope<wire::AppealStatus>),
    PublicationHits(Envelope<Vec<wire::PublicationHit>>),
    CaptureHits(Envelope<Vec<wire::CaptureHit>>),
    Capture(Box<Envelope<Box<wire::Capture>>>),
    Updated(Envelope<Vec<Updated>>),
    Verified(Envelope<wire::Verification>),
    Stats(Envelope<wire::Stats>),
    Path(Envelope<String>),
}

/// What one law did during an update.
#[derive(Debug, Serialize)]
pub struct Updated {
    pub id: String,
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub articles: Option<u32>,
    /// Present when this law failed. The batch continues either way, which is
    /// the one place a printed-only error is the right shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Answer {
    pub fn to_json(&self, out: &mut impl Write) -> io::Result<()> {
        fn write<T: Serialize>(out: &mut impl Write, value: &T) -> io::Result<()> {
            serde_json::to_writer(out, value).map_err(io::Error::other)
        }

        match self {
            Self::LawHits(e) => write(out, e),
            Self::Ranking(e) => write(out, e),
            Self::Versions(e) => write(out, e),
            Self::Changes(e) => write(out, e),
            Self::Articles(e) => write(out, e),
            Self::CaseHits(e) => write(out, e),
            Self::Case(e) => write(out, e),
            Self::Appeal(e) => write(out, e),
            Self::PublicationHits(e) => write(out, e),
            Self::CaptureHits(e) => write(out, e),
            Self::Capture(e) => write(out, e),
            Self::Updated(e) => write(out, e),
            Self::Verified(e) => write(out, e),
            Self::Stats(e) => write(out, e),
            Self::Path(e) => write(out, e),
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
    let mirror = Mirror::resolve(cli.direct);

    match &cli.command {
        Command::Law(law) => run_law(law, today, quiet, mirror.as_ref()),
        Command::Case(case) => run_case(case, quiet),
        Command::Publication(pub_) => run_publication(pub_, quiet),
        Command::Archive(archive) => run_archive(archive, today, quiet),
        Command::Find {
            query,
            limit,
            moment,
        } => run_find(
            &query.join(" "),
            *limit,
            moment,
            today,
            quiet,
            mirror.as_ref(),
        ),
        Command::Update(args) => run_update(args, quiet, mirror.as_ref()),
        Command::Verify => run_verify(),
        Command::Db(command) => run_db(command),
    }
}

/// Cuts an answer down to one lid, and one item of it.
///
/// A filter over stored text rather than a level of its own. The letter opens
/// the line because the parser puts it there, so asking for `sub b` costs no
/// schema and no second address space.
///
/// What it does not do is narrow the span. An item has no byte range in this
/// corpus: offsets are held per article and per lid, and inventing one for an
/// item would be an offset nobody measured. So the excerpt stays the lid's and
/// the answer is honest about what it points at.
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

        article.content = lid.content.clone();
        article.leden = vec![lid];
        narrowed.push(article);
    }

    if narrowed.is_empty() {
        return Err(Fault::not_found("lid", number));
    }

    Ok(narrowed)
}

/// Ranks the corpus against a question and answers with identifiers.
///
/// The one command with no publisher behind it. Every other route through the
/// mirror is an accelerant in front of a source of last resort; a ranking over
/// the shared corpus has none, because the corpus is the source. So no mirror
/// is a refusal rather than an empty answer: reporting nothing found would
/// record an absence bron never established, which is the distinction the whole
/// contract rests on.
fn run_find(
    query: &str,
    limit: u32,
    moment: &crate::cli::Moment,
    today: NaiveDate,
    quiet: bool,
    mirror: Option<&Mirror>,
) -> Outcome<Answer> {
    if query.trim().is_empty() {
        return Err(Fault::usage("say what to search for"));
    }

    let Some(mirror) = mirror else {
        return Err(Fault::usage(
            "find reads the shared corpus, which only the service holds, so it needs a mirror. \
             Set BRON_MIRROR and drop --direct. Everything else here answers without one.",
        ));
    };

    let (date, view) = moment.resolve(today);
    let (as_of, as_known_on) = match view {
        wire::View::InForce => (Some(date.to_string()), None),
        wire::View::AsKnown => (None, Some(date.to_string())),
    };

    let client = client(quiet)?;
    let answer = mirror
        .search(
            &client,
            &mirror::SearchRequest {
                q: query.to_string(),
                limit,
                as_of,
                as_known_on,
            },
        )
        .map_err(|e| from_source("bron", e))?;

    let hits = answer
        .data
        .hits
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
        .collect();

    // The service already says which corpora it searched, and that fact travels
    // rather than being restated: a CLI that decided for itself would be a
    // second place for the same claim to be made differently.
    let unsearched: Vec<String> = ["wetgeving", "rechtspraak", "publicaties"]
        .into_iter()
        .filter(|lane| !answer.data.searched.iter().any(|s| s == lane))
        .map(str::to_string)
        .collect();

    let mut caveats = Vec::new();

    if !unsearched.is_empty() {
        caveats.push(Caveat::LanesUnsearched { lanes: unsearched });
    }

    Ok(Answer::Ranking(
        Envelope::ok(wire::Ranking {
            searched: answer.data.searched,
            hits,
        })
        .bound_to(date, view)
        .with_caveats(caveats),
    ))
}

/// Fetches one version, through the mirror when there is one.
///
/// `meta` always came from the publisher's own manifest, because the manifest
/// is never mirrored, so the digest a mirrored body is held to is the
/// publisher's. That is what makes the two routes interchangeable: whichever
/// answers, the bytes are the ones the publisher published or they are refused.
///
/// A mirror that cannot answer is not a failure. It costs one wasted request
/// and the publisher is asked instead. A mirror that answers with bytes failing
/// the digest is a different matter and says so on stderr, because bytes served
/// under a digest they do not match is the one thing a mirror must never do.
fn fetch_version(
    client: &Client,
    mirror: Option<&Mirror>,
    law: &BwbId,
    meta: &VersionMeta,
    quiet: bool,
) -> anyhow::Result<(FetchedLaw, ResolvedVia)> {
    if let Some(mirror) = mirror {
        let offered = match mirror.version_bytes(client, law, meta) {
            Ok(offered) => offered,
            Err(e) => {
                if !quiet {
                    eprintln!(
                        "bron: {} could not answer, asking the publisher: {e:#}",
                        mirror.base()
                    );
                }
                None
            }
        };

        if let Some(fetched) = offered {
            // A mirror is worth exactly the digest it can be held to. Anything
            // short of a match, including a version the manifest states no digest
            // for, goes back to the publisher.
            if bwb::digest_matches(fetched.text.as_bytes(), meta) == Some(true) {
                let parsed = bwb::parse(Source::from(&fetched), law, meta)?;
                return Ok((parsed, ResolvedVia::Mirror));
            }

            if !quiet {
                eprintln!(
                    "bron: {} served {law} {} without matching the digest the publisher \
                     stated; asking the publisher instead",
                    mirror.base(),
                    meta.version
                );
            }
        }
    }

    Ok((bwb::fetch(client, law, meta)?, ResolvedVia::Publisher))
}

// -- wetgeving -----------------------------------------------------------

fn run_law(
    law_cmd: &Law,
    today: NaiveDate,
    quiet: bool,
    mirror: Option<&Mirror>,
) -> Outcome<Answer> {
    match law_cmd {
        Law::Search { query, limit } => {
            let client = client(quiet)?;
            let hits = bwb::search(&client, &query.join(" "), *limit)
                .map_err(|e| from_source("bwb", e))?;

            Ok(Answer::LawHits(Envelope::ok(
                hits.into_iter().map(Into::into).collect(),
            )))
        }

        Law::Versions { id } => {
            let law: BwbId = parse_id(id)?;
            let client = client(quiet)?;
            let db = open_db()?;

            // The manifest is authoritative for which versions exist; the
            // corpus only knows which of them have been fetched.
            let metas = bwb::versions(&client, &law).map_err(|e| from_source("bwb", e))?;
            let held: Vec<String> = db
                .versions(&law)
                .map_err(from_corpus)?
                .into_iter()
                .map(|v| v.version)
                .collect();

            Ok(Answer::Versions(Envelope::ok(
                metas
                    .into_iter()
                    .map(|m| wire::VersionSummary {
                        held: held.contains(&m.version),
                        version: m.version,
                        in_force: m.in_force.into(),
                        as_known: m.visible.map(Into::into),
                        upstream_hash: m.upstream_hash,
                        byte_len: m.byte_len,
                    })
                    .collect(),
            )))
        }

        Law::Changes {
            id,
            from,
            to,
            all,
            article,
        } => {
            let law: BwbId = parse_id(id)?;
            let to = to.unwrap_or(today);
            if *from > to {
                return Err(Fault::usage(format!("--from {from} is after --to {to}")));
            }

            let client = client(quiet)?;
            let metas = bwb::versions(&client, &law).map_err(|e| from_source("bwb", e))?;

            let earlier = bwb::version_on(&metas, *from)
                .ok_or_else(|| Fault::not_found("version of", format!("{law} on {from}")))?;
            let later = bwb::version_on(&metas, to)
                .ok_or_else(|| Fault::not_found("version of", format!("{law} on {to}")))?;

            let (before, before_via) = fetch_version(&client, mirror, &law, earlier, quiet)
                .map_err(|e| from_source("bwb", e))?;
            let (after, after_via) = fetch_version(&client, mirror, &law, later, quiet)
                .map_err(|e| from_source("bwb", e))?;
            // Two documents, so the answer may only claim one route when both
            // took it.
            let via = if before_via == after_via {
                before_via
            } else {
                ResolvedVia::Publisher
            };

            // Both sides are stored, so what was compared is on the record and
            // every span in the answer stays resolvable afterwards.
            let stamped_at = Utc::now();
            let mut db = open_db()?;
            let tx = db.transaction().map_err(from_corpus)?;
            db::put_law_version(&tx, &before, stamped_at).map_err(from_corpus)?;
            db::put_law_version(&tx, &after, stamped_at).map_err(from_corpus)?;
            tx.commit().map_err(|e| from_corpus(e.into()))?;

            // Read back from the store rather than holding two full documents
            // in memory: a BWB book runs to megabytes and the blob is the thing
            // the spans are recorded against anyway.
            let before_text = read_document(before.provenance.blob)?;
            let after_text = read_document(after.provenance.blob)?;
            let before_prov = provenance_of(&db, before.provenance.blob)?;
            let after_prov = provenance_of(&db, after.provenance.blob)?;

            // Both sides, because a diff rests on two documents and either can
            // be the one the publisher does not vouch for.
            let mut caveats = digest_caveats(before.upstream_hash.as_deref(), &before_text);
            caveats.extend(digest_caveats(after.upstream_hash.as_deref(), &after_text));

            let mut summary = wire::ChangeSummary::default();
            let mut articles = Vec::new();

            for change in diff::compare(&before, &after) {
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
                if article
                    .as_deref()
                    .is_some_and(|wanted| change.number != wanted)
                {
                    continue;
                }

                articles.push(wire::ArticleChange {
                    change: change.change.as_str(),
                    number: change.number,
                    label: change.label,
                    path: change.path,
                    before: change
                        .before
                        .and_then(|a| side(&a, &before.version, &before_text, &before_prov)),
                    after: change
                        .after
                        .and_then(|a| side(&a, &after.version, &after_text, &after_prov)),
                });
            }

            Ok(Answer::Changes(
                Envelope::ok(Box::new(wire::Changes {
                    law_id: law.into_string(),
                    from: compared(earlier, before.provenance.blob),
                    to: compared(later, after.provenance.blob),
                    articles,
                    summary,
                }))
                .via(via)
                .with_caveats(caveats),
            ))
        }

        Law::Show {
            id,
            article,
            paragraph,
            item,
            moment,
            refresh,
        } => {
            let law: BwbId = parse_id(id)?;
            let (date, view) = moment.resolve(today);

            let mut db = open_db()?;

            // The corpus first. It already holds the validity windows, so it can
            // answer which version covers a date without asking anyone. That is
            // not only faster: a frozen corpus must stay readable when the
            // publisher is unreachable, and an answer that silently changes
            // because someone republished is the opposite of reproducible.
            //
            // --refresh is how you ask whether that is still the whole story.
            let held = if *refresh {
                None
            } else {
                match view {
                    View::InForce => db.version_in_force(&law, date).map_err(from_corpus)?,
                    // Zicht is not indexed locally, so it always asks upstream.
                    View::AsKnown => None,
                }
            };

            let (version, mut caveats) = match held {
                Some(stored) => (stored.version, Vec::new()),
                None => {
                    let client = client(quiet)?;
                    let metas = bwb::versions(&client, &law).map_err(|e| from_source("bwb", e))?;

                    let meta = select_version(&metas, date, view).ok_or_else(|| {
                        Fault::not_found("version of", format!("{law} on {date}"))
                    })?;

                    if db
                        .version_id(&law, &meta.version)
                        .map_err(from_corpus)?
                        .is_none()
                    {
                        // The route is not carried onto this answer: what is
                        // rendered below is read back out of the corpus, and
                        // naming a route for a corpus read would describe how
                        // the bytes arrived once rather than where this answer
                        // came from.
                        let (fetched, _) = fetch_version(&client, mirror, &law, meta, quiet)
                            .map_err(|e| from_source("bwb", e))?;
                        let tx = db.transaction().map_err(from_corpus)?;
                        db::put_law_version(&tx, &fetched, Utc::now()).map_err(from_corpus)?;
                        tx.commit().map_err(|e| from_corpus(e.into()))?;
                    }

                    (meta.version.clone(), window_caveats(&metas, meta, date))
                }
            };

            let stored = db
                .version(&law, &version)
                .map_err(from_corpus)?
                .ok_or_else(|| Fault::not_found("version of", &law))?;
            let version_id = db
                .version_id(&law, &version)
                .map_err(from_corpus)?
                .ok_or_else(|| Fault::not_found("version of", &law))?;

            let provenance = provenance_of(&db, stored.blob)?;
            let document = read_document(stored.blob)?;
            caveats.extend(digest_caveats(stored.upstream_hash.as_deref(), &document));

            let stored_articles = db
                .articles(version_id, article.as_deref())
                .map_err(from_corpus)?;

            // Read off the stored articles rather than the wire ones, because
            // the count is how the text was rendered and not something a
            // consumer needs per article: the caveat names them once.
            let flattened: Vec<String> = stored_articles
                .iter()
                .filter(|a| a.tables > 0)
                .map(|a| a.number.clone())
                .collect();

            let illustrated: Vec<String> = stored_articles
                .iter()
                .filter(|a| a.illustrations > 0)
                .map(|a| a.number.clone())
                .collect();

            let uncommenced: Vec<String> = stored_articles
                .iter()
                .filter(|a| a.status.as_deref() == Some("nogniet"))
                .map(|a| a.number.clone())
                .collect();

            let articles = stored_articles
                .into_iter()
                .map(|a| {
                    let number = a.number.clone();
                    wire::Article::new(a, &document, provenance.clone()).ok_or_else(|| {
                        Fault::Corpus(anyhow::anyhow!(
                            "article {number} has spans that do not fit its document"
                        ))
                    })
                })
                .collect::<Outcome<Vec<_>>>()?;

            // Asking for one number and getting several means the number named
            // several. Both are answered, because picking would be bron
            // deciding which was meant, but the reader has to be told: two
            // headings reading "1" are otherwise indistinguishable from a
            // duplicated row.
            if let Some(number) = article.as_deref()
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

            if articles.is_empty() {
                let what = match article {
                    Some(n) => format!("article {n} of {law}"),
                    None => format!("articles of {law}"),
                };
                return Err(Fault::not_found("stored", what));
            }

            if !flattened.is_empty() {
                caveats.push(Caveat::TableFlattened {
                    articles: flattened,
                    cell_separator: bwb::CELL,
                });
            }

            if !uncommenced.is_empty() {
                caveats.push(Caveat::NotCommenced {
                    articles: uncommenced,
                });
            }

            if !illustrated.is_empty() {
                caveats.push(Caveat::IllustrationOmitted {
                    articles: illustrated,
                });
            }

            if view == View::AsKnown && stored.visible.is_none() {
                caveats.push(Caveat::NoZichtWindow);
            }

            let articles = narrow(articles, paragraph.as_deref(), item.as_deref())?;

            Ok(Answer::Articles(
                Envelope::ok(articles)
                    .bound_to(date, view)
                    .with_caveats(caveats),
            ))
        }
    }
}

/// States what the caller cannot see: that this text applied before it was
/// visible, and that the other window resolves the same date to another text.
///
/// Which of the two governs is a question of transitional law. bron does not
/// answer it, because that is a legal judgement and nothing here may make one.
/// Saying nothing, however, hands over one text as though it were the only
/// candidate, and for most tax legislation it is not.
fn window_caveats(metas: &[VersionMeta], chosen: &VersionMeta, date: NaiveDate) -> Vec<Caveat> {
    let mut caveats = Vec::new();

    if let Some(zicht) = chosen.visible.filter(|z| z.from > chosen.in_force.from) {
        caveats.push(Caveat::Retroactive {
            in_force_from: chosen.in_force.from,
            first_visible: zicht.from,
        });
    }

    let in_force = bwb::version_on(metas, date);
    let as_known = metas
        .iter()
        .rev()
        .find(|m| m.visible.is_some_and(|z| z.contains(date)));

    if let (Some(a), Some(b)) = (in_force, as_known)
        && a.version != b.version
    {
        caveats.push(Caveat::WindowsDisagree {
            in_force: a.version.clone(),
            as_known: b.version.clone(),
        });
    }

    caveats
}

/// Picks the version a date resolves to, in the window the caller asked for.
fn select_version(metas: &[VersionMeta], date: NaiveDate, view: View) -> Option<&VersionMeta> {
    match view {
        View::InForce => bwb::version_on(metas, date),
        // Zicht is only answerable where upstream stated it. Silently falling
        // back to geldigheid is the substitution the two windows exist to stop.
        View::AsKnown => metas
            .iter()
            .rev()
            .find(|m| m.visible.is_some_and(|w| w.contains(date))),
    }
}

// -- rechtspraak ---------------------------------------------------------

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
            let mut db = open_db()?;

            if db.case(&ecli).map_err(from_corpus)?.is_none() {
                let client = client(quiet)?;
                let fetched = rechtspraak::fetch(&client, &ecli)
                    .map_err(|e| from_source("rechtspraak", e))?;
                let tx = db.transaction().map_err(from_corpus)?;
                db::put_case(&tx, &fetched, Utc::now()).map_err(from_corpus)?;
                tx.commit().map_err(|e| from_corpus(e.into()))?;
            }

            let case = db
                .case(&ecli)
                .map_err(from_corpus)?
                .ok_or_else(|| Fault::not_found("decision", &ecli))?;
            let appeal = db.appeal_status(&ecli).map_err(from_corpus)?;
            let provenance = provenance_of(&db, case.blob)?;

            Ok(Answer::Case(Envelope::ok(Box::new(wire::Case {
                ecli: case.id.into_string(),
                court: case.court,
                decided_on: case.decided_on,
                published_on: case.published_on,
                procedure_type: case.procedure_type,
                summary: case.summary,
                full_text: case.full_text,
                relations: case.relations.into_iter().map(Into::into).collect(),
                appeal: appeal.into(),
                provenance,
            }))))
        }

        Case::Appeal { ecli } => {
            let ecli: Ecli = parse_id(ecli)?;
            let db = open_db()?;
            let status = db.appeal_status(&ecli).map_err(from_corpus)?;

            // Unknown means the corpus has never seen it, which is a real
            // answer rather than a miss: fetch it first to get a better one.
            Ok(Answer::Appeal(Envelope::ok(status.into())))
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
            let mut db = open_db()?;

            if db.publication(&id).map_err(from_corpus)?.is_none() {
                let client = client(quiet)?;
                let fetched = publications::fetch(&client, &id)
                    .map_err(|e| from_source("officiële bekendmakingen", e))?;
                let tx = db.transaction().map_err(from_corpus)?;
                db::put_publication(&tx, &fetched, Utc::now()).map_err(from_corpus)?;
                tx.commit().map_err(|e| from_corpus(e.into()))?;
            }

            let doc = db
                .publication(&id)
                .map_err(from_corpus)?
                .ok_or_else(|| Fault::not_found("publication", &id))?;

            Ok(Answer::PublicationHits(Envelope::ok(vec![
                wire::PublicationHit {
                    id: doc.id.into_string(),
                    title: doc.title,
                    dossier: doc.dossier,
                    published_on: doc.published_on,
                },
            ])))
        }
    }
}

// -- update --------------------------------------------------------------

fn run_update(args: &UpdateArgs, quiet: bool, mirror: Option<&Mirror>) -> Outcome<Answer> {
    let mut db = open_db()?;
    let client = client(quiet)?;

    let laws: Vec<BwbId> = if args.ids.is_empty() {
        db.laws()
            .map_err(from_corpus)?
            .into_iter()
            .map(|l| l.id)
            .collect()
    } else {
        args.ids
            .iter()
            .map(|id| parse_id(id))
            .collect::<Outcome<_>>()?
    };

    // One timestamp for the whole run, so every row written here agrees.
    let stamped_at = Utc::now();
    let mut report = Vec::new();

    for law in laws {
        // A per-item failure is logged and the batch continues. This is the one
        // place an error is only ever printed, so anyhow is the right shape.
        match update_one(
            &client, &mut db, &law, args.force, stamped_at, mirror, quiet,
        ) {
            Ok(updated) => report.push(updated),
            Err(e) => report.push(Updated {
                id: law.into_string(),
                outcome: "failed",
                from: None,
                to: None,
                articles: None,
                error: Some(e.to_string()),
            }),
        }
    }

    Ok(Answer::Updated(Envelope::ok(report)))
}

fn update_one(
    client: &Client,
    db: &mut Db,
    law: &BwbId,
    force: bool,
    stamped_at: chrono::DateTime<Utc>,
    mirror: Option<&Mirror>,
    quiet: bool,
) -> anyhow::Result<Updated> {
    let metas = bwb::versions(client, law)?;
    let current = bwb::current_version(&metas)
        .ok_or_else(|| anyhow::anyhow!("{law} has no current version"))?;

    let held = db.newest_version(law)?;
    let from = held.as_ref().map(|v| v.version.clone());

    if !force && from.as_deref() == Some(current.version.as_str()) {
        return Ok(Updated {
            id: law.to_string(),
            outcome: "current",
            from,
            to: None,
            articles: None,
            error: None,
        });
    }

    let (fetched, _) = fetch_version(client, mirror, law, current, quiet)?;
    let articles = fetched.articles.len() as u32;

    let tx = db.transaction()?;
    db::put_law_version(&tx, &fetched, stamped_at)?;
    tx.commit()?;

    Ok(Updated {
        id: law.to_string(),
        outcome: "updated",
        from,
        to: Some(current.version.clone()),
        articles: Some(articles),
        error: None,
    })
}

// -- verify --------------------------------------------------------------

fn run_verify() -> Outcome<Answer> {
    let db = open_db()?;
    let blobs = BlobStore::new(paths::blob_dir().map_err(from_corpus)?);

    let documents = db.documents().map_err(from_corpus)?;
    let mut missing = Vec::new();
    let mut corrupt = Vec::new();

    for document in &documents {
        if !blobs.contains(&document.blob) {
            missing.push(document.blob.to_hex());
        } else if !blobs.verify(&document.blob).map_err(from_corpus)? {
            corrupt.push(document.blob.to_hex());
        }
    }

    let verification = wire::Verification {
        documents: documents.len() as u32,
        intact: (documents.len() - missing.len() - corrupt.len()) as u32,
        missing,
        corrupt,
    };

    if !verification.is_intact() {
        return Err(Fault::Corpus(anyhow::anyhow!(
            "{} document(s) missing, {} corrupt",
            verification.missing.len(),
            verification.corrupt.len()
        )));
    }

    Ok(Answer::Verified(Envelope::ok(verification)))
}

// -- corpus --------------------------------------------------------------

fn run_db(command: &DbCommand) -> Outcome<Answer> {
    match command {
        DbCommand::Path => {
            let path = paths::db_path().map_err(from_corpus)?;
            Ok(Answer::Path(Envelope::ok(path.display().to_string())))
        }
        DbCommand::Stats => {
            let db = open_db()?;
            let stats = db.stats().map_err(from_corpus)?;
            Ok(Answer::Stats(Envelope::ok(stats.into())))
        }
    }
}

// -- shared --------------------------------------------------------------

/// One side of a comparison, with the excerpt that proves it.
fn side(
    article: &bron::sources::FetchedArticle,
    version: &str,
    document: &str,
    provenance: &wire::Provenance,
) -> Option<wire::Side> {
    Some(wire::Side {
        version: version.to_string(),
        excerpt: wire::Excerpt::new(article.span, &article.content, document)?,
        content: article.content.clone(),
        amended_by: article.amended_by.as_ref().map(|p| p.as_str().to_string()),
        effect: article.effect.clone(),
        provenance: provenance.clone(),
    })
}

fn compared(meta: &VersionMeta, blob: bron::blob::BlobId) -> wire::ComparedVersion {
    wire::ComparedVersion {
        version: meta.version.clone(),
        in_force: meta.in_force.into(),
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
            let mut db = open_db()?;
            let client = client(quiet)?;

            // The index decides which capture answers the question; the corpus
            // only ever saves the fetch of its bytes. Asking the corpus first
            // would answer "the page in 2025" with whatever happens to be held,
            // which for a corpus holding one 2016 capture is a nine-year-old
            // page returned in ten milliseconds with nothing to say so.
            let row = wayback::capture_on(&client, url, on)
                .map_err(|e| from_source(ARCHIVE, e))?
                .ok_or_else(|| Fault::not_found("capture", format!("{url} on or before {on}")))?;

            let urlkey = wayback::urlkey_of(url).map_err(|e| Fault::usage(e.to_string()))?;
            let held = db
                .capture_at(&urlkey, on)
                .map_err(from_corpus)?
                .filter(|capture| capture.id == row.id && capture.content.is_some());

            let capture = match held {
                Some(capture) => capture,
                None => {
                    let fetched = wayback::fetch_capture(&client, &row)
                        .map_err(|e| from_source(ARCHIVE, e))?;

                    let tx = db.transaction().map_err(from_corpus)?;
                    db::put_capture(&tx, &fetched, false, Utc::now()).map_err(from_corpus)?;
                    tx.commit().map_err(|e| from_corpus(e.into()))?;

                    db.capture_at(&urlkey, on)
                        .map_err(from_corpus)?
                        .ok_or_else(|| Fault::not_found("capture", url))?
                }
            };

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

            let fresh = wayback::caused_by_bron(newest_before.as_ref(), &stamp);
            let fetched =
                wayback::fetch_capture(&client, row).map_err(|e| from_source(ARCHIVE, e))?;

            let mut db = open_db()?;
            let tx = db.transaction().map_err(from_corpus)?;
            db::put_capture(&tx, &fetched, fresh, Utc::now()).map_err(from_corpus)?;
            tx.commit().map_err(|e| from_corpus(e.into()))?;

            let urlkey = wayback::urlkey_of(url).map_err(|e| Fault::usage(e.to_string()))?;
            let capture = db
                .capture_at(&urlkey, stamp.date())
                .map_err(from_corpus)?
                .ok_or_else(|| Fault::not_found("capture", url))?;

            // `fresh` decides whether this capture is recorded as one bron
            // caused. When the archive answered with a job for a capture it
            // already held, it is not, and `capture_caveats` reads that back
            // off the stored row rather than off what was asked for.
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

fn open_db() -> Outcome<Db> {
    Db::open().map_err(from_corpus)
}

fn client(quiet: bool) -> Outcome<Client> {
    let client = Client::new().map_err(from_corpus)?;
    if quiet {
        return Ok(client);
    }

    Ok(client.on_wait(|host, waited| {
        // A sweep that pauses must say so rather than look hung.
        eprintln!("bron: waiting {:.1}s for {host}", waited.as_secs_f32());
    }))
}

/// What the publisher's own manifest says about these bytes.
///
/// `bron verify` checks that a blob still hashes to its own name, which is a
/// claim about this disk and not about the register. This is the other half:
/// the manifest publishes a SHA-512 per rendition, and until now bron stored it
/// and never looked at it again on the read path. A citation resting on bytes
/// the publisher does not vouch for is exactly the thing this corpus exists to
/// make visible.
///
/// Recomputed rather than stored. Measured at 1,2 ms over the 1,3 MB BW6
/// document, against a blob read that has already happened by the time this is
/// called, so a column would buy nothing and add a second place for the truth
/// to go stale.
fn digest_caveats(published: Option<&str>, document: &str) -> Vec<Caveat> {
    let Some(published) = published else {
        return vec![Caveat::UpstreamDigestAbsent];
    };

    let published = published.trim().to_ascii_lowercase();
    let ours = HEXLOWER.encode(&Sha512::digest(document.as_bytes()));

    if ours == published {
        return Vec::new();
    }

    vec![Caveat::UpstreamDigestDiffers { published, ours }]
}

fn provenance_of(db: &Db, blob: bron::blob::BlobId) -> Outcome<wire::Provenance> {
    db.document(&blob)
        .map_err(from_corpus)?
        .map(wire::Provenance::from)
        .ok_or_else(|| Fault::Corpus(anyhow::anyhow!("no ledger entry for document {blob}")))
}

/// Reads a frozen document back, verifying it still hashes to its own name.
fn read_document(blob: bron::blob::BlobId) -> Outcome<String> {
    let blobs = BlobStore::new(paths::blob_dir().map_err(from_corpus)?);
    let bytes = blobs.get_verified(&blob).map_err(from_corpus)?;
    String::from_utf8(bytes)
        .map_err(|e| Fault::Corpus(anyhow::anyhow!("document {blob} is not UTF-8: {e}")))
}

#[cfg(test)]
mod digest_tests {
    use super::digest_caveats;
    use bron::wire::Caveat;
    use data_encoding::HEXLOWER;
    use sha2::{Digest, Sha512};

    fn sha512(text: &str) -> String {
        HEXLOWER.encode(&Sha512::digest(text.as_bytes()))
    }

    #[test]
    fn bytes_the_publisher_vouched_for_say_nothing() {
        let text = "<toestand/>";
        assert!(digest_caveats(Some(&sha512(text)), text).is_empty());
    }

    /// The register publishes hashes in upper case in places, and a caveat
    /// raised over letter case would be bron inventing a disagreement.
    #[test]
    fn the_comparison_ignores_case_and_surrounding_space() {
        let text = "<toestand/>";
        let published = format!("  {}  ", sha512(text).to_uppercase());

        assert!(digest_caveats(Some(&published), text).is_empty());
    }

    #[test]
    fn bytes_that_disagree_name_both_digests() {
        let caveats = digest_caveats(Some(&sha512("something else")), "<toestand/>");

        match caveats.as_slice() {
            [Caveat::UpstreamDigestDiffers { published, ours }] => {
                assert_eq!(*ours, sha512("<toestand/>"));
                assert_ne!(published, ours);
            }
            other => panic!("expected one differs caveat, got {other:?}"),
        }
    }

    /// 'Nobody vouched for this' and 'the voucher and the bytes disagree' are
    /// different facts, and a consumer may care about the difference.
    #[test]
    fn a_rendition_with_no_published_hash_says_so_separately() {
        assert!(matches!(
            digest_caveats(None, "<toestand/>").as_slice(),
            [Caveat::UpstreamDigestAbsent]
        ));
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
                start: 0,
                end: 1,
                text: "<artikel/>".into(),
                rendered: String::new(),
            },
            leden: leden
                .iter()
                .map(|(number, content)| Lid {
                    number: Some((*number).into()),
                    content: (*content).to_string(),
                    excerpt: Excerpt {
                        start: 0,
                        end: 1,
                        text: "<lid/>".into(),
                        rendered: String::new(),
                    },
                    jci: None,
                })
                .collect(),
            amended_by: None,
            effect: None,
            in_force_from: None,
            jci: None,
            status: None,
            provenance: Provenance {
                blob: bron::blob::BlobId::of(b"<toestand/>"),
                url: "https://example.invalid".into(),
                byte_len: 1,
                fetched_at: chrono::Utc::now(),
            },
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
