//! The binary, against a service that answers on purpose.
//!
//! The service's JSON is the contract. For a command that is exactly one call
//! to the service, `--json` prints what the service said and nothing bron made
//! of it, so a field this build has never heard of still reaches the consumer.
//! What has to be proved is that nothing is re-serialised on the way through,
//! that both coordinates of a moment reach the service, and that a limit this
//! build has no sentence for is still printed for a person.

use std::process::Output;

use wiremock::matchers::{
    body_json, body_partial_json, method, path, query_param, query_param_is_missing,
};
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
  "resolved":[{"reading":"book","source":"wetgeving","regulation_id":"BWBR0005289","expression":"2019-04-01_0","held":true,"number":"162","label":"Artikel 162","content_hash":"f8125445","caveats":[]}]},
 "caveats":[{"kind":"something_later","count":3,"labels":["a","b"]}]}"#;

const REGULATIONS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"id":"BWBR0005537","title":"Algemene wet bestuursrecht","type":"wet","coverage":"history","later_field":true}]}"#;

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
/// the register had it that day. The service reads it so, and bron sends it
/// alone rather than copying it into the other coordinate.
#[tokio::test(flavor = "multi_thread")]
async fn a_zicht_date_alone_is_sent_alone() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/search"))
        .and(body_json(serde_json::json!({
            "q": "6:162",
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

/// A moment named by no flag is sent by none: the service reads it as today
/// and states the day it resolved in its answer, so no clock on this machine
/// decides which text answers. The later day of a comparison is the same.
#[tokio::test(flavor = "multi_thread")]
async fn an_unnamed_moment_is_left_to_the_service() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/search"))
        .and(body_json(serde_json::json!({"q": "6:162"})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(LOOKUP, "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    for route in [
        "/api/v1/regulations/BWBR0005289/articles",
        "/api/v1/regulations/BWBR0005289/parts",
        "/api/v1/regulations/BWBR0005289/relations",
        "/api/v1/regulations/BWBR0005289/changes",
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .and(query_param_is_missing("as_of"))
            .and(query_param_is_missing("as_known_on"))
            .and(query_param_is_missing("to"))
            .respond_with(ResponseTemplate::new(404).set_body_raw(
                r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","message":"seen","detail":[]}}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;
    }

    let found = bron(&server, &["find", "6:162"]).await;
    assert!(found.status.success(), "{found:?}");

    for args in [
        &["law", "show", "BWBR0005289"][..],
        &["law", "parts", "BWBR0005289"][..],
        &["law", "relations", "BWBR0005289"][..],
        &["law", "changes", "BWBR0005289", "--from", "2020-01-01"][..],
    ] {
        // The mock answers only where no moment was sent, and its answer is
        // the not-found this asserts on.
        let output = bron(&server, args).await;
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
}

/// `--lang` is the service's to judge: bron sends it as typed on every route
/// that declares it, rather than refusing it locally as an unknown flag.
#[tokio::test(flavor = "multi_thread")]
async fn lang_reaches_every_route_that_declares_it() {
    for (route, args) in [
        (
            "/api/v1/regulations/BWBR0001489/articles",
            vec!["law", "show", "BWBR0001489", "--lang", "en"],
        ),
        (
            "/api/v1/regulations/BWBR0001489/parts",
            vec!["law", "parts", "BWBR0001489", "--lang", "en"],
        ),
        (
            "/api/v1/regulations/BWBR0001489/changes",
            vec![
                "law",
                "changes",
                "BWBR0001489",
                "--from",
                "2020-01-01",
                "--lang",
                "en",
            ],
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(route))
            .and(query_param("lang", "en"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","message":"seen","detail":[]}}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let output = bron(&server, &args).await;

        // The mock answers only where lang=en arrived; a not-found means the
        // service was asked, not that it refused the language.
        assert_eq!(output.status.code(), Some(2), "{route}: {output:?}");
    }
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
 "data":[{"regulation_id":"BWBR0020368","label":"2026-09-01_0","in_force":{"from":"2026-09-01"},"withdrawn":false,"held":true,"upstream_hash_check":"matches","later_field":9}],
 "caveats":[{"kind":"something_later","window":50}]}"#;

const PUBLISHED_CUT: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0020368","label":"2026-09-01_0","in_force":{"from":"2026-09-01"},"withdrawn":false,"held":true,"upstream_hash_check":"matches"}],
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

/// A command line bron cannot parse never reaches the service, so no code is
/// its to carry: presence says who stated the failure.
///
/// Every value a service route reads reaches it as typed, so what still fails
/// here is what no route takes: `--item` without `--paragraph`.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_bron_states_itself_carries_no_code() {
    let server = MockServer::start().await;

    let output = bron(
        &server,
        &["--json", "law", "show", "BWBR0005289", "--item", "a"],
    )
    .await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "usage");
    assert!(envelope["error"].get("code").is_none(), "{envelope}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// `case search` reads rechtspraak.nl directly rather than the bron service,
/// so bron is the only judge there is for `--limit`: it stays a `u32` and a
/// bad one never leaves this process, exactly as a malformed date does not.
#[tokio::test(flavor = "multi_thread")]
async fn case_search_still_judges_its_own_limit_locally() {
    let server = MockServer::start().await;

    let output = bron(&server, &["--json", "case", "search", "--limit", "abc"]).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "usage");
    assert!(envelope["error"].get("code").is_none(), "{envelope}");
    assert!(server.received_requests().await.unwrap().is_empty());
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
                "withdrawn": false,
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
                "withdrawn": false,
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

    // Asked for lid 2 the service answers the one article that carries it,
    // and states the same caveat about the number.
    let mut narrowed = answer.clone();
    narrowed["data"]["articles"] = serde_json::json!([article("/Bijlage2/Artikel1", &["2"])]);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0008691/articles"))
        .and(query_param("number", "1"))
        .and(query_param("paragraph", "2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(narrowed.to_string(), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0008691/articles"))
        .and(query_param("number", "1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(answer.to_string(), "application/json"),
        )
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
    assert_eq!(
        envelope["data"]["articles"].as_array().map(Vec::len),
        Some(2)
    );
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
 "data":[{"regulation_id":"BWBR0008691","label":"2019-01-01_0","in_force":{"from":"2019-01-01"},"withdrawn":false,"held":true,"upstream_hash_check":"matches"},
         {"regulation_id":"BWBR0008691","label":"2016-07-01_0","in_force":{"from":"2016-07-01","until":"2018-12-31"},"withdrawn":false,"held":true,"upstream_hash_check":"matches"}]}"#;

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
        &["--json", "law", "history", "BWBR0005289"][..],
        &["law", "history", "--json", "BWBR0005289"][..],
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

    let output = bron(&server, &["law", "history", "BWBR0005289"]).await;
    let said = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(said.contains("--article <ARTICLE>"), "{said}");
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

/// Where stdout is rows, a limit on them goes to stderr beside the cursor, so
/// a shell script reading line by line never reads a sentence as a row.
#[tokio::test(flavor = "multi_thread")]
async fn a_list_of_rows_keeps_its_caveats_off_stdout() {
    const HITS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"id":"BWBR0005537","title":"Algemene wet bestuursrecht","type":"wet","coverage":"history"}],
 "caveats":[{"kind":"truncated","window":1}]}"#;

    const VERSIONS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0008691","label":"2019-01-01_0","in_force":{"from":"2019-01-01"},"withdrawn":false,"held":true,"upstream_hash_check":"matches","unread_because":"too_large"}],
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
                "withdrawn": false,
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

/// `commencement_pending` reaches `law show` in words: a besluit names the
/// act behind a waiting unit and no rendition here has taken it in yet.
#[tokio::test(flavor = "multi_thread")]
async fn commencement_pending_reaches_law_show_in_words() {
    let answer = serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "as_of": "2026-06-01",
        "data": {
            "regulation_id": "BWBR0005289",
            "expression": {
                "regulation_id": "BWBR0005289",
                "label": "2026-01-01_0",
                "in_force": {"from": "2026-01-01"},
                "withdrawn": false,
                "held": true,
                "upstream_hash_check": "matches",
                "blob": EARLIER_BLOB,
            },
            "articles": [],
        },
        "caveats": [{
            "kind": "commencement_pending",
            "decrees": [{"decree": "stb-2026-8", "published_on": "2026-01-21", "act": "stb-2025-62"}],
        }],
    });

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0005289/articles"))
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

    let output = bron(
        &server,
        &["law", "show", "BWBR0005289", "--as-of", "2026-06-01"],
    )
    .await;
    // No article stands in the answer, so its note stands on stdout beside
    // the sentence that says so.
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.contains("stb-2026-8 commences stb-2025-62"), "{text}");
    assert!(text.contains("published 2026-01-21"), "{text}");
    assert!(text.contains("bron does not read"), "{text}");
    assert!(!text.contains("no sentence for"), "{text}");
}

/// Runs one failing command against a service answering `status` with `body`,
/// and a `Retry-After` where one is given. Returns the envelope and stderr.
async fn failing(
    status: u16,
    retry_after: Option<&str>,
    body: &str,
) -> (Output, serde_json::Value, String) {
    let server = MockServer::start().await;
    let mut response = ResponseTemplate::new(status).set_body_raw(body, "application/json");
    if let Some(seconds) = retry_after {
        response = response.insert_header("Retry-After", seconds);
    }
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0008691/articles"))
        .respond_with(response)
        .mount(&server)
        .await;

    let json = bron(&server, &["--json", "law", "show", "BWBR0008691"]).await;
    let envelope = serde_json::from_str(&stdout(&json)).unwrap();
    let human = bron(&server, &["law", "show", "BWBR0008691"]).await;
    let said = String::from_utf8(human.stderr).unwrap();

    (json, envelope, said)
}

/// A 503 carries the delay the service named and, where its body is the
/// envelope, its kind, code and message as it stated them. Where the body
/// states no delay and the header does, `--json` carries it as `retry_after`,
/// once, after the service's own fields.
#[tokio::test(flavor = "multi_thread")]
async fn the_service_unable_to_fetch_says_so_with_its_code_and_its_delay() {
    let (output, envelope, said) = failing(
        503,
        Some("60"),
        r#"{"bron":1,"grade":"authority","error":{"kind":"source","code":"source_unavailable","message":"the register did not serve 2016-07-01_0","detail":[]}}"#,
    )
    .await;
    let message = envelope["error"]["message"].as_str().unwrap();

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "source");
    assert_eq!(envelope["error"]["code"], "source_unavailable");
    assert_eq!(message, "the register did not serve 2016-07-01_0");
    assert!(
        stdout(&output).contains(
            r#""error":{"kind":"source","code":"source_unavailable","message":"the register did not serve 2016-07-01_0","detail":[],"retry_after":60}}"#
        ),
        "{output:?}"
    );
    assert!(said.contains("the service could not answer: "), "{said}");
    assert!(said.contains("60s"), "{said}");
}

/// Where the body states the delay itself, its error object crosses as sent
/// and the header's delay is not written beside it: one `retry_after`, the
/// body's. A field this build has no name for crosses too.
#[tokio::test(flavor = "multi_thread")]
async fn a_delay_the_body_states_crosses_as_sent_and_once() {
    let error = r#"{"kind":"source","code":"source_unavailable","message":"the register did not serve 2016-07-01_0","detail":[],"retry_after":60,"later":true}"#;
    let (output, _, said) = failing(
        503,
        Some("30"),
        &format!(r#"{{"bron":1,"grade":"authority","error":{error}}}"#),
    )
    .await;

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(
        stdout(&output),
        format!("{{\"bron\":1,\"grade\":\"authority\",\"error\":{error}}}\n")
    );
    assert!(said.contains("60s"), "{said}");
}

/// 'You were too fast' and 'everyone was' are two refusals under one status,
/// and a person told the wrong one changes the wrong thing.
#[tokio::test(flavor = "multi_thread")]
async fn a_spent_shared_budget_is_not_told_as_the_callers_own_limit() {
    let (output, envelope, said) = failing(
        429,
        Some("30"),
        r#"{"bron":1,"grade":"authority","error":{"kind":"throttled","code":"fetch_budget_spent","message":"the fetch budget every caller shares is spent","detail":[]}}"#,
    )
    .await;

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "throttled");
    assert_eq!(envelope["error"]["code"], "fetch_budget_spent");
    assert!(said.contains("every caller shares"), "{said}");
    assert!(said.contains("30s"), "{said}");
}

/// A 503 whose body is no envelope came from the service or something in front
/// of it, and is said to be that, with its status and the delay it named, and
/// with no code nobody stated.
#[tokio::test(flavor = "multi_thread")]
async fn a_503_without_an_envelope_is_named_with_its_delay() {
    let (output, envelope, said) =
        failing(503, Some("60"), "<html>Service Unavailable</html>").await;
    let message = envelope["error"]["message"].as_str().unwrap();

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "upstream");
    assert!(envelope["error"].get("code").is_none(), "{envelope}");
    assert_eq!(envelope["error"]["retry_after"], 60);
    assert!(message.contains("503"), "{envelope}");
    assert!(message.contains("60s"), "{envelope}");
    assert!(!said.contains("envelope"), "{said}");
    assert!(!said.contains("unusable"), "{said}");
}

/// The edge answers a 504 of its own, as plain text, when the service takes
/// longer than it waits. That is a timeout and reads as one: what a caller
/// does about it is ask again later or narrow the question, never look for a
/// contract bug that is not there.
#[tokio::test(flavor = "multi_thread")]
async fn a_504_from_the_edge_reads_as_a_timeout() {
    let (output, envelope, said) = failing(504, None, "error code: 504").await;

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "upstream");
    assert!(envelope["error"].get("code").is_none(), "{envelope}");
    assert!(said.contains("504"), "{said}");
    assert!(said.contains("did not answer in time"), "{said}");
    assert!(said.contains("ask again later"), "{said}");
    assert!(!said.contains("envelope"), "{said}");
    assert!(!said.contains("unusable"), "{said}");
}

/// A 502 is something in front of the service getting nothing it could pass
/// on, which is named for what it is and not read as an answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_502_from_the_edge_is_named_for_what_it_is() {
    let (output, envelope, said) = failing(502, None, "error code: 502").await;

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert!(envelope["error"].get("code").is_none(), "{envelope}");
    assert!(said.contains("502"), "{said}");
    assert!(!said.contains("envelope"), "{said}");
    assert!(!said.contains("unusable"), "{said}");
}

/// A 429 from in front of the service carries no envelope either. It is a
/// refusal with its delay, and no answer was given to call unusable.
#[tokio::test(flavor = "multi_thread")]
async fn a_429_without_an_envelope_is_a_refusal_and_not_an_answer() {
    let (output, envelope, said) = failing(429, Some("30"), "Too Many Requests").await;

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert!(envelope["error"].get("code").is_none(), "{envelope}");
    assert!(
        said.contains("refused the request and asked for 30s"),
        "{said}"
    );
    assert!(!said.contains("unusable"), "{said}");
}

/// A 500 whose body is no envelope is not a gateway: the service itself sent
/// something unreadable, and that is still what it is called.
#[tokio::test(flavor = "multi_thread")]
async fn a_500_without_an_envelope_is_still_unreadable() {
    let (output, _, said) = failing(500, None, "<html>Whoops</html>").await;

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert!(said.contains("unusable"), "{said}");
}

/// The service read the question and stated why it could not answer it. That
/// is not an unusable response, and saying so reads to a pipeline as 'retry'.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_the_service_states_is_not_called_unusable() {
    let (output, envelope, said) = failing(
        500,
        None,
        r#"{"bron":1,"grade":"authority","error":{"kind":"corpus","code":"rendition_too_large","message":"BWBR0008691 rendition 2016-07-01_0 is longer than bron reads","detail":[]}}"#,
    )
    .await;

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(envelope["error"]["kind"], "corpus");
    assert_eq!(envelope["error"]["code"], "rendition_too_large");
    assert!(
        said.contains("the service could not answer: BWBR0008691 rendition 2016-07-01_0 is longer than bron reads"),
        "{said}"
    );
    assert!(!said.contains("unusable"), "{said}");
}

