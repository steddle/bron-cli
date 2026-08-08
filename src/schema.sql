-- Provenance ledger. One row per document ever fetched, keyed by the digest of
-- its own bytes. Everything else in this file refers back to here.
CREATE TABLE IF NOT EXISTS documents (
    blob       BLOB PRIMARY KEY,
    url        TEXT NOT NULL,
    byte_len   INTEGER NOT NULL,
    fetched_at TEXT NOT NULL
);

-- A law, independent of its text. Titles change between versions; this holds
-- the most recently seen one purely so a list reads sensibly.
CREATE TABLE IF NOT EXISTS laws (
    id    TEXT PRIMARY KEY,
    title TEXT NOT NULL
);

-- One row per published version. This is what makes --as-of a lookup.
--
-- in_force_* is geldigheid, visible_* is zicht. They are stored separately
-- because they answer different questions and diverge on retroactive change.
-- visible_* is nullable and is never filled in from in_force_*: a manifest that
-- omits the zicht dates leaves them absent rather than asserting they matched.
-- An open-ended window is NULL here, never the 9999-12-31 sentinel upstream
-- uses, so that date comparison needs no special case.
CREATE TABLE IF NOT EXISTS law_versions (
    id            INTEGER PRIMARY KEY,
    law_id        TEXT NOT NULL REFERENCES laws (id) ON DELETE CASCADE,
    version       TEXT NOT NULL,
    title         TEXT NOT NULL,
    law_type      TEXT,
    in_force_from TEXT NOT NULL,
    in_force_to   TEXT,
    visible_from  TEXT,
    visible_to    TEXT,
    blob          BLOB NOT NULL REFERENCES documents (blob),
    upstream_hash TEXT,
    fetched_at    TEXT NOT NULL,
    UNIQUE (law_id, version)
);

CREATE INDEX IF NOT EXISTS idx_versions_in_force
    ON law_versions (law_id, in_force_from, in_force_to);

