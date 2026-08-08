//! End-to-end tests over the real documents in `tests/fixtures/`.
//!
//! The unit tests in each module use hand-written XML, which proves the parser
//! handles the format as the author imagined it. These use bytes that actually
//! came off the wire, which is a different and better question.

use chrono::{NaiveDate, TimeZone, Utc};

use bron::blob::BlobId;
use bron::db::{self, Db};
use bron::ids::{BwbId, CaptureId, Ecli};
use bron::model::{AppealStatus, Captured, DigestCheck, Direction, Validity};
use bron::sources::{
    FetchedArticle, FetchedCapture, FetchedLaw, FetchedLid, Provenance, Source, VersionMeta, bwb,
    rechtspraak, wayback,
};

const URL: &str = "https://repository.invalid/doc.xml";

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn date(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

/// A fixed timestamp. Writes take the clock as an argument precisely so a test
/// can pin it.
fn stamp() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0).unwrap()
}

fn bw6() -> BwbId {
    "BWBR0005289".parse().unwrap()
}

fn open_db() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_at(&dir.path().join("bron.db")).unwrap();
    (dir, db)
}

// -- the real manifest ---------------------------------------------------

#[test]
fn the_real_manifest_yields_a_full_version_history() {
    let body = fixture("bwb-manifest-BWBR0005289.xml");
    let versions = bwb::parse_manifest(&bw6(), &body).unwrap();

    assert_eq!(versions.len(), 63, "Boek 6 has 63 published versions");

    // Sorted oldest first, and exactly one open-ended version at the end.
    let open: Vec<_> = versions
        .iter()
        .filter(|v| v.in_force.is_open_ended())
        .collect();
    assert_eq!(open.len(), 1);
    assert_eq!(
        bwb::current_version(&versions).unwrap().version,
        "2026-07-16_0"
    );

    // Every version carries the publisher's own digest, which is what lets a
    // frozen copy be checked against the source rather than only against us.
    assert!(versions.iter().all(|v| v.upstream_hash.is_some()));
    assert!(versions.iter().all(|v| v.byte_len.is_some_and(|n| n > 0)));
}

#[test]
fn time_travel_selects_different_texts_for_different_dates() {
    let body = fixture("bwb-manifest-BWBR0005289.xml");
    let versions = bwb::parse_manifest(&bw6(), &body).unwrap();

    let in_2015 = bwb::version_on(&versions, date("2015-06-01")).unwrap();
    let in_2023 = bwb::version_on(&versions, date("2023-06-01")).unwrap();
    let now = bwb::version_on(&versions, date("2026-08-06")).unwrap();

    // The whole point: judging 2023 conduct must not reach for today's text.
    assert_ne!(in_2023.version, now.version);
    assert_ne!(in_2015.version, in_2023.version);

    assert!(in_2023.in_force.contains(date("2023-06-01")));
    assert!(!in_2023.in_force.contains(date("2026-08-06")));
}

#[test]
fn no_version_is_claimed_for_a_date_before_the_law_existed() {
    let body = fixture("bwb-manifest-BWBR0005289.xml");
    let versions = bwb::parse_manifest(&bw6(), &body).unwrap();
    assert!(bwb::version_on(&versions, date("1900-01-01")).is_none());
}

// -- the real law text ---------------------------------------------------

fn real_law() -> FetchedLaw {
    let body = fixture("bwb-law-BWBR0005289-excerpt.xml");
    let meta = VersionMeta {
        version: "2026-07-16_0".into(),
        in_force: Validity {
            from: date("2026-07-16"),
            until: None,
        },
        visible: Some(Validity {
            from: date("2026-07-16"),
            until: None,
        }),
        upstream_hash: None,
        byte_len: None,
    };
    // Leaked so spans can be checked against the same bytes that were parsed.
    let body: &'static str = Box::leak(body.into_boxed_str());
    bwb::parse(
        Source::new(BlobId::of(body.as_bytes()), body, URL),
        &bw6(),
        &meta,
    )
    .unwrap()
}

#[test]
fn parses_the_real_law_document() {
    let law = real_law();
    assert_eq!(law.title, "Burgerlijk Wetboek Boek 6");
    assert_eq!(law.law_type.as_deref(), Some("wet"));
    assert_eq!(law.articles.len(), 4);
    assert_eq!(law.articles[0].number, "1");
    assert_eq!(
        law.articles[0].path.as_deref(),
        Some("/Boek6/Titeldeel1/Afdeling1/Artikel1")
    );
}

