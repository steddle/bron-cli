//! The binary, against a service that answers on purpose.
//!
//! The service's JSON is the contract. For a command that is exactly one call
//! to the service, `--json` prints what the service said and nothing bron made
//! of it, so a field this build has never heard of still reaches the consumer.
//! What has to be proved is that nothing is re-serialised on the way through,
//! that both coordinates of a moment reach the service, and that a limit this
//! build has no sentence for is still printed for a person.

use std::process::Output;

use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Runs `bron` against a mock service, off the runtime serving that mock.
async fn bron(server: &MockServer, args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    let mirror = server.uri();
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();

    tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_bron"))
            .args(&args)
            .env("BRON_MIRROR", mirror)
            .env("BRON_HOME", home.path())
            // A proxy in the developer's environment must not get between the
            // binary and the mock on loopback.
            .env("NO_PROXY", "127.0.0.1,localhost")
            .output()
            .unwrap()
    })
    .await
    .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// Key order, spacing, a field and a caveat this build has no name for: all of
/// it is the service's, and re-serialising would lose every one of them.
const LOOKUP: &str = r#"{"bron":1,  "grade":"authority","as_of":"2019-06-01","as_known_on":"2020-01-01",
 "data":{"kind":"lookup","citation":"6:162","later_field":{"x":1},
  "resolved":[{"reading":"book","source":"wetgeving","regulation_id":"BWBR0005289","expression":"2019-04-01_0","held":true,"number":"162","label":"Artikel 162","content_hash":"f8125445"}]},
 "caveats":[{"kind":"something_later","count":3,"labels":["a","b"]}]}"#;

const REGULATIONS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"id":"BWBR0005537","title":"Algemene wet bestuursrecht","type":"wet","later_field":true}]}"#;

const EXPRESSIONS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"label":"2016-07-01_0","in_force":{"from":"2016-07-01","until":"2018-12-31"},"size":49606,"withdrawn":false,"held":true,"upstream_hash_check":"matches","blob":"35e2"}]}"#;