/// A held rendition that was not parsed into articles has none to answer, and
/// a person reading the list is told which and why. `--json` is the envelope
/// as sent, so the field reaches a consumer untouched.
#[tokio::test(flavor = "multi_thread")]
async fn law_versions_says_which_held_renditions_were_not_read() {
    const UNREAD: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0008691","label":"2019-01-01_0","in_force":{"from":"2019-01-01"},"withdrawn":false,"held":true,"upstream_hash_check":"matches","unread_because":"too_large"},
         {"regulation_id":"BWBR0008691","label":"2016-07-01_0","in_force":{"from":"2016-07-01","until":"2018-12-31"},"withdrawn":false,"held":true,"upstream_hash_check":"matches"}]}"#;

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
    let articles = envelope["data"]["articles"].as_array().unwrap();
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
             "kind":"aanhef","content_hash":"a61a","distance":0.21,"places":1,"found_by":["vector"]},
            {"source":"wetgeving","unit":"article","regulation_id":"BWBR0001941","expression":"2026-09-17_0",
             "kind":"bijlage","label":"Lijst I","content_hash":"b72b","distance":0.25,"places":1,"found_by":["vector"]}]}}"#;

    let server = answering("POST", "/api/v1/search", NUMBERLESS).await;

    let output = bron(&server, &["find", "opium"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(
        text.lines()
            .any(|l| l.contains("BWBR0001941") && l.ends_with("  aanhef")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.contains("BWBR0001941") && l.ends_with("  Lijst I")),
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

/// The rendition bounds `/parts`, so `none` is a limit the service takes: bron
/// sends it as typed rather than refusing it as a word no integer parses as.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_sends_limit_none_as_typed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/parts"))
        .and(query_param("limit", "none"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(PARTS, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(&server, &["law", "parts", "BWBR0001941", "--limit", "none"]).await;

    assert!(output.status.success(), "{output:?}");
}

/// bron judges nothing the service judges: `NONE` is not the literal the
/// service reads, but that is the service's call, not the parser's. It
/// reaches the service as typed and comes back as the service's own refusal.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_sends_an_uppercase_limit_as_typed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/parts"))
        .and(query_param("limit", "NONE"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"usage","code":"invalid_parameter","message":"limit must be between 1 and 1000000, or none for the whole set","detail":[]}}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &["--json", "law", "parts", "BWBR0001941", "--limit", "NONE"],
    )
    .await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(envelope["error"]["code"], "invalid_parameter");
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

/// `law show` answers articles, and says so where a reader looks for it.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_says_it_answers_articles_only() {
    let server = MockServer::start().await;

    let output = bron(&server, &["law", "show", "--help"]).await;

    assert!(output.status.success(), "{output:?}");
    assert!(stdout(&output).contains("law parts"), "{output:?}");
}

/// One unit's side of a comparison, as `/changes` sends it with `rendered=1`.
fn side(kind: &str, status: &str, in_force_from: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": kind, "position": 1, "stam_id": "2986423", "versie_id": "1", "content_hash": "a61a",
        "amended_by": "stb-2026-262", "status": status, "in_force_from": in_force_from,
        "span": {"start": 10, "end": 90},
        "paragraphs": [{"content_hash": "a61a", "rendered": text, "span": {"start": 20, "end": 80}}],
    })
}

/// The Opiumwet between 2026-09-16 and 2026-09-17 as the service compares it:
/// Lijst I amended, which a comparison of its articles alone never saw, beside
/// an article that commenced.
fn changes_of_the_opiumwet() -> String {
    let expression = |label: &str, blob: &str| {
        serde_json::json!({
            "regulation_id": "BWBR0001941", "label": label, "in_force": {"from": &label[..10]},
            "withdrawn": false, "held": true, "upstream_hash_check": "matches", "blob": blob,
        })
    };

    serde_json::json!({
        "bron": 1, "grade": "authority", "as_of": "2026-09-17", "as_known_on": "2026-09-19",
        "data": {
            "regulation_id": "BWBR0001941",
            "from": expression("2026-07-09_0", EARLIER_BLOB),
            "to": expression("2026-09-17_0", LATER_BLOB),
            "summary": {"unchanged": 47, "amended": 1, "added": 0, "commenced": 1, "repealed": 0, "withdrawn": 0},
            "articles": [
                {
                    "change": "amended", "kind": "bijlage", "number": "I", "label": "Lijst I", "path": "/BijlageI",
                    "from": side("bijlage", "goed", "2026-07-09", "Cocaïne | Heroïne"),
                    "to": side("bijlage", "goed", "2026-09-17", "Cocaïne | Heroïne | Nitazenen"),
                },
                {
                    "change": "commenced", "kind": "artikel", "number": "13c", "label": "Artikel 13c",
                    "from": side("artikel", "nogniet", "2026-07-09", "Tekst."),
                    "to": side("artikel", "goed", "2026-09-17", "Tekst."),
                },
            ],
        },
        "caveats": [{"kind": "articles_not_compared", "repeated": ["2760090"]}],
    })
    .to_string()
}

/// `law changes` is one call to the service's own comparison, so the summary
/// and the rows are the service's, and `--json` is the envelope as sent.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_prints_the_comparison_the_service_made() {
    let answer = changes_of_the_opiumwet();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/changes"))
        .and(query_param("from", "2026-09-16"))
        .and(query_param("to", "2026-09-17"))
        .and(query_param("rendered", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(answer.clone(), "application/json"))
        .mount(&server)
        .await;

    let args = [
        "law",
        "changes",
        "BWBR0001941",
        "--from",
        "2026-09-16",
        "--to",
        "2026-09-17",
    ];

    let json = bron(&server, &[&["--json"][..], &args[..]].concat()).await;
    assert!(json.status.success(), "{json:?}");
    assert_eq!(stdout(&json), format!("{answer}\n"));

    let human = bron(&server, &args).await;
    let text = stdout(&human);
    assert!(human.status.success(), "{human:?}");
    assert!(
        text.contains("from  2026-07-09_0  2026-07-09 onwards, hash matches"),
        "{text}"
    );
    assert!(
        text.contains("to    2026-09-17_0  2026-09-17 onwards, hash matches"),
        "{text}"
    );
    assert!(
        text.contains("1 amended, 1 commenced, 0 added, 0 repealed, 0 withdrawn, 47 unchanged"),
        "{text}"
    );
    assert!(text.contains("AMENDED    Lijst I"), "{text}");
    assert!(text.contains("{+| Nitazenen+}"), "{text}");
    assert!(text.contains("through stb-2026-262"), "{text}");
    assert!(text.contains("COMMENCED  Artikel 13c"), "{text}");
    assert!(text.contains("commenced on 2026-09-17"), "{text}");
    assert!(text.contains("stam-id 2760090"), "{text}");
    assert!(!text.contains("no sentence for"), "{text}");
}

/// What narrows the comparison is the service's to apply, so every flag
/// reaches it as asked.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_sends_what_narrows_the_comparison() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/changes"))
        .and(query_param("from", "2026-09-16"))
        .and(query_param("to", "2026-09-17"))
        .and(query_param("kind", "bijlage"))
        .and(query_param("article", "I"))
        .and(query_param("all", "1"))
        .and(query_param("raw", "1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(changes_of_the_opiumwet(), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "law",
            "changes",
            "BWBR0001941",
            "--from",
            "2026-09-16",
            "--to",
            "2026-09-17",
            "--kind",
            "bijlage",
            "--article",
            "I",
            "--all",
            "--raw",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
}

/// A number or a kind neither rendition prints is the service's not-found,
/// with its code: an all-zero summary would read as 'nothing changed'.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_for_a_unit_in_neither_rendition_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/changes"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"article_not_found","message":"no article of BWBR0001941 between 2026-09-16 and 2026-09-17 [9999]","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "--json",
            "law",
            "changes",
            "BWBR0001941",
            "--from",
            "2026-09-16",
            "--to",
            "2026-09-17",
            "--article",
            "9999",
        ],
    )
    .await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(envelope["error"]["code"], "article_not_found");
}

/// What the service states per rendition reaches a person on that rendition's
/// row: the name the register gave the regulation that day, what the
/// publisher's hash says of the held bytes, and how many images it prints.
#[tokio::test(flavor = "multi_thread")]
async fn law_versions_carries_the_title_the_hash_and_the_images() {
    const VERSIONS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0001941","label":"2026-09-17_0","regulation_title":"Opiumwet","in_force":{"from":"2026-09-17"},"withdrawn":false,"held":true,"upstream_hash_check":"matches",
          "images":[{"name":"272866.png","blob":"225d1cedd0e062c153a163b13180192af4d7f541b9ac7e2ecefb971a27ff4721"},{"name":"272867.png","blob":"89282172ef556d917c6a4c120136c4559574accad09f9e97559a028973973266"}]},
         {"regulation_id":"BWBR0001941","label":"1928-12-12_0","regulation_title":"Opiumwet 1928","in_force":{"from":"1928-12-12","until":"1976-10-31"},"withdrawn":false,"held":true,"upstream_hash_check":"differs"},
         {"regulation_id":"BWBR0001941","label":"1976-11-01_0","in_force":{"from":"1976-11-01","until":"1985-01-01"},"withdrawn":false,"held":false,"upstream_hash_check":"unchecked"}]}"#;

    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0001941/expressions",
        VERSIONS,
    )
    .await;

    let output = bron(&server, &["law", "versions", "BWBR0001941"]).await;
    let text = stdout(&output);
    let err = String::from_utf8_lossy(&output.stderr);
    let row = |label: &str| text.lines().find(|l| l.contains(label)).unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(text.lines().count(), 3, "{text}");

    assert!(row("2026-09-17_0").contains("matches"), "{text}");
    assert!(row("2026-09-17_0").contains("2 images"), "{text}");
    assert!(row("2026-09-17_0").ends_with("Opiumwet"), "{text}");

    assert!(row("1928-12-12_0").contains("differs"), "{text}");
    assert!(row("1928-12-12_0").contains("0 images"), "{text}");
    assert!(row("1928-12-12_0").ends_with("Opiumwet 1928"), "{text}");

    // Nothing held, so nothing is known about its images.
    assert!(!row("1976-11-01_0").contains("images"), "{text}");
    assert!(row("1976-11-01_0").contains("unchecked"), "{text}");

    // Each state starts in the same place, whatever the window before it.
    assert_eq!(
        row("2026-09-17_0").find("matches"),
        row("1928-12-12_0").find("differs"),
        "{text}"
    );

    assert!(err.contains("publisher's hash"), "{err}");
}

/// A bijlage prints words beside its leden that no lid holds, and the unit's
/// own rendering carries them. A change only there is still a change a person
/// reads, in `law changes` and in `law parts` alike.
#[tokio::test(flavor = "multi_thread")]
async fn a_bijlage_changed_outside_its_leden_shows_the_words_that_changed() {
    let bijlage = |rendered: &str| {
        let mut side = side(
            "bijlage",
            "goed",
            "2026-07-09",
            "1 Stofgroep: fenethylaminen",
        );
        side["rendered"] = format!("{rendered}\n1 Stofgroep: fenethylaminen").into();
        side
    };
    let expression = |label: &str, blob: &str| {
        serde_json::json!({
            "regulation_id": "BWBR0001941", "label": label, "in_force": {"from": &label[..10]},
            "withdrawn": false, "held": true, "upstream_hash_check": "matches", "blob": blob,
        })
    };
    let changes = serde_json::json!({
        "bron": 1, "grade": "authority",
        "data": {
            "regulation_id": "BWBR0001941",
            "from": expression("2026-01-28_0", EARLIER_BLOB),
            "to": expression("2026-07-09_0", LATER_BLOB),
            "summary": {"unchanged": 0, "amended": 1, "added": 0, "commenced": 0, "repealed": 0, "withdrawn": 0},
            "articles": [{
                "change": "amended", "kind": "bijlage", "number": "Ia", "label": "Lijst Ia",
                "from": bijlage("Toelichting bij de lijst."),
                "to": bijlage("Toelichting bij de gewijzigde lijst."),
            }],
        },
    });
    let parts = serde_json::json!({
        "bron": 1, "grade": "authority", "as_of": "2026-07-09",
        "data": {
            "regulation_id": "BWBR0001941",
            "expression": expression("2026-07-09_0", LATER_BLOB),
            "parts": [bijlage("Toelichting bij de gewijzigde lijst.")],
        },
    });

    let server = MockServer::start().await;
    for (route, body) in [("changes", changes), ("parts", parts)] {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/regulations/BWBR0001941/{route}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_string(), "application/json"),
            )
            .mount(&server)
            .await;
    }

    let compared = bron(
        &server,
        &[
            "law",
            "changes",
            "BWBR0001941",
            "--from",
            "2026-01-28",
            "--to",
            "2026-07-09",
        ],
    )
    .await;
    let text = stdout(&compared);
    assert!(compared.status.success(), "{compared:?}");
    assert!(text.contains("{+gewijzigde+}"), "{text}");

    let shown = bron(&server, &["law", "parts", "BWBR0001941"]).await;
    let text = stdout(&shown);
    assert!(shown.status.success(), "{shown:?}");
    assert!(
        text.contains("Toelichting bij de gewijzigde lijst."),
        "{text}"
    );
}

/// The service counts every unit whatever narrowed the list, so above a list
/// narrowed to one article the counts are said to be the whole regulation's.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_says_the_counts_above_a_narrowed_list_are_the_whole_regulations() {
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0001941/changes",
        &changes_of_the_opiumwet(),
    )
    .await;
    let window = [
        "law",
        "changes",
        "BWBR0001941",
        "--from",
        "2026-09-16",
        "--to",
        "2026-09-17",
    ];
    let counts = "1 amended, 1 commenced, 0 added, 0 repealed, 0 withdrawn, 47 unchanged";

    let whole = stdout(&bron(&server, &window).await);
    assert!(whole.lines().any(|l| l == counts), "{whole}");

    for narrowing in [&["--article", "I"][..], &["--kind", "bijlage"][..]] {
        let text = stdout(&bron(&server, &[&window[..], narrowing].concat()).await);
        assert!(
            text.lines()
                .any(|l| l == format!("in the whole regulation: {counts}")),
            "{narrowing:?}: {text}"
        );
    }
}

/// A rendition the service holds no text of names no blob, and no source line
/// is printed with an empty slot where the digest would stand.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_prints_no_source_where_the_rendition_names_no_blob() {
    let unheld = PARTS.replace(
        r#""blob":"1eb72c0011119cf2fdcbc0660f12e7a0cf0612a40cb602f3a30a42a1db5b8de9","#,
        "",
    );
    assert_ne!(unheld, PARTS);
    let server = answering("GET", "/api/v1/regulations/BWBR0001941/parts", &unheld).await;

    let output = bron(&server, &["law", "parts", "BWBR0001941"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(!text.contains("source:"), "{text}");
}

/// One article of BWBR0005289 with two leden, as the service answers it. A
/// field and a caveat this build has no name for ride along.
const ARTICLE: &str = r#"{"bron":1,"grade":"authority","as_of":"2019-06-01","as_known_on":"2026-09-19",
 "data":{"regulation_id":"BWBR0005289",
  "expression":{"regulation_id":"BWBR0005289","label":"2019-04-01_0","in_force":{"from":"2019-04-01"},"withdrawn":false,"held":true,"upstream_hash_check":"matches","blob":"35e29887c98339a23c835cd45fa22c0569f2db6e7d05fbca6c8de0d4fb15a0fb","later_field":7},
  "articles":[{"kind":"artikel","position":3,"number":"3","label":"Artikel 3","path":"/Boek6/Artikel3","stam_id":"2760033","versie_id":"1","content_hash":"a61a",
   "jci":"jci1.3:c:BWBR0005289&artikel=3","span":{"start":1000,"end":2000},
   "rendered":"Een natuurlijke verbintenis is niet afdwingbaar.\nEen natuurlijke verbintenis bestaat:",
   "paragraphs":[
    {"number":"1","content_hash":"b1","rendered":"Een natuurlijke verbintenis is niet afdwingbaar.","jci":"jci1.3:c:BWBR0005289&artikel=3&lid=1","span":{"start":1100,"end":1150}},
    {"number":"2","content_hash":"b2","rendered":"Een natuurlijke verbintenis bestaat:\na. wanneer de wet dat bepaalt;\nb. wanneer een morele plicht bestaat.","jci":"jci1.3:c:BWBR0005289&artikel=3&lid=2","span":{"start":1200,"end":1250}}]}]},
 "caveats":[{"kind":"something_later","count":3}]}"#;

/// `law show` is one call to the service, so `--json` is the envelope as sent,
/// and a caveat this build has no name for keeps the name the service gave it.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_prints_the_envelope_the_service_sent() {
    let server = answering("GET", "/api/v1/regulations/BWBR0005289/articles", ARTICLE).await;

    let output = bron(
        &server,
        &[
            "--json",
            "law",
            "show",
            "BWBR0005289",
            "--article",
            "3",
            "--as-of",
            "2019-06-01",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{ARTICLE}\n"));
}

/// Narrowing to a lid is the service's own: it answers the lid's span, its
/// Juriconnect reference and its label, and a second implementation here would
/// be a second rule.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_asks_the_service_for_one_lid() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0005289/articles"))
        .and(query_param("number", "3"))
        .and(query_param("paragraph", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(ARTICLE, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "law",
            "show",
            "BWBR0005289",
            "--article",
            "3",
            "--paragraph",
            "2",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
}

/// A lid the article does not carry is the service's not-found, with its code.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_for_a_lid_the_article_does_not_carry_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0005289/articles"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"paragraph_not_found","message":"no lid 9 of BWBR0005289 artikel 3","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "--json",
            "law",
            "show",
            "BWBR0005289",
            "--article",
            "3",
            "--paragraph",
            "9",
        ],
    )
    .await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(envelope["error"]["code"], "paragraph_not_found");
}

/// `--item` is a cut of the text the service answered, for a person reading a
/// terminal: no route answers an item, and the span stays the lid's. `--json`
/// is the envelope as sent, lid and all.
#[tokio::test(flavor = "multi_thread")]
async fn an_item_cuts_what_a_person_reads_and_not_the_envelope() {
    let server = answering("GET", "/api/v1/regulations/BWBR0005289/articles", ARTICLE).await;
    let args = [
        "law",
        "show",
        "BWBR0005289",
        "--article",
        "3",
        "--paragraph",
        "2",
        "--item",
        "b",
    ];

    let human = bron(&server, &args).await;
    let text = stdout(&human);
    assert!(human.status.success(), "{human:?}");
    assert!(
        text.contains("b. wanneer een morele plicht bestaat."),
        "{text}"
    );
    assert!(!text.contains("a. wanneer de wet dat bepaalt;"), "{text}");

    let json = bron(&server, &[&["--json"][..], &args[..]].concat()).await;
    assert_eq!(stdout(&json), format!("{ARTICLE}\n"));
}

/// An item is a cut of what a person reads, and no route answers one. Where
/// the lid prints no such item a person reads that it does not, and `--json`
/// is the lid the service sent, exit 0 either way.
#[tokio::test(flavor = "multi_thread")]
async fn an_item_the_lid_does_not_print_is_said_in_words() {
    let server = answering("GET", "/api/v1/regulations/BWBR0005289/articles", ARTICLE).await;
    let asked = [
        "law",
        "show",
        "BWBR0005289",
        "--article",
        "3",
        "--paragraph",
        "2",
        "--item",
        "z",
    ];

    let human = bron(&server, &asked).await;
    assert_eq!(human.status.code(), Some(0), "{human:?}");
    assert!(
        stdout(&human).contains("This lid prints no item z."),
        "{human:?}"
    );

    let json: Vec<&str> = ["--json"].iter().chain(&asked).copied().collect();
    let output = bron(&server, &json).await;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(stdout(&output), format!("{ARTICLE}\n"));
}