#[test]
fn real_article_text_is_free_of_provenance_metadata() {
    let law = real_law();

    // The excerpt keeps its real <meta-data> blocks, which is the whole reason
    // this fixture exists. Nothing from them may reach statutory text.
    for article in &law.articles {
        assert!(!article.content.contains("jci1.3"), "{}", article.content);
        assert!(!article.content.contains("BWBR"), "{}", article.content);
        assert!(!article.content.is_empty());
    }

    assert_eq!(
        law.articles[0].content,
        "Verbintenissen kunnen slechts ontstaan, indien dit uit de wet voortvloeit."
    );
}

#[test]
fn real_spans_quote_the_document_they_came_from() {
    let body = fixture("bwb-law-BWBR0005289-excerpt.xml");
    let body: &'static str = Box::leak(body.into_boxed_str());
    let meta = VersionMeta {
        version: "v".into(),
        in_force: Validity {
            from: date("2026-07-16"),
            until: None,
        },
        visible: Some(Validity {
            from: date("2026-07-16"),
            until: None,
        }),
        upstream_hash: None,
        byte_len: None,
    };
    let law = bwb::parse(
        Source::new(BlobId::of(body.as_bytes()), body, URL),
        &bw6(),
        &meta,
    )
    .unwrap();

    for article in &law.articles {
        let quoted = article
            .span
            .slice(body)
            .expect("article span is inside the document");
        assert!(
            quoted.starts_with("<artikel"),
            "{}",
            &quoted[..40.min(quoted.len())]
        );
        assert!(quoted.ends_with("</artikel>"));

        for lid in &article.leden {
            let quoted = lid
                .span
                .slice(body)
                .expect("lid span is inside the document");
            assert!(quoted.starts_with("<lid") || quoted.starts_with("<artikel"));
        }
    }
}

