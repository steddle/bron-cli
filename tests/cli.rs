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
  "resolved":[{"reading":"book","source":"wetgeving","regulation_id":"BWBR0005289","expression":"2019-04-01_0","held":true,"upstream_hash_check":"matches","number":"162","label":"Artikel 162","content_hash":"f8125445","caveats":[]}]},
 "caveats":[{"kind":"something_later","count":3,"labels":["a","b"]}]}"#;

const REGULATIONS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"id":"BWBR0005537","title":"Algemene wet bestuursrecht","type":"wet","later_field":true}]}"#;

const EXPRESSIONS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0008691","label":"2016-07-01_0","in_force":{"from":"2016-07-01","until":"2018-12-31"},"size":49606,"withdrawn":false,"held":true,"upstream_hash_check":"matches","blob":"35e2"}]}"#;

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
 "data":[{"regulation_id":"BWBR0020368","label":"2026-09-01_0","in_force":{"from":"2026-09-01"},"held":true,"upstream_hash_check":"matches","later_field":9}],
 "caveats":[{"kind":"something_later","window":50}]}"#;

const PUBLISHED_CUT: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0020368","label":"2026-09-01_0","in_force":{"from":"2026-09-01"},"held":true,"upstream_hash_check":"matches"}],
 "caveats":[{"kind":"truncated","window":1}]}"#;

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
    // Beside the cursor on stderr, since stdout is rows.
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{output:?}");
    assert!(err.contains("Start again from the head"), "{err}");
    assert!(!err.contains("no sentence for"), "{err}");
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
                "regulation_id": "BWBR0008691",
                "label": label,
                "in_force": {"from": &label[..10]},
                "held": true,
                "upstream_hash_check": "matches",
                "blob": blob,
            },
            "articles": [{
                "kind": "artikel",
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
            "kind": "artikel",
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
                "regulation_id": "BWBR0008691",
                "label": "2016-07-01_0",
                "in_force": {"from": "2016-07-01"},
                "held": true,
                "upstream_hash_check": "matches",
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
 "data":[{"regulation_id":"BWBR0008691","label":"2019-01-01_0","in_force":{"from":"2019-01-01"},"held":true,"upstream_hash_check":"matches"},
         {"regulation_id":"BWBR0008691","label":"2016-07-01_0","in_force":{"from":"2016-07-01","until":"2018-12-31"},"held":true,"upstream_hash_check":"matches"}]}"#;

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

/// Two articles per rendition: 1 amended between them, 2 left alone.
fn two_articles_of(as_of: &str, label: &str, blob: &str, first: &str) -> String {
    let article = |number: &str, stam: &str, text: &str| {
        serde_json::json!({
            "kind": "artikel",
            "number": number,
            "label": format!("Artikel {number}"),
            "stam_id": stam,
            "span": {"start": 10, "end": 90},
            "paragraphs": [{"number": "1", "rendered": text, "span": {"start": 20, "end": 80}}],
        })
    };

    serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "as_of": as_of,
        "data": {
            "regulation_id": "BWBR0008691",
            "expression": {
                "regulation_id": "BWBR0008691",
                "label": label,
                "in_force": {"from": &label[..10]},
                "held": true,
                "upstream_hash_check": "matches",
                "blob": blob,
            },
            "articles": [
                article("1", "2760033", first),
                article("2", "2760034", "Ongewijzigd."),
            ],
        },
    })
    .to_string()
}

async fn holding_two_articles(
    server: &MockServer,
    as_of: &str,
    label: &str,
    blob: &str,
    first: &str,
) {
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0008691/articles"))
        .and(query_param("as_of", as_of))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            two_articles_of(as_of, label, blob, first),
            "application/json",
        ))
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

