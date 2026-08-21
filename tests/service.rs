//! The wetgeving lane, against a service that answers on purpose.
//!
//! Every rendition, window and span in this lane is the service's answer, so
//! what has to be proved offline is how bron reads one: that a limit the
//! service states reaches the caller, that a failure keeps the kind and the
//! detail the service gave it, and that an envelope is read without being
//! frozen. Search is here too, because it is the one call whose parameter is
//! the caller's own question and the one whose answer must never be mistaken
//! for an absence.
//!
//! [`bron::sources::Client`] is blocking, so every call goes through
//! [`off_thread`]: driving it from inside the runtime serving the mock would
//! park the server answering it.

use std::path::PathBuf;

use bron::blob::BlobStore;
use bron::ids::BwbId;
use bron::service::{Ask, SearchRequest, Service};
use bron::sources::Client;
use bron::wire::Caveat;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Somewhere for one test's blobs and buckets to live.
struct Home {
    _dir: tempfile::TempDir,
    blobs: PathBuf,
    buckets: PathBuf,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path().join("blobs");
        let buckets = dir.path().join("buckets");
        Self {
            _dir: dir,
            blobs,
            buckets,
        }
    }
}

async fn off_thread<T, F>(home: &Home, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce(Client) -> T + Send + 'static,
{
    let (blobs, buckets) = (home.blobs.clone(), home.buckets.clone());

    tokio::task::spawn_blocking(move || f(Client::with_dirs(blobs, buckets).unwrap()))
        .await
        .unwrap()
}

fn law() -> BwbId {
    "BWBR0008691".parse().unwrap()
}

fn asking() -> Ask<'static> {
    Ask {
        as_of: "2018-01-01".parse().unwrap(),
        as_known_on: None,
        number: Some("1"),
        raw: false,
    }
}

/// One rendition chosen over a withdrawn one, shaped as the service shapes it.
const ANSWER: &str = r#"{
  "bron": 1,
  "grade": "authority",
  "as_of": "2018-01-01",
  "as_known_on": "2026-08-21",
  "data": {
    "regulation_id": "BWBR0008691",
    "expression": {
      "label": "2016-07-01_0",
      "in_force": {"from": "2016-07-01", "until": "2018-12-31"},
      "as_known": {"from": "2016-07-01"},
      "upstream_hash": "c390",
      "size": 49606,
      "withdrawn": false,
      "held": true,
      "verified": true,
      "blob": "35e29887c98339a23c835cd45fa22c0569f2db6e7d05fbca6c8de0d4fb15a0fb"
    },
    "articles": [{
      "number": "1",
      "label": "Artikel 1",
      "stam_id": "2760033",
      "versie_id": "21519612",
      "effect": "wijziging",
      "status": "goed",
      "content_hash": "a61a",
      "span": {"start": 7108, "end": 15301},
      "paragraphs": [
        {"number": "1", "rendered": "Eerste lid.", "span": {"start": 7200, "end": 7300}},
        {"number": "2", "rendered": "Tweede lid.", "span": {"start": 7300, "end": 7400}}
      ]
    }]
  },
  "caveats": [
    {"kind": "withdrawn", "excluded": ["2017-07-01_0"]},
    {"kind": "hash_disagrees"},
    {"kind": "something_this_bron_predates", "window": 4}
  ]
}"#;

async fn answering(body: &str, status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0008691/articles"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("Content-Type", "application/json")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread")]
async fn a_withdrawn_rendition_is_named_all_the_way_to_the_caller() {
    let server = answering(ANSWER, 200).await;
    let home = Home::new();
    let base = server.uri();

    let answered = off_thread(&home, move |client| {
        Service::at(&base)
            .articles(&client, &law(), &asking())
            .map_err(|e| e.to_string())
    })
    .await
    .expect("the service answered");

    assert_eq!(answered.data.expression.label, "2016-07-01_0");

    // The limit this whole lane exists to carry.
    assert!(
        answered.caveats.contains(&Caveat::WithdrawnRenditions {
            excluded: vec!["2017-07-01_0".into()],
        }),
        "{:?}",
        answered.caveats
    );

    // A limit stated under a name this bron has never heard of is reported
    // under that name rather than dropped: a reader one release behind must
    // still be able to see that something was said.
    assert!(
        answered.caveats.contains(&Caveat::Unrecognised {
            stated: "something_this_bron_predates".into(),
        }),
        "{:?}",
        answered.caveats
    );

    // The leden arrive in order and carry the numbers bron opens them with.
    let leden = &answered.data.articles[0].paragraphs;
    assert_eq!(leden.len(), 2);
    assert_eq!(leden[0].number.as_deref(), Some("1"));
    assert_eq!(leden[0].text(), "Eerste lid.");
}