#[test]
fn leden_are_counted_once_each() {
    let law = real_law();
    let article_2 = law.articles.iter().find(|a| a.number == "2").unwrap();

    // Article 6:2 has two leden. Collecting them by descending rather than by
    // structural child would double-count any that nest.
    assert_eq!(article_2.leden.len(), 2);
    assert_eq!(article_2.leden[0].number.as_deref(), Some("1"));
    assert_eq!(article_2.leden[1].number.as_deref(), Some("2"));

    let joined = article_2
        .leden
        .iter()
        .map(|l| l.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(article_2.content, joined);
}

// -- the real decision ---------------------------------------------------

#[test]
fn parses_the_real_decision_with_its_relations() {
    let body = fixture("rechtspraak-content-ECLI-NL-HR-2019-1278.xml");
    let ecli: Ecli = "ECLI:NL:HR:2019:1278".parse().unwrap();
    let case =
        rechtspraak::parse(Source::new(BlobId::of(body.as_bytes()), &body, URL), &ecli).unwrap();

    assert_eq!(case.court.as_deref(), Some("Hoge Raad"));
    assert_eq!(case.decided_on, Some(date("2019-07-19")));
    assert!(case.full_text.is_some_and(|t| t.len() > 1_000));

    // One relation, pointing backwards at the conclusie.
    assert_eq!(case.relations.len(), 1);
    assert_eq!(case.relations[0].direction, Direction::Earlier);
    assert_eq!(case.relations[0].kind.as_deref(), Some("conclusie"));
    assert_eq!(case.relations[0].target.as_str(), "ECLI:NL:PHR:2019:496");
}

// -- storage -------------------------------------------------------------

fn article(number: &str, path: &str, text: &str) -> FetchedArticle {
    FetchedArticle {
        stam_id: Some(format!("stam-{path}")),
        number: number.into(),
        label: Some(format!("Artikel {number}")),
        title: None,
        path: Some(path.into()),
        content: text.into(),
        span: bron::model::Span {
            start: 0,
            end: text.len() as u32,
        },
        leden: vec![FetchedLid {
            number: None,
            content: text.into(),
            span: bron::model::Span {
                start: 0,
                end: text.len() as u32,
            },
            jci: Some(format!("jci1.3:c:BWBR0005289&artikel={number}")),
        }],
        amended_by: "Stb.2023-41".parse().ok(),
        effect: Some("wijziging".into()),
        in_force_from: "1992-01-01".parse().ok(),
        tables: 0,
        illustrations: 0,
        versie_id: Some(format!("versie-{path}")),
        status: Some("goed".into()),
        jci: Some(format!("jci1.3:c:BWBR0005289&artikel={number}")),
    }
}

#[test]
fn a_law_round_trips_through_the_corpus() {
    let (_dir, mut db) = open_db();
    let law = real_law();

    let tx = db.transaction().unwrap();
    db::put_law_version(&tx, &law, stamp()).unwrap();
    tx.commit().unwrap();

    let stored = db.law(&bw6()).unwrap().expect("law was stored");
    assert_eq!(stored.title, "Burgerlijk Wetboek Boek 6");
    assert_eq!(stored.version_count, 1);

    let version = db.newest_version(&bw6()).unwrap().unwrap();
    assert_eq!(version.article_count, 4);
    assert_eq!(
        version.blob, law.provenance.blob,
        "the version names the document it came from"
    );

    let id = db.version_id(&bw6(), &version.version).unwrap().unwrap();
    let articles = db.articles(id, None).unwrap();
    assert_eq!(articles.len(), 4);
    assert_eq!(articles[0].leden.len(), 1);
    assert_eq!(
        articles[0].span, law.articles[0].span,
        "spans survive the round trip"
    );
}

/// The Awb carries 584 articles across 582 distinct numbers: each of its three
/// annexes restarts at Artikel 1. Keying articles on the printed number would
/// abort the whole import with a unique-constraint failure.
#[test]
fn a_law_whose_annexes_repeat_article_numbers_stores_completely() {
    let (_dir, mut db) = open_db();
    let awb: BwbId = "BWBR0005537".parse().unwrap();

    let law = FetchedLaw {
        id: awb.clone(),
        title: "Algemene wet bestuursrecht".into(),
        law_type: Some("wet".into()),
        version: "2026-08-01_0".into(),
        in_force: Validity {
            from: date("2026-08-01"),
            until: None,
        },
        visible: Some(Validity {
            from: date("2026-08-01"),
            until: None,
        }),
        upstream_hash: None,
        provenance: Provenance {
            blob: BlobId::of(b"awb"),
            url: URL.into(),
            byte_len: 3,
        },
        skipped_articles: 0,
        articles: vec![
            article("1", "/Hoofdstuk1/Artikel1", "Hoofdtekst artikel 1."),
            article("2", "/Hoofdstuk1/Artikel2", "Hoofdtekst artikel 2."),
            article("1", "/BijlageBevoegdheid/Artikel1", "Bijlage artikel 1."),
            article("2", "/BijlageBevoegdheid/Artikel2", "Bijlage artikel 2."),
        ],
    };

    let tx = db.transaction().unwrap();
    db::put_law_version(&tx, &law, stamp()).unwrap();
    tx.commit().unwrap();

    let version = db.newest_version(&awb).unwrap().unwrap();
    assert_eq!(version.article_count, 4, "no article was dropped");

    // Asking for article 1 returns both, and the path tells them apart.
    let id = db.version_id(&awb, &version.version).unwrap().unwrap();
    let ones = db.articles(id, Some("1")).unwrap();
    assert_eq!(ones.len(), 2);
    assert_eq!(ones[0].path.as_deref(), Some("/Hoofdstuk1/Artikel1"));
    assert_eq!(
        ones[1].path.as_deref(),
        Some("/BijlageBevoegdheid/Artikel1")
    );
}

#[test]
fn two_versions_of_one_law_coexist_and_resolve_by_date() {
    let (_dir, mut db) = open_db();
    let base = real_law();

    let old = FetchedLaw {
        version: "2015-01-01_0".into(),
        in_force: Validity {
            from: date("2015-01-01"),
            until: Some(date("2026-07-15")),
        },
        visible: Some(Validity {
            from: date("2015-01-01"),
            until: None,
        }),
        provenance: Provenance {
            blob: BlobId::of(b"old text"),
            url: URL.into(),
            byte_len: 8,
        },
        articles: vec![article("1", "/Boek6/Artikel1", "De oude tekst.")],
        ..base.clone()
    };

    let tx = db.transaction().unwrap();
    for law in [&old, &base] {
        db::put_law_version(&tx, law, stamp()).unwrap();
    }
    tx.commit().unwrap();

    assert_eq!(db.versions(&bw6()).unwrap().len(), 2);
    assert_eq!(db.law(&bw6()).unwrap().unwrap().version_count, 2);

    let then = db
        .version_in_force(&bw6(), date("2020-01-01"))
        .unwrap()
        .unwrap();
    assert_eq!(then.version, "2015-01-01_0");

    let now = db
        .version_in_force(&bw6(), date("2026-08-06"))
        .unwrap()
        .unwrap();
    assert_eq!(now.version, "2026-07-16_0");

    // Same article number, two versions, different text.
    let then_id = db.version_id(&bw6(), &then.version).unwrap().unwrap();
    let now_id = db.version_id(&bw6(), &now.version).unwrap().unwrap();
    assert_eq!(
        db.articles(then_id, Some("1")).unwrap()[0].content,
        "De oude tekst."
    );
    assert!(
        db.articles(now_id, Some("1")).unwrap()[0]
            .content
            .starts_with("Verbintenissen")
    );
}

#[test]
fn restoring_a_version_replaces_its_articles_rather_than_accumulating_them() {
    let (_dir, mut db) = open_db();
    let law = real_law();

    for _ in 0..3 {
        let tx = db.transaction().unwrap();
        db::put_law_version(&tx, &law, stamp()).unwrap();
        tx.commit().unwrap();
    }

    assert_eq!(db.versions(&bw6()).unwrap().len(), 1);
    assert_eq!(db.newest_version(&bw6()).unwrap().unwrap().article_count, 4);
}

#[test]
fn a_failed_write_leaves_the_corpus_untouched() {
    let (_dir, mut db) = open_db();
    let law = real_law();

    let tx = db.transaction().unwrap();
    db::put_law_version(&tx, &law, stamp()).unwrap();
    drop(tx); // rolls back

    assert!(db.law(&bw6()).unwrap().is_none());
    assert_eq!(db.stats().unwrap().articles, 0);
}

// -- the appeal question -------------------------------------------------

fn store_case(db: &mut Db, id: &str, relations: Vec<bron::model::Relation>) {
    let case = bron::sources::FetchedCase {
        id: id.parse().unwrap(),
        court: Some("Hoge Raad".into()),
        decided_on: Some(date("2019-07-19")),
        published_on: None,
        procedure_type: None,
        summary: None,
        full_text: None,
        provenance: Provenance {
            blob: BlobId::of(id.as_bytes()),
            url: URL.into(),
            byte_len: id.len(),
        },
        relations,
    };

    let tx = db.transaction().unwrap();
    db::put_case(&tx, &case, stamp()).unwrap();
    tx.commit().unwrap();
}

fn relation(target: &str, direction: Direction) -> bron::model::Relation {
    bron::model::Relation {
        target: target.parse().unwrap(),
        kind: Some("cassatie".into()),
        direction,
    }
}

#[test]
fn appeal_status_distinguishes_its_three_answers() {
    let (_dir, mut db) = open_db();

    // Never seen: nothing can be said.
    let absent: Ecli = "ECLI:NL:RBAMS:2017:1017".parse().unwrap();
    assert_eq!(db.appeal_status(&absent).unwrap(), AppealStatus::Unknown);

    // Stored, carries a forward link.
    store_case(
        &mut db,
        "ECLI:NL:GHARL:2015:10068",
        vec![relation("ECLI:NL:HR:2016:2326", Direction::Later)],
    );
    let appealed: Ecli = "ECLI:NL:GHARL:2015:10068".parse().unwrap();
    assert_eq!(
        db.appeal_status(&appealed).unwrap(),
        AppealStatus::Appealed {
            successors: vec!["ECLI:NL:HR:2016:2326".parse().unwrap()]
        }
    );

    // Stored, no forward link. This is the strongest true claim available, and
    // it is deliberately not "was not appealed".
    store_case(&mut db, "ECLI:NL:RBAMS:2020:1", vec![]);
    let quiet: Ecli = "ECLI:NL:RBAMS:2020:1".parse().unwrap();
    assert!(matches!(
        db.appeal_status(&quiet).unwrap(),
        AppealStatus::NoPublishedSuccessor { as_of } if as_of == stamp()
    ));
}

#[test]
fn a_backward_relation_alone_is_not_an_appeal() {
    let (_dir, mut db) = open_db();

    // A cassation decision points back at the hof. That says something was
    // appealed *to* it, not that it was itself appealed.
    store_case(
        &mut db,
        "ECLI:NL:HR:2016:2326",
        vec![relation("ECLI:NL:GHARL:2015:10068", Direction::Earlier)],
    );

    let hr: Ecli = "ECLI:NL:HR:2016:2326".parse().unwrap();
    assert!(matches!(
        db.appeal_status(&hr).unwrap(),
        AppealStatus::NoPublishedSuccessor { .. }
    ));
}

// -- schema guard --------------------------------------------------------

/// Neither direction is safe to open blind. A newer corpus may hold columns
/// this build cannot read; an older one cannot be migrated by re-running
/// `CREATE TABLE IF NOT EXISTS`, which cannot add a column, so accepting it
/// would stamp it current while it still held the old shape.
#[test]
fn a_corpus_from_another_schema_is_refused_in_both_directions() {
    for foreign in [99, 1_000] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bron.db");

        Db::open_at(&path).unwrap();
        rusqlite::Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", foreign)
            .unwrap();

        let error = match Db::open_at(&path) {
            Ok(_) => panic!("schema v{foreign} was accepted"),
            Err(e) => e.to_string(),
        };
        assert!(error.contains(&format!("schema v{foreign}")), "{error}");
    }
}