/// The summary counts what was asked about. With `--article` that is the
/// articles bearing that number, so an unchanged article elsewhere in the law
/// is not counted into an answer about article 1.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_counts_only_the_article_asked_for() {
    let server = MockServer::start().await;
    holding_two_articles(
        &server,
        "2018-01-01",
        "2016-07-01_0",
        EARLIER_BLOB,
        "Eerste lid.",
    )
    .await;
    holding_two_articles(
        &server,
        "2019-01-01",
        "2019-01-01_0",
        LATER_BLOB,
        "Eerste lid, gewijzigd.",
    )
    .await;

    let changes = |article: &'static str| {
        let server = &server;
        async move {
            let mut args = vec![
                "--json",
                "law",
                "changes",
                "BWBR0008691",
                "--from",
                "2018-01-01",
                "--to",
                "2019-01-01",
            ];
            args.extend(["--article", article]);
            let output = bron(server, &args).await;
            assert!(output.status.success(), "{output:?}");
            serde_json::from_str::<serde_json::Value>(&stdout(&output)).unwrap()["data"]["summary"]
                .clone()
        }
    };

    assert_eq!(
        changes("1").await,
        serde_json::json!({"unchanged": 0, "amended": 1, "commenced": 0, "added": 0, "repealed": 0, "withdrawn": 0})
    );
    assert_eq!(
        changes("2").await,
        serde_json::json!({"unchanged": 1, "amended": 0, "commenced": 0, "added": 0, "repealed": 0, "withdrawn": 0})
    );

    let human = bron(
        &server,
        &[
            "law",
            "changes",
            "BWBR0008691",
            "--from",
            "2018-01-01",
            "--to",
            "2019-01-01",
            "--article",
            "1",
        ],
    )
    .await;
    assert!(
        stdout(&human)
            .contains("1 amended, 0 commenced, 0 added, 0 repealed, 0 withdrawn, 0 unchanged"),
        "{human:?}"
    );
}

/// A number that names no article on either side is no such thing, as a lid
/// that is not there is: an all-zero summary would read as 'nothing changed'.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_for_an_article_in_neither_rendition_is_not_found() {
    let server = MockServer::start().await;
    holding_two_articles(
        &server,
        "2018-01-01",
        "2016-07-01_0",
        EARLIER_BLOB,
        "Eerste lid.",
    )
    .await;
    holding_two_articles(
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
            "--article",
            "9",
        ],
    )
    .await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "not_found");
    assert!(envelope["error"].get("code").is_none(), "{envelope}");
}

/// Where stdout is rows, a limit on them goes to stderr beside the cursor, so
/// a shell script reading line by line never reads a sentence as a row.
#[tokio::test(flavor = "multi_thread")]
async fn a_list_of_rows_keeps_its_caveats_off_stdout() {
    const HITS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"id":"BWBR0005537","title":"Algemene wet bestuursrecht","type":"wet"}],
 "caveats":[{"kind":"truncated","window":1}]}"#;

    const VERSIONS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0008691","label":"2019-01-01_0","in_force":{"from":"2019-01-01"},"held":true,"upstream_hash_check":"matches","unread_because":"too_large"}],
 "caveats":[{"kind":"truncated","window":1}]}"#;

    for (route, body, args) in [
        ("/api/v1/feed", FEED, &["law", "feed", "--since", "0"][..]),
        ("/api/v1/regulations", HITS, &["law", "search", "Awb"][..]),
        (
            "/api/v1/expressions",
            PUBLISHED_CUT,
            &["law", "published", "--from", "2026-09-01"][..],
        ),
        (
            "/api/v1/regulations/BWBR0008691/expressions",
            VERSIONS,
            &["law", "versions", "BWBR0008691"][..],
        ),
    ] {
        let server = answering("GET", route, body).await;

        let output = bron(&server, args).await;
        let err = String::from_utf8_lossy(&output.stderr);

        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(!stdout(&output).contains("note:"), "{args:?}: {output:?}");
        // A legend is a note too, and read as a row it names a rendition `*`.
        assert!(!stdout(&output).contains(" = "), "{args:?}: {output:?}");
        assert!(
            err.contains("note: filtered over a window of 1"),
            "{args:?}: {err}"
        );
    }
}