/// The heading of a lid answer says which lid, whether or not the service's
/// own label for the article is there to carry it.
#[tokio::test(flavor = "multi_thread")]
async fn a_lid_answer_says_which_lid_even_without_a_label() {
    let unlabelled = ARTICLE.replace(r#""label":"Artikel 3","#, "");
    assert_ne!(unlabelled, ARTICLE);
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0005289/articles",
        &unlabelled,
    )
    .await;

    let output = bron(
        &server,
        &[
            "law",
            "show",
            "BWBR0005289",
            "--article",
            "3",
            "--paragraph",
            "2",
        ],
    )
    .await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.lines().any(|l| l == "3 lid 2"), "{text}");
}

/// A rendition whose text the service does not hold has no article to answer,
/// and the rendition resolved: the service answers that with an empty list,
/// and so does `--json`. A person reads which rendition and why, where the
/// publisher still serves it.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_names_the_rendition_whose_text_is_not_held() {
    let unheld = serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "as_of": "2015-01-01",
        "data": {
            "regulation_id": "BWBR0008691",
            "expression": {
                "regulation_id": "BWBR0008691",
                "label": "2014-07-01_0",
                "in_force": {"from": "2014-07-01"},
                "withdrawn": false,
                "held": false,
                "upstream_hash_check": "unchecked",
            },
            "articles": [],
        },
    });
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0008691/articles",
        &unheld.to_string(),
    )
    .await;

    let output = bron(&server, &["law", "show", "BWBR0008691"]).await;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        stdout(&output),
        "The service holds no text of BWBR0008691 rendition 2014-07-01_0, so it answers no articles.\n"
    );

    let json = bron(&server, &["--json", "law", "show", "BWBR0008691"]).await;
    assert_eq!(json.status.code(), Some(0), "{json:?}");
    assert_eq!(stdout(&json), format!("{unheld}\n"));
}

/// A rendition the service holds and could not parse has no article to answer
/// either, and a person reads which rendition and why rather than a law with
/// no articles in it.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_says_why_a_held_rendition_answered_no_article() {
    let unread = serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "as_of": "2019-01-01",
        "data": {
            "regulation_id": "BWBR0008691",
            "expression": {
                "regulation_id": "BWBR0008691",
                "label": "2019-01-01_0",
                "in_force": {"from": "2019-01-01"},
                "withdrawn": false,
                "held": true,
                "unread_because": "too_large",
                "upstream_hash_check": "matches",
                "blob": EARLIER_BLOB,
            },
            "articles": [],
        },
    });
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0008691/articles",
        &unread.to_string(),
    )
    .await;

    let output = bron(&server, &["law", "show", "BWBR0008691"]).await;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        stdout(&output),
        "BWBR0008691 rendition 2019-01-01_0 is held and was not parsed (too_large), so it answers no articles.\n"
    );
}

/// The address printed under a span is built from what the service called the
/// document. A digest that is no digest names no document, and a URL made of
/// it would send a reader somewhere that answers nothing.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_prints_no_address_for_a_digest_that_is_no_digest() {
    let nonsense = ARTICLE.replace(
        "35e29887c98339a23c835cd45fa22c0569f2db6e7d05fbca6c8de0d4fb15a0fb",
        "../../etc/passwd",
    );
    assert_ne!(nonsense, ARTICLE);
    let server = answering("GET", "/api/v1/regulations/BWBR0005289/articles", &nonsense).await;

    let output = bron(&server, &["law", "show", "BWBR0005289", "--article", "3"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.contains("source: ../../etc/passwd"), "{text}");
    assert!(!text.contains("/api/v1/blobs/"), "{text}");
}

// -- slice 2: the rows a person reads -----------------------------------

/// Which rendition answered is part of the answer: a person reading 'as in
/// force on 2019-06-01' is otherwise not told which text that date resolved to.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_names_the_rendition_that_answered() {
    let server = answering("GET", "/api/v1/regulations/BWBR0005289/articles", ARTICLE).await;

    let text = stdout(&bron(&server, &["law", "show", "BWBR0005289", "--article", "3"]).await);

    assert!(
        text.lines()
            .any(|l| l == "from rendition 2019-04-01_0, in force 2019-04-01 onwards"),
        "{text}"
    );
}

/// `law parts` reads a unit the way `law show` reads an article: the rendition
/// that answered, the address of its document, and whether the unit applies.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_reads_a_unit_the_way_law_show_reads_an_article() {
    let waiting = PARTS.replace(
        r#""kind":"bijlage","position":2,"#,
        r#""kind":"bijlage","position":2,"status":"nogniet","in_force_from":"2027-01-01","amended_by":"stb-2026-300","effect":"wijziging","#,
    );
    assert_ne!(waiting, PARTS);
    let server = answering("GET", "/api/v1/regulations/BWBR0001941/parts", &waiting).await;

    let text = stdout(&bron(&server, &["law", "parts", "BWBR0001941"]).await);

    assert!(text.contains("from rendition 2026-09-17_0"), "{text}");
    assert!(
        text.contains(
            "/api/v1/blobs/1eb72c0011119cf2fdcbc0660f12e7a0cf0612a40cb602f3a30a42a1db5b8de9"
        ),
        "{text}"
    );
    assert!(
        text.contains("not yet applied, applies from 2027-01-01"),
        "{text}"
    );
    assert!(
        text.contains("published by stb-2026-300 (wijziging)"),
        "{text}"
    );
}

/// The not-yet-applied line stands ahead of the unit's own name and words, so
/// a reader is told before reading them rather than after, and the dossiers
/// the BWB states for the act stand beside it, not only in the JSON envelope.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_puts_the_waiting_line_first_and_names_its_dossiers() {
    let waiting = PARTS.replace(
        r#""kind":"bijlage","position":2,"#,
        r#""kind":"bijlage","position":2,"status":"nogniet","in_force_from":"2027-01-01","amended_by":"stb-2026-300","dossiers":[{"dossier":"36212","stated_in":"1eb72c0011119cf2fdcbc0660f12e7a0cf0612a40cb602f3a30a42a1db5b8de9"}],"effect":"wijziging","#,
    );
    assert_ne!(waiting, PARTS);
    let server = answering("GET", "/api/v1/regulations/BWBR0001941/parts", &waiting).await;

    let text = stdout(&bron(&server, &["law", "parts", "BWBR0001941"]).await);

    let not_yet = text
        .find("not yet applied, applies from 2027-01-01")
        .unwrap_or_else(|| panic!("{text}"));
    let name = text.find("Lijst Ia").unwrap_or_else(|| panic!("{text}"));
    assert!(not_yet < name, "{text}");
    assert!(
        text.contains("published by stb-2026-300 (wijziging), dossier 36212"),
        "{text}"
    );
}

/// `dossiers` absent, from a rendition never read for them, and `dossiers: []`,
/// from one read that states none, both leave the act's line as it always
/// was: only a stated dossier earns the words.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_names_no_dossier_where_the_service_states_none() {
    for dossiers in ["", r#""dossiers":[],"#] {
        let fixture = PARTS.replace(
            r#""kind":"bijlage","position":2,"#,
            &format!(
                r#""kind":"bijlage","position":2,"status":"nogniet","in_force_from":"2027-01-01","amended_by":"stb-2026-300",{dossiers}"effect":"wijziging","#
            ),
        );
        assert_ne!(fixture, PARTS);
        let server = answering("GET", "/api/v1/regulations/BWBR0001941/parts", &fixture).await;

        let text = stdout(&bron(&server, &["law", "parts", "BWBR0001941"]).await);

        assert!(
            text.contains("published by stb-2026-300 (wijziging)\n"),
            "{dossiers}: {text}"
        );
        assert!(!text.contains("dossier"), "{dossiers}: {text}");
    }
}

/// `--not-commenced` asks the service for the waiting units of every kind,
/// and an answer holding only those is bound to the rendition that publishes
/// them rather than said to be in force.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_asks_for_what_is_published_and_not_in_force() {
    let waiting = PARTS
        .replace(
            r#"{"kind":"aanhef","position":1,"stam_id":"1","versie_id":"1","content_hash":"a61a","span":{"start":100,"end":400},
    "paragraphs":[{"content_hash":"a61a","rendered":"Wij WILHELMINA, bij de gratie Gods.","span":{"start":100,"end":400}}]},
   "#,
            "",
        )
        .replace(
            r#""kind":"bijlage","position":2,"#,
            r#""kind":"bijlage","position":2,"status":"nogniet","in_force_from":"2027-01-01","amended_by":"stb-2026-300","effect":"wijziging","#,
        );
    assert!(!waiting.contains("aanhef"), "{waiting}");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/parts"))
        .and(query_param("not_commenced", "1"))
        .and(query_param("rendered", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(waiting, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(&server, &["law", "parts", "BWBR0001941", "--not-commenced"]).await;
    let text = stdout(&output);

    // The mock answers only where the flag reached the service.
    assert!(output.status.success(), "{output:?}");
    assert!(
        text.starts_with("published in the rendition current on 2026-09-19, and not yet applied\n"),
        "{text}"
    );
    assert!(!text.contains("in force on"), "{text}");
    assert!(!text.contains("in force through"), "{text}");
    assert!(
        text.contains("not yet applied, applies from 2027-01-01"),
        "{text}"
    );
}

/// Nothing waiting in a rendition is a not-found, and a rendition the parser
/// has read for its articles alone cannot say what else waits: both keep the
/// kind and the code the service stated.
#[tokio::test(flavor = "multi_thread")]
async fn law_parts_not_commenced_failures_keep_the_kind_the_service_gave_them() {
    for (status, body, exit, code) in [
        (
            404,
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"nothing_not_commenced","message":"BWBR0001941 rendition 2026-09-17_0 publishes nothing that does not yet apply","detail":[]}}"#,
            2,
            "nothing_not_commenced",
        ),
        (
            503,
            r#"{"bron":1,"grade":"authority","error":{"kind":"corpus","code":"rendition_read_for_articles","message":"BWBR0001941 rendition 2026-09-17_0 was read for its articles alone","detail":[]}}"#,
            4,
            "rendition_read_for_articles",
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/regulations/BWBR0001941/parts"))
            .and(query_param("not_commenced", "1"))
            .respond_with(ResponseTemplate::new(status).set_body_raw(body, "application/json"))
            .mount(&server)
            .await;

        let output = bron(
            &server,
            &["--json", "law", "parts", "BWBR0001941", "--not-commenced"],
        )
        .await;
        let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        assert_eq!(envelope["error"]["code"], code, "{envelope}");
    }
}

/// A unit of a treaty says which language it stands in and whether that text
/// is the authentic one, because a treaty can print one number twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_unit_of_a_treaty_says_its_language_and_whether_it_is_authentic() {
    let treaty = ARTICLE.replace(
        r#""kind":"artikel","position":3,"#,
        r#""kind":"artikel","position":3,"lang":"en","text_status":"vertaling","#,
    );
    assert_ne!(treaty, ARTICLE);
    let server = answering("GET", "/api/v1/regulations/BWBR0005289/articles", &treaty).await;

    let text = stdout(&bron(&server, &["law", "show", "BWBR0005289", "--article", "3"]).await);

    assert!(
        text.lines().any(|l| l == "Artikel 3  (en, vertaling)"),
        "{text}"
    );
}

/// Each side of a comparison is checkable: which document it came from, what
/// the publisher's hash says of it, and where the unit stands in it.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_names_each_side_and_where_each_unit_stands() {
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0001941/changes",
        &changes_of_the_opiumwet(),
    )
    .await;

    let text = stdout(
        &bron(
            &server,
            &[
                "law",
                "changes",
                "BWBR0001941",
                "--from",
                "2026-09-16",
                "--to",
                "2026-09-17",
            ],
        )
        .await,
    );

    assert!(
        text.lines()
            .any(|l| l == "from  2026-07-09_0  2026-07-09 onwards, hash matches"),
        "{text}"
    );
    assert!(
        text.lines().any(|l| l
            == format!(
                "      http://127.0.0.1:{}/api/v1/blobs/{EARLIER_BLOB}",
                server.address().port()
            )),
        "{text}"
    );
    assert!(
        text.contains("to    2026-09-17_0  2026-09-17 onwards, hash matches"),
        "{text}"
    );
    assert!(text.contains("           spans 10..90 -> 10..90"), "{text}");
}

/// A rendition row reads the same whichever command lists it, and says what a
/// person needs to tell renditions apart: both windows, and whether the
/// publisher withdrew it.
#[tokio::test(flavor = "multi_thread")]
async fn a_rendition_row_reads_the_same_in_law_versions_and_law_published() {
    const ROWS: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"regulation_id":"BWBR0005537","label":"2018-07-01_0","regulation_title":"Algemene wet bestuursrecht","in_force":{"from":"2018-07-01","until":"2018-12-31"},"as_known":{"from":"2018-09-12"},"withdrawn":false,"held":true,"upstream_hash_check":"matches"},
         {"regulation_id":"BWBR0005537","label":"2017-07-01_0","in_force":{"from":"2017-07-01"},"withdrawn":true,"held":false,"upstream_hash_check":"unchecked"}]}"#;

    let versions = answering("GET", "/api/v1/regulations/BWBR0005537/expressions", ROWS).await;
    let published = answering("GET", "/api/v1/expressions", ROWS).await;

    let listed = stdout(&bron(&versions, &["law", "versions", "BWBR0005537"]).await);
    let window = stdout(&bron(&published, &["law", "published", "--from", "2017-01-01"]).await);

    let retro = listed.lines().find(|l| l.contains("2018-07-01_0")).unwrap();
    assert!(retro.contains("2018-07-01 to 2018-12-31"), "{listed}");
    assert!(retro.contains("2018-09-12 onwards"), "{listed}");

    let withdrawn = listed.lines().find(|l| l.contains("2017-07-01_0")).unwrap();
    assert!(withdrawn.contains("withdrawn"), "{listed}");
    // The manifest states no zicht window for it, and the row says so.
    assert!(withdrawn.contains("  -  "), "{listed}");

    for (row, label) in listed.lines().zip(["2018-07-01_0", "2017-07-01_0"]) {
        let beside = window.lines().find(|l| l.contains(label)).unwrap();
        assert_eq!(
            beside.replacen("BWBR0005537  ", "", 1),
            row.to_string(),
            "{listed}\n{window}"
        );
    }
}

/// An event that names a rendition says which windows it has, and whether it
/// was withdrawn, so a poller reading rows need not ask again to know.
#[tokio::test(flavor = "multi_thread")]
async fn a_feed_event_carries_its_rendition_windows() {
    const EVENTS: &str = r#"{"bron":1,"grade":"authority",
 "data":{"origin":"a","head":42,"next":42,"events":[
  {"seq":41,"event":"withdrawn","regulation_id":"BWBR0024282","label":"2026-07-01_0","in_force":{"from":"2026-07-01"},"as_known":{"from":"2026-07-02"},"withdrawn":true},
  {"seq":42,"event":"listed","regulation_id":"BWBR0024283","withdrawn":false}]}}"#;
    let server = answering("GET", "/api/v1/feed", EVENTS).await;

    let text = stdout(&bron(&server, &["law", "feed", "--since", "40"]).await);
    let withdrawn = text.lines().find(|l| l.contains("BWBR0024282")).unwrap();
    let listed = text.lines().find(|l| l.contains("BWBR0024283")).unwrap();

    assert!(withdrawn.contains("2026-07-01 onwards"), "{text}");
    assert!(withdrawn.contains("2026-07-02 onwards"), "{text}");
    assert!(withdrawn.trim_end().ends_with("withdrawn"), "{text}");
    assert!(listed.trim_end().ends_with("BWBR0024283"), "{text}");
}

/// A window the event does not state reads as `-` on either axis, one
/// convention in one row.
#[tokio::test(flavor = "multi_thread")]
async fn a_feed_event_marks_a_window_it_does_not_state_one_way() {
    const HALF: &str = r#"{"bron":1,"grade":"authority",
 "data":{"origin":"a","head":41,"next":41,"events":[
  {"seq":41,"event":"rescheduled","regulation_id":"BWBR0024282","label":"2026-07-01_0","as_known":{"from":"2026-07-02"},"withdrawn":false}]}}"#;
    let server = answering("GET", "/api/v1/feed", HALF).await;

    let text = stdout(&bron(&server, &["law", "feed", "--since", "40"]).await);
    let row = text.lines().find(|l| l.contains("BWBR0024282")).unwrap();

    assert!(row.contains("2026-07-01_0  -  "), "{text}");
    assert!(row.ends_with("2026-07-02 onwards"), "{text}");
}

/// A regulation with no renditions to list prints no legend for rows that are
/// not there, as `law published` does not for an empty window.
#[tokio::test(flavor = "multi_thread")]
async fn law_versions_with_nothing_to_list_prints_no_legend() {
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0005537/expressions",
        r#"{"bron":1,"grade":"authority","data":[]}"#,
    )
    .await;

    let output = bron(&server, &["law", "versions", "BWBR0005537"]).await;

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

/// A hit on `law search` says whether `law show` can answer it: the type,
/// how much of it the corpus means to hold, and how many renditions it holds.
#[tokio::test(flavor = "multi_thread")]
async fn a_law_search_row_says_how_much_of_the_law_is_held() {
    const TWO: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"id":"BWBR0005537","title":"Algemene wet bestuursrecht","type":"wet","coverage":"history","expression_count":236,"held_count":236},
         {"id":"BWBR0035184","title":"Regeling Awb","type":"ministeriele-regeling","coverage":"current","expression_count":2,"held_count":1}]}"#;
    let server = answering("GET", "/api/v1/regulations", TWO).await;

    let text = stdout(&bron(&server, &["law", "search", "awb"]).await);
    let awb = text.lines().find(|l| l.starts_with("BWBR0005537")).unwrap();
    let regeling = text.lines().find(|l| l.starts_with("BWBR0035184")).unwrap();

    assert!(awb.contains("236 of 236 held"), "{text}");
    assert!(awb.contains("history"), "{text}");
    assert!(regeling.contains("1 of 2 held"), "{text}");
    assert!(regeling.contains("ministeriele-regeling"), "{text}");
    // The title starts in one place on every row.
    assert_eq!(
        awb.find("Algemene wet"),
        regeling.find("Regeling Awb"),
        "{text}"
    );
}