// -- the two findings that mattered most ---------------------------------

/// The manifest omitting zicht must not be answered with geldigheid. Inventing
/// the window asserts the register showed a text exactly while it applied, which
/// is the claim the two windows were separated to keep apart.
#[test]
fn a_manifest_without_zicht_dates_leaves_zicht_absent() {
    const NO_ZICHT: &str = r#"<work label="BWBR0005289">
        <expression label="2020-01-01_0"><metadata>
          <datum_inwerkingtreding>2020-01-01</datum_inwerkingtreding>
          <einddatum>9999-12-31</einddatum>
        </metadata></expression>
      </work>"#;

    let versions = bwb::parse_manifest(&bw6(), NO_ZICHT).unwrap();
    assert_eq!(versions[0].in_force.from, date("2020-01-01"));
    assert_eq!(
        versions[0].visible, None,
        "zicht must not be fabricated from geldigheid"
    );
}

#[test]
fn absent_zicht_survives_the_round_trip_as_absent() {
    let (_dir, mut db) = open_db();
    let law = FetchedLaw {
        visible: None,
        ..real_law()
    };

    let tx = db.transaction().unwrap();
    db::put_law_version(&tx, &law, stamp()).unwrap();
    tx.commit().unwrap();

    assert_eq!(db.newest_version(&bw6()).unwrap().unwrap().visible, None);
}