/// A provision the rendition publishes and does not yet apply is never said to
/// be in force, and the day it applies is said where the register states one.
#[tokio::test(flavor = "multi_thread")]
async fn a_provision_not_yet_applied_is_not_said_to_be_in_force() {
    let article = |number: &str, in_force_from: Option<&str>| {
        let mut article = serde_json::json!({
            "kind": "artikel",
            "number": number,
            "label": format!("Artikel {number}"),
            "stam_id": number,
            "amended_by": "stb-2026-219",
            "effect": "nieuw",
            "status": "nogniet",
            "span": {"start": 10, "end": 90},
            "paragraphs": [{"rendered": "Tekst.", "span": {"start": 20, "end": 80}}],
        });
        if let Some(day) = in_force_from {
            article["in_force_from"] = day.into();
        }
        article
    };
    let answer = serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "as_of": "2026-09-18",
        "data": {
            "regulation_id": "BWBR0020368",
            "expression": {
                "regulation_id": "BWBR0020368",
                "label": "2026-09-01_0",
                "in_force": {"from": "2026-09-01"},
                "held": true,
                "upstream_hash_check": "matches",
                "blob": EARLIER_BLOB,
            },
            "articles": [article("1:54d", None), article("1:76e", Some("2027-01-01"))],
        },
        "caveats": [{"kind": "not_commenced", "articles": ["1:54d", "1:76e"]}],
    });

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0020368/articles"))
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

    let output = bron(&server, &["law", "show", "BWBR0020368", "--not-commenced"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(!text.contains("in force on"), "{text}");
    assert!(!text.contains("in force through"), "{text}");
    // The rendition prints a stub or the settled text, and the answer claims
    // neither.
    assert!(!text.contains("will have"), "{text}");
    assert!(text.contains("not yet applied"), "{text}");
    assert!(text.contains("applies from 2027-01-01"), "{text}");
    assert!(text.contains("stb-2026-219"), "{text}");
}

/// A 503 is read off the status: `upstream`, with the delay the service named.
/// The body is not read, so its `code` stays the service's, as the contract
/// states for every 429 and 503.
#[tokio::test(flavor = "multi_thread")]
async fn the_service_unable_to_fetch_is_upstream_with_its_delay() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0008691/articles"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("Retry-After", "60")
                .set_body_raw(
                    r#"{"bron":1,"grade":"authority","error":{"kind":"source","code":"source_unavailable","message":"the register did not serve 2016-07-01_0","detail":[]}}"#,
                    "application/json",
                ),
        )
        .mount(&server)
        .await;

    let output = bron(&server, &["--json", "law", "show", "BWBR0008691"]).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "upstream");
    assert!(envelope["error"].get("code").is_none(), "{envelope}");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .unwrap()
            .contains("60s"),
        "{envelope}"
    );
}

/// A held rendition that was not parsed into articles has none to answer, and
/// a person reading the list is told which and why. `--json` is the envelope
/// as sent, so the field reaches a consumer untouched.
#[tokio::test(flavor = "multi_thread")]
async fn law_versions_says_which_held_renditions_were_not_read() {
    const UNREAD: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0008691","label":"2019-01-01_0","in_force":{"from":"2019-01-01"},"held":true,"upstream_hash_check":"matches","unread_because":"too_large"},
         {"regulation_id":"BWBR0008691","label":"2016-07-01_0","in_force":{"from":"2016-07-01","until":"2018-12-31"},"held":true,"upstream_hash_check":"matches"}]}"#;

    let server = answering("GET", "/api/v1/regulations/BWBR0008691/expressions", UNREAD).await;

    let json = bron(&server, &["--json", "law", "versions", "BWBR0008691"]).await;
    assert_eq!(stdout(&json), format!("{UNREAD}\n"));

    let output = bron(&server, &["law", "versions", "BWBR0008691"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    let unread = text.lines().find(|l| l.contains("2019-01-01_0")).unwrap();
    let read = text.lines().find(|l| l.contains("2016-07-01_0")).unwrap();
    assert!(unread.contains("too_large"), "{text}");
    assert!(!read.contains("unread"), "{text}");
}

/// One rendition holding artikel 119b with the status and day it prints.
fn waiting_or_applying(
    label: &str,
    blob: &str,
    status: &str,
    in_force_from: Option<&str>,
) -> String {
    let mut article = serde_json::json!({
        "kind": "artikel",
        "number": "119b",
        "label": "Artikel 119b",
        "stam_id": "2760119",
        "status": status,
        "span": {"start": 10, "end": 90},
        "paragraphs": [{"number": "1", "rendered": "Tekst.", "span": {"start": 20, "end": 80}}],
    });
    if let Some(day) = in_force_from {
        article["in_force_from"] = day.into();
    }

    serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "data": {
            "regulation_id": "BWBR0005289",
            "expression": {
                "regulation_id": "BWBR0005289",
                "label": label,
                "in_force": {"from": &label[..10]},
                "held": true,
                "upstream_hash_check": "matches",
                "blob": blob,
            },
            "articles": [article],
        },
    })
    .to_string()
}

