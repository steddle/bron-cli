//! The corpus.
//!
//! Reads hang off [`Db`], which exists to carry the invariant that the schema
//! has been applied and checked. Writes are free functions over a
//! [`Transaction`], so a caller decides what is atomic: an update touching a
//! law, its version, its articles and their leden is one unit or it is nothing.
//!
//! Nothing here calls the clock. Timestamps arrive as arguments, because a
//! single update run should stamp one time, and because a write you cannot
//! predict is a write you cannot test.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::blob::BlobId;
use crate::ids::{BwbId, Ecli, PublicationId};
use crate::model::{
    AppealStatus, Article, Capture, Case, DigestCheck, Direction, Law, LawVersion, Lid,
    Publication, Relation, Span, Validity,
};
use crate::paths;
use crate::sources::{
    FetchedCapture, FetchedCase, FetchedLaw, FetchedPublication, Provenance, wayback,
};

/// Bumped whenever `schema.sql` changes in a way an older database cannot
/// absorb by re-running it.
const SCHEMA_VERSION: i32 = 4;

/// Row id of a stored law version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionId(i64);

pub struct Db {
    conn: Connection,
    path: PathBuf,
}

/// A row of the provenance ledger: one document, once fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub blob: BlobId,
    pub url: String,
    pub byte_len: usize,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct Stats {
    pub path: PathBuf,
    pub laws: u32,
    pub versions: u32,
    pub articles: u32,
    pub cases: u32,
    pub publications: u32,
    pub documents: u32,
}

impl Db {
    /// Opens the corpus in the user's data directory, creating it if absent.
    pub fn open() -> Result<Self> {
        Self::open_at(&paths::db_path()?)
    }

    pub fn open_at(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("cannot open corpus at {}", path.display()))?;

        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;

        // A database written by a newer binary may hold tables and columns this
        // one does not understand, and re-running CREATE TABLE IF NOT EXISTS
        // over it would report success and then read the wrong shape.
        // Downward is no safer: re-running it cannot add a column, so an older
        // corpus would be stamped current while still holding the old shape.
        // Until there is a real migration path, accept only a fresh database or
        // an exact match, and refuse the rest rather than misread it.
        let found: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        anyhow::ensure!(
            found == 0 || found == SCHEMA_VERSION,
            "the corpus at {} is schema v{found}; this build speaks v{SCHEMA_VERSION}. \
             Point BRON_HOME elsewhere or remove it to start fresh.",
            path.display(),
        );