/// Every write records the document it came from. Before this, the only callers
/// of the ledger were tests supplying a url by hand, so no real path could
/// populate it and every `documents` row was a fiction.
#[test]
fn storing_a_law_records_its_document_in_the_provenance_ledger() {
    let (_dir, mut db) = open_db();
    let law = real_law();

    let tx = db.transaction().unwrap();
    db::put_law_version(&tx, &law, stamp()).unwrap();
    tx.commit().unwrap();

    assert_eq!(
        db.stats().unwrap().documents,
        1,
        "the ledger was written without being asked"
    );

    let document = db
        .document(&law.provenance.blob)
        .unwrap()
        .expect("ledger row");
    assert_eq!(document.url, URL);
    assert_eq!(document.byte_len, law.provenance.byte_len);
    assert_eq!(document.fetched_at, stamp());
}

#[test]
fn storing_a_case_records_its_document_too() {
    let (_dir, mut db) = open_db();
    store_case(&mut db, "ECLI:NL:HR:2019:1278", vec![]);
    assert_eq!(db.stats().unwrap().documents, 1);
}

/// An article whose body sits outside `<al>` is not silently dropped from a
/// legal corpus; the count of what was skipped travels with the parse.
#[test]
fn articles_that_yield_no_text_are_counted_rather_than_vanishing() {
    const WITH_EMPTY: &str = r#"<toestand><wetgeving soort="wet">
        <citeertitel>Test</citeertitel>
        <artikel label="Artikel 1"><kop><nr>1</nr></kop><al>Tekst.</al></artikel>
        <artikel label="Artikel 2"><kop><nr>2</nr></kop></artikel>
        <artikel label="Artikel 3"><kop><nr>3</nr></kop></artikel>
      </wetgeving></toestand>"#;

    let meta = VersionMeta {
        version: "v".into(),
        in_force: Validity {
            from: date("2020-01-01"),
            until: None,
        },
        visible: None,
        upstream_hash: None,
        byte_len: None,
    };
    let law = bwb::parse(
        Source::new(BlobId::of(WITH_EMPTY.as_bytes()), WITH_EMPTY, URL),
        &bw6(),
        &meta,
    )
    .unwrap();

    assert_eq!(law.articles.len(), 1);
    assert_eq!(law.skipped_articles, 2);
}