/// Words that stay and a status that goes from `nogniet` to `goed`: the
/// provision started to apply, on the day the later rendition states. Each
/// side's status is its own rendition's.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_says_a_provision_commenced_and_on_which_day() {
    let server = MockServer::start().await;
    for (as_of, label, blob, status, day) in [
        ("2026-01-01", "2025-07-01_0", EARLIER_BLOB, "nogniet", None),
        (
            "2027-06-01",
            "2027-01-01_0",
            LATER_BLOB,
            "goed",
            Some("2027-01-01"),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path("/api/v1/regulations/BWBR0005289/articles"))
            .and(query_param("as_of", as_of))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                waiting_or_applying(label, blob, status, day),
                "application/json",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/blobs/{blob}/provenance")))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"bron":1,"grade":"authority","data":{"size":547732}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;
    }

    let args = [
        "law",
        "changes",
        "BWBR0005289",
        "--from",
        "2026-01-01",
        "--to",
        "2027-06-01",
    ];

    let json = bron(&server, &[&["--json"][..], &args[..]].concat()).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&json)).unwrap();
    let change = &envelope["data"]["articles"][0];

    assert!(json.status.success(), "{json:?}");
    assert_eq!(change["change"], "commenced", "{envelope}");
    assert_eq!(change["before"]["status"], "nogniet", "{envelope}");
    assert_eq!(change["after"]["status"], "goed", "{envelope}");
    assert_eq!(change["after"]["in_force_from"], "2027-01-01", "{envelope}");
    assert_eq!(envelope["data"]["summary"]["commenced"], 1, "{envelope}");
    assert_eq!(envelope["data"]["summary"]["unchanged"], 0, "{envelope}");

    let human = bron(&server, &args).await;
    let text = stdout(&human);
    assert!(human.status.success(), "{human:?}");
    assert!(text.contains("commenced on 2027-01-01"), "{text}");
    assert!(text.contains("1 commenced"), "{text}");
}

/// A rendition of BWBR0020368 as the service answers it on 2015-01-01: one of
/// its articles prints no number, so the service leaves `number` out, and one
/// unit prints neither a number nor a label.
fn a_rendition_with_an_article_that_prints_no_number() -> String {
    serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "as_of": "2015-01-01",
        "as_known_on": "2026-09-19",
        "data": {
            "regulation_id": "BWBR0020368",
            "expression": {
                "regulation_id": "BWBR0020368",
                "label": "2014-07-01_0",
                "in_force": {"from": "2014-07-01", "until": "2015-06-30"},
                "withdrawn": false,
                "held": true,
                "upstream_hash_check": "matches",
                "blob": EARLIER_BLOB,
            },
            "articles": [
                {
                    "kind": "artikel",
                    "position": 759,
                    "number": "3:280",
                    "label": "Artikel 3:280",
                    "stam_id": "2760280",
                    "versie_id": "1",
                    "content_hash": EARLIER_BLOB,
                    "span": {"start": 10, "end": 90},
                    "paragraphs": [{"content_hash": EARLIER_BLOB, "rendered": "Gewoon.", "span": {"start": 20, "end": 80}}],
                },
                {
                    "kind": "artikel",
                    "position": 760,
                    "label": "Artikel 3:281",
                    "stam_id": "2760281",
                    "versie_id": "2",
                    "content_hash": EARLIER_BLOB,
                    "status": "vervallen",
                    "span": {"start": 90, "end": 120},
                },
                {
                    "kind": "artikel",
                    "position": 761,
                    "stam_id": "2760282",
                    "versie_id": "3",
                    "content_hash": EARLIER_BLOB,
                    "span": {"start": 120, "end": 150},
                    "paragraphs": [{"content_hash": EARLIER_BLOB, "rendered": "Naamloos.", "span": {"start": 120, "end": 150}}],
                },
            ],
        },
    })
    .to_string()
}