async fn answering(verb: &str, route: &str, body: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread")]
async fn find_prints_the_envelope_the_service_sent() {
    let server = answering("POST", "/api/v1/search", LOOKUP).await;

    let output = bron(&server, &["--json", "find", "6:162", "BW"]).await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{LOOKUP}\n"));
}

/// Every answer ends in one newline. Where the service already sent it, bron
/// adds none: an empty line is a record to anything reading line by line.
#[tokio::test(flavor = "multi_thread")]
async fn an_envelope_that_ends_in_a_newline_ends_in_one() {
    let sent = format!("{REGULATIONS}\n");
    let server = answering("GET", "/api/v1/regulations", &sent).await;

    let output = bron(&server, &["--json", "law", "search", "Awb"]).await;

    assert_eq!(stdout(&output), sent);
}

#[tokio::test(flavor = "multi_thread")]
async fn law_search_prints_the_envelope_the_service_sent() {
    let server = answering("GET", "/api/v1/regulations", REGULATIONS).await;

    let output = bron(&server, &["--json", "law", "search", "Awb"]).await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{REGULATIONS}\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn law_versions_prints_the_envelope_the_service_sent() {
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0008691/expressions",
        EXPRESSIONS,
    )
    .await;

    let output = bron(&server, &["--json", "law", "versions", "BWBR0008691"]).await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{EXPRESSIONS}\n"));
}

/// Two windows, two coordinates. A question names the day the conduct happened
/// and the day the register is read on, and the service resolves the pair.
#[tokio::test(flavor = "multi_thread")]
async fn both_coordinates_of_a_moment_reach_the_service() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/search"))
        .and(body_json(serde_json::json!({
            "q": "6:162",
            "limit": 20,
            "as_of": "2019-06-01",
            "as_known_on": "2020-01-01",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(LOOKUP, "application/json"))
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "find",
            "6:162",
            "--as-of",
            "2019-06-01",
            "--as-known-on",
            "2020-01-01",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
}

/// One date names both coordinates of a zicht question: the text for a day, as
/// the register had it that day. Sent alone, the service reads the other
/// coordinate as today and resolves a rendition in force now.
#[tokio::test(flavor = "multi_thread")]
async fn a_zicht_date_alone_names_the_day_it_is_about_too() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/search"))
        .and(body_json(serde_json::json!({
            "q": "6:162",
            "limit": 20,
            "as_of": "2020-01-01",
            "as_known_on": "2020-01-01",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(LOOKUP, "application/json"))
        .mount(&server)
        .await;

    let output = bron(&server, &["find", "6:162", "--as-known-on", "2020-01-01"]).await;

    assert!(output.status.success(), "{output:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn law_show_sends_both_coordinates_too() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0005289/articles"))
        .and(query_param("as_of", "2019-06-01"))
        .and(query_param("as_known_on", "2020-01-01"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","message":"seen","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "law",
            "show",
            "BWBR0005289",
            "--as-of",
            "2019-06-01",
            "--as-known-on",
            "2020-01-01",
        ],
    )
    .await;

    // The mock answers only where both coordinates arrived, and its answer is
    // the not-found this asserts on.
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

/// A limit on what an answer supports reaches the reader whether or not this
/// build has a sentence for it: its name and what it carried.
#[tokio::test(flavor = "multi_thread")]
async fn a_limit_this_build_has_no_sentence_for_is_printed_with_its_fields() {
    let server = answering("POST", "/api/v1/search", LOOKUP).await;

    let output = bron(&server, &["find", "6:162"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.contains("something_later"), "{text}");
    assert!(text.contains(r#""count":3"#), "{text}");
    assert!(text.contains(r#""labels":["a","b"]"#), "{text}");
    assert!(!text.to_lowercase().contains("upgrade"), "{text}");
}

/// What the service bound the answer to is what a person reads above it.
#[tokio::test(flavor = "multi_thread")]
async fn a_person_reads_both_coordinates_the_service_bound_the_answer_to() {
    let server = answering("POST", "/api/v1/search", LOOKUP).await;

    let text = stdout(&bron(&server, &["find", "6:162"]).await);

    assert!(text.contains("2019-06-01"), "{text}");
    assert!(text.contains("2020-01-01"), "{text}");
    assert!(text.contains("Artikel 162"), "{text}");
}

/// A field and a caveat this build has no name for, on both new routes, so the
/// pass-through is proved by what survives rather than by what is read.
const PUBLISHED: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0020368","label":"2026-09-01_0","in_force":{"from":"2026-09-01"},"held":true,"later_field":9}],
 "caveats":[{"kind":"something_later","window":50}]}"#;

const FEED: &str = r#"{"bron":1,"grade":"authority",
 "data":{"origin":"8539df1d82a7060824be7ba17f500781","head":41,"next":40,"events":[
  {"seq":40,"event":"published","regulation_id":"BWBR0024282","label":"2026-07-01_0","in_force":{"from":"2026-07-01"},"withdrawn":false,"later_field":true}]},
 "caveats":[{"kind":"truncated","window":1}]}"#;

#[tokio::test(flavor = "multi_thread")]
async fn law_published_prints_the_envelope_the_service_sent() {
    let server = answering("GET", "/api/v1/expressions", PUBLISHED).await;

    let output = bron(
        &server,
        &["--json", "law", "published", "--from", "2026-09-01"],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{PUBLISHED}\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn law_published_sends_both_windows_and_the_narrowing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/expressions"))
        .and(query_param("in_force_from", "2026-01-01"))
        .and(query_param("in_force_to", "2026-09-17"))
        .and(query_param("visible_from", "2026-02-01"))
        .and(query_param("visible_to", "2026-09-17"))
        .and(query_param("type", "wet"))
        .and(query_param("regulation", "BWBR0020368"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(PUBLISHED, "application/json"))
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "law",
            "published",
            "--from",
            "2026-01-01",
            "--to",
            "2026-09-17",
            "--known-from",
            "2026-02-01",
            "--known-to",
            "2026-09-17",
            "--type",
            "wet",
            "--regulation",
            "BWBR0020368",
        ],
    )
    .await;

    // The mock answers only when every parameter arrived, so a success here is
    // the assertion.
    assert!(output.status.success(), "{output:?}");
    assert!(
        stdout(&output).contains("BWBR0020368  2026-09-01_0"),
        "{output:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn law_published_refuses_a_window_that_names_nothing_or_runs_backwards() {
    let server = answering("GET", "/api/v1/expressions", PUBLISHED).await;

    let none = bron(&server, &["law", "published", "--type", "wet"]).await;
    assert_eq!(none.status.code(), Some(1), "{none:?}");

    let backwards = bron(
        &server,
        &[
            "law",
            "published",
            "--from",
            "2026-09-01",
            "--to",
            "2026-01-01",
        ],
    )
    .await;
    assert_eq!(backwards.status.code(), Some(1), "{backwards:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn law_feed_prints_the_envelope_the_service_sent() {
    let server = answering("GET", "/api/v1/feed", FEED).await;

    let output = bron(&server, &["--json", "law", "feed", "--since", "0"]).await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{FEED}\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn law_feed_puts_the_events_on_stdout_and_the_cursor_on_stderr() {
    let server = answering("GET", "/api/v1/feed", FEED).await;

    let output = bron(&server, &["law", "feed", "--since", "0"]).await;
    let err = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{output:?}");
    assert!(stdout(&output).contains("published"), "{output:?}");
    // A shell script reads rows on stdout; the cursor it stores is a field in
    // --json and a sentence here, never mixed into the rows.
    assert!(!stdout(&output).contains("origin"), "{output:?}");
    assert!(err.contains("next 40"), "{err}");
    assert!(
        err.contains("origin 8539df1d82a7060824be7ba17f500781"),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn law_feed_walks_an_empty_answer_without_calling_it_a_failure() {
    const QUIET: &str =
        r#"{"bron":1,"grade":"authority","data":{"origin":"a","head":7,"next":7,"events":[]}}"#;

    let server = answering("GET", "/api/v1/feed", QUIET).await;

    let output = bron(&server, &["law", "feed", "--since", "7"]).await;

    // Nothing happened is an answer, not a not-found.
    assert!(output.status.success(), "{output:?}");
    assert!(
        stdout(&output).contains("has done nothing since 7"),
        "{output:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn law_feed_says_in_words_that_a_cursor_belongs_to_another_corpus() {
    const AHEAD: &str = r#"{"bron":1,"grade":"authority","data":{"origin":"a","head":7,"next":900,"events":[]},
 "caveats":[{"kind":"cursor_ahead","asked":900,"head":7}]}"#;

    let server = answering("GET", "/api/v1/feed", AHEAD).await;

    let output = bron(&server, &["law", "feed", "--since", "900"]).await;

    // The one caveat a poller most needs in plain words: throw the cursor away.
    assert!(output.status.success(), "{output:?}");
    assert!(
        stdout(&output).contains("Start again from the head"),
        "{output:?}"
    );
    assert!(!stdout(&output).contains("no sentence for"), "{output:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn law_show_asks_for_what_is_published_and_not_in_force() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0020368/articles"))
        .and(query_param("not_commenced", "1"))
        .and(query_param("rendered", "1"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","message":"no provision published and not in force","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let output = bron(&server, &["law", "show", "BWBR0020368", "--not-commenced"]).await;

    // The mock answers only when the flag reached the service, and a law with
    // nothing coming is a not-found rather than an empty answer.
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

/// The service states a `code` beside the kind, and a consumer reading
/// `--json` gets it as sent. A person does not: the sentence already says what
/// went wrong, and the exit code is what a script branches on.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_the_service_stated_carries_its_code_to_the_consumer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0000001/articles"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"regulation_not_found","message":"no regulation [BWBR0000001]","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let output = bron(&server, &["--json", "law", "show", "BWBR0000001"]).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "not_found");
    assert_eq!(envelope["error"]["code"], "regulation_not_found");

    let said = String::from_utf8(output.stderr).unwrap();
    assert!(said.contains("no regulation [BWBR0000001]"), "{said}");
    assert!(!said.contains("regulation_not_found"), "{said}");
}

/// A refusal bron makes itself never reached the service, so no code is its to
/// carry: presence says who stated the failure.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_bron_states_itself_carries_no_code() {
    let server = MockServer::start().await;

    let output = bron(&server, &["--json", "law", "published"]).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "usage");
    assert!(envelope["error"].get("code").is_none(), "{envelope}");
}

/// One rendition's articles, shaped as the service shapes them. A label opens
/// with the day its rendition commenced.
fn articles_of(as_of: &str, label: &str, blob: &str, text: &str) -> String {
    serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "as_of": as_of,
        "as_known_on": "2026-09-18",
        "data": {
            "regulation_id": "BWBR0008691",
            "expression": {
                "label": label,
                "in_force": {"from": &label[..10]},
                "held": true,
                "upstream_hash_check": "matches",
                "blob": blob,
            },
            "articles": [{
                "number": "1",
                "label": "Artikel 1",
                "stam_id": "2760033",
                "span": {"start": 10, "end": 90},
                "paragraphs": [
                    {"number": "1", "rendered": text, "span": {"start": 20, "end": 80}},
                ],
            }],
        },
    })
    .to_string()
}

const EARLIER_BLOB: &str = "35e29887c98339a23c835cd45fa22c0569f2db6e7d05fbca6c8de0d4fb15a0fb";
const LATER_BLOB: &str = "4ba2172e7f7630289a08992d33c6a8b181dd6167300989f143f3a506357c6364";

/// Answers one rendition's articles, and only to a request for the whole of
/// it: the service pages this route by default, and `limit=none` is how the
/// whole set is asked for.
async fn holding_the_whole_of(
    server: &MockServer,
    as_of: &str,
    label: &str,
    blob: &str,
    text: &str,
) {
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0008691/articles"))
        .and(query_param("as_of", as_of))
        .and(query_param("limit", "none"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(articles_of(as_of, label, blob, text), "application/json"),
        )
        .expect(1)
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/api/v1/blobs/{blob}/provenance")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"bron":1,"grade":"authority","data":{"size":547732}}"#,
            "application/json",
        ))
        .mount(server)
        .await;
}

/// A comparison is between two whole renditions. A page of each would compare
/// the first page and report the rest of the law as unchanged by saying nothing
/// about it.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_asks_for_the_whole_of_both_renditions() {
    let server = MockServer::start().await;
    holding_the_whole_of(
        &server,
        "2018-01-01",
        "2016-07-01_0",
        EARLIER_BLOB,
        "Eerste lid.",
    )
    .await;
    holding_the_whole_of(
        &server,
        "2019-01-01",
        "2019-01-01_0",
        LATER_BLOB,
        "Eerste lid, gewijzigd.",
    )
    .await;

    let output = bron(
        &server,
        &[
            "--json",
            "law",
            "changes",
            "BWBR0008691",
            "--from",
            "2018-01-01",
            "--to",
            "2019-01-01",
        ],
    )
    .await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    // Each mock answers only a request carrying `limit=none`, and expects one.
    assert!(output.status.success(), "{output:?}");
    assert_eq!(envelope["data"]["summary"]["amended"], 1, "{envelope}");
}

/// At a terminal the whole law is what `law show` means.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_asks_for_the_whole_law() {
    let server = MockServer::start().await;
    holding_the_whole_of(
        &server,
        "2018-01-01",
        "2016-07-01_0",
        EARLIER_BLOB,
        "Eerste lid.",
    )
    .await;

    let output = bron(
        &server,
        &[
            "--json",
            "law",
            "show",
            "BWBR0008691",
            "--as-of",
            "2018-01-01",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
}

/// The whole history is what `law versions` means, and the service pages a
/// regulation's renditions by default.
#[tokio::test(flavor = "multi_thread")]
async fn law_versions_asks_for_the_whole_history() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0008691/expressions"))
        .and(query_param("limit", "none"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(EXPRESSIONS, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(&server, &["law", "versions", "BWBR0008691"]).await;

    assert!(output.status.success(), "{output:?}");
}

/// What the service states where a number names two articles.
fn two_articles_numbered_one() -> serde_json::Value {
    serde_json::json!({
        "kind": "ambiguous_article",
        "number": "1",
        "paths": ["/Bijlage2/Artikel1", "/Bijlage3/Artikel1"],
    })
}

/// A service holding two articles numbered 1. The first has a second lid and
/// the other does not, so `--paragraph 2` keeps one of them.
async fn holding_two_articles_numbered_one() -> MockServer {
    let article = |path: &str, leden: &[&str]| {
        serde_json::json!({
            "number": "1",
            "path": path,
            "stam_id": path,
            "span": {"start": 10, "end": 90},
            "paragraphs": leden
                .iter()
                .map(|lid| serde_json::json!({
                    "number": lid,
                    "rendered": "Tekst.",
                    "span": {"start": 20, "end": 80},
                }))
                .collect::<Vec<_>>(),
        })
    };
    let answer = serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "as_of": "2018-01-01",
        "as_known_on": "2026-09-18",
        "data": {
            "regulation_id": "BWBR0008691",
            "expression": {
                "label": "2016-07-01_0",
                "in_force": {"from": "2016-07-01"},
                "held": true,
                "blob": EARLIER_BLOB,
            },
            "articles": [
                article("/Bijlage2/Artikel1", &["1", "2"]),
                article("/Bijlage3/Artikel1", &["1"]),
            ],
        },
        "caveats": [two_articles_numbered_one()],
    });

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0008691/articles"))
        .and(query_param("number", "1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(answer.to_string(), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/blobs/{EARLIER_BLOB}/provenance")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"bron":1,"grade":"authority","data":{"size":547732}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    server
}

/// A number that names two articles is the service's to say: it reads the
/// rendition and states the caveat with both paths. bron carries that one
/// statement and adds none of its own, so the reader is told once.
#[tokio::test(flavor = "multi_thread")]
async fn a_number_naming_two_articles_is_said_once() {
    let server = holding_two_articles_numbered_one().await;

    let output = bron(
        &server,
        &["--json", "law", "show", "BWBR0008691", "--article", "1"],
    )
    .await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(envelope["data"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        envelope["caveats"],
        serde_json::json!([two_articles_numbered_one()])
    );
}

/// The note names every article the number names, and `--paragraph` shows only
/// the ones that have that lid: a path in the note is not always an article in
/// the answer.
#[tokio::test(flavor = "multi_thread")]
async fn the_note_names_an_article_a_narrowed_answer_leaves_out() {
    let server = holding_two_articles_numbered_one().await;

    /// An article prints its path on a line of its own; the note indents.
    fn shown(text: &str, path: &str) -> bool {
        text.lines().any(|line| line == path)
    }

    let whole = bron(&server, &["law", "show", "BWBR0008691", "--article", "1"]).await;
    let narrowed = bron(
        &server,
        &[
            "law",
            "show",
            "BWBR0008691",
            "--article",
            "1",
            "--paragraph",
            "2",
        ],
    )
    .await;

    for (output, keeps_the_second) in [(&whole, true), (&narrowed, false)] {
        let text = stdout(output);

        assert!(output.status.success(), "{output:?}");
        assert!(shown(&text, "/Bijlage2/Artikel1"), "{text}");
        assert_eq!(
            shown(&text, "/Bijlage3/Artikel1"),
            keeps_the_second,
            "{text}"
        );

        assert!(text.contains("      /Bijlage2/Artikel1"), "{text}");
        assert!(text.contains("      /Bijlage3/Artikel1"), "{text}");
    }
}

/// Two renditions as the service lists them, newest first.
const NEWEST_FIRST: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0008691","label":"2019-01-01_0","in_force":{"from":"2019-01-01"},"held":true},
         {"regulation_id":"BWBR0008691","label":"2016-07-01_0","in_force":{"from":"2016-07-01","until":"2018-12-31"},"held":true}]}"#;

/// The order of a list of renditions is the service's, and every channel shows
/// the one it sends. bron prints rows as they arrive and sorts nothing.
#[tokio::test(flavor = "multi_thread")]
async fn renditions_are_printed_in_the_order_the_service_sent_them() {
    for (route, args) in [
        (
            "/api/v1/regulations/BWBR0008691/expressions",
            &["law", "versions", "BWBR0008691"][..],
        ),
        (
            "/api/v1/expressions",
            &["law", "published", "--from", "2016-01-01"][..],
        ),
    ] {
        let server = answering("GET", route, NEWEST_FIRST).await;

        let output = bron(&server, args).await;
        assert!(output.status.success(), "{args:?}: {output:?}");

        let text = stdout(&output);
        let newer = text.find("2019-01-01_0");
        let older = text.find("2016-07-01_0");
        assert!(newer.is_some() && older.is_some(), "{args:?}: {output:?}");
        assert!(newer < older, "{args:?}: {output:?}");
    }
}

/// With `--json` a command line clap will not take is still the envelope on
/// stdout, under the kind a script branches on. clap's own sentence goes to
/// stderr, where every other failure's does.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_line_clap_refuses_is_the_envelope_under_json() {
    let server = MockServer::start().await;

    for args in [
        &["--json", "law", "show"][..],
        &["--json", "law", "show", "BWBR0005289", "--paragraph", "2"][..],
        &["--json", "law", "changes", "BWBR0005289"][..],
        &["law", "feed", "--json"][..],
    ] {
        let output = bron(&server, args).await;
        let envelope: serde_json::Value = serde_json::from_str(&stdout(&output))
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {output:?}"));

        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert_eq!(envelope["bron"], 1, "{args:?}");
        assert_eq!(envelope["error"]["kind"], "usage", "{args:?}");
        assert!(envelope["error"].get("code").is_none(), "{args:?}");
        assert!(
            !envelope["error"]["message"].as_str().unwrap().is_empty(),
            "{args:?}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Usage:"),
            "{args:?}: {output:?}"
        );
    }
}

/// Without `--json` nothing on stdout, and clap's message as clap writes it.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_line_clap_refuses_is_claps_message_without_json() {
    let server = MockServer::start().await;

    let output = bron(&server, &["law", "feed"]).await;
    let said = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(said.contains("--since <SINCE>"), "{said}");
    assert!(said.contains("Usage:"), "{said}");
}

/// Asking for help is not a failure, with `--json` or without.
#[tokio::test(flavor = "multi_thread")]
async fn help_asked_for_under_json_is_still_help() {
    let server = MockServer::start().await;

    let output = bron(&server, &["--json", "law", "feed", "--help"]).await;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stdout(&output).contains("--since"), "{output:?}");
}