-- Articles are keyed on (version, ordinal), never on the printed number.
-- Numbers repeat: the Awb carries 584 articles across 582 distinct numbers
-- because each of its three annexes restarts at Artikel 1.
CREATE TABLE IF NOT EXISTS articles (
    id         INTEGER PRIMARY KEY,
    version_id INTEGER NOT NULL REFERENCES law_versions (id) ON DELETE CASCADE,
    ordinal    INTEGER NOT NULL,
    number     TEXT NOT NULL,
    label      TEXT,
    title      TEXT,
    path       TEXT,
    content    TEXT NOT NULL,
    span_start INTEGER NOT NULL,
    span_end   INTEGER NOT NULL,
    -- The publication that produced this text, and what it did. The amending
    -- act is where transitional law lives, so this is the pointer that makes
    -- 'which regime applies' answerable without bron answering it.
    amended_by TEXT,
    effect     TEXT,
    -- Upstream's identity for this article across versions, which is what a
    -- diff matches on. Numbers repeat and paths move; this does neither.
    stam_id    TEXT,
    -- Upstream's identity for this text of this article, which is the other
    -- half of the key the corpus guidelines require. stam_id says which
    -- article; this says which of its texts.
    versie_id  TEXT,
    -- `goed`, or `nogniet` for a provision published as a stub because it has
    -- not commenced. The difference between a provision that does not exist
    -- and one that does not apply yet.
    status     TEXT,
    -- The day this article took the form it has here. For a repealed provision
    -- it is the day it went, which is the difference between knowing a
    -- provision is gone and knowing when.
    in_force_from TEXT,
    -- How many tables this article prints. Its text stores one line per row
    -- with cells separated, which is a linearisation and not a grid, and an
    -- answer has to be able to say so.
    tables     INTEGER NOT NULL DEFAULT 0,
    -- How many images this article prints. bron holds none of them, and an
    -- article that renders a formula as an image is otherwise stored as its
    -- introduction followed by its variable definitions with the formula gone.
    illustrations INTEGER NOT NULL DEFAULT 0,
    -- The Juriconnect reference the register states for this article, carrying
    -- this rendition's own zicht and geldigheid dates.
    jci        TEXT,
    UNIQUE (version_id, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_articles_stam ON articles (stam_id);
CREATE INDEX IF NOT EXISTS idx_articles_amended_by ON articles (amended_by);
CREATE INDEX IF NOT EXISTS idx_articles_number ON articles (version_id, number);
CREATE INDEX IF NOT EXISTS idx_articles_path ON articles (version_id, path);

-- Leden carry their own spans. An article-level offset is too coarse to cite:
-- practice cites 'art. 6:162 lid 2 BW', so lid 2 needs its own byte range.
CREATE TABLE IF NOT EXISTS leden (
    id         INTEGER PRIMARY KEY,
    article_id INTEGER NOT NULL REFERENCES articles (id) ON DELETE CASCADE,
    ordinal    INTEGER NOT NULL,
    number     TEXT,
    content    TEXT NOT NULL,
    span_start INTEGER NOT NULL,
    span_end   INTEGER NOT NULL,
    -- The Juriconnect reference the register states for this lid.
    jci        TEXT,
    UNIQUE (article_id, ordinal)
);

CREATE TABLE IF NOT EXISTS cases (
    id             TEXT PRIMARY KEY,
    court          TEXT,
    decided_on     TEXT,
    published_on   TEXT,
    procedure_type TEXT,
    summary        TEXT,
    full_text      TEXT,
    blob           BLOB NOT NULL REFERENCES documents (blob),
    fetched_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cases_court ON cases (court);
CREATE INDEX IF NOT EXISTS idx_cases_decided ON cases (decided_on);

-- Formal relations between decisions, in both directions as published.
-- 'later' links are what answer the appeal question; their absence is a weaker
-- claim than their presence, which is why direction is stored explicitly.
CREATE TABLE IF NOT EXISTS case_relations (
    case_id   TEXT NOT NULL REFERENCES cases (id) ON DELETE CASCADE,
    target    TEXT NOT NULL,
    kind      TEXT,
    direction TEXT NOT NULL CHECK (direction IN ('earlier', 'later')),
    PRIMARY KEY (case_id, target, direction)
);

CREATE INDEX IF NOT EXISTS idx_relations_target ON case_relations (target);

CREATE TABLE IF NOT EXISTS publications (
    id            TEXT PRIMARY KEY,
    title         TEXT,
    dossier       TEXT,
    document_type TEXT,
    vergaderjaar  TEXT,
    published_on  TEXT,
    url           TEXT,
    full_text     TEXT,
    blob          BLOB REFERENCES documents (blob),
    fetched_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_publications_dossier ON publications (dossier);
CREATE INDEX IF NOT EXISTS idx_publications_date ON publications (published_on);

-- One capture of one page, as a third party recorded it.
--
-- Two dates, and they answer different questions. captured_at is when the
-- archive took the capture and is the date any answer built from this row is
-- bound to; fetched_at is when bron read it. Reporting the second as the first
-- would date a 2019 page to today, which is the error nobody notices.
--
-- archive_status is what the SITE answered the ARCHIVE, kept apart from
-- anything a direct fetch would report, because 'the site told us' and 'a third
-- party recorded' are different claims. NULL is not missing data: it is the
-- archive declining to say, which is what it writes for a robots-excluded
-- crawl, and it must not read as 'no capture'.
CREATE TABLE IF NOT EXISTS captures (
    id             TEXT PRIMARY KEY,
    urlkey         TEXT NOT NULL,
    original_url   TEXT NOT NULL,
    captured_at    TEXT NOT NULL,
    archive_status INTEGER,
    -- The archive's own hash of the payload, base32 over SHA-1. NULL on older
    -- rows, where there is nothing to check the replayed bytes against.
    index_digest   TEXT,
    -- How the replayed bytes stand against index_digest. NULL when the bytes
    -- were never fetched.
    --
    -- Only 'matches' is positive evidence. 'differs' is ordinary and says
    -- nothing about the capture: the index digest covers the payload as stored
    -- in the WARC, chunk framing included, and the replay strips that before
    -- serving, so the two disagree on captures that are entirely sound.
    -- Confirmed against the live archive with curl, which produces the same
    -- difference. Treating it as tampering failed 16 of 47 good captures.
    verified       TEXT CHECK (verified IN ('matches', 'differs', 'absent')),
    -- The archive stored no new payload and replayed an earlier capture's
    -- bytes, so the content is older than captured_at.
    is_revisit     INTEGER NOT NULL DEFAULT 0,
    mime_type      TEXT,
    -- Which encoding the text was read with, and what said so. A reading nobody
    -- can reproduce is not provenance.
    encoding       TEXT,
    encoding_from  TEXT,
    -- bron asked the archive to make this capture, rather than finding one it
    -- already held. A tool that cites evidence it caused should say which.
    --
    -- LOCAL TO THIS CORPUS. Never carry this column across into a shared or
    -- server-side corpus. The capture is public; who wanted it captured, and on
    -- what day, is not, and a row saying 'we asked for this one' is a record of
    -- interest rather than of evidence.
    requested      INTEGER NOT NULL DEFAULT 0,
    content        TEXT,
    -- NULL until the capture's bytes are fetched; an index read stores rows
    -- without spending a request per capture.
    blob           BLOB REFERENCES documents (blob),
    fetched_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_captures_url_time ON captures (urlkey, captured_at);
CREATE INDEX IF NOT EXISTS idx_captures_original ON captures (original_url);

-- Full-text search over the rendered text. A convenience for finding things,
-- not the citable representation: that is the spans above.
--
-- remove_diacritics 2 folds the accents in older statutory spelling; it leaves
-- the ij digraph alone, which unicode61 already treats as two letters.

CREATE VIRTUAL TABLE IF NOT EXISTS articles_fts USING fts5 (
    content,
    title,
    content = 'articles',
    content_rowid = 'id',
    tokenize = "unicode61 remove_diacritics 2"
);

CREATE TRIGGER IF NOT EXISTS articles_fts_insert AFTER INSERT ON articles BEGIN
    INSERT INTO articles_fts (rowid, content, title) VALUES (new.id, new.content, new.title);
END;

CREATE TRIGGER IF NOT EXISTS articles_fts_delete AFTER DELETE ON articles BEGIN
    INSERT INTO articles_fts (articles_fts, rowid, content, title)
    VALUES ('delete', old.id, old.content, old.title);
END;

CREATE TRIGGER IF NOT EXISTS articles_fts_update AFTER UPDATE ON articles BEGIN
    INSERT INTO articles_fts (articles_fts, rowid, content, title)
    VALUES ('delete', old.id, old.content, old.title);
    INSERT INTO articles_fts (rowid, content, title) VALUES (new.id, new.content, new.title);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS cases_fts USING fts5 (
    summary,
    full_text,
    content = 'cases',
    content_rowid = 'rowid',
    tokenize = "unicode61 remove_diacritics 2"
);

CREATE TRIGGER IF NOT EXISTS cases_fts_insert AFTER INSERT ON cases BEGIN
    INSERT INTO cases_fts (rowid, summary, full_text) VALUES (new.rowid, new.summary, new.full_text);
END;

CREATE TRIGGER IF NOT EXISTS cases_fts_delete AFTER DELETE ON cases BEGIN
    INSERT INTO cases_fts (cases_fts, rowid, summary, full_text)
    VALUES ('delete', old.rowid, old.summary, old.full_text);
END;

CREATE TRIGGER IF NOT EXISTS cases_fts_update AFTER UPDATE ON cases BEGIN
    INSERT INTO cases_fts (cases_fts, rowid, summary, full_text)
    VALUES ('delete', old.rowid, old.summary, old.full_text);
    INSERT INTO cases_fts (rowid, summary, full_text) VALUES (new.rowid, new.summary, new.full_text);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS publications_fts USING fts5 (
    title,
    full_text,
    content = 'publications',
    content_rowid = 'rowid',
    tokenize = "unicode61 remove_diacritics 2"
);

CREATE TRIGGER IF NOT EXISTS publications_fts_insert AFTER INSERT ON publications BEGIN
    INSERT INTO publications_fts (rowid, title, full_text) VALUES (new.rowid, new.title, new.full_text);
END;

CREATE TRIGGER IF NOT EXISTS publications_fts_delete AFTER DELETE ON publications BEGIN
    INSERT INTO publications_fts (publications_fts, rowid, title, full_text)
    VALUES ('delete', old.rowid, old.title, old.full_text);
END;

CREATE TRIGGER IF NOT EXISTS publications_fts_update AFTER UPDATE ON publications BEGIN
    INSERT INTO publications_fts (publications_fts, rowid, title, full_text)
    VALUES ('delete', old.rowid, old.title, old.full_text);
    INSERT INTO publications_fts (rowid, title, full_text) VALUES (new.rowid, new.title, new.full_text);
END;