/// The schema does not require `number`, and the service leaves a null out. One
/// row without it is a row like any other, and never the reason a whole answer
/// is unusable.
#[tokio::test(flavor = "multi_thread")]
async fn an_article_without_a_number_does_not_break_the_answer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0020368/articles"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            a_rendition_with_an_article_that_prints_no_number(),
            "application/json",
        ))
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

    let args = ["law", "show", "BWBR0020368", "--as-of", "2015-01-01"];

    let human = bron(&server, &args).await;
    let text = stdout(&human);
    assert!(human.status.success(), "{human:?}");
    assert!(text.lines().any(|l| l == "Artikel 3:280"), "{text}");
    assert!(text.lines().any(|l| l == "Artikel 3:281"), "{text}");
    // Neither a number nor a label: named by what it is and where it stands.
    assert!(text.lines().any(|l| l == "artikel 761"), "{text}");
    assert!(text.contains("Naamloos."), "{text}");

    let json = bron(&server, &[&["--json"][..], &args[..]].concat()).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&json)).unwrap();
    assert!(json.status.success(), "{json:?}");
    let articles = envelope["data"].as_array().unwrap();
    assert_eq!(articles.len(), 3, "{envelope}");
    assert_eq!(articles[0]["number"], "3:280", "{envelope}");
    assert!(articles[1].get("number").is_none(), "{envelope}");
    assert_eq!(articles[1]["kind"], "artikel", "{envelope}");
    assert_eq!(articles[2]["position"], 761, "{envelope}");
}

/// A hit on a unit that prints no number is ranked like any other, and named
/// by its label, or by its kind where it has none.
#[tokio::test(flavor = "multi_thread")]
async fn a_hit_without_a_number_is_ranked_like_any_other() {
    const NUMBERLESS: &str = r#"{"bron":1,"grade":"authority","as_of":"2026-09-19","data":{"kind":"ranking",
    "searched":["wetgeving"],
    "hits":[{"source":"wetgeving","unit":"article","regulation_id":"BWBR0001941","expression":"2026-09-17_0",
             "kind":"aanhef","content_hash":"a61a","distance":0.21,"places":[],"found_by":["vector"]},
            {"source":"wetgeving","unit":"article","regulation_id":"BWBR0001941","expression":"2026-09-17_0",
             "kind":"bijlage","label":"Lijst I","content_hash":"b72b","distance":0.25,"places":[],"found_by":["vector"]}]}}"#;

    let server = answering("POST", "/api/v1/search", NUMBERLESS).await;

    let output = bron(&server, &["find", "opium"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(
        text.lines().any(|l| l.ends_with("BWBR0001941  aanhef")),
        "{text}"
    );
    assert!(
        text.lines().any(|l| l.ends_with("BWBR0001941  Lijst I")),
        "{text}"
    );
}

/// Two units of the Opiumwet that are no article, as `/parts` answers them: an
/// aanhef, which prints no number, and a bijlage that prints images.
const PARTS: &str = r#"{"bron":1,"grade":"authority","as_of":"2026-09-19","as_known_on":"2026-09-19",
 "data":{"regulation_id":"BWBR0001941",
  "expression":{"regulation_id":"BWBR0001941","label":"2026-09-17_0","in_force":{"from":"2026-09-17"},"withdrawn":false,"held":true,"upstream_hash_check":"matches","blob":"1eb72c0011119cf2fdcbc0660f12e7a0cf0612a40cb602f3a30a42a1db5b8de9","later_field":1},
  "parts":[
   {"kind":"aanhef","position":1,"stam_id":"1","versie_id":"1","content_hash":"a61a","span":{"start":100,"end":400},
    "paragraphs":[{"content_hash":"a61a","rendered":"Wij WILHELMINA, bij de gratie Gods.","span":{"start":100,"end":400}}]},
   {"kind":"bijlage","position":2,"number":"Ia","label":"Lijst Ia","path":"/BijlageIa","stam_id":"17364373","versie_id":"31676452","content_hash":"53c7",
    "jci":"jci1.3:c:BWBR0001941&bijlage=Ia&z=2026-09-17&g=2026-09-17","span":{"start":339090,"end":365513},"images":["272866.png","272867.png"],
    "paragraphs":[{"content_hash":"53c7","rendered":"1 Stofgroep: fenethylaminen","span":{"start":339090,"end":365513}}]}]},
 "caveats":[{"kind":"illustration_omitted","articles":["Lijst Ia"]}]}"#;