/// The labels of the withdrawn renditions covering a date are the whole point
/// of that failure. Reporting the date as uncovered without them says the
/// register never had it, which is a different fact.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_keeps_the_kind_and_the_detail_the_service_gave_it() {
    let body = r#"{"bron":1,"grade":"authority","error":{
        "kind":"not_found",
        "message":"no published rendition of BWBR0008691 in force on [2016-01-01]",
        "detail":["2016-01-01_0","2016-06-16_0"]}}"#;
    let server = answering(body, 404).await;
    let home = Home::new();
    let base = server.uri();

    let fault = off_thread(&home, move |client| {
        Service::at(&base)
            .articles(&client, &law(), &asking())
            .err()
            .map(|e| (e.kind(), e.to_string()))
    })
    .await
    .expect("a 404 must not read as success");

    assert_eq!(fault.0, "not_found");
    assert!(fault.1.contains("2016-01-01_0"), "{}", fault.1);
    assert!(fault.1.contains("2016-06-16_0"), "{}", fault.1);
}

/// An envelope is nothing's evidence: no offset points into it, no ledger row
/// names it, and what it does point at is a blob already addressed by the hash
/// of itself. Freezing one would put a copy of every answer ever read in a
/// store that is deliberately never evicted.
#[tokio::test(flavor = "multi_thread")]
async fn an_answer_from_the_service_is_read_without_being_frozen() {
    let server = answering(ANSWER, 200).await;
    let home = Home::new();
    let base = server.uri();
    let blobs = home.blobs.clone();

    let held = off_thread(&home, move |client| {
        Service::at(&base)
            .articles(&client, &law(), &asking())
            .expect("the service answered");

        BlobStore::new(&blobs).contains(&bron::blob::BlobId::of(ANSWER.as_bytes()))
    })
    .await;

    assert!(!held, "the envelope was written to the blob store");
}

async fn searching(body: &str, status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/search"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("Content-Type", "application/json")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    server
}

fn asking_for(term: &str) -> SearchRequest {
    SearchRequest {
        q: term.to_string(),
        limit: 20,
        as_of: None,
        as_known_on: None,
    }
}

const RANKING: &str = r#"{"bron":1,"grade":"authority","data":{
    "searched":["wetgeving"],
    "hits":[{"source":"wetgeving","unit":"article","regulation_id":"BWBR0005289",
             "expression":"2026-07-16_0","number":"162","label":"Artikel 162","path":null,
             "content_hash":"a61a","distance":0.21}]},
  "caveats":[{"kind":"truncated","window":20}]}"#;

/// Search went round the one place that reads an envelope, so a failure from it
/// was classified by status code: a `usage` answer came back as `upstream`,
/// exit 4, where the same failure on any other endpoint exits 1.
#[tokio::test(flavor = "multi_thread")]
async fn a_search_failure_keeps_the_kind_the_service_gave_it() {
    let body = r#"{"bron":1,"grade":"authority","error":{
        "kind":"usage","message":"limit must be between 1 and 100","detail":[]}}"#;
    let server = searching(body, 400).await;
    let home = Home::new();
    let base = server.uri();

    let kind = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("onrechtmatige daad"))
            .err()
            .map(|e| e.kind())
    })
    .await
    .expect("a 400 must not read as success");

    assert_eq!(kind, "usage");
}

/// A limit on a ranking is the caller's to see. `--limit` cut this one and
/// nothing else in the answer says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_ranking_carries_the_limits_the_service_stated() {
    let server = searching(RANKING, 200).await;
    let home = Home::new();
    let base = server.uri();

    let answered = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("onrechtmatige daad"))
            .map(|answered| (answered.data.searched, answered.caveats))
            .map_err(|e| e.to_string())
    })
    .await
    .expect("the service answered");

    assert_eq!(answered.0, vec!["wetgeving".to_string()]);
    assert_eq!(answered.1, vec![Caveat::Truncated { window: 20 }]);
}

/// The question travels in a POST body precisely so it is not written down.
/// Freezing the answer writes it down anyway: the ranking for that question,
/// under a digest, in a store the guideline says is never evicted.
#[tokio::test(flavor = "multi_thread")]
async fn a_ranking_is_read_without_being_frozen() {
    let server = searching(RANKING, 200).await;
    let home = Home::new();
    let base = server.uri();
    let blobs = home.blobs.clone();

    let held = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("onrechtmatige daad"))
            .expect("the service answered");

        BlobStore::new(&blobs).contains(&bron::blob::BlobId::of(RANKING.as_bytes()))
    })
    .await;

    assert!(!held, "the ranking was written to the blob store");
}

/// The service shares no rate-limit budget, so nothing writes a deadline for
/// it. That leaves the delay it named as the only thing a caller can be told,
/// and this lane has no publisher to fall back to while it waits.
#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_from_the_service_says_how_long_it_asked_for() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0008691/articles"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "17"))
        .mount(&server)
        .await;

    let home = Home::new();
    let base = server.uri();

    let message = off_thread(&home, move |client| {
        Service::at(&base)
            .articles(&client, &law(), &asking())
            .err()
            .map(|e| e.chain().join(" | "))
    })
    .await
    .expect("a 429 must not read as success");

    assert!(message.contains("17s"), "{message}");
}