/// Where a character column starts, counted in characters as `{:<w$}` pads,
/// not in bytes.
fn column(line: &str, needle: &str) -> Option<usize> {
    line.find(needle).map(|at| line[..at].chars().count())
}

/// A type the register spells with a diaeresis pads like any other, so the
/// title starts in one place; a row without a title ends at its last column;
/// and a column no row fills is not printed as an empty gap.
#[tokio::test(flavor = "multi_thread")]
async fn a_law_search_row_pads_by_character_and_leaves_out_what_no_row_has() {
    const ACCENTED: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"id":"BWBR0035184","title":"Regeling Awb","type":"ministeriële regeling","coverage":"current","expression_count":2,"held_count":1},
         {"id":"BWBR0005537","type":"wet","coverage":"history","expression_count":236,"held_count":236}]}"#;
    const UNTYPED: &str = r#"{"bron":1,"grade":"authority",
 "data":[{"id":"BWBR0005537","title":"Algemene wet bestuursrecht","coverage":"history"}]}"#;

    let accented = answering("GET", "/api/v1/regulations", ACCENTED).await;
    let text = stdout(&bron(&accented, &["law", "search", "awb"]).await);
    let regeling = text.lines().find(|l| l.starts_with("BWBR0035184")).unwrap();
    let awb = text.lines().find(|l| l.starts_with("BWBR0005537")).unwrap();

    assert_eq!(
        column(regeling, "current"),
        column(awb, "history"),
        "{text}"
    );
    assert_eq!(awb, awb.trim_end(), "{text}");

    let untyped = answering("GET", "/api/v1/regulations", UNTYPED).await;
    let text = stdout(&bron(&untyped, &["law", "search", "awb"]).await);
    assert!(
        text.lines()
            .any(|l| l == "BWBR0005537  history  Algemene wet bestuursrecht"),
        "{text}"
    );
}

/// A hit names the rendition it was found in, how it was found, and the other
/// places the same text sits.
#[tokio::test(flavor = "multi_thread")]
async fn a_hit_says_where_it_was_found_and_how() {
    const RANKED: &str = r#"{"bron":1,"grade":"authority","as_of":"2026-09-19","data":{"kind":"ranking","searched":["wetgeving"],
 "hits":[{"source":"wetgeving","unit":"article","regulation_id":"BWBR0005289","expression":"2026-07-16_0","kind":"artikel","number":"162","label":"Artikel 162",
          "content_hash":"f812","distance":0.313,"places":30,"found_by":["vector","text"],
          "also":[{"unit":"article","regulation_id":"BWBR0028749","expression":"2010-10-10_0","kind":"artikel","number":"162","label":"Artikel 162"},
                  {"unit":"lid","lid":{"ordinal":0},"regulation_id":"BWBR0005289","expression":"2026-07-16_0","kind":"artikel","number":"162","label":"Artikel 162"}]}]}}"#;
    let server = answering("POST", "/api/v1/search", RANKED).await;

    let text = stdout(&bron(&server, &["find", "onrechtmatige", "daad"]).await);
    let hit = text.lines().find(|l| l.contains("BWBR0005289")).unwrap();

    assert!(hit.contains("2026-07-16_0"), "{text}");
    assert!(hit.contains("vector+text"), "{text}");

    let also: Vec<&str> = text.lines().filter(|l| l.contains(" also ")).collect();
    assert_eq!(also.len(), 2, "{text}");

    // Every column of an `also` row stands where the hit row has it, so a
    // reader comparing the two compares like with like.
    let expression = hit.find("2026-07-16_0");
    for (row, rendition) in also.iter().zip(["2010-10-10_0", "2026-07-16_0"]) {
        assert_eq!(row.find("BWBR"), hit.find("BWBR"), "{text}");
        assert_eq!(row.find(rendition), expression, "{text}");
        assert_eq!(row.find("Artikel 162"), hit.find("Artikel 162"), "{text}");
    }
    let unit = hit.find("article").unwrap();
    assert_eq!(also[0].find("article"), Some(unit), "{text}");
    // The same words as the article's own lid are another place, and read as
    // one rather than as the hit printed twice.
    assert_eq!(also[1].find("lid "), Some(unit), "{text}");
    assert!(
        text.lines().any(|l| l.trim() == "and 27 more place(s)"),
        "{text}"
    );
}

/// A citation that names a lid, or its law by name, is read that way, and the
/// line above the readings says so rather than printing the bare number.
#[tokio::test(flavor = "multi_thread")]
async fn a_lookup_says_the_lid_and_the_name_it_read() {
    const LID: &str = r#"{"bron":1,"grade":"authority","as_of":"2026-09-19","data":{"kind":"lookup","citation":"8:4","paragraph":"2","name":"Awb",
 "resolved":[{"reading":"named","source":"wetgeving","regulation_id":"BWBR0005537","expression":"2026-08-15_0","held":true,"number":"8:4","kind":"artikel","label":"Artikel 8:4","caveats":[]}]}}"#;
    let server = answering("POST", "/api/v1/search", LID).await;

    let text = stdout(&bron(&server, &["find", "8:4", "lid", "2", "Awb"]).await);

    assert!(text.lines().any(|l| l == "8:4 lid 2 Awb"), "{text}");
    assert!(
        text.contains("law show BWBR0005537 --article 8:4 --paragraph 2"),
        "{text}"
    );
}

// -- slice 3: the routes nothing reached ---------------------------------

/// A document the service holds, and the digest of its own bytes.
const DOCUMENT: &[u8] = b"<toestand><artikel>Eerste lid.</artikel></toestand>";

fn digest_of(bytes: &[u8]) -> String {
    bron::blob::BlobId::of(bytes).to_string()
}

async fn serving_blob(digest: &str, bytes: &[u8]) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/blobs/{digest}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(bytes.to_vec(), "application/octet-stream"),
        )
        .mount(&server)
        .await;
    server
}

/// The check the README tells every reader to make, as one command: fetch the
/// bytes a digest names, hash them, and hand them over untouched, so a pipe to
/// any SHA-256 tool prints the digest back.
#[tokio::test(flavor = "multi_thread")]
async fn bron_blob_hands_over_the_bytes_a_digest_names_and_nothing_else() {
    let digest = digest_of(DOCUMENT);
    let server = serving_blob(&digest, DOCUMENT).await;

    let output = bron(&server, &["blob", &digest]).await;
    let said = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, DOCUMENT);
    assert!(
        said.contains(&format!("{} bytes hash to {digest}", DOCUMENT.len())),
        "{said}"
    );
}

/// A span is cut from bytes bron hashed itself, which is what makes the slice
/// a proof rather than a report.
#[tokio::test(flavor = "multi_thread")]
async fn bron_blob_cuts_a_span_out_of_bytes_it_hashed() {
    let digest = digest_of(DOCUMENT);
    let server = serving_blob(&digest, DOCUMENT).await;

    let output = bron(&server, &["blob", &digest, "--span", "19..30"]).await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"Eerste lid.");
}

/// Bytes that do not hash to the digest they were served under are refused,
/// and none of them reach stdout: handing them over would be quoting a
/// document the digest does not name.
#[tokio::test(flavor = "multi_thread")]
async fn bron_blob_refuses_bytes_that_do_not_hash_to_their_digest() {
    let digest = digest_of(DOCUMENT);
    let server = serving_blob(&digest, b"<toestand>something else</toestand>").await;

    let output = bron(&server, &["blob", &digest]).await;
    let said = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(said.contains(&digest), "{said}");
    assert!(said.contains("hash to"), "{said}");
}

/// A digest the service holds nothing under is its not-found, with its code.
#[tokio::test(flavor = "multi_thread")]
async fn bron_blob_for_a_digest_that_names_nothing_is_not_found() {
    let digest = "0".repeat(64);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/blobs/{digest}")))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            format!(r#"{{"bron":1,"grade":"authority","error":{{"kind":"not_found","code":"blob_not_found","message":"no blob [{digest}]","detail":[]}}}}"#),
            "application/json",
        ))
        .mount(&server)
        .await;

    let output = bron(&server, &["--json", "blob", &digest]).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(envelope["error"]["code"], "blob_not_found");
}

/// A span that is no byte range, or one outside the document bron hashed, is
/// the caller's to fix: only bron holds those bytes, so only bron can say.
#[tokio::test(flavor = "multi_thread")]
async fn bron_blob_refuses_a_span_that_is_none_or_outside_the_bytes() {
    let digest = digest_of(DOCUMENT);
    let server = serving_blob(&digest, DOCUMENT).await;

    for args in [
        &["blob", &digest, "--span", "30..19"][..],
        &["blob", &digest, "--span", "0..999"][..],
        &["blob", &digest, "--span", "nine"][..],
    ] {
        let output = bron(&server, args).await;
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
    }
}

/// `--json` answers what was checked, in bron's own envelope: the digest, the
/// address, the length and the span, and the text where the bytes are text.
/// Bytes that are no text, an image, are left for the human form to hand over.
#[tokio::test(flavor = "multi_thread")]
async fn bron_blob_answers_what_it_checked_under_json() {
    let digest = digest_of(DOCUMENT);
    let server = serving_blob(&digest, DOCUMENT).await;

    let output = bron(&server, &["--json", "blob", &digest, "--span", "19..30"]).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let data = &envelope["data"];

    assert!(output.status.success(), "{output:?}");
    assert_eq!(data["provenance"]["blob"], digest.as_str(), "{envelope}");
    assert_eq!(data["provenance"]["byte_len"], DOCUMENT.len(), "{envelope}");
    assert_eq!(data["provenance"]["frozen_by"], "bron", "{envelope}");
    assert_eq!(
        data["span"],
        serde_json::json!({"start": 19, "end": 30}),
        "{envelope}"
    );
    assert_eq!(data["text"], "Eerste lid.", "{envelope}");

    let image = [0x89, b'P', b'N', b'G', 0xff, 0xfe];
    let png = digest_of(&image);
    let server = serving_blob(&png, &image).await;
    let output = bron(&server, &["--json", "blob", &png]).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(envelope["data"].get("text").is_none(), "{envelope}");
    assert_eq!(envelope["data"]["provenance"]["byte_len"], 6, "{envelope}");
}

/// What one article has been, as the service answers it: two texts it had, and
/// a window whose renditions the corpus does not hold.
const HISTORY: &str = r#"{"bron":1,"grade":"authority",
 "data":{"regulation_id":"BWBR0005289","number":"119a","as_known_on":"2026-09-19",
  "articles":[{"kind":"artikel","stam_id":"3231133","number":"119a","label":"Artikel 119a","path":"/Boek6/Artikel119a","later_field":1,
   "entries":[
    {"state":"text","in_force":{"from":"2022-07-01"},"number":"119a","label":"Artikel 119a","content_hash":"e53c","amended_by":"stb-2022-146","commenced_by":"stb-2022-175","effect":"wijziging","status":"goed","text_in_force_from":"2022-07-01","renditions":{"first":"2022-07-01_0","last":"2026-07-16_0","count":13}},
    {"state":"text","in_force":{"from":"2017-07-01","until":"2022-06-30"},"number":"119a","label":"Artikel 119a","content_hash":"cc5e","amended_by":"stb-2017-170","effect":"wijziging","status":"goed","renditions":{"first":"2017-07-01_0","last":"2022-05-28_0","count":12}},
    {"state":"unknown","in_force":{"from":"2002-11-07","until":"2017-06-30"},"renditions":{"first":"2002-11-07_0","last":"2017-03-10_0","count":9}}]}]},
 "caveats":[{"kind":"renditions_not_held","count":9,"renditions":34}]}"#;

/// `law history` is one call to the service, so `--json` is the envelope as
/// sent, and the narrowing reaches the service as asked.
#[tokio::test(flavor = "multi_thread")]
async fn law_history_prints_the_envelope_the_service_sent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v1/regulations/BWBR0005289/articles/119a/history",
        ))
        .and(query_param("as_known_on", "2024-01-01"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(HISTORY, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "--json",
            "law",
            "history",
            "BWBR0005289",
            "--article",
            "119a",
            "--as-known-on",
            "2024-01-01",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{HISTORY}\n"));
}

/// One row per text the article has had: the window it applied in, whether the
/// corpus holds it, the act that produced it, and which renditions carry it.
/// The rows are stdout and every note on them stderr.
#[tokio::test(flavor = "multi_thread")]
async fn law_history_prints_one_row_per_text_the_article_had() {
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0005289/articles/119a/history",
        HISTORY,
    )
    .await;

    let output = bron(
        &server,
        &["law", "history", "BWBR0005289", "--article", "119a"],
    )
    .await;
    let text = stdout(&output);
    let said = String::from_utf8_lossy(&output.stderr);
    let rows: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(rows.len(), 3, "{text}");
    assert!(rows[0].contains("2022-07-01 onwards"), "{text}");
    assert!(rows[0].contains("stb-2022-146 (wijziging)"), "{text}");
    assert!(
        rows[0].contains("13 renditions, 2022-07-01_0 to 2026-07-16_0"),
        "{text}"
    );
    // One is one, and a row that says otherwise reads as a count nobody checked.
    assert!(rows[2].contains("9 renditions,"), "{text}");
    // A window whose renditions are not held says so rather than reading as a
    // window in which the article said nothing.
    assert!(rows[2].contains("not held"), "{text}");
    assert!(!text.contains("note:"), "{text}");
    assert!(
        said.contains("note: the service does not hold the text of 9"),
        "{said}"
    );
}

/// A unit with no number of its own is asked for by the kind it is printed
/// under, and a number the regulation never printed is the service's
/// not-found.
#[tokio::test(flavor = "multi_thread")]
async fn law_history_asks_for_a_bijlage_by_kind_and_reports_a_number_that_is_not_there() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/articles/I/history"))
        .and(query_param("kind", "bijlage"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(HISTORY, "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0001941/articles/9999/history"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"article_not_found","message":"no article [9999]","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let bijlage = bron(
        &server,
        &[
            "law",
            "history",
            "BWBR0001941",
            "--article",
            "I",
            "--kind",
            "bijlage",
        ],
    )
    .await;
    assert!(bijlage.status.success(), "{bijlage:?}");

    let missing = bron(
        &server,
        &[
            "--json",
            "law",
            "history",
            "BWBR0001941",
            "--article",
            "9999",
        ],
    )
    .await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&missing)).unwrap();
    assert_eq!(missing.status.code(), Some(2), "{missing:?}");
    assert_eq!(envelope["error"]["code"], "article_not_found");
}

/// One regulation as the index holds it, which says whether `law show` can
/// answer about it at all.
const REGULATION: &str = r#"{"bron":1,"grade":"authority",
 "data":{"id":"BWBR0005537","title":"Algemene wet bestuursrecht","type":"wet","language":"nl","authority":"Veiligheid en Justitie",
  "legal_areas":["Staats- en bestuursrecht","Bestuursrecht"],"government_domains":["Overheid, bestuur en koninkrijk"],
  "established_by":[],"coverage":"history","expression_count":236,"held_count":236,"later_field":true}}"#;

/// `law about` is one call to the service, and says what the index knows: how
/// much of the regulation the corpus means to hold, and how much it does.
#[tokio::test(flavor = "multi_thread")]
async fn law_about_says_what_the_index_holds() {
    let server = answering("GET", "/api/v1/regulations/BWBR0005537", REGULATION).await;

    let json = bron(&server, &["--json", "law", "about", "BWBR0005537"]).await;
    assert!(json.status.success(), "{json:?}");
    assert_eq!(stdout(&json), format!("{REGULATION}\n"));

    let text = stdout(&bron(&server, &["law", "about", "BWBR0005537"]).await);
    assert!(
        text.lines()
            .any(|l| l == "BWBR0005537  Algemene wet bestuursrecht"),
        "{text}"
    );
    assert!(
        text.contains("wet, history, 236 of 236 renditions held"),
        "{text}"
    );
    assert!(text.contains("Veiligheid en Justitie"), "{text}");
    assert!(
        text.contains("Staats- en bestuursrecht, Bestuursrecht"),
        "{text}"
    );
}

/// A regulation the register holds no record of is the service's not-found.
#[tokio::test(flavor = "multi_thread")]
async fn law_about_for_a_regulation_that_is_not_there_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0000001"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"regulation_not_found","message":"no regulation [BWBR0000001]","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let output = bron(&server, &["--json", "law", "about", "BWBR0000001"]).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(envelope["error"]["code"], "regulation_not_found");
}

/// The texts the act that established a regulation produced elsewhere.
const AMENDMENTS: &str = r#"{"bron":1,"grade":"authority",
 "data":{"regulation_id":"BWBR0046553","established_by":["stb-2022-146"],
  "articles":[
   {"regulation_id":"BWBR0002565","kind":"artikel","number":"183b","label":"Artikel 183b","stam_id":"13989213","versie_id":"28541682","in_force_from":"2022-07-01","effect":"wijziging","status":"goed","content_hash":"85c3"},
   {"regulation_id":"BWBR0046553","kind":"aanhef","stam_id":"16326653","versie_id":"28541862","in_force_from":"2022-07-01","effect":"nieuwe-regeling","status":"goed","content_hash":"ffb6"}]},
 "caveats":[{"kind":"held_text_only","held_regulations":46908,"known_regulations":46915}]}"#;