        conn.execute_batch(include_str!("schema.sql"))
            .context("cannot apply schema")?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;

        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn transaction(&mut self) -> Result<Transaction<'_>> {
        self.conn.transaction().context("cannot begin transaction")
    }

    // -- laws ------------------------------------------------------------

    pub fn law(&self, id: &BwbId) -> Result<Option<Law>> {
        self.conn
            .query_row(&law_select("WHERE l.id = ?1"), params![id], law_from_row)
            .optional()
            .context("cannot read law")
    }

    pub fn laws(&self) -> Result<Vec<Law>> {
        let mut stmt = self
            .conn
            .prepare(&format!("{} ORDER BY l.title", law_select("")))?;
        let rows = stmt
            .query_map([], law_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn versions(&self, law: &BwbId) -> Result<Vec<LawVersion>> {
        let mut stmt = self.conn.prepare(&format!(
            "{VERSION_SELECT} WHERE law_id = ?1 ORDER BY in_force_from"
        ))?;
        let rows = stmt
            .query_map(params![law], version_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The version of a law in force on a date, if it is stored.
    ///
    /// This is geldigheid, not zicht: it answers what governed conduct on that
    /// date, which is the question `--as-of` exists to ask.
    pub fn version_in_force(&self, law: &BwbId, on: NaiveDate) -> Result<Option<LawVersion>> {
        self.conn
            .query_row(
                &format!(
                    "{VERSION_SELECT}
                     WHERE law_id = ?1
                       AND in_force_from <= ?2
                       AND (in_force_to IS NULL OR in_force_to >= ?2)
                     ORDER BY in_force_from DESC
                     LIMIT 1"
                ),
                params![law, on],
                version_from_row,
            )
            .optional()
            .context("cannot read version")
    }

    /// The most recent version stored for a law.
    pub fn newest_version(&self, law: &BwbId) -> Result<Option<LawVersion>> {
        self.conn
            .query_row(
                &format!("{VERSION_SELECT} WHERE law_id = ?1 ORDER BY in_force_from DESC LIMIT 1"),
                params![law],
                version_from_row,
            )
            .optional()
            .context("cannot read version")
    }

    /// One specific stored version, by its upstream label.
    pub fn version(&self, law: &BwbId, version: &str) -> Result<Option<LawVersion>> {
        self.conn
            .query_row(
                &format!("{VERSION_SELECT} WHERE law_id = ?1 AND version = ?2"),
                params![law, version],
                version_from_row,
            )
            .optional()
            .context("cannot read version")
    }

    /// Every row of the provenance ledger.
    pub fn documents(&self) -> Result<Vec<Document>> {
        let mut stmt = self
            .conn
            .prepare("SELECT blob, url, byte_len, fetched_at FROM documents ORDER BY fetched_at")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Document {
                    blob: row.get("blob")?,
                    url: row.get("url")?,
                    byte_len: row.get::<_, i64>("byte_len")? as usize,
                    fetched_at: row.get("fetched_at")?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn version_id(&self, law: &BwbId, version: &str) -> Result<Option<VersionId>> {
        self.conn
            .query_row(
                "SELECT id FROM law_versions WHERE law_id = ?1 AND version = ?2",
                params![law, version],
                |r| r.get::<_, i64>(0).map(VersionId),
            )
            .optional()
            .context("cannot read version id")
    }

    /// Articles of a stored version, in document order.
    ///
    /// `number` narrows to a printed article number, which is not unique: an
    /// annex restarts its own numbering, so this can legitimately return more
    /// than one article. The structural path distinguishes them.
    pub fn articles(&self, version: VersionId, number: Option<&str>) -> Result<Vec<Article>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.law_id, v.version, a.id, a.number, a.label, a.title, a.path,
                    a.content, a.span_start, a.span_end, a.amended_by, a.effect, a.stam_id,
                    a.in_force_from, a.tables, a.jci, a.illustrations,
                    a.versie_id, a.status
             FROM articles a
             JOIN law_versions v ON v.id = a.version_id
             WHERE a.version_id = ?1 AND (?2 IS NULL OR a.number = ?2)
             ORDER BY a.ordinal",
        )?;

        let rows = stmt
            .query_map(params![version.0, number], |row| {
                Ok((row.get::<_, i64>("id")?, article_from_row(row)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        rows.into_iter()
            .map(|(id, mut article)| {
                article.leden = self.leden(id)?;
                Ok(article)
            })
            .collect()
    }

    fn leden(&self, article_id: i64) -> Result<Vec<Lid>> {
        let mut stmt = self.conn.prepare(
            "SELECT number, content, span_start, span_end, jci
             FROM leden WHERE article_id = ?1 ORDER BY ordinal",
        )?;
        let rows = stmt
            .query_map(params![article_id], |row| {
                Ok(Lid {
                    number: row.get("number")?,
                    content: row.get("content")?,
                    span: Span {
                        start: row.get::<_, i64>("span_start")? as u32,
                        end: row.get::<_, i64>("span_end")? as u32,
                    },
                    jci: row.get("jci")?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // -- cases -----------------------------------------------------------

    pub fn case(&self, id: &Ecli) -> Result<Option<Case>> {
        let case = self
            .conn
            .query_row(
                &format!("{CASE_SELECT} WHERE id = ?1"),
                params![id],
                case_from_row,
            )
            .optional()
            .context("cannot read case")?;

        match case {
            Some(mut case) => {
                case.relations = self.relations(id)?;
                Ok(Some(case))
            }
            None => Ok(None),
        }
    }

    pub fn relations(&self, id: &Ecli) -> Result<Vec<Relation>> {
        let mut stmt = self.conn.prepare(
            "SELECT target, kind, direction FROM case_relations
             WHERE case_id = ?1 ORDER BY direction, target",
        )?;
        let rows = stmt
            .query_map(params![id], |row| {
                Ok(Relation {
                    target: row.get("target")?,
                    kind: row.get("kind")?,
                    direction: match row.get::<_, String>("direction")?.as_str() {
                        "later" => Direction::Later,
                        _ => Direction::Earlier,
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// What the corpus can honestly say about a decision having been appealed.
    ///
    /// Absence of a forward link is only meaningful for a decision we actually
    /// hold; for anything else the answer is [`AppealStatus::Unknown`]. See the
    /// type's own documentation for why this is never a boolean.
    pub fn appeal_status(&self, id: &Ecli) -> Result<AppealStatus> {
        let fetched_at: Option<DateTime<Utc>> = self
            .conn
            .query_row(
                "SELECT fetched_at FROM cases WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;

        let Some(fetched_at) = fetched_at else {
            return Ok(AppealStatus::Unknown);
        };

        let successors: Vec<Ecli> = self
            .relations(id)?
            .into_iter()
            .filter(|r| r.direction == Direction::Later)
            .map(|r| r.target)
            .collect();

        Ok(if successors.is_empty() {
            // Dated, because this is a claim about what bron holds and what
            // bron holds began going stale the moment it was fetched.
            AppealStatus::NoPublishedSuccessor { as_of: fetched_at }
        } else {
            AppealStatus::Appealed { successors }
        })
    }

    // -- publications ----------------------------------------------------

    pub fn publication(&self, id: &PublicationId) -> Result<Option<Publication>> {
        self.conn
            .query_row(
                &format!("{PUBLICATION_SELECT} WHERE id = ?1"),
                params![id],
                publication_from_row,
            )
            .optional()
            .context("cannot read publication")
    }

    pub fn publications(&self, dossier: Option<&str>, limit: u32) -> Result<Vec<Publication>> {
        let mut stmt = self.conn.prepare(&format!(
            "{PUBLICATION_SELECT}
             WHERE (?1 IS NULL OR dossier = ?1)
             ORDER BY published_on DESC LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![dossier, i64::from(limit)], publication_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// A row from the provenance ledger.
    pub fn document(&self, blob: &BlobId) -> Result<Option<Document>> {
        self.conn
            .query_row(
                "SELECT blob, url, byte_len, fetched_at FROM documents WHERE blob = ?1",
                params![blob],
                |row| {
                    Ok(Document {
                        blob: row.get("blob")?,
                        url: row.get("url")?,
                        byte_len: row.get::<_, i64>("byte_len")? as usize,
                        fetched_at: row.get("fetched_at")?,
                    })
                },
            )
            .optional()
            .context("cannot read document")
    }

    // -- housekeeping ----------------------------------------------------

    pub fn stats(&self) -> Result<Stats> {
        let count = |table: &'static str| -> Result<u32> {
            let n: i64 =
                self.conn
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
            Ok(u32::try_from(n).unwrap_or(u32::MAX))
        };
        Ok(Stats {
            path: self.path.clone(),
            laws: count("laws")?,
            versions: count("law_versions")?,
            articles: count("articles")?,
            cases: count("cases")?,
            publications: count("publications")?,
            documents: count("documents")?,
        })
    }

    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute_batch("VACUUM")?;
        Ok(())
    }
}

const CAPTURE_SELECT: &str = "SELECT id, original_url, captured_at, archive_status, index_digest, \
                              verified, is_revisit, mime_type, encoding, encoding_from, \
                              requested, content, blob, fetched_at FROM captures";

impl Db {
    /// The capture in force for a date: the newest one taken at or before it.
    ///
    /// Takes an [`wayback::ApproxUrlKey`], because a local guess at the key is
    /// the only kind a caller holding a plain address can have. The column
    /// holds the keys the index itself assigned, so a guess that is finer than
    /// the archive's simply misses.
    ///
    /// At-or-before rather than nearest, because a capture taken after the date
    /// cannot evidence what the page said on it. Answering with the nearest one
    /// would sometimes hand back a page that had already changed.
    pub fn capture_at(
        &self,
        urlkey: &wayback::ApproxUrlKey,
        on: NaiveDate,
    ) -> Result<Option<Capture>> {
        self.conn
            .query_row(
                &format!(
                    "{CAPTURE_SELECT} WHERE urlkey = ?1 AND captured_at <= ?2
                     ORDER BY captured_at DESC LIMIT 1"
                ),
                params![urlkey.as_str(), end_of_day(on)],
                capture_from_row,
            )
            .optional()
            .context("cannot read capture")
    }

    /// The newest capture held for an address.
    pub fn newest_capture(&self, urlkey: &wayback::ApproxUrlKey) -> Result<Option<Capture>> {
        self.conn
            .query_row(
                &format!("{CAPTURE_SELECT} WHERE urlkey = ?1 ORDER BY captured_at DESC LIMIT 1"),
                params![urlkey.as_str()],
                capture_from_row,
            )
            .optional()
            .context("cannot read capture")
    }

    /// Every capture held for an address, newest first.
    pub fn captures(&self, urlkey: &wayback::ApproxUrlKey, limit: u32) -> Result<Vec<Capture>> {
        let mut stmt = self.conn.prepare(&format!(
            "{CAPTURE_SELECT} WHERE urlkey = ?1 ORDER BY captured_at DESC LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![urlkey.as_str(), i64::from(limit)], capture_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// The last instant of a day, so a date bound includes captures made that
/// afternoon rather than only those made at midnight.
fn end_of_day(on: NaiveDate) -> DateTime<Utc> {
    on.and_hms_opt(23, 59, 59)
        .expect("a valid time of day")
        .and_utc()
}

fn capture_from_row(row: &Row) -> rusqlite::Result<Capture> {
    Ok(Capture {
        id: row.get("id")?,
        original_url: row.get("original_url")?,
        captured_at: row.get("captured_at")?,
        archive_status: row
            .get::<_, Option<i64>>("archive_status")?
            .map(|s| s as u16),
        index_digest: row.get("index_digest")?,
        verified: row
            .get::<_, Option<String>>("verified")?
            .map(|v| match v.as_str() {
                "matches" => DigestCheck::Matches,
                "differs" => DigestCheck::Differs,
                _ => DigestCheck::Absent,
            }),
        is_revisit: row.get("is_revisit")?,
        mime_type: row.get("mime_type")?,
        encoding: row.get("encoding")?,
        encoding_from: row.get("encoding_from")?,
        requested: row.get("requested")?,
        content: row.get("content")?,
        blob: row.get("blob")?,
        fetched_at: row.get("fetched_at")?,
    })
}

// -- writes --------------------------------------------------------------

/// Records a fetched document in the provenance ledger.
///
/// Called by every write below rather than by their callers: the row and the
/// document it came from are written together, so the ledger cannot be
/// forgotten and a foreign key cannot be left dangling.
fn put_document(tx: &Transaction, doc: &Provenance, fetched_at: DateTime<Utc>) -> Result<()> {
    tx.execute(
        "INSERT INTO documents (blob, url, byte_len, fetched_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (blob) DO NOTHING",
        params![doc.blob, doc.url, i64::try_from(doc.byte_len)?, fetched_at],
    )?;
    Ok(())
}

/// Stores one capture.
///
/// `requested` records whether bron asked the archive for this capture or found
/// one it already held. A tool that can cause the evidence it later cites
/// should say which it did, so the two are stored apart rather than blurred.
pub fn put_capture(
    tx: &Transaction,
    capture: &FetchedCapture,
    requested: bool,
    fetched_at: DateTime<Utc>,
) -> Result<()> {
    put_document(tx, &capture.provenance, fetched_at)?;

    let (content, encoding, encoding_from) = match &capture.reading {
        wayback::Reading::Text {
            text,
            encoding,
            declared,
            ..
        } => (
            Some(text.as_str()),
            Some(*encoding),
            Some(format!("{declared:?}").to_lowercase()),
        ),
        wayback::Reading::Binary => (None, None, None),
    };

    tx.execute(
        "INSERT INTO captures
             (id, urlkey, original_url, captured_at, archive_status, index_digest, verified,
              is_revisit, mime_type, encoding, encoding_from, requested, content, blob, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT (id) DO UPDATE SET
             archive_status = excluded.archive_status,
             index_digest = excluded.index_digest,
             verified = excluded.verified,
             is_revisit = excluded.is_revisit,
             mime_type = excluded.mime_type,
             encoding = excluded.encoding,
             encoding_from = excluded.encoding_from,
             -- Once true, always true: a later read that merely found the
             -- capture does not undo the fact that bron caused it.
             requested = captures.requested OR excluded.requested,
             content = excluded.content,
             blob = excluded.blob,
             fetched_at = excluded.fetched_at",
        params![
            capture.id,
            capture.id.urlkey(),
            capture.original,
            capture.id.captured_at(),
            capture.status.code(),
            capture.digest,
            match capture.verified {
                wayback::Verified::Matches => "matches",
                wayback::Verified::Differs => "differs",
                wayback::Verified::Absent => "absent",
            },
            capture.is_revisit,
            capture.mime_type,
            encoding,
            encoding_from,
            requested,
            content,
            capture.provenance.blob,
            fetched_at,
        ],
    )?;

    Ok(())
}

/// Stores one version of a law together with its articles and their leden.
///
/// Re-storing the same version replaces its articles rather than merging: an
/// amended text renumbers and repeals, so anything absent from the new parse is
/// by definition gone from that version.
pub fn put_law_version(
    tx: &Transaction,
    law: &FetchedLaw,
    fetched_at: DateTime<Utc>,
) -> Result<VersionId> {
    put_document(tx, &law.provenance, fetched_at)?;

    tx.execute(
        "INSERT INTO laws (id, title) VALUES (?1, ?2)
         ON CONFLICT (id) DO UPDATE SET title = excluded.title",
        params![law.id, law.title],
    )?;

    tx.execute(
        "INSERT INTO law_versions
             (law_id, version, title, law_type, in_force_from, in_force_to,
              visible_from, visible_to, blob, upstream_hash, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (law_id, version) DO UPDATE SET
             title = excluded.title,
             law_type = excluded.law_type,
             in_force_from = excluded.in_force_from,
             in_force_to = excluded.in_force_to,
             visible_from = excluded.visible_from,
             visible_to = excluded.visible_to,
             blob = excluded.blob,
             upstream_hash = excluded.upstream_hash,
             fetched_at = excluded.fetched_at",
        params![
            law.id,
            law.version,
            law.title,
            law.law_type,
            law.in_force.from,
            law.in_force.until,
            law.visible.map(|v| v.from),
            law.visible.and_then(|v| v.until),
            law.provenance.blob,
            law.upstream_hash,
            fetched_at,
        ],
    )?;

    let version = VersionId(tx.query_row(
        "SELECT id FROM law_versions WHERE law_id = ?1 AND version = ?2",
        params![law.id, law.version],
        |r| r.get(0),
    )?);

    tx.execute(
        "DELETE FROM articles WHERE version_id = ?1",
        params![version.0],
    )?;

    let mut insert_article = tx.prepare(
        "INSERT INTO articles
             (version_id, ordinal, number, label, title, path, content, span_start, span_end,
              amended_by, effect, stam_id, in_force_from, tables, jci,
              illustrations, versie_id, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                 ?16, ?17, ?18)
         RETURNING id",
    )?;
    let mut insert_lid = tx.prepare(
        "INSERT INTO leden (article_id, ordinal, number, content, span_start, span_end, jci)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;

    for (ordinal, article) in law.articles.iter().enumerate() {
        let article_id: i64 = insert_article.query_row(
            params![
                version.0,
                i64::try_from(ordinal)?,
                article.number,
                article.label,
                article.title,
                article.path,
                article.content,
                article.span.start,
                article.span.end,
                article.amended_by,
                article.effect,
                article.stam_id,
                article.in_force_from,
                article.tables,
                article.jci,
                article.illustrations,
                article.versie_id,
                article.status,
            ],
            |r| r.get(0),
        )?;

        for (lid_ordinal, lid) in article.leden.iter().enumerate() {
            insert_lid.execute(params![
                article_id,
                i64::try_from(lid_ordinal)?,
                lid.number,
                lid.content,
                lid.span.start,
                lid.span.end,
                lid.jci,
            ])?;
        }
    }

    Ok(version)
}

pub fn put_case(tx: &Transaction, case: &FetchedCase, fetched_at: DateTime<Utc>) -> Result<()> {
    put_document(tx, &case.provenance, fetched_at)?;

    tx.execute(
        "INSERT INTO cases
             (id, court, decided_on, published_on, procedure_type, summary, full_text,
              blob, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (id) DO UPDATE SET
             court = excluded.court,
             decided_on = excluded.decided_on,
             published_on = excluded.published_on,
             procedure_type = excluded.procedure_type,
             summary = excluded.summary,
             full_text = excluded.full_text,
             blob = excluded.blob,
             fetched_at = excluded.fetched_at",
        params![
            case.id,
            case.court,
            case.decided_on,
            case.published_on,
            case.procedure_type,
            case.summary,
            case.full_text,
            case.provenance.blob,
            fetched_at,
        ],
    )?;

    tx.execute(
        "DELETE FROM case_relations WHERE case_id = ?1",
        params![case.id],
    )?;

    let mut insert = tx.prepare(
        "INSERT INTO case_relations (case_id, target, kind, direction) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT DO NOTHING",
    )?;
    for relation in &case.relations {
        insert.execute(params![
            case.id,
            relation.target,
            relation.kind,
            relation.direction.as_str()
        ])?;
    }

    Ok(())
}

pub fn put_publication(
    tx: &Transaction,
    doc: &FetchedPublication,
    fetched_at: DateTime<Utc>,
) -> Result<()> {
    if let Some(provenance) = &doc.provenance {
        put_document(tx, provenance, fetched_at)?;
    }

    tx.execute(
        "INSERT INTO publications
             (id, title, dossier, document_type, vergaderjaar, published_on, url,
              full_text, blob, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (id) DO UPDATE SET
             title = excluded.title,
             dossier = excluded.dossier,
             document_type = excluded.document_type,
             vergaderjaar = excluded.vergaderjaar,
             published_on = excluded.published_on,
             url = excluded.url,
             full_text = excluded.full_text,
             blob = excluded.blob,
             fetched_at = excluded.fetched_at",
        params![
            doc.id,
            doc.title,
            doc.dossier,
            doc.document_type,
            doc.vergaderjaar,
            doc.published_on,
            doc.url,
            doc.full_text,
            doc.provenance.as_ref().map(|p| p.blob),
            fetched_at,
        ],
    )?;
    Ok(())
}

// -- row mapping ---------------------------------------------------------

/// `filter` is `&'static str` so that no string built from input can reach it.
fn law_select(filter: &'static str) -> String {
    format!(
        "SELECT l.id, l.title,
                (SELECT count(*) FROM law_versions v WHERE v.law_id = l.id) AS version_count
         FROM laws l {filter}"
    )
}

fn law_from_row(row: &Row) -> rusqlite::Result<Law> {
    Ok(Law {
        id: row.get("id")?,
        title: row.get("title")?,
        version_count: row.get::<_, i64>("version_count")? as u32,
    })
}

const VERSION_SELECT: &str = "SELECT law_id, version, title, law_type, in_force_from, in_force_to,
                                     visible_from, visible_to, blob, upstream_hash, fetched_at,
                                     (SELECT count(*) FROM articles a WHERE a.version_id = law_versions.id)
                                         AS article_count
                              FROM law_versions";

fn version_from_row(row: &Row) -> rusqlite::Result<LawVersion> {
    let visible_to: Option<NaiveDate> = row.get("visible_to")?;
    Ok(LawVersion {
        law_id: row.get("law_id")?,
        version: row.get("version")?,
        title: row.get("title")?,
        law_type: row.get("law_type")?,
        in_force: Validity {
            from: row.get("in_force_from")?,
            until: row.get("in_force_to")?,
        },
        visible: row
            .get::<_, Option<NaiveDate>>("visible_from")?
            .map(|from| Validity {
                from,
                until: visible_to,
            }),
        blob: row.get("blob")?,
        upstream_hash: row.get("upstream_hash")?,
        fetched_at: row.get("fetched_at")?,
        article_count: row.get::<_, i64>("article_count")? as u32,
    })
}

fn article_from_row(row: &Row) -> rusqlite::Result<Article> {
    Ok(Article {
        law_id: row.get("law_id")?,
        version: row.get("version")?,
        stam_id: row.get("stam_id")?,
        number: row.get("number")?,
        label: row.get("label")?,
        title: row.get("title")?,
        path: row.get("path")?,
        content: row.get("content")?,
        span: Span {
            start: row.get::<_, i64>("span_start")? as u32,
            end: row.get::<_, i64>("span_end")? as u32,
        },
        leden: Vec::new(),
        amended_by: row.get("amended_by")?,
        effect: row.get("effect")?,
        in_force_from: row.get("in_force_from")?,
        tables: row.get::<_, i64>("tables")? as u32,
        jci: row.get("jci")?,
        illustrations: row.get::<_, i64>("illustrations")? as u32,
        versie_id: row.get("versie_id")?,
        status: row.get("status")?,
    })
}

const CASE_SELECT: &str = "SELECT id, court, decided_on, published_on, procedure_type, summary,
                                  full_text, blob, fetched_at
                           FROM cases";

fn case_from_row(row: &Row) -> rusqlite::Result<Case> {
    Ok(Case {
        id: row.get("id")?,
        court: row.get("court")?,
        decided_on: row.get("decided_on")?,
        published_on: row.get("published_on")?,
        procedure_type: row.get("procedure_type")?,
        summary: row.get("summary")?,
        full_text: row.get("full_text")?,
        blob: row.get("blob")?,
        fetched_at: row.get("fetched_at")?,
        relations: Vec::new(),
    })
}

const PUBLICATION_SELECT: &str = "SELECT id, title, dossier, document_type, vergaderjaar,
                                         published_on, url, full_text, blob, fetched_at
                                  FROM publications";

fn publication_from_row(row: &Row) -> rusqlite::Result<Publication> {
    Ok(Publication {
        id: row.get("id")?,
        title: row.get("title")?,
        dossier: row.get("dossier")?,
        document_type: row.get("document_type")?,
        vergaderjaar: row.get("vergaderjaar")?,
        published_on: row.get("published_on")?,
        url: row.get("url")?,
        full_text: row.get("full_text")?,
        blob: row.get("blob")?,
        fetched_at: row.get("fetched_at")?,
    })
}
