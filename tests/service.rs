//! The wetgeving lane, against a service that answers on purpose.
//!
//! Every rendition, window and span in this lane is the service's answer, so
//! what has to be proved offline is how bron reads one: that a limit the
//! service states reaches the caller, that a failure keeps the kind and the
//! detail the service gave it, and that the two shapes an answer comes in are
//! read apart. Search is here too, because it is the one call whose parameter is
//! the caller's own question and the one whose answer must never be mistaken
//! for an absence.
//!
//! [`bron::sources::Client`] is blocking, so every call goes through
//! [`off_thread`]: driving it from inside the runtime serving the mock would
//! park the server answering it.

use std::path::PathBuf;

use bron::ids::BwbId;
use bron::service::{Ask, SearchData, SearchRequest, Service};
use bron::sources::Client;
use bron::wire::Caveat;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Somewhere for one test's rate-limit buckets to live.
struct Home {
    _dir: tempfile::TempDir,
    buckets: PathBuf,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let buckets = dir.path().join("buckets");
        Self { _dir: dir, buckets }
    }
}

async fn off_thread<T, F>(home: &Home, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce(Client) -> T + Send + 'static,
{
    let buckets = home.buckets.clone();

    tokio::task::spawn_blocking(move || f(Client::with_buckets(buckets).unwrap()))
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
        not_commenced: false,
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

    // A limit stated under a name this bron has never heard of is carried
    // whole rather than dropped: a reader one release behind must still be able
    // to see what was said.
    assert!(
        answered.caveats.contains(&Caveat::Unrecognised {
            stated: serde_json::json!({"kind": "something_this_bron_predates", "window": 4}),
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
        as_of: "2026-08-24".parse().unwrap(),
        as_known_on: None,
    }
}

const RANKING: &str = r#"{"bron":1,"grade":"authority","data":{"kind":"ranking",
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

    let caveats = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("onrechtmatige daad"))
            .map(|answered| answered.caveats)
            .map_err(|e| e.to_string())
    })
    .await
    .expect("the service answered");

    assert_eq!(caveats, vec![Caveat::Truncated { window: 20 }]);
}

/// The service shares no rate-limit budget, so nothing writes a deadline for
/// it. That leaves the delay it named as the only thing a caller can be told,
/// and the service is the only route this lane has while it waits.
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

/// A citation the register reads two ways, answered both ways. The service
/// keeps this shape apart from a ranking because a resolved entry carries no
/// distance, and one invented for it would lie about how the answer came about.
const LOOKUP: &str = r#"{"bron":1,"grade":"authority","as_of":"2026-08-24","as_known_on":"2026-08-24",
  "data":{"kind":"lookup","citation":"8:4","resolved":[
    {"reading":"book","source":"wetgeving","regulation_id":"BWBR0005034",
     "expression":"2026-07-01_0","held":true,"number":"4","label":"Artikel 4",
     "path":"/Boek8/HoofdstukI/Titeldeel1/Artikel4",
     "content_hash":"e0bd9a23c78b4c6a14be2e4e87c7ca61b706e1a5edecd4d409416e5998d8d199",
     "caveats":[]},
    {"reading":"literal","source":"wetgeving","regulation_id":"BWBR0005537",
     "expression":"2026-08-15_0","held":true,"number":"8:4","label":"Artikel 8:4",
     "path":"/Hoofdstuk8/Titeldeel8.1/Afdeling8.1.1/Artikel8:4",
     "content_hash":"4ba2172e7f7630289a08992d33c6a8b181dd6167300989f143f3a506357c6364",
     "caveats":[{"kind":"hash_disagrees"}]}]},
  "caveats":[{"kind":"several_readings","readings":["book","literal"]}]}"#;

/// A reading that resolved to a rendition whose text the corpus does not hold.
/// The dates resolved and the rendition is named; whether it carries the
/// article is a fact nobody has yet, so no hash travels.
const LOOKUP_UNHELD: &str = r#"{"bron":1,"grade":"authority",
  "data":{"kind":"lookup","citation":"3:305a","resolved":[
    {"reading":"book","source":"wetgeving","regulation_id":"BWBR0005291",
     "expression":"2025-07-01_0","held":false,"number":"305a","caveats":[]}]},
  "caveats":[{"kind":"reading_unresolved","reading":"literal","because":"no_regulation_prints_it"}]}"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_citation_is_looked_up_rather_than_ranked() {
    let server = searching(LOOKUP, 200).await;
    let home = Home::new();
    let base = server.uri();

    let answered = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("8:4"))
            .map_err(|e| e.to_string())
    })
    .await
    .expect("the service answered");

    let SearchData::Lookup { citation, resolved } = answered.data else {
        panic!("a citation must not be read as a ranking");
    };

    assert_eq!(citation, "8:4");

    // Both readings, in the order the service read them. Picking one would be
    // bron deciding what a citation means, which is interpretation.
    assert_eq!(
        resolved
            .iter()
            .map(|r| r.reading.as_str())
            .collect::<Vec<_>>(),
        ["book", "literal"]
    );
    assert_eq!(resolved[0].regulation_id, "BWBR0005034");
    assert_eq!(resolved[1].number, "8:4");

    // A limit on one reading is that reading's, and travels with it.
    assert_eq!(resolved[0].caveats.len(), 0);
    assert_eq!(resolved[1].caveats.len(), 1);

    assert!(
        answered.caveats.contains(&Caveat::SeveralReadings {
            readings: vec!["book".into(), "literal".into()],
        }),
        "{:?}",
        answered.caveats
    );
}