/// `law amendments` is one call, and answers what the act did elsewhere: one
/// row per text it produced, and the act itself on stderr, off the rows.
#[tokio::test(flavor = "multi_thread")]
async fn law_amendments_lists_what_the_establishing_act_produced() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0046553/amendments"))
        .and(query_param("limit", "5"))
        .and(query_param("offset", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(AMENDMENTS, "application/json"))
        // Asked twice: once for a consumer, once for a person.
        .expect(2)
        .mount(&server)
        .await;

    let json = bron(
        &server,
        &[
            "--json",
            "law",
            "amendments",
            "BWBR0046553",
            "--limit",
            "5",
            "--offset",
            "10",
        ],
    )
    .await;
    assert!(json.status.success(), "{json:?}");
    assert_eq!(stdout(&json), format!("{AMENDMENTS}\n"));

    let output = bron(
        &server,
        &[
            "law",
            "amendments",
            "BWBR0046553",
            "--limit",
            "5",
            "--offset",
            "10",
        ],
    )
    .await;
    let text = stdout(&output);
    let said = String::from_utf8_lossy(&output.stderr);

    assert!(
        text.lines()
            .any(|l| l == "BWBR0002565  Artikel 183b  wijziging        2022-07-01"),
        "{text}"
    );
    // A unit the register prints no number for is named by what it is.
    assert!(
        text.lines()
            .any(|l| l == "BWBR0046553  aanhef        nieuwe-regeling  2022-07-01"),
        "{text}"
    );
    assert!(said.contains("established by stb-2022-146"), "{said}");
    assert!(!text.contains("established by"), "{text}");
}

/// What the register states rests on a regulation, and what cites it.
const RELATIONS: &str = r#"{"bron":1,"grade":"authority","as_of":"2026-09-19","as_known_on":"2026-09-19",
 "data":{"regulation_id":"BWBR0005289","relations":[
  {"kind":"grondslag_voor","related":{"regulation_id":"BWBR0007122","title":"Uitvoeringsbesluit aansprakelijkheid gevaarlijke stoffen","jci":"jci1.3:c:BWBR0007122"},"in_force":{"from":"1995-02-01"},"as_known":{"from":"1995-02-01"}},
  {"kind":"grondslag_voor","article":"Artikel 175","related":{"regulation_id":"BWBR0007122","title":"Uitvoeringsbesluit aansprakelijkheid gevaarlijke stoffen","jci":"jci1.3:c:BWBR0007122"},"in_force":{"from":"1995-02-01"},"as_known":{"from":"1995-02-01"}}]}}"#;

/// `law relations` is one call, and its rows say which article a relation is
/// stated for, where the register states one, and which it points at.
#[tokio::test(flavor = "multi_thread")]
async fn law_relations_says_what_rests_on_a_law_and_from_which_article() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0005289/relations"))
        .and(query_param("kind", "grondslag_voor"))
        .and(query_param("article", "Artikel 175"))
        .and(query_param("as_of", "2019-06-01"))
        .and(query_param_is_missing("limit"))
        .and(query_param_is_missing("offset"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(RELATIONS, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "law",
            "relations",
            "BWBR0005289",
            "--kind",
            "grondslag_voor",
            "--article",
            "Artikel 175",
            "--as-of",
            "2019-06-01",
        ],
    )
    .await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    // Stated for the regulation whole, where the register names no article.
    assert!(
        text.lines().any(|l| l
            == "grondslag_voor               BWBR0007122  1995-02-01 onwards        Uitvoeringsbesluit aansprakelijkheid gevaarlijke stoffen"),
        "{text}"
    );
    assert!(
        text.lines().any(|l| l
            == "grondslag_voor  Artikel 175  BWBR0007122  1995-02-01 onwards        Uitvoeringsbesluit aansprakelijkheid gevaarlijke stoffen"),
        "{text}"
    );
}

/// A law nothing rests on is an answer, not a failure, and it says so rather
/// than printing nothing at all.
#[tokio::test(flavor = "multi_thread")]
async fn law_relations_says_in_words_that_the_register_states_none() {
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0005289/relations",
        r#"{"bron":1,"grade":"authority","data":{"regulation_id":"BWBR0005289","relations":[]}}"#,
    )
    .await;

    let output = bron(&server, &["law", "relations", "BWBR0005289"]).await;

    assert!(output.status.success(), "{output:?}");
    assert!(
        stdout(&output).contains("The register states no relation for that."),
        "{output:?}"
    );

    // Measured on production: the service matches an article by the label the
    // register prints, so a bare number answers nothing, which would read as
    // an article nothing rests on.
    let numbered = bron(
        &server,
        &["law", "relations", "BWBR0005289", "--article", "175"],
    )
    .await;
    assert!(
        stdout(&numbered).contains("--article 'Artikel 175'"),
        "{numbered:?}"
    );
}

/// Every name a route declares that a command had no way to send. A list cut
/// at its window is unreadable past it without `--offset`, a comparison cannot
/// be asked what the register showed, a question cannot be put to one law, and
/// a search cannot be narrowed or dated.
#[tokio::test(flavor = "multi_thread")]
async fn every_name_the_routes_declare_can_be_sent() {
    let server = MockServer::start().await;

    for (route, params) in [
        (
            "/api/v1/expressions",
            vec![("in_force_from", "2026-01-01"), ("offset", "50")],
        ),
        (
            "/api/v1/regulations",
            vec![
                ("q", "awb"),
                ("type", "wet"),
                ("as_of", "2019-06-01"),
                ("as_known_on", "2020-01-01"),
                ("offset", "20"),
            ],
        ),
        (
            "/api/v1/regulations/BWBR0005289/changes",
            vec![("from", "2019-01-01"), ("as_known_on", "2020-01-01")],
        ),
        (
            "/api/v1/regulations/BWBR0001941/parts",
            vec![("kind", "artikel"), ("number", "2"), ("paragraph", "1")],
        ),
    ] {
        let mut mock = Mock::given(method("GET")).and(path(route));
        for (name, value) in params {
            mock = mock.and(query_param(name, value));
        }
        mock.respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"route_not_found","message":"asked","detail":[]}}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;
    }

    Mock::given(method("POST"))
        .and(path("/api/v1/search"))
        .and(body_json(serde_json::json!({
            "q": "onrechtmatige daad",
            "regulation": "BWBR0005289,BWBR0005537",
            "as_of": "2026-09-19",
        })))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"citation_not_found","message":"asked","detail":[]}}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    // Each mock answers only where every name arrived, and its answer is the
    // not-found these assert on.
    for args in [
        &["law", "published", "--from", "2026-01-01", "--offset", "50"][..],
        &[
            "law",
            "search",
            "awb",
            "--type",
            "wet",
            "--as-of",
            "2019-06-01",
            "--as-known-on",
            "2020-01-01",
            "--offset",
            "20",
        ][..],
        &[
            "law",
            "changes",
            "BWBR0005289",
            "--from",
            "2019-01-01",
            "--as-known-on",
            "2020-01-01",
        ][..],
        &[
            "law",
            "parts",
            "BWBR0001941",
            "--kind",
            "artikel",
            "--number",
            "2",
            "--paragraph",
            "1",
        ][..],
        &[
            "find",
            "onrechtmatige daad",
            "--regulation",
            "BWBR0005289",
            "--regulation",
            "BWBR0005537",
            "--as-of",
            "2026-09-19",
        ][..],
    ] {
        let output = bron(&server, args).await;
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
}

/// Production's answer for artikel 3:305a BW lid 3 on 2026-09-24: narrowed
/// to one lid, with the `provenance_of_article` caveat.
const LID_WITH_ARTICLE_PROVENANCE: &str = "bron-articles-BWBR0005291-305a-lid-3.json";

/// The fields the register records per article are said to be the
/// article's, not the lid's, in words and not as a limit without a sentence.
#[tokio::test(flavor = "multi_thread")]
async fn provenance_of_article_says_the_fields_describe_the_article() {
    let sent = fixture(LID_WITH_ARTICLE_PROVENANCE);
    let server = answering("GET", "/api/v1/regulations/BWBR0005291/articles", &sent).await;

    let output = bron(
        &server,
        &[
            "law",
            "show",
            "BWBR0005291",
            "--article",
            "305a",
            "--paragraph",
            "3",
        ],
    )
    .await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(
        text.contains(
            "these fields describe article(s) 305a, not the lid: amended_by, dossiers, \
             commenced_by, in_force_from, effect, remarks. The register records provenance per \
             article."
        ),
        "{text}"
    );
    assert!(!text.contains("no sentence for"), "{text}");

    let json = bron(&server, &["--json", "law", "show", "BWBR0005291"]).await;
    assert_eq!(stdout(&json), format!("{sent}\n"));
}

/// Production's answer for artikel 7:271 BW as in force on 2024-08-01, asked
/// on 2026-09-25: the memorie van toelichting's passage on onderdeel B of the
/// Wet vaste huurcontracten, read through the bill, whose words the
/// Staatsblad prints otherwise after the amendementen.
const ARTICLE_WITH_PASSAGES: &str = "bron-articles-BWBR0005290-271-passages.json";

/// A passage bron read is printed as bron's reading, with the paper's
/// citation, the digest and span it rests on, the bill's formula it went
/// through and that it explains an earlier text, and the caveats in words.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_prints_the_passages_that_explain_an_article_as_bron_s_reading() {
    let sent = fixture(ARTICLE_WITH_PASSAGES);
    let server = answering("GET", "/api/v1/regulations/BWBR0005290/articles", &sent).await;

    let output = bron(&server, &["law", "show", "BWBR0005290", "--article", "271"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(
        text.contains(
            "explained in Kamerstukken II 2021/22, 36195, nr. 3 (bron's reading): \
             b0de7efd6ae84c8adcdf8e6d25ffb33f0749ff8b73acacdb50c8e3634d1324af (61395..63412)"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "  through the bill kst-36195-2: \
             564d0ed1e4adebd93315ff11ae63ed2fb151545dd1f881153d8479e0e4e25727 (2698..2749)"
        ),
        "{text}"
    );
    assert!(
        text.contains("  explains the bill's words, which stb-2023-480 prints otherwise"),
        "{text}"
    );
    assert!(
        text.contains("(reading 3), which no publisher states"),
        "{text}"
    );
    assert!(
        text.contains("explains the bill's words at a place the act prints otherwise"),
        "{text}"
    );
    assert!(!text.contains("no sentence for"), "{text}");

    let json = bron(
        &server,
        &["--json", "law", "show", "BWBR0005290", "--article", "271"],
    )
    .await;
    assert_eq!(stdout(&json), format!("{sent}\n"));
}

/// Production's answer for artikel 7:232 BW as in force on 2024-08-01, asked
/// on 2026-09-25: amendement nr. 20 and its replacement nr. 31 of the Wet
/// vaste huurcontracten, each read as changing the article and each found in
/// the act, the besluit taken on nr. 31.
const ARTICLE_WITH_AMENDMENTS: &str = "bron-articles-BWBR0005290-232-amendments.json";

/// An amendement's change bron read is printed as bron's reading, with the
/// paper's citation, the digest and range it rests on, and whether the act
/// prints its words.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_prints_the_amendementen_that_change_an_article_as_bron_s_reading() {
    let sent = fixture(ARTICLE_WITH_AMENDMENTS);
    let server = answering("GET", "/api/v1/regulations/BWBR0005290/articles", &sent).await;

    let output = bron(&server, &["law", "show", "BWBR0005290", "--article", "232"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(
        text.contains(
            "changed by amendement Kamerstukken II 2022/23, 36195, nr. 31 (bron's reading): \
             cf1191078f129bf06b8434edced2ed923ff4566500b320e3ea8cb0aa13c269ed (1867..2122)"
        ),
        "{text}"
    );
    assert!(
        text.contains("  the act prints every word it puts in"),
        "{text}"
    );
    assert!(!text.contains("besluit on"), "{text}");
    assert!(!text.contains("no sentence for"), "{text}");

    let json = bron(
        &server,
        &["--json", "law", "show", "BWBR0005290", "--article", "232"],
    )
    .await;
    assert_eq!(stdout(&json), format!("{sent}\n"));
}

/// Production's ranking for 'schadevergoeding bij onrechtmatige daad' at a
/// window of 3 on 2026-09-24, folded by hand as the service now folds it:
/// artikel 6:162 BW lid 1, which ranked as a hit of its own, stands under
/// its article in `leden`. The article and its lid are each given one other
/// place, in the rendition before, so the answer holds `also` on both.
const RANKING_WITH_LEDEN: &str = "bron-search-onrechtmatige-daad-limit-3-edited-leden.json";

/// A unit's other leden that ranked are rows under it, each with its own
/// distance and finder, and not hits of their own. They follow the unit's own
/// other places, and each lid's places follow it, so no `also` row reads as
/// another text's.
#[tokio::test(flavor = "multi_thread")]
async fn find_prints_the_leden_that_ranked_under_their_unit() {
    let sent = fixture(RANKING_WITH_LEDEN);
    let server = answering("POST", "/api/v1/search", &sent).await;

    let output = bron(
        &server,
        &["find", "schadevergoeding bij onrechtmatige daad"],
    )
    .await;
    let text = stdout(&output);
    let rows: Vec<&str> = text.lines().collect();

    assert!(output.status.success(), "{output:?}");
    let article = rows
        .iter()
        .position(|row| row.contains("article") && row.contains("BWBR0005289"))
        .expect("the article's row");
    let [place, lid, lid_place] = [1, 2, 3].map(|below| rows[article + below]);
    assert!(
        place.contains("also") && place.contains("article"),
        "{text}"
    );
    assert!(place.contains("2025-07-01_0"), "{text}");
    assert!(lid.contains("same unit") && lid.contains("lid 1"), "{text}");
    assert!(lid.contains("0.387") && lid.contains("vector"), "{text}");
    assert!(
        lid_place.contains("also") && lid_place.contains("lid 1"),
        "{text}"
    );
    assert!(lid_place.contains("2025-07-01_0"), "{text}");
    assert!(rows[article + 4].contains("BWBR0028749"), "{text}");

    let json = bron(&server, &["--json", "find", "schadevergoeding"]).await;
    assert_eq!(stdout(&json), format!("{sent}\n"));
}

/// Production's comparison of artikel 3:305a BW between 2019-06-01 and
/// 2020-06-01 on 2026-09-24, with `renumbered_from` added by hand to lid 7 of
/// the later side: production states it only where a lid moved.
const CHANGES_WITH_RENUMBERED_LID: &str =
    "bron-changes-BWBR0005291-305a-2019-06-01-to-2020-06-01-edited-renumbered.json";

/// A lid whose words stand under another number on the later side reads as
/// moved, in words.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_says_a_lid_was_renumbered_word_for_word() {
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0005291/changes",
        &fixture(CHANGES_WITH_RENUMBERED_LID),
    )
    .await;

    let output = bron(
        &server,
        &[
            "law",
            "changes",
            "BWBR0005291",
            "--from",
            "2019-06-01",
            "--to",
            "2020-06-01",
            "--article",
            "305a",
        ],
    )
    .await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.contains("lid 7 was lid 6, word for word"), "{text}");
    assert_eq!(text.matches("word for word").count(), 1, "{text}");
}