/// A blob that no longer hashes to its own name must not yield a confident
/// quotation. The citation path reads through verification.
#[test]
fn a_tampered_blob_is_refused_on_the_citation_path() {
    let dir = tempfile::tempdir().unwrap();
    let store = bron::blob::BlobStore::new(dir.path());

    let id = store.put(b"<al>De echte tekst.</al>").unwrap();
    assert_eq!(
        store.get_verified(&id).unwrap(),
        b"<al>De echte tekst.</al>"
    );

    let path = dir
        .path()
        .join(&id.to_hex()[0..2])
        .join(&id.to_hex()[2..4])
        .join(id.to_hex());
    std::fs::write(&path, b"<al>De valse tekst.</al>").unwrap();

    // get() still hands back what is on disk; get_verified refuses it.
    assert_eq!(store.get(&id).unwrap(), b"<al>De valse tekst.</al>");
    let error = match store.get_verified(&id) {
        Ok(_) => panic!("a tampered blob was accepted"),
        Err(e) => e.to_string(),
    };
    assert!(error.contains("does not match"), "{error}");
}

// -- overgangsrecht ------------------------------------------------------

/// A version that applies from before it was published is the ordinary case in
/// tax legislation: 612 of the 733 versions of the Wet IB 2001 do it. It is
/// read off the manifest, never inferred.
#[test]
fn a_version_that_predates_its_own_publication_is_marked_retroactive() {
    const RETRO: &str = r#"<work label="BWBR0011353">
        <expression label="2015-04-01_0"><metadata>
          <datum_inwerkingtreding>2015-04-01</datum_inwerkingtreding>
          <einddatum>9999-12-31</einddatum>
          <zichtdatum_start>2015-04-01</zichtdatum_start>
          <zichtdatum_eind>2020-12-31</zichtdatum_eind>
        </metadata></expression>
        <expression label="2015-04-01_4"><metadata>
          <datum_inwerkingtreding>2015-04-01</datum_inwerkingtreding>
          <einddatum>9999-12-31</einddatum>
          <zichtdatum_start>2021-01-01</zichtdatum_start>
          <zichtdatum_eind>9999-12-31</zichtdatum_eind>
        </metadata></expression>
      </work>"#;

    let id: BwbId = "BWBR0011353".parse().unwrap();
    let versions = bwb::parse_manifest(&id, RETRO).unwrap();

    let original = versions
        .iter()
        .find(|v| v.version == "2015-04-01_0")
        .unwrap();
    let amended = versions
        .iter()
        .find(|v| v.version == "2015-04-01_4")
        .unwrap();

    assert!(!original.is_retroactive());
    assert!(
        amended.is_retroactive(),
        "applies from 2015 but only visible from 2021"
    );
}

