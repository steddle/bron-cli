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
 "data":[{"label":"2016-07-01_0","in_force":{"from":"2016-07-01","until":"2018-12-31"},"size":49606,"withdrawn":false,"held":true,"verified":true,"blob":"35e2"}]}"#;

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