/// A bare question sends no page: the service's window is the default, so the
/// CLI and the MCP answer the same question alike. `law show` and `law
/// versions` are the exception, asking for the whole set with `limit=none`.
#[tokio::test(flavor = "multi_thread")]
async fn an_untyped_page_is_left_to_the_service() {
    let server = MockServer::start().await;
    for route in [
        "/api/v1/regulations",
        "/api/v1/regulations/BWBR0005289/parts",
        "/api/v1/regulations/BWBR0005289/amendments",
        "/api/v1/expressions",
        "/api/v1/feed",
        "/api/v1/publications/stb-2026-56",
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .and(query_param_is_missing("limit"))
            .and(query_param_is_missing("offset"))
            .respond_with(ResponseTemplate::new(404).set_body_raw(
                r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","message":"seen","detail":[]}}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;
    }

    for args in [
        &["law", "search", "awb"][..],
        &["law", "parts", "BWBR0005289"][..],
        &["law", "amendments", "BWBR0005289"][..],
        &["law", "published", "--from", "2026-01-01"][..],
        &["law", "feed", "--since", "0"][..],
        &["publication", "show", "stb-2026-56"][..],
    ] {
        // The mock answers only where no page was sent, and its answer is the
        // not-found this asserts on.
        let output = bron(&server, args).await;
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
}

/// A response the service really sent, frozen under `tests/fixtures/`.
fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);

    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The list production answered on 2026-09-23, edited by hand to add `stage`
/// and `linked_decrees` to every row: production predates the Staatsblad lane,
/// and no capture of it carries either.
const UPCOMING: &str = "bron-upcoming-limit-5-edited-for-stage.json";
/// The same edit, on the window production answered for 2027 Q1.
const UPCOMING_2027_Q1: &str =
    "bron-upcoming-from-2027-01-01-to-2027-03-31-limit-5-edited-for-stage.json";
/// [`UPCOMING`] with a second edit: BWBR0035474 states a day for 1 of its 13
/// waiting units rather than all 13.
const UPCOMING_PARTLY_DATED: &str = "bron-upcoming-limit-5-edited-dated-below-units.json";

#[tokio::test(flavor = "multi_thread")]
async fn law_upcoming_prints_the_envelope_the_service_sent() {
    let sent = fixture(UPCOMING_2027_Q1);
    let server = answering("GET", "/api/v1/upcoming", &sent).await;

    let output = bron(
        &server,
        &[
            "--json",
            "law",
            "upcoming",
            "--from",
            "2027-01-01",
            "--to",
            "2027-03-31",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{sent}\n"));
}

/// One row per regulation in the order the service sent them, and every note
/// on the rows, the legend and the limits the service stated, on stderr.
#[tokio::test(flavor = "multi_thread")]
async fn law_upcoming_prints_one_row_per_regulation_and_its_notes_apart() {
    let server = answering("GET", "/api/v1/upcoming", &fixture(UPCOMING)).await;

    let output = bron(&server, &["law", "upcoming", "--limit", "5"]).await;
    let err = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        stdout(&output),
        "2026-10-01  BWBR0035072  2026-07-01_0   1 waiting  Regeling genetisch gemodificeerde organismen milieubeheer 2013  acts stcrt-2026-18052  decrees stcrt-2026-18052  linked decrees -\n\
         2026-10-01  BWBR0035474  2026-09-03_0  13 waiting  Regeling nationale EZ-, LVVN- en KGG-subsidies  acts stcrt-2026-31575  decrees stcrt-2026-31575  linked decrees -\n\
         2026-10-01  BWBR0045685  2026-09-16_0  13 waiting  Regeling Europese EZ-, LVVN- en KGG-subsidies 2021  acts stcrt-2026-29272  decrees stcrt-2026-29272  linked decrees -\n\
         2026-10-01  BWBR0051172  2025-07-03_0   3 waiting  Besluit Aanmeldprocedure Staatsexamen Nt2 2027  acts stcrt-2025-22378  decrees -  linked decrees -\n\
         2026-10-01  BWBR0046685  2024-10-01_0   1 waiting  Wet kwaliteit incassodienstverlening  acts stb-2022-186  decrees -  linked decrees -\n"
    );
    assert!(
        err.contains("the rendition names as commencing it"),
        "{err}"
    );
    assert!(err.contains("note: filtered over a window of 5"), "{err}");
    assert!(err.contains("note: 268 treaty(ies)"), "{err}");
    assert!(err.contains("what the BWB prints"), "{err}");
    assert!(!err.contains("no sentence for"), "{err}");
}

/// Where not every waiting unit states a day, the row says how many do, so
/// the first day is not read as the day all of them apply. The column after
/// it starts in one place on every row.
#[tokio::test(flavor = "multi_thread")]
async fn a_row_says_how_many_of_its_waiting_units_state_a_day() {
    let server = answering("GET", "/api/v1/upcoming", &fixture(UPCOMING_PARTLY_DATED)).await;

    let output = bron(&server, &["law", "upcoming", "--limit", "5"]).await;
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        lines[1],
        "2026-10-01  BWBR0035474  2026-09-03_0  13 waiting, 1 dated  Regeling nationale EZ-, LVVN- en KGG-subsidies  acts stcrt-2026-31575  decrees stcrt-2026-31575  linked decrees -"
    );
    assert_eq!(
        lines[0],
        "2026-10-01  BWBR0035072  2026-07-01_0   1 waiting           Regeling genetisch gemodificeerde organismen milieubeheer 2013  acts stcrt-2026-18052  decrees stcrt-2026-18052  linked decrees -"
    );
    assert!(!lines[2].contains("dated"), "{text}");
}

/// A regulation whose waiting units state no day is listed as waiting with no
/// day, never with a date bron made up, and several acts are named in one
/// column.
#[tokio::test(flavor = "multi_thread")]
async fn a_regulation_whose_units_state_no_day_says_so() {
    const UNDATED: &str = r#"{"bron":1,"grade":"authority","as_of":"2026-09-23","as_known_on":"2026-09-23",
 "data":[{"stage":"consolidated","regulation_id":"BWBR0005537","regulation_title":"Algemene wet bestuursrecht","type":"wet","label":"2026-08-15_0","units":4,"dated":0,"acts":["stb-1994-1","stb-2021-2"],"decrees":[],"linked_decrees":[]}],
 "caveats":[{"kind":"not_in_bwb"}]}"#;

    let server = answering("GET", "/api/v1/upcoming", UNDATED).await;

    let output = bron(&server, &["law", "upcoming"]).await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        stdout(&output),
        "no day yet  BWBR0005537  2026-08-15_0  4 waiting  Algemene wet bestuursrecht  acts stb-1994-1, stb-2021-2  decrees -  linked decrees -\n"
    );
}

/// Nothing waiting is an answer, not a not-found, and with no rows to keep
/// clean the limits stand under the sentence that says so.
#[tokio::test(flavor = "multi_thread")]
async fn law_upcoming_with_nothing_waiting_is_an_answer() {
    const NOTHING: &str = r#"{"bron":1,"grade":"authority","as_of":"2026-09-23","as_known_on":"2026-09-23",
 "data":[],"caveats":[{"kind":"not_in_bwb"}]}"#;

    let server = answering("GET", "/api/v1/upcoming", NOTHING).await;

    let output = bron(
        &server,
        &["law", "upcoming", "--from", "2090-01-01", "--type", "wet"],
    )
    .await;
    let text = stdout(&output);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        text.contains("No regulation prints a unit waiting"),
        "{text}"
    );
    assert!(text.contains("what the BWB prints"), "{text}");
}

/// A row of stage `published` is an act the Staatsblad holds and the BWB
/// consolidates nowhere. It carries no regulation, so the row says plainly
/// that this is a publication and not a consolidated text, names it as cited,
/// and lists its dossiers and the besluiten bron holds that name it.
#[tokio::test(flavor = "multi_thread")]
async fn a_published_row_says_it_is_not_consolidated_and_names_its_dossiers() {
    const PUBLISHED: &str = "bron-upcoming-published-row.json";
    let server = answering("GET", "/api/v1/upcoming", &fixture(PUBLISHED)).await;

    let output = bron(&server, &["law", "upcoming"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        text,
        "no day yet  stb-2026-56  wet, published in the Staatsblad on 2026-03-13, not \
         consolidated in the BWB  Wet tot vaststelling van het nieuwe Wetboek van \
         Strafvordering  dossiers 34608  linked decrees -\n"
    );
}

/// The two stages sit in one list, in the order the service sent them, and
/// `--json` prints the envelope exactly as sent regardless of the mix.
#[tokio::test(flavor = "multi_thread")]
async fn an_answer_mixes_both_stages_and_json_carries_them_as_sent() {
    const MIXED: &str = "bron-upcoming-mixed-stages.json";
    let sent = fixture(MIXED);
    let server = answering("GET", "/api/v1/upcoming", &sent).await;

    let output = bron(&server, &["law", "upcoming"]).await;
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();
    let err = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{output:?}");
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].starts_with("2027-01-01  BWBR0000001"), "{lines:?}");
    assert!(lines[1].contains("stb-2026-56"), "{lines:?}");
    assert!(lines[1].contains("not consolidated"), "{lines:?}");
    assert!(lines[2].starts_with("no day yet  BWBR0009616"), "{lines:?}");
    // acts_left_out and founding_acts_unread, alongside the two stages the
    // rows themselves carry.
    assert!(err.contains("2 budget acts, 1 treaty approval"), "{err}");
    assert!(
        err.contains("2 regulation(s) have not had their founding act read"),
        "{err}"
    );

    let json = bron(&server, &["--json", "law", "upcoming"]).await;
    assert!(json.status.success(), "{json:?}");
    assert_eq!(stdout(&json), format!("{sent}\n"));
}

/// A stage this build has never heard of does not sink the rows beside it:
/// the answer still exits ok, the row it cannot read says so in words rather
/// than being silently dropped, and `--json` still carries it as sent.
#[tokio::test(flavor = "multi_thread")]
async fn a_stage_this_build_does_not_know_does_not_sink_the_answer() {
    const UNKNOWN: &str = "bron-upcoming-unknown-stage.json";
    let sent = fixture(UNKNOWN);
    let server = answering("GET", "/api/v1/upcoming", &sent).await;

    let output = bron(&server, &["law", "upcoming"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.contains("BWBR0009616"), "{text}");
    assert!(text.contains("stage deferred"), "{text}");
    assert!(text.contains("has no sentence for"), "{text}");
    assert!(
        text.contains("a stage this build has never heard of"),
        "{text}"
    );
    assert!(!text.to_lowercase().contains("upgrade"), "{text}");
    assert!(!text.contains("no day yet  stage"), "{text}");

    let json = bron(&server, &["--json", "law", "upcoming"]).await;
    assert!(json.status.success(), "{json:?}");
    assert_eq!(stdout(&json), format!("{sent}\n"));
}

/// `decrees_unmatched` counts every besluit bron could not link to an act and
/// names the recent ones, in words rather than as an unrecognised limit.
#[tokio::test(flavor = "multi_thread")]
async fn decrees_unmatched_names_the_besluiten_bron_could_not_link() {
    const UNMATCHED: &str = "bron-upcoming-decrees-unmatched.json";
    let server = answering("GET", "/api/v1/upcoming", &fixture(UNMATCHED)).await;

    let output = bron(&server, &["law", "upcoming"]).await;
    // With no rows to keep clean, the caveats stand on stdout, under the
    // sentence that says nothing waits.
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.contains("262 held besluit(en) name no act"), "{text}");
    assert!(text.contains("stb-2026-280 (2026-09-20)"), "{text}");
    assert!(text.contains("cannot say what they commence"), "{text}");
    assert!(!text.contains("no sentence for"), "{text}");
}

/// The caveat stands wherever a besluit is unlinked, also where none of them
/// is recent: a person reads the count and no empty list, and `--json`
/// carries the envelope as sent.
#[tokio::test(flavor = "multi_thread")]
async fn decrees_unmatched_with_no_recent_besluit_states_the_count() {
    let sent = fixture("bron-upcoming-decrees-unlinked-none-recent.json");
    let server = answering("GET", "/api/v1/upcoming", &sent).await;

    let output = bron(&server, &["law", "upcoming"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.contains("262 held besluit(en) name no act"), "{text}");
    assert!(!text.contains("last 15 days"), "{text}");
    assert!(!text.contains("no sentence for"), "{text}");

    let json = bron(&server, &["--json", "law", "upcoming"]).await;
    assert!(json.status.success(), "{json:?}");
    assert_eq!(stdout(&json), sent);
}

/// Answered by production on 2026-09-24 for artikel 6:2:15 Sv: printed
/// `nogniet`, with the register's remark that the change will not take
/// effect, left out of `not_commenced` and counted in
/// `units_withdrawn_by_remark`.
const WITHDRAWN_ARTICLE: &str = "bron-articles-BWBR0001903-6-2-15.json";

/// A unit whose remark says the change will not take effect reads as that,
/// with the register's words at their range, and its lead line never says it
/// does not apply yet. The note on it is the service's.
#[tokio::test(flavor = "multi_thread")]
async fn a_withdrawn_unit_reads_as_the_register_states_it() {
    let sent = fixture(WITHDRAWN_ARTICLE);
    let server = answering("GET", "/api/v1/regulations/BWBR0001903/articles", &sent).await;

    let output = bron(
        &server,
        &["law", "show", "BWBR0001903", "--article", "6:2:15"],
    )
    .await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(
        text.contains("the register states this change will not take effect"),
        "{text}"
    );
    assert!(
        text.contains(
            "register's remark (5320159..5320304): Deze wijziging treedt niet meer in werking. \
             De wijziging is ingetrokken door Stb. 2019/504."
        ),
        "{text}"
    );
    assert!(text.contains("published by stb-2017-82 (nieuw)"), "{text}");
    assert!(!text.contains("not yet applied"), "{text}");
    assert!(
        text.contains("1 unit(s) the rendition prints as not yet in force are left out"),
        "{text}"
    );
    assert!(!text.contains("no sentence for"), "{text}");

    let json = bron(&server, &["--json", "law", "show", "BWBR0001903"]).await;
    assert_eq!(stdout(&json), format!("{sent}\n"));
}

/// A caveat the service stated is printed as it stated it: bron names no
/// unit out of `not_commenced`, whatever the units beside it say.
#[tokio::test(flavor = "multi_thread")]
async fn not_commenced_is_printed_as_the_service_stated_it() {
    let mut sent: serde_json::Value = serde_json::from_str(&fixture(WITHDRAWN_ARTICLE)).unwrap();
    sent["caveats"] = serde_json::json!([{"kind": "not_commenced", "articles": ["6:2:15"]}]);
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0001903/articles",
        &sent.to_string(),
    )
    .await;

    let text = stdout(&bron(&server, &["law", "show", "BWBR0001903"]).await);

    assert!(
        text.contains("article(s) 6:2:15 have not commenced"),
        "{text}"
    );
}

/// Each side of a comparison carries its own remarks, at their range in its
/// own document, and the later side says the change will not take effect.
#[tokio::test(flavor = "multi_thread")]
async fn law_changes_prints_each_sides_remarks() {
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0001903/changes",
        &fixture("bron-changes-BWBR0001903-6-2-15-2025-12-31-to-2026-07-01.json"),
    )
    .await;

    let output = bron(
        &server,
        &[
            "law",
            "changes",
            "BWBR0001903",
            "--from",
            "2025-12-31",
            "--to",
            "2026-07-01",
            "--article",
            "6:2:15",
        ],
    )
    .await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(
        text.contains("           the register states this change will not take effect"),
        "{text}"
    );
    assert!(
        text.contains("           to register's remark (5320159..5320304): Deze wijziging"),
        "{text}"
    );
    assert!(!text.contains("from register's remark"), "{text}");
}

/// A history row whose text will not take effect says so in its state, and
/// the register's remark goes to stderr with the rows' other notes.
#[tokio::test(flavor = "multi_thread")]
async fn law_history_marks_a_withdrawn_text_and_notes_its_remark() {
    let server = answering(
        "GET",
        "/api/v1/regulations/BWBR0001903/articles/6:2:15/history",
        &fixture("bron-history-BWBR0001903-6-2-15-edited-remarks.json"),
    )
    .await;

    let output = bron(
        &server,
        &["law", "history", "BWBR0001903", "--article", "6:2:15"],
    )
    .await;
    let rows = stdout(&output);
    let notes = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{output:?}");
    assert!(rows.contains("withdrawn"), "{rows}");
    assert!(!rows.contains("register's remark"), "{rows}");
    assert!(
        notes.contains("Artikel 6:2:15 in 2026-01-01_0: register's remark (5320159..5320304)"),
        "{notes}"
    );
    assert!(
        notes.contains("the register states this change will not take effect"),
        "{notes}"
    );
}