/// Known but not held is an answer: the dates resolved, the rendition is named,
/// and there is no hash because nobody has read the text.
#[tokio::test(flavor = "multi_thread")]
async fn a_reading_that_found_a_rendition_without_text_says_so() {
    let server = searching(LOOKUP_UNHELD, 200).await;
    let home = Home::new();
    let base = server.uri();

    let answered = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("3:305a"))
            .map_err(|e| e.to_string())
    })
    .await
    .expect("the service answered");

    let SearchData::Lookup { resolved, .. } = answered.data else {
        panic!("a citation must not be read as a ranking");
    };

    // The service's own shape: a flag and three fields that are only there when
    // it is set. What bron answers with is one type that cannot say both.
    assert!(!resolved[0].held);
    assert!(resolved[0].content_hash.is_none());
    assert!(resolved[0].label.is_none());

    // The reading that came to nothing says why rather than going missing.
    assert!(
        answered.caveats.contains(&Caveat::ReadingUnresolved {
            reading: "literal".into(),
            because: "no_regulation_prints_it".into(),
            regulation_id: None,
            renditions: Vec::new(),
        }),
        "{:?}",
        answered.caveats
    );
}

/// A question is still ranked, and a ranking still carries the distance that
/// orders it. The two shapes are read apart, not flattened into one.
#[tokio::test(flavor = "multi_thread")]
async fn a_question_is_still_ranked() {
    let server = searching(RANKING, 200).await;
    let home = Home::new();
    let base = server.uri();

    let answered = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("onrechtmatige daad"))
            .map_err(|e| e.to_string())
    })
    .await
    .expect("the service answered");

    let SearchData::Ranking { searched, hits } = answered.data else {
        panic!("a question must not be read as a citation");
    };

    assert_eq!(searched, vec!["wetgeving".to_string()]);
    assert_eq!(hits[0].distance, Some(0.21));
}

/// The service orders a ranking by the vector and seats the texts only full
/// text found from place eight. One of those may carry no vector, and so no
/// distance; the hit is still an answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_hit_only_the_words_found_may_carry_no_distance() {
    const WORDS_ONLY: &str = r#"{"bron":1,"grade":"authority","data":{"kind":"ranking",
        "searched":["wetgeving"],"hits":[{"source":"wetgeving","unit":"article",
        "regulation_id":"BWBR0005290","expression":"2026-07-01_0","number":"18",
        "label":"Artikel 18","content_hash":"b7c2","places":1,"found_by":["text"]}]}}"#;

    let server = searching(WORDS_ONLY, 200).await;
    let home = Home::new();
    let base = server.uri();

    let answered = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("updates"))
            .map_err(|e| e.to_string())
    })
    .await
    .expect("a hit without a distance is still a hit");

    let SearchData::Ranking { hits, .. } = answered.data else {
        panic!("a question must not be read as a citation");
    };

    assert_eq!(hits[0].distance, None);
}

/// A citation that resolves to nothing is a not-found, never an empty ranking
/// in its place: an empty list cannot say whether the corpus holds no such
/// article or the term named none.
#[tokio::test(flavor = "multi_thread")]
async fn a_citation_that_resolves_to_nothing_is_not_found() {
    let body = r#"{"bron":1,"grade":"authority","error":{
        "kind":"not_found","message":"no article cited as [3:99999]",
        "detail":["book: BWBR0005291 rendition 2025-07-01_0 does not carry artikel 99999",
                  "literal: no regulation prints artikel 3:99999"]}}"#;
    let server = searching(body, 404).await;
    let home = Home::new();
    let base = server.uri();

    let fault = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("3:99999"))
            .err()
            .map(|e| (e.kind(), e.to_string()))
    })
    .await
    .expect("a citation that resolves to nothing must not read as success");

    assert_eq!(fault.0, "not_found");
    assert!(
        fault.1.contains("does not carry artikel 99999"),
        "{}",
        fault.1
    );
    assert!(fault.1.contains("no regulation prints"), "{}", fault.1);
}