/// `law parts` is one call to the service, so `--json` is what it answered,
/// and the narrowing reaches the service as asked.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_prints_the_envelope_the_service_sent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/parts"))
        .and(query_param("kind", "bijlage"))
        .and(query_param("number", "Ia"))
        .and(query_param("as_of", "2026-09-19"))
        .and(query_param("as_known_on", "2026-09-18"))
        .and(query_param("rendered", "1"))
        .and(query_param("raw", "1"))
        .and(query_param("limit", "5"))
        .and(query_param("offset", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(PARTS, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "--json",
            "law",
            "parts",
            "BWBR0001941",
            "--kind",
            "bijlage",
            "--number",
            "Ia",
            "--as-of",
            "2026-09-19",
            "--as-known-on",
            "2026-09-18",
            "--raw",
            "--limit",
            "5",
            "--offset",
            "2",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{PARTS}\n"));
}

/// A unit that prints no number is asked for by its place among its kind.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_asks_for_a_unit_by_its_place() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/parts"))
        .and(query_param("kind", "aanhef"))
        .and(query_param("position", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(PARTS, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "law",
            "parts",
            "BWBR0001941",
            "--kind",
            "aanhef",
            "--position",
            "1",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
}

/// Each unit under its name, its text, where it stands and the images it
/// prints; the note on the answer beneath it.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_names_each_unit_and_prints_its_text() {
    let server = answering("GET", "/api/v1/regulations/BWBR0001941/parts", PARTS).await;

    let output = bron(&server, &["law", "parts", "BWBR0001941"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.contains("as in force on 2026-09-19"), "{text}");
    assert!(text.lines().any(|l| l == "aanhef 1"), "{text}");
    assert!(
        text.contains("Wij WILHELMINA, bij de gratie Gods."),
        "{text}"
    );
    assert!(text.lines().any(|l| l == "Lijst Ia"), "{text}");
    assert!(text.lines().any(|l| l == "/BijlageIa"), "{text}");
    assert!(text.contains("1 Stofgroep: fenethylaminen"), "{text}");
    assert!(
        text.contains("source: 1eb72c0011119cf2fdcbc0660f12e7a0cf0612a40cb602f3a30a42a1db5b8de9 (339090..365513)"),
        "{text}"
    );
    assert!(text.contains("jci1.3:c:BWBR0001941&bijlage=Ia"), "{text}");
    assert!(text.contains("images: 272866.png, 272867.png"), "{text}");
    assert!(text.contains("note: Lijst Ia print(s) an image"), "{text}");
}

/// No such unit is a not-found, with the code the service stated.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_for_a_unit_the_rendition_does_not_print_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/parts"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"part_not_found","message":"no part of BWBR0001941 rendition 2026-09-17_0 [nope]","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &["--json", "law", "parts", "BWBR0001941", "--kind", "nope"],
    )
    .await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(envelope["error"]["code"], "part_not_found");
}

/// A number or a place is a number or a place among units of one kind, and
/// the two name one unit two ways.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_refuses_a_number_or_a_place_without_a_kind() {
    let server = MockServer::start().await;

    for args in [
        &["law", "parts", "BWBR0001941", "--number", "Ia"][..],
        &["law", "parts", "BWBR0001941", "--position", "1"][..],
        &[
            "law",
            "parts",
            "BWBR0001941",
            "--kind",
            "bijlage",
            "--number",
            "Ia",
            "--position",
            "1",
        ][..],
    ] {
        let output = bron(&server, args).await;
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
    }
}

/// `law show` answers articles, and says so where a reader looks for it.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_says_it_answers_articles_only() {
    let server = MockServer::start().await;

    let output = bron(&server, &["law", "show", "--help"]).await;

    assert!(output.status.success(), "{output:?}");
    assert!(stdout(&output).contains("law parts"), "{output:?}");
}