/// Every caveat production stated on `/upcoming` on 2026-09-24 has a
/// sentence, the units a remark took out among them.
#[tokio::test(flavor = "multi_thread")]
async fn every_caveat_on_upcoming_has_a_sentence() {
    let sent = fixture("bron-upcoming-limit-1.json");
    let server = answering("GET", "/api/v1/upcoming", &sent).await;

    let output = bron(&server, &["law", "upcoming", "--limit", "1"]).await;
    let said = format!(
        "{}{}",
        stdout(&output),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(output.status.success(), "{output:?}");
    assert!(
        said.contains("9 unit(s) the rendition prints as not yet in force are left out"),
        "{said}"
    );
    assert!(
        said.contains("the acts the Staatsblad published that the BWB does not print"),
        "{said}"
    );
    assert!(!said.contains("no sentence for"), "{said}");

    let json = bron(&server, &["--json", "law", "upcoming", "--limit", "1"]).await;
    assert_eq!(stdout(&json), format!("{sent}\n"));
}

/// Every name the route declares reaches it, and nothing else: the service
/// refuses a name it does not declare. With no moment named neither
/// coordinate is sent, and a zicht date alone is sent alone: the service reads
/// it as that day for both. With no page named neither `limit` nor `offset`
/// is sent, so the service's window is the default.
#[tokio::test(flavor = "multi_thread")]
async fn law_upcoming_sends_every_name_its_route_declares_and_no_other() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/upcoming"))
        .and(query_param("as_of", "2026-06-01"))
        .and(query_param("as_known_on", "2026-07-01"))
        .and(query_param("type", "AMvB"))
        .and(query_param("from", "2027-01-01"))
        .and(query_param("to", "2027-03-31"))
        .and(query_param("limit", "7"))
        .and(query_param("offset", "14"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture(UPCOMING_2027_Q1), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/upcoming"))
        .and(query_param_is_missing("as_of"))
        .and(query_param("as_known_on", "2026-07-01"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture(UPCOMING), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/upcoming"))
        .and(query_param_is_missing("as_of"))
        .and(query_param_is_missing("as_known_on"))
        .and(query_param_is_missing("limit"))
        .and(query_param_is_missing("offset"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(fixture(UPCOMING), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    for args in [
        &[
            "law",
            "upcoming",
            "--as-of",
            "2026-06-01",
            "--as-known-on",
            "2026-07-01",
            "--type",
            "AMvB",
            "--from",
            "2027-01-01",
            "--to",
            "2027-03-31",
            "--limit",
            "7",
            "--offset",
            "14",
        ][..],
        &["law", "upcoming", "--as-known-on", "2026-07-01"][..],
        &["law", "upcoming"][..],
    ] {
        let output = bron(&server, args).await;
        assert!(output.status.success(), "{args:?}: {output:?}");
    }

    let declared = [
        "as_of",
        "as_known_on",
        "type",
        "from",
        "to",
        "limit",
        "offset",
    ];
    let asked = server.received_requests().await.unwrap();
    let names = |request: &wiremock::Request| {
        let mut names: Vec<String> = request
            .url
            .query_pairs()
            .map(|(name, _)| name.into_owned())
            .collect();
        names.sort();
        names
    };

    for request in &asked {
        for name in names(request) {
            assert!(declared.contains(&name.as_str()), "{name}: {}", request.url);
        }
    }
    assert_eq!(names(&asked[1]), ["as_known_on"], "{}", asked[1].url);
    assert!(names(&asked[2]).is_empty(), "{}", asked[2].url);
}

/// A failure the service states keeps its kind: a question it refuses exits
/// 1 with its code, and one it understood and could not answer exits 4.
#[tokio::test(flavor = "multi_thread")]
async fn law_upcoming_failures_keep_the_kind_the_service_gave_them() {
    for (status, body, exit, code) in [
        (
            400,
            r#"{"bron":1,"grade":"authority","error":{"kind":"usage","code":"invalid_parameter","message":"type must be a regulation type, e.g. wet or AMvB","detail":[]}}"#,
            1,
            "invalid_parameter",
        ),
        (
            500,
            r#"{"bron":1,"grade":"authority","error":{"kind":"corpus","code":"internal","message":"the service could not list what waits","detail":[]}}"#,
            4,
            "internal",
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/upcoming"))
            .respond_with(ResponseTemplate::new(status).set_body_raw(body, "application/json"))
            .mount(&server)
            .await;

        let output = bron(&server, &["--json", "law", "upcoming", "--type", "wet"]).await;
        let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        assert_eq!(envelope["error"]["code"], code, "{envelope}");
    }
}

/// What a request has to carry for the mock to answer it: a name with a
/// value, a name left out, or a field of a JSON body.
enum Carries {
    Param(&'static str, &'static str),
    Missing(&'static str),
    Body(serde_json::Value),
}

/// Runs `args` against a service that refuses the request as production does,
/// on a mock that answers only where the request carries what `carries`
/// names. The refusal is the service's: its kind, code and message reach
/// `--json`, its message reaches a person, and the exit is the one `usage`
/// maps to.
async fn refused_by_the_service(
    args: &[&str],
    verb: &str,
    route: &str,
    carries: Vec<Carries>,
    code: &str,
    message: &str,
) {
    let server = MockServer::start().await;
    let mut mock = Mock::given(method(verb)).and(path(route));
    for carried in carries {
        mock = match carried {
            Carries::Param(name, value) => mock.and(query_param(name, value)),
            Carries::Missing(name) => mock.and(query_param_is_missing(name)),
            Carries::Body(fields) => mock.and(body_partial_json(fields)),
        };
    }
    let body = serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "error": {"kind": "usage", "code": code, "message": message, "detail": []},
    });
    mock.respond_with(
        ResponseTemplate::new(400).set_body_raw(body.to_string(), "application/json"),
    )
    .expect(2)
    .mount(&server)
    .await;

    let json: Vec<&str> = ["--json"].iter().chain(args).copied().collect();
    let output = bron(&server, &json).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
    assert_eq!(envelope["error"]["kind"], "usage", "{envelope}");
    assert_eq!(envelope["error"]["code"], code, "{envelope}");
    assert_eq!(envelope["error"]["message"], message, "{envelope}");

    let human = bron(&server, args).await;
    let said = String::from_utf8_lossy(&human.stderr);
    assert_eq!(human.status.code(), Some(1), "{args:?}: {human:?}");
    assert!(said.contains(message), "{said}");
}

#[tokio::test(flavor = "multi_thread")]
async fn law_published_without_a_window_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "published", "--type", "wet"],
        "GET",
        "/api/v1/expressions",
        vec![
            Carries::Param("type", "wet"),
            Carries::Missing("in_force_from"),
            Carries::Missing("in_force_to"),
            Carries::Missing("visible_from"),
            Carries::Missing("visible_to"),
        ],
        "missing_parameter",
        "name a window: in_force_from, in_force_to, visible_from or visible_to, as YYYY-MM-DD",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_published_with_an_in_force_window_that_runs_backwards_is_the_services_refusal() {
    refused_by_the_service(
        &[
            "law",
            "published",
            "--from",
            "2026-09-01",
            "--to",
            "2026-01-01",
        ],
        "GET",
        "/api/v1/expressions",
        vec![
            Carries::Param("in_force_from", "2026-09-01"),
            Carries::Param("in_force_to", "2026-01-01"),
        ],
        "reversed_window",
        "in_force_from 2026-09-01 is after in_force_to 2026-01-01",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_published_with_a_zicht_window_that_runs_backwards_is_the_services_refusal() {
    refused_by_the_service(
        &[
            "law",
            "published",
            "--known-from",
            "2026-09-01",
            "--known-to",
            "2026-01-01",
        ],
        "GET",
        "/api/v1/expressions",
        vec![
            Carries::Param("visible_from", "2026-09-01"),
            Carries::Param("visible_to", "2026-01-01"),
        ],
        "reversed_window",
        "visible_from 2026-09-01 is after visible_to 2026-01-01",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_published_for_a_regulation_that_is_no_identifier_is_the_services_refusal() {
    refused_by_the_service(
        &[
            "law",
            "published",
            "--from",
            "2026-09-01",
            "--regulation",
            "Awb",
        ],
        "GET",
        "/api/v1/expressions",
        vec![Carries::Param("regulation", "Awb")],
        "invalid_regulation_id",
        "[Awb] is not a BWB identifier, e.g. BWBR0005289",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_feed_without_a_cursor_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "feed"],
        "GET",
        "/api/v1/feed",
        vec![Carries::Missing("since")],
        "missing_parameter",
        "since is required: the cursor to walk from, and 0 for the whole log. GET /api/v1 states the head.",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_feed_for_a_regulation_that_is_no_identifier_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "feed", "--since", "0", "--regulation", "Awb"],
        "GET",
        "/api/v1/feed",
        vec![
            Carries::Param("since", "0"),
            Carries::Param("regulation", "Awb"),
        ],
        "invalid_regulation_id",
        "[Awb] is not a BWB identifier, e.g. BWBR0005289",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_upcoming_with_a_window_that_runs_backwards_is_the_services_refusal() {
    refused_by_the_service(
        &[
            "law",
            "upcoming",
            "--from",
            "2027-02-01",
            "--to",
            "2027-01-01",
        ],
        "GET",
        "/api/v1/upcoming",
        vec![
            Carries::Param("from", "2027-02-01"),
            Carries::Param("to", "2027-01-01"),
        ],
        "reversed_window",
        "from 2027-02-01 is after to 2027-01-01",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_upcoming_with_a_limit_of_zero_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "upcoming", "--limit", "0"],
        "GET",
        "/api/v1/upcoming",
        vec![Carries::Param("limit", "0")],
        "invalid_parameter",
        "limit must be between 1 and 500",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_changes_with_days_that_run_backwards_is_the_services_refusal() {
    refused_by_the_service(
        &[
            "law",
            "changes",
            "BWBR0005289",
            "--from",
            "2020-01-01",
            "--to",
            "2019-01-01",
        ],
        "GET",
        "/api/v1/regulations/BWBR0005289/changes",
        vec![
            Carries::Param("from", "2020-01-01"),
            Carries::Param("to", "2019-01-01"),
        ],
        "reversed_window",
        "from 2020-01-01 is after to 2019-01-01",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_changes_without_an_earlier_day_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "changes", "BWBR0005289", "--to", "2020-01-01"],
        "GET",
        "/api/v1/regulations/BWBR0005289/changes",
        vec![Carries::Param("to", "2020-01-01"), Carries::Missing("from")],
        "missing_parameter",
        "from must be a date as YYYY-MM-DD",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_parts_with_a_number_and_no_kind_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "parts", "BWBR0001941", "--number", "Ia"],
        "GET",
        "/api/v1/regulations/BWBR0001941/parts",
        vec![Carries::Param("number", "Ia"), Carries::Missing("kind")],
        "invalid_parameter",
        "number and position name a unit of one kind, so either needs kind, e.g. kind=bijlage&number=2",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_parts_with_a_place_and_no_kind_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "parts", "BWBR0001941", "--position", "1"],
        "GET",
        "/api/v1/regulations/BWBR0001941/parts",
        vec![Carries::Param("position", "1"), Carries::Missing("kind")],
        "invalid_parameter",
        "number and position name a unit of one kind, so either needs kind, e.g. kind=bijlage&number=2",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_parts_with_a_number_and_a_place_is_the_services_refusal() {
    refused_by_the_service(
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
        ],
        "GET",
        "/api/v1/regulations/BWBR0001941/parts",
        vec![
            Carries::Param("number", "Ia"),
            Carries::Param("position", "1"),
        ],
        "invalid_parameter",
        "number and position are two ways to name one unit: send one",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_parts_with_a_place_of_zero_is_the_services_refusal() {
    refused_by_the_service(
        &[
            "law",
            "parts",
            "BWBR0001941",
            "--kind",
            "aanhef",
            "--position",
            "0",
        ],
        "GET",
        "/api/v1/regulations/BWBR0001941/parts",
        vec![Carries::Param("position", "0")],
        "invalid_parameter",
        "position must be a whole number from 1: the place of a unit among the units of its kind",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_parts_with_a_limit_of_zero_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "parts", "BWBR0001941", "--limit", "0"],
        "GET",
        "/api/v1/regulations/BWBR0001941/parts",
        vec![Carries::Param("limit", "0")],
        "invalid_parameter",
        "limit must be between 1 and 1000000, or none for the whole set",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_amendments_with_a_limit_of_zero_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "amendments", "BWBR0046553", "--limit", "0"],
        "GET",
        "/api/v1/regulations/BWBR0046553/amendments",
        vec![Carries::Param("limit", "0")],
        "invalid_parameter",
        "limit must be between 1 and 500",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_relations_with_a_limit_of_zero_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "relations", "BWBR0005289", "--limit", "0"],
        "GET",
        "/api/v1/regulations/BWBR0005289/relations",
        vec![Carries::Param("limit", "0")],
        "invalid_parameter",
        "limit must be between 1 and 500",
    )
    .await;
}

/// `law search` is a route this change moved off a local `u32`: `--limit 0`
/// reaches the service exactly as typed, and its refusal is the service's,
/// not clap's own exit 2.
#[tokio::test(flavor = "multi_thread")]
async fn law_search_with_a_limit_of_zero_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "search", "Awb", "--limit", "0"],
        "GET",
        "/api/v1/regulations",
        vec![Carries::Param("limit", "0")],
        "invalid_parameter",
        "limit must be between 1 and 100",
    )
    .await;
}

/// A date the service reads is the service's to judge: one that names no day
/// reaches it as typed and comes back as its own refusal, as a bad `--limit`
/// does. A zicht date alone travels alone.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_with_a_zicht_date_that_names_no_day_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "show", "BWBR0005289", "--as-known-on", "2026-02-30"],
        "GET",
        "/api/v1/regulations/BWBR0005289/articles",
        vec![
            Carries::Param("as_known_on", "2026-02-30"),
            Carries::Missing("as_of"),
        ],
        "invalid_parameter",
        "as_known_on must be a date as YYYY-MM-DD",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_show_with_a_date_that_names_no_day_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "show", "BWBR0005289", "--as-of", "2026-02-30"],
        "GET",
        "/api/v1/regulations/BWBR0005289/articles",
        vec![
            Carries::Param("as_of", "2026-02-30"),
            Carries::Missing("as_known_on"),
        ],
        "invalid_parameter",
        "as_of must be a date as YYYY-MM-DD",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_published_with_a_day_that_is_no_date_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "published", "--from", "gisteren"],
        "GET",
        "/api/v1/expressions",
        vec![Carries::Param("in_force_from", "gisteren")],
        "invalid_parameter",
        "in_force_from must be a date as YYYY-MM-DD",
    )
    .await;
}

/// A negative number is the value of the flag before it, so `--limit -1` as
/// two tokens reaches the service rather than reading as an unknown flag.
#[tokio::test(flavor = "multi_thread")]
async fn law_upcoming_with_a_negative_limit_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "upcoming", "--limit", "-1"],
        "GET",
        "/api/v1/upcoming",
        vec![Carries::Param("limit", "-1")],
        "invalid_parameter",
        "limit must be between 1 and 500",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_feed_with_a_negative_cursor_is_the_services_refusal() {
    refused_by_the_service(
        &["law", "feed", "--since", "-1"],
        "GET",
        "/api/v1/feed",
        vec![Carries::Param("since", "-1")],
        "invalid_parameter",
        "since must be a whole number from 0",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dossier_show_with_a_negative_limit_is_the_services_refusal() {
    refused_by_the_service(
        &[
            "dossier", "show", "34608", "--limit", "-1", "--offset", "-1",
        ],
        "GET",
        "/api/v1/dossiers/34608",
        vec![
            Carries::Param("limit", "-1"),
            Carries::Param("offset", "-1"),
        ],
        "invalid_parameter",
        "limit must be between 1 and 1000",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn law_parts_with_a_negative_place_is_the_services_refusal() {
    refused_by_the_service(
        &[
            "law",
            "parts",
            "BWBR0001941",
            "--kind",
            "aanhef",
            "--position",
            "-1",
        ],
        "GET",
        "/api/v1/regulations/BWBR0001941/parts",
        vec![Carries::Param("position", "-1")],
        "invalid_parameter",
        "position must be a whole number from 1: the place of a unit among the units of its kind",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn find_with_no_question_is_the_services_refusal() {
    refused_by_the_service(
        &["find"],
        "POST",
        "/api/v1/search",
        vec![Carries::Body(serde_json::json!({"q": ""}))],
        "missing_parameter",
        "q must be the text to search for",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn find_in_a_regulation_that_is_no_identifier_is_the_services_refusal() {
    refused_by_the_service(
        &["find", "daad", "--regulation", "Awb"],
        "POST",
        "/api/v1/search",
        vec![Carries::Body(
            serde_json::json!({"q": "daad", "regulation": "Awb"}),
        )],
        "invalid_regulation_id",
        "[Awb] is not a BWB identifier, e.g. BWBR0005289",
    )
    .await;
}

/// Every command that names a regulation in the path sends it as typed, and
/// the service says whether it is one.
#[tokio::test(flavor = "multi_thread")]
async fn a_regulation_that_is_no_identifier_is_the_services_refusal_on_every_route() {
    for (args, route) in [
        (&["law", "about", "Awb"][..], "/api/v1/regulations/Awb"),
        (
            &["law", "versions", "Awb"][..],
            "/api/v1/regulations/Awb/expressions",
        ),
        (
            &["law", "show", "Awb"][..],
            "/api/v1/regulations/Awb/articles",
        ),
        (
            &["law", "parts", "Awb"][..],
            "/api/v1/regulations/Awb/parts",
        ),
        (
            &["law", "history", "Awb", "--article", "1"][..],
            "/api/v1/regulations/Awb/articles/1/history",
        ),
        (
            &["law", "changes", "Awb", "--from", "2020-01-01"][..],
            "/api/v1/regulations/Awb/changes",
        ),
        (
            &["law", "amendments", "Awb"][..],
            "/api/v1/regulations/Awb/amendments",
        ),
        (
            &["law", "relations", "Awb"][..],
            "/api/v1/regulations/Awb/relations",
        ),
    ] {
        refused_by_the_service(
            args,
            "GET",
            route,
            Vec::new(),
            "invalid_regulation_id",
            "[Awb] is not a BWB identifier, e.g. BWBR0005289",
        )
        .await;
    }
}

/// The service reads a BWB identifier in any case, so the CLI sends one as
/// typed and gets the answer the API gives. Text that holds a `/` stays one
/// path segment and cannot reach another route.
#[tokio::test(flavor = "multi_thread")]
async fn a_regulation_goes_out_as_typed_and_as_one_segment() {
    let server = answering(
        "GET",
        "/api/v1/regulations/bwbr0005537",
        r#"{"bron":1,"grade":"authority","data":{"id":"BWBR0005537","title":"Algemene wet bestuursrecht","type":"wet","coverage":"history"}}"#,
    )
    .await;

    let output = bron(&server, &["law", "about", "bwbr0005537"]).await;
    assert!(output.status.success(), "{output:?}");

    let _ = bron(&server, &["law", "about", "../feed"]).await;
    let asked: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_string())
        .collect();
    assert_eq!(
        asked,
        [
            "/api/v1/regulations/bwbr0005537",
            "/api/v1/regulations/..%2Ffeed"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bron_blob_for_what_is_no_digest_is_the_services_refusal() {
    refused_by_the_service(
        &["blob", "abc"],
        "GET",
        "/api/v1/blobs/abc",
        Vec::new(),
        "invalid_digest",
        "[abc] is not a SHA-256 digest as 64 lowercase hex characters",
    )
    .await;
}

/// A failure the service stated reads in `--json` as it stated it: the
/// message as sent and the detail as sent, the one never folded into the
/// other. A person reads the detail on lines of its own.
#[tokio::test(flavor = "multi_thread")]
async fn a_stated_failure_carries_its_message_and_its_detail_as_sent() {
    let (output, envelope, said) = failing(
        404,
        None,
        r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"rendition_withdrawn","message":"no published rendition of BWBR0008691 in force on [2016-01-01]","detail":["2016-01-01_0","2016-06-16_0"]}}"#,
    )
    .await;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        envelope["error"],
        serde_json::json!({
            "kind": "not_found",
            "code": "rendition_withdrawn",
            "message": "no published rendition of BWBR0008691 in force on [2016-01-01]",
            "detail": ["2016-01-01_0", "2016-06-16_0"],
        })
    );
    assert!(
        said.contains(
            "bron: no published rendition of BWBR0008691 in force on [2016-01-01]\n  2016-01-01_0\n  2016-06-16_0\n"
        ),
        "{said}"
    );
}

/// A lid without an article is lid N of every article that prints one, as
/// the API and the MCP answer it, and a person reads each as any list of
/// articles.
#[tokio::test(flavor = "multi_thread")]
async fn law_show_asks_for_one_lid_of_every_article() {
    let two = ARTICLE.replace(
        r#"]}]},
 "caveats""#,
        r#"]},{"kind":"artikel","position":4,"number":"4","label":"Artikel 4","stam_id":"2760034","span":{"start":3000,"end":4000},"paragraphs":[{"number":"2","rendered":"Het tweede lid van artikel 4.","span":{"start":3100,"end":3200}}]}]},
 "caveats""#,
    );
    assert_ne!(two, ARTICLE);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/regulations/BWBR0005289/articles"))
        .and(query_param("paragraph", "2"))
        .and(query_param_is_missing("number"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(two, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(&server, &["law", "show", "BWBR0005289", "--paragraph", "2"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.lines().any(|l| l == "Artikel 3"), "{text}");
    assert!(text.lines().any(|l| l == "Artikel 4"), "{text}");
    assert!(text.contains("2. Het tweede lid van artikel 4."), "{text}");
}

/// A rendition that is held and read and prints no article is an answer:
/// `--json` is the empty list as sent, and a person reads in one line that it
/// prints none and where its other units are.
#[tokio::test(flavor = "multi_thread")]
async fn a_rendition_that_prints_no_articles_says_so_and_points_at_its_parts() {
    let none = serde_json::json!({
        "bron": 1,
        "grade": "authority",
        "as_of": "2026-09-23",
        "as_known_on": "2026-09-23",
        "data": {
            "regulation_id": "BWBR0045754",
            "expression": {
                "regulation_id": "BWBR0045754",
                "label": "2026-09-11_0",
                "in_force": {"from": "2026-09-11"},
                "withdrawn": false,
                "held": true,
                "upstream_hash_check": "matches",
                "blob": EARLIER_BLOB,
            },
            "articles": [],
        },
    })
    .to_string();
    let server = answering("GET", "/api/v1/regulations/BWBR0045754/articles", &none).await;

    let json = bron(&server, &["--json", "law", "show", "BWBR0045754"]).await;
    assert_eq!(json.status.code(), Some(0), "{json:?}");
    assert_eq!(stdout(&json), format!("{none}\n"));

    for (flags, line) in [
        (
            &[][..],
            "BWBR0045754 rendition 2026-09-11_0 prints no articles; `law parts BWBR0045754` answers the other units it prints.\n",
        ),
        (
            &["--not-commenced"][..],
            "BWBR0045754 rendition 2026-09-11_0 prints no article it does not yet apply; `law parts BWBR0045754 --not-commenced` answers its waiting units of every kind.\n",
        ),
        (
            &["--paragraph", "9"][..],
            "BWBR0045754 rendition 2026-09-11_0 prints no article with a lid 9; `law parts BWBR0045754` answers the other units it prints.\n",
        ),
    ] {
        let args: Vec<&str> = ["law", "show", "BWBR0045754"]
            .iter()
            .chain(flags)
            .copied()
            .collect();
        let output = bron(&server, &args).await;
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert_eq!(stdout(&output), line, "{args:?}");
    }
}

/// A segment an address cannot carry is the one thing bron refuses before
/// asking: the URL standard drops `.` and `..`, so the question would reach
/// another route, and no request is made. Encoded, the same characters are
/// text like any other and go out as typed.
#[tokio::test(flavor = "multi_thread")]
async fn a_regulation_an_address_cannot_carry_is_refused_before_asking() {
    let server = MockServer::start().await;

    for args in [
        &["--json", "law", "versions", ".."][..],
        &["--json", "law", "show", "."][..],
        &["--json", "law", "about", ""][..],
    ] {
        let output = bron(&server, args).await;
        let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert_eq!(envelope["error"]["kind"], "usage", "{args:?}");
        assert!(envelope["error"].get("code").is_none(), "{args:?}");
        assert!(
            envelope["error"]["message"]
                .as_str()
                .unwrap()
                .contains("cannot be sent as part of an address"),
            "{envelope}"
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());

    let _ = bron(&server, &["law", "versions", "%2E%2E"]).await;
    let asked: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_string())
        .collect();
    assert_eq!(asked, ["/api/v1/regulations/%252E%252E/expressions"]);
}

// -- officiële publicaties ------------------------------------------------

fn publication_fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);

    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `publication show` is one call to the service, so `--json` is what it
/// answered: `tests/fixtures/bron-publication-stb-2026-56.json` is a real
/// held answer, captured against production on 2026-09-23.
#[tokio::test(flavor = "multi_thread")]
async fn publication_show_prints_the_envelope_the_service_sent() {
    let held = publication_fixture("bron-publication-stb-2026-56.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/publications/stb-2026-56"))
        .and(query_param("limit", "2"))
        .and(query_param("rendered", "1"))
        .and(query_param("raw", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(held.clone(), "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "--json",
            "publication",
            "show",
            "stb-2026-56",
            "--limit",
            "2",
            "--rendered",
            "--raw",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{held}\n"));
}

/// A publication passed on rather than kept still answers its units, under
/// the caveat that says so: `tests/fixtures/bron-publication-trb-2020-1.json`
/// is a real answer against production on 2026-09-23.
#[tokio::test(flavor = "multi_thread")]
async fn publication_show_prints_a_passed_through_publication() {
    let passed = publication_fixture("bron-publication-trb-2020-1.json");
    let server = answering("GET", "/api/v1/publications/trb-2020-1", &passed).await;

    let output = bron(&server, &["publication", "show", "trb-2020-1"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.contains(NOT_HELD_MARKER), "{text}");
    assert!(text.contains("Article I"), "{text}");
    assert!(text.contains("passed on"), "{text}");
}

const NOT_HELD_MARKER: &str = "not held";

/// A publication id the service refuses, refused with its own code: bron
/// pre-validates nothing this route judges.
#[tokio::test(flavor = "multi_thread")]
async fn publication_show_of_an_id_the_service_refuses_exits_on_its_code() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/publications/xyz-2020-1"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"usage","code":"invalid_publication_id","message":"[xyz-2020-1] is no identifier of the officiële publicaties, which read as stb-2026-56 or kst-36327-3","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let output = bron(&server, &["--json", "publication", "show", "xyz-2020-1"]).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(envelope["error"]["code"], "invalid_publication_id");
}

/// A publication bron is known to keep out is refused the same way: usage,
/// exit 1, the service's own code in `--json`. Real body, read from
/// production on 2026-09-23 for `stb-1998-310`, an AMvB that names the
/// geslachtsnaam of children of the Royal House.
#[tokio::test(flavor = "multi_thread")]
async fn publication_show_outside_scope_exits_on_its_code() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/publications/stb-1998-310"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"usage","code":"publication_outside_scope","message":"stb-1998-310 is a AMvB of the Staatsblad, a kind bron neither holds nor passes on: decision F keeps out what names private persons","detail":["held: the Staatsblad from 2020-01-01 of type Wet, AMvB, Klein Koninklijk Besluit, Rijkswet, RijksAMvB, Verbeterblad","held: the Staatscourant of kind algemeen verbindend voorschrift (ministeriële regeling), beleidsregel, ander besluit van algemene strekking","held: a kamerstuk of a dossier the BWB names as an act's own","passed through: any other kamerstuk, a bijlage, the Tractatenblad, the Staatsblad before 2020-01-01"]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let output = bron(&server, &["--json", "publication", "show", "stb-1998-310"]).await;
    let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(envelope["error"]["code"], "publication_outside_scope");
}

// -- kamerstukdossiers ----------------------------------------------------

/// `dossier show` is one call to the service, and `--kind`, `--limit` and
/// `--offset` go as typed: `tests/fixtures/bron-dossier-34608.json` is a real
/// answer, captured against production on 2026-09-24.
#[tokio::test(flavor = "multi_thread")]
async fn dossier_show_prints_the_envelope_the_service_sent() {
    let answer = publication_fixture("bron-dossier-34608.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/dossiers/34608"))
        .and(query_param("kind", "amendement"))
        .and(query_param("limit", "5"))
        .and(query_param("offset", "x"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(answer.clone(), "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "--json",
            "dossier",
            "show",
            "34608",
            "--kind",
            "amendement",
            "--limit",
            "5",
            "--offset",
            "x",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{answer}\n"));
}

/// A paper per row on stdout, what the Tweede Kamer decided on it beside the
/// kind; the dossier's head and every note go to stderr. Nothing untyped is
/// sent.
#[tokio::test(flavor = "multi_thread")]
async fn dossier_show_prints_papers_as_rows_and_the_rest_on_stderr() {
    let answer = publication_fixture("bron-dossier-34608.json");
    let server = answering("GET", "/api/v1/dossiers/34608", &answer).await;

    let output = bron(&server, &["dossier", "show", "34608"]).await;
    let text = stdout(&output);
    let err = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{output:?}");
    assert!(
        text.starts_with("kst-34608-12  2018-12-21  amendement  no besluit stated\n  Amendement"),
        "{text}"
    );
    assert!(!text.contains("Kamerstukken 34608"), "{text}");
    assert!(err.starts_with("Kamerstukken 34608\n"), "{err}");
    assert!(
        err.contains("stb-2019-130  BWBR0001827 Wetboek van Burgerlijke Rechtsvordering, "),
        "{err}"
    );
    assert!(err.contains("1 amendement(en) or motie(s)"), "{err}");

    // Untyped, --limit and --offset are the service's to default.
    let asked = server.received_requests().await.unwrap();
    assert_eq!(asked[0].url.query(), None);
}

/// Every besluit the Tweede Kamer states stands on the paper's row, and a
/// paper of a kind nobody votes on carries none.
#[tokio::test(flavor = "multi_thread")]
async fn dossier_show_prints_every_stated_besluit_on_its_paper() {
    let server = answering(
        "GET",
        "/api/v1/dossiers/34608",
        r#"{"bron":1,"grade":"authority","data":{"dossier":"34608","acts":[],"papers":[{"id":"kst-34608-3","number":"3","kind":"memorie_van_toelichting","title":"Memorie van toelichting","published_on":"2016-11-17","held":true},{"id":"kst-34608-21","number":"21","kind":"amendement","published_on":"2019-01-30","held":false,"fates":[{"decision":"Stemmen - uitstellen","zaak":"2019Z01877","stated_in":"acd2"},{"decision":"Stemmen - verworpen","zaak":"2019Z01877","stated_in":"acd2"}]}]}}"#,
    )
    .await;

    let output = bron(&server, &["dossier", "show", "34608"]).await;

    assert_eq!(
        stdout(&output),
        "kst-34608-3  2016-11-17  memorie_van_toelichting\n  Memorie van toelichting\n\
         kst-34608-21  not held  2019-01-30  amendement  Stemmen - uitstellen; Stemmen - verworpen\n"
    );
}

/// A dossier of acts from before 1995 answers no paper, and says why.
#[tokio::test(flavor = "multi_thread")]
async fn dossier_show_of_papers_before_digital_says_so() {
    let server = answering(
        "GET",
        "/api/v1/dossiers/19529",
        r#"{"bron":1,"grade":"authority","data":{"dossier":"19529","acts":[{"act":"stb-1990-90","regulations":[{"regulation_id":"BWBR0005289"}]}],"papers":[]},"caveats":[{"kind":"papers_before_digital"}]}"#,
    )
    .await;

    let output = bron(&server, &["dossier", "show", "19529"]).await;
    let text = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(text.contains("No paper of dossier 19529"), "{text}");
    assert!(text.contains("Staten-Generaal Digitaal"), "{text}");
}

/// A dossier no held text's act names is not found, and a value that is no
/// dossier number is the service's to refuse: bron pre-validates neither.
#[tokio::test(flavor = "multi_thread")]
async fn dossier_show_exits_on_the_code_the_service_states() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/dossiers/99999"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"not_found","code":"dossier_not_named","message":"no act behind a held text names dossier [99999]","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/dossiers/34608-IX"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(
            r#"{"bron":1,"grade":"authority","error":{"kind":"usage","code":"invalid_dossier_number","message":"[34608-IX] is no kamerstukdossier number, which reads as 34608","detail":[]}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    for (dossier, exit, code) in [
        ("99999", 2, "dossier_not_named"),
        ("34608-IX", 1, "invalid_dossier_number"),
    ] {
        let output = bron(&server, &["--json", "dossier", "show", dossier]).await;
        let envelope: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        assert_eq!(envelope["error"]["code"], code);
    }
}

/// `papers_not_held` says how many of the papers the publisher lists the
/// answer leaves out while bron reads them. The fixture is production's
/// answer for 34608 on 2026-09-24 with the caveat added by hand, in the shape
/// `openapi.json` states: `papers` the count listed, `count` the count left
/// out. No dossier production answered that day carried it.
#[tokio::test(flavor = "multi_thread")]
async fn dossier_show_says_how_many_papers_are_not_held_yet() {
    let answer = publication_fixture("bron-dossier-34608-edited-papers-not-held.json");
    let server = answering("GET", "/api/v1/dossiers/34608", &answer).await;

    let output = bron(&server, &["dossier", "show", "34608"]).await;
    let said = format!(
        "{}{}",
        stdout(&output),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(output.status.success(), "{output:?}");
    assert!(
        said.contains(
            "3 of the 27 paper(s) the publisher lists for this dossier are not held and are \
             left out while they are read."
        ),
        "{said}"
    );
    assert!(!said.contains("no sentence for"), "{said}");

    let json = bron(&server, &["--json", "dossier", "show", "34608"]).await;
    assert_eq!(stdout(&json), format!("{answer}\n"));
}

/// Each paper prints the citation a lawyer writes, and a gewijzigd
/// amendement's predecessor says what replaced it. The fixture is
/// production's answer for 36195 on 2026-09-25, after citations and
/// replacement chains shipped.
#[tokio::test(flavor = "multi_thread")]
async fn dossier_show_prints_each_papers_citation_and_what_replaced_it() {
    let answer = publication_fixture("bron-dossier-36195-cited.json");
    let server = answering("GET", "/api/v1/dossiers/36195", &answer).await;

    let output = bron(&server, &["dossier", "show", "36195"]).await;
    let rows = stdout(&output);

    assert!(output.status.success(), "{output:?}");
    assert!(
        rows.contains("  Kamerstukken II 2022/23, 36195, nr. 16\n"),
        "{rows}"
    );
    assert!(
        rows.contains("  Kamerstukken I 2022/23, 36195, D\n"),
        "{rows}"
    );
    assert!(
        rows.lines()
            .any(|row| row.starts_with("kst-36195-8 ") && row.ends_with("replaced by kst-36195-16")),
        "{rows}"
    );

    let json = bron(&server, &["--json", "dossier", "show", "36195"]).await;
    assert_eq!(stdout(&json), format!("{answer}\n"));
}

/// `fates_not_read` says bron has not asked the Tweede Kamer about a
/// dossier's amendementen yet, and such a paper prints no besluit at all,
/// because 'no besluit stated' would say the Kamer answered. The fixture is
/// production's answer for 36195 on 2026-09-25, before `bron:fates` first ran.
#[tokio::test(flavor = "multi_thread")]
async fn dossier_show_prints_no_besluit_where_the_tweede_kamer_was_not_asked() {
    let answer = publication_fixture("bron-dossier-36195-fates-not-read.json");
    let server = answering("GET", "/api/v1/dossiers/36195", &answer).await;

    let output = bron(&server, &["dossier", "show", "36195"]).await;
    let rows = stdout(&output);
    let notes = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{output:?}");
    assert!(rows.contains("kst-36195-16"), "{rows}");
    assert!(!rows.contains("no besluit stated"), "{rows}");
    assert!(
        notes.contains(
            "bron has not yet asked the Tweede Kamer about 21 amendement(en) or motie(s) here"
        ),
        "{notes}"
    );
    assert!(!notes.contains("no sentence for"), "{notes}");

    let json = bron(&server, &["--json", "dossier", "show", "36195"]).await;
    assert_eq!(stdout(&json), format!("{answer}\n"));
}

/// `publications_left_out` says a page holds fewer hits than the publisher
/// ranked, and that `next` still walks on. The fixture is production's page
/// for `Tesla` at a window of 8 on 2026-09-24: six hits, two left out.
#[tokio::test(flavor = "multi_thread")]
async fn publication_search_says_how_many_hits_are_left_out() {
    let page = publication_fixture("bron-publication-search-tesla-limit-8.json");
    let server = answering("POST", "/api/v1/publications/search", &page).await;

    let output = bron(&server, &["publication", "search", "Tesla", "--limit", "8"]).await;
    let said = format!(
        "{}{}",
        stdout(&output),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(output.status.success(), "{output:?}");
    assert!(
        said.contains(
            "2 hit(s) on this page are of a kind bron does not open and are left out; the \
             publisher's next page may still hold more."
        ),
        "{said}"
    );
    assert!(!said.contains("no sentence for"), "{said}");

    let json = bron(&server, &["--json", "publication", "search", "Tesla"]).await;
    assert_eq!(stdout(&json), format!("{page}\n"));
}

/// `publication search` posts the body the routes declares and reads its
/// `next`: `tests/fixtures/bron-publication-search-dossier-36195.json` is a
/// real page against production on 2026-09-23.
#[tokio::test(flavor = "multi_thread")]
async fn publication_search_prints_the_envelope_and_walks_on_next() {
    let page = publication_fixture("bron-publication-search-dossier-36195.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/publications/search"))
        .and(body_json(
            serde_json::json!({"dossier": "36195", "limit": "2"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(page.clone(), "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/publications/search"))
        .and(body_json(
            serde_json::json!({"dossier": "36195", "limit": "2", "next": "3"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"bron":1,"grade":"authority","data":{"hits":[],"source_total":53}}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let output = bron(
        &server,
        &[
            "--json",
            "publication",
            "search",
            "--dossier",
            "36195",
            "--limit",
            "2",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), format!("{page}\n"));

    let output = bron(
        &server,
        &[
            "publication",
            "search",
            "--dossier",
            "36195",
            "--limit",
            "2",
            "--next",
            "3",
        ],
    )
    .await;

    assert!(output.status.success(), "{output:?}");
}

/// A collection is sent exactly as typed: `h`, the Handelingen, among them.
#[tokio::test(flavor = "multi_thread")]
async fn publication_search_sends_the_collection_as_typed() {
    for code in ["stb", "h"] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/publications/search"))
            .and(body_json(serde_json::json!({"collection": code})))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"bron":1,"grade":"authority","data":{"hits":[],"source_total":0}}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let output = bron(&server, &["publication", "search", "--collection", code]).await;

        assert!(output.status.success(), "{code}: {output:?}");
    }
}

/// Which collections exist is the service's to say: a word it does not read,
/// the name of a collection among them, reaches it as typed and comes back
/// as its own refusal, as the API and the MCP answer it.
#[tokio::test(flavor = "multi_thread")]
async fn publication_search_in_a_collection_the_service_does_not_read_is_its_refusal() {
    refused_by_the_service(
        &["publication", "search", "--collection", "kamerstuk"],
        "POST",
        "/api/v1/publications/search",
        vec![Carries::Body(
            serde_json::json!({"collection": "kamerstuk"}),
        )],
        "invalid_parameter",
        "collection is one of kst, stb, stcrt, trb, blg, h",
    )
    .await;
}