/// Reads the shape at its tag, so what is wrong with a ranking is reported
/// against the ranking.
///
/// Told apart by shape instead, every one of these falls out the same way: the
/// ranking fails to match, the lookup fails too, and all that comes back is
/// that the answer matched no variant of an untagged enum. No field, no type,
/// no position, on exactly the kind of failure this has to catch.
#[tokio::test(flavor = "multi_thread")]
async fn a_broken_ranking_is_reported_against_the_ranking() {
    async fn refused(body: &str) -> String {
        let server = searching(body, 200).await;
        let home = Home::new();
        let base = server.uri();

        off_thread(&home, move |client| {
            Service::at(&base)
                .search(&client, &asking_for("onrechtmatige daad"))
                .err()
                .map(|e| e.chain().join(" | "))
        })
        .await
        .expect("a broken ranking must not read as success")
    }

    let hit = r#"{"source":"wetgeving","unit":"article","regulation_id":"BWBR0005289",
                  "expression":"2026-07-16_0","number":"162","label":"Artikel 162","path":null,
                  "content_hash":"a61a""#;

    // A field renamed upstream is named as the one that went missing.
    let renamed = refused(&format!(
        r#"{{"bron":1,"grade":"authority","data":{{"kind":"ranking",
           "searched":["wetgeving"],"hitz":[{hit},"distance":0.21}}]}}}}"#
    ))
    .await;
    assert!(renamed.contains("missing field `hits`"), "{renamed}");

    // So is one the service stopped sending.
    let missing = refused(
        r#"{"bron":1,"grade":"authority","data":{"kind":"ranking",
           "searched":["wetgeving"],"hits":[{"source":"wetgeving","unit":"article"}]}}"#,
    )
    .await;
    assert!(
        missing.contains("missing field `regulation_id`"),
        "{missing}"
    );

    // A field that arrived as the wrong thing is named by what was expected and
    // where it stands, which is what serde has to say about a value rather than
    // about a key.
    let wrong = refused(&format!(
        r#"{{"bron":1,"grade":"authority","data":{{"kind":"ranking",
           "searched":["wetgeving"],"hits":[{hit},"distance":"near"}}]}}}}"#
    ))
    .await;
    assert!(wrong.contains("expected f64"), "{wrong}");
    assert!(wrong.contains("line"), "{wrong}");

    for message in [&renamed, &missing, &wrong] {
        assert!(!message.contains("did not match any variant"), "{message}");
    }
}

/// A shape this bron has no name for is refused rather than read as the other
/// one. Guessing at it would answer a question the service did not.
///
/// And it says which of the two is behind. A reader that reports its own
/// ignorance as the service answering nonsense sends whoever reads it to the
/// wrong half of the system.
#[tokio::test(flavor = "multi_thread")]
async fn a_shape_this_bron_does_not_know_is_refused() {
    let body = r#"{"bron":1,"grade":"authority","data":{"kind":"something_later","x":1}}"#;
    let server = searching(body, 200).await;
    let home = Home::new();
    let base = server.uri();

    let fault = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("onrechtmatige daad"))
            .err()
            .map(|e| (e.kind(), e.chain().join(" | ")))
    })
    .await
    .expect("an unknown shape must not read as success");

    assert_eq!(fault.0, "upstream");
    assert!(fault.1.contains("something_later"), "{}", fault.1);
    assert!(
        fault.1.contains("Upgrade to read what it says"),
        "{}",
        fault.1
    );
    assert!(
        !fault.1.contains("did not answer the bron envelope"),
        "{}",
        fault.1
    );
}

/// The service sends `held` and the three fields that belong with it as
/// separate keys, so the two can disagree in either direction. A reading whose
/// two halves disagree arrives as the service sent it, and takes none of the
/// citation's other readings down with it: one malformed row must not lose a
/// sound one.
#[tokio::test(flavor = "multi_thread")]
async fn a_reading_the_service_contradicts_itself_about_still_arrives() {
    let body = r#"{"bron":1,"grade":"authority","data":{"kind":"lookup","citation":"8:4",
      "resolved":[
        {"reading":"book","source":"wetgeving","regulation_id":"BWBR0005034",
         "expression":"2026-07-01_0","held":true,"number":"4","label":"Artikel 4",
         "content_hash":"e0bd9a23","caveats":[]},
        {"reading":"literal","source":"wetgeving","regulation_id":"BWBR0005537",
         "expression":"2026-08-15_0","held":true,"number":"8:4","caveats":[]}]}}"#;
    let server = searching(body, 200).await;
    let home = Home::new();
    let base = server.uri();

    let answered = off_thread(&home, move |client| {
        Service::at(&base)
            .search(&client, &asking_for("8:4"))
            .map_err(|e| e.to_string())
    })
    .await
    .expect("one malformed reading must not sink the answer");

    let SearchData::Lookup { resolved, .. } = answered.data else {
        panic!("a citation must not be read as a ranking");
    };

    // Both readings arrive, in the order the service read them.
    assert_eq!(
        resolved
            .iter()
            .map(|r| r.reading.as_str())
            .collect::<Vec<_>>(),
        ["book", "literal"]
    );

    // The sound one keeps what it came with.
    assert_eq!(resolved[0].content_hash.as_deref(), Some("e0bd9a23"));

    // The contradiction is readable as the service sent it: held, no hash.
    assert!(resolved[1].held);
    assert!(resolved[1].content_hash.is_none());
}