/// The two windows can resolve one date to two different texts, and a caller
/// handed only one of them cannot know the other exists. Which governs is
/// transitional law and not bron's question; that both exist is.
#[test]
fn the_two_windows_can_disagree_about_the_same_date() {
    const RETRO: &str = r#"<work label="BWBR0011353">
        <expression label="2015-04-01_0"><metadata>
          <datum_inwerkingtreding>2015-04-01</datum_inwerkingtreding>
          <einddatum>2020-12-31</einddatum>
          <zichtdatum_start>2015-04-01</zichtdatum_start>
          <zichtdatum_eind>2020-12-31</zichtdatum_eind>
        </metadata></expression>
        <expression label="2015-04-01_4"><metadata>
          <datum_inwerkingtreding>2015-04-01</datum_inwerkingtreding>
          <einddatum>9999-12-31</einddatum>
          <zichtdatum_start>2021-01-01</zichtdatum_start>
          <zichtdatum_eind>9999-12-31</zichtdatum_eind>
        </metadata></expression>
      </work>"#;

    let id: BwbId = "BWBR0011353".parse().unwrap();
    let versions = bwb::parse_manifest(&id, RETRO).unwrap();
    let on = date("2015-06-01");

    // Geldigheid picks the amended text, which nobody could read in 2015.
    let in_force = bwb::version_on(&versions, on).unwrap();
    assert_eq!(in_force.version, "2015-04-01_4");

    // Zicht picks what the register actually showed that day.
    let as_known = versions
        .iter()
        .rev()
        .find(|v| v.visible.is_some_and(|z| z.contains(on)))
        .unwrap();
    assert_eq!(as_known.version, "2015-04-01_0");

    assert_ne!(in_force.version, as_known.version);
}

// -- captures ------------------------------------------------------------

fn capture(
    timestamp: &str,
    original: &str,
    status: wayback::ArchiveStatus,
    body: &[u8],
) -> FetchedCapture {
    let digest = wayback::digest_of(body);
    // A real capture takes its key from the index. Standing the approximation
    // in for it is a fixture convenience and has to be spelled out, which is
    // the point of the two types being separate.
    let id = CaptureId::new(
        timestamp.parse().unwrap(),
        wayback::urlkey_of(original).unwrap().as_str().to_string(),
    );

    FetchedCapture {
        id,
        original: original.to_string(),
        status,
        digest: Some(digest),
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

fn store(db: &mut Db, captures: &[FetchedCapture]) {
    let tx = db.transaction().unwrap();
    for capture in captures {
        db::put_capture(&tx, capture, false, stamp()).unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn a_capture_answers_the_date_it_was_taken_and_not_the_date_it_was_read() {
    let (_dir, mut db) = open_db();
    let url = "https://steddle.com/pricing";

    store(
        &mut db,
        &[capture(
            "20240101120000",
            url,
            wayback::ArchiveStatus::Answered(200),
            b"old",
        )],
    );

    let key = wayback::urlkey_of(url).unwrap();
    let held = db.capture_at(&key, date("2024-06-01")).unwrap().unwrap();

    // The capture is from January; it was read in August. An answer bound to
    // the read date would date a 2024 page to today.
    assert_eq!(held.captured_at.date_naive(), date("2024-01-01"));
    assert_eq!(held.fetched_at, stamp());
}

#[test]
fn as_of_resolves_to_the_newest_capture_at_or_before_the_date() {
    let (_dir, mut db) = open_db();
    let url = "https://steddle.com/pricing";

    store(
        &mut db,
        &[
            capture(
                "20240101120000",
                url,
                wayback::ArchiveStatus::Answered(200),
                b"january",
            ),
            capture(
                "20240601120000",
                url,
                wayback::ArchiveStatus::Answered(200),
                b"june",
            ),
            capture(
                "20241201120000",
                url,
                wayback::ArchiveStatus::Answered(200),
                b"december",
            ),
        ],
    );

    let key = wayback::urlkey_of(url).unwrap();
    let held = |on: &str| db.capture_at(&key, date(on)).unwrap().unwrap();

    assert_eq!(held("2024-03-01").content.as_deref(), Some("january"));
    assert_eq!(held("2024-06-30").content.as_deref(), Some("june"));
    // The capture taken that same day counts: the bound is the whole day.
    assert_eq!(held("2024-06-01").content.as_deref(), Some("june"));
    assert_eq!(held("2025-01-01").content.as_deref(), Some("december"));
}

#[test]
fn a_date_before_every_capture_holds_nothing_rather_than_the_nearest_one() {
    let (_dir, mut db) = open_db();
    let url = "https://steddle.com/pricing";

    store(
        &mut db,
        &[capture(
            "20240601120000",
            url,
            wayback::ArchiveStatus::Answered(200),
            b"x",
        )],
    );

    let key = wayback::urlkey_of(url).unwrap();

    // Answering with the June capture would evidence what the page said in
    // January using bytes that did not exist yet.
    assert!(db.capture_at(&key, date("2024-01-01")).unwrap().is_none());
}

#[test]
fn a_withheld_status_survives_the_corpus_as_unknown() {
    let (_dir, mut db) = open_db();
    let url = "https://steddle.com/blocked";

    store(
        &mut db,
        &[capture(
            "20240101120000",
            url,
            wayback::ArchiveStatus::Unknown,
            b"body",
        )],
    );

    let key = wayback::urlkey_of(url).unwrap();
    let held = db.capture_at(&key, date("2024-06-01")).unwrap().unwrap();

    assert_eq!(held.archive_status, None);
    // Not `No`: there is a capture and the archive will not say what it got.
    assert_eq!(held.captured_the_page(), Captured::Unknown);
}

#[test]
fn a_captured_error_page_reads_back_as_not_the_page() {
    let (_dir, mut db) = open_db();
    let url = "https://steddle.com/gone";

    store(
        &mut db,
        &[capture(
            "20240101120000",
            url,
            wayback::ArchiveStatus::Answered(404),
            b"404",
        )],
    );

    let key = wayback::urlkey_of(url).unwrap();
    let held = db.capture_at(&key, date("2024-06-01")).unwrap().unwrap();

    assert_eq!(held.archive_status, Some(404));
    assert_eq!(held.captured_the_page(), Captured::No);
}

#[test]
fn an_unverifiable_capture_is_not_stored_as_a_verified_one() {
    let (_dir, mut db) = open_db();
    let url = "https://steddle.com/old";

    let mut old = capture(
        "20040101120000",
        url,
        wayback::ArchiveStatus::Answered(200),
        b"ancient",
    );
    old.digest = None;
    old.verified = wayback::Verified::Absent;
    store(&mut db, &[old]);

    let key = wayback::urlkey_of(url).unwrap();
    let held = db.capture_at(&key, date("2024-01-01")).unwrap().unwrap();

    // Two different answers, and the corpus keeps them apart.
    assert_eq!(held.verified, Some(DigestCheck::Absent));
    assert_ne!(held.verified, Some(DigestCheck::Matches));
}

#[test]
fn a_capture_bron_caused_stays_marked_when_a_later_read_finds_it() {
    let (_dir, mut db) = open_db();
    let url = "https://steddle.com/pricing";
    let row = capture(
        "20240101120000",
        url,
        wayback::ArchiveStatus::Answered(200),
        b"x",
    );

    // Asked for, then simply found again by an ordinary read.
    let tx = db.transaction().unwrap();
    db::put_capture(&tx, &row, true, stamp()).unwrap();
    db::put_capture(&tx, &row, false, stamp()).unwrap();
    tx.commit().unwrap();

    let key = wayback::urlkey_of(url).unwrap();
    let held = db.capture_at(&key, date("2024-06-01")).unwrap().unwrap();

    // Re-reading a capture does not undo the fact that bron caused it.
    assert!(
        held.requested,
        "a later read must not launder how the capture came to exist"
    );
}

#[test]
fn two_spellings_of_one_address_reach_one_capture() {
    let (_dir, mut db) = open_db();

    store(
        &mut db,
        &[capture(
            "20240101120000",
            "https://www.steddle.com/x",
            wayback::ArchiveStatus::Answered(200),
            b"x",
        )],
    );

    // The record is one record, so every spelling of its address finds it.
    for spelling in [
        "https://steddle.com/x",
        "http://www.steddle.com/x",
        "https://STEDDLE.com/x",
    ] {
        let key = wayback::urlkey_of(spelling).unwrap();
        assert!(
            db.capture_at(&key, date("2024-06-01")).unwrap().is_some(),
            "{spelling}"
        );
    }
}

#[test]
fn every_capture_records_its_document_in_the_ledger() {
    let (_dir, mut db) = open_db();
    let url = "https://steddle.com/pricing";

    store(
        &mut db,
        &[capture(
            "20240101120000",
            url,
            wayback::ArchiveStatus::Answered(200),
            b"body",
        )],
    );

    // Provenance is never optional: the row and the document it came from are
    // written together or not at all.
    let documents = db.documents().unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].blob, BlobId::of(b"body"));
}
