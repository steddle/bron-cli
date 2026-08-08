//! The transport, against a server that misbehaves on purpose.
//!
//! Everything here is behaviour every lane depends on and none of them can
//! prove on their own: what happens when a host refuses, whether a body that
//! cannot be decoded still leaves a citable digest behind, and whether the
//! shared budget is written where another process will see it.
//!
//! These are the paths least exercised in ordinary use and most expensive to
//! get wrong, because each fails by looking like something else. A refusal that
//! is not honoured looks like a slow host; a blob written after decoding rather
//! than before looks fine until the first body that is not UTF-8.
//!
//! [`bron::sources::Client`] is deliberately blocking, so every call into it
//! goes through [`off_thread`]: driving a blocking client from inside the
//! runtime that is serving the mock would park the server answering it.

use std::path::PathBuf;
use std::time::Duration;

use bron::blob::{BlobId, BlobStore};
use bron::sources::Client;
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

    /// The bucket file's two lines: when the last request returned, and until
    /// when every caller must hold off.
    fn bucket(&self, host: &str) -> (u64, u64) {
        let key: String = host
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let text = std::fs::read_to_string(self.buckets.join(format!("{key}.bucket")))
            .expect("a bucket file");
        let mut lines = text.lines();

        (
            lines.next().unwrap().trim().parse().unwrap(),
            lines.next().unwrap().trim().parse().unwrap(),
        )
    }

    /// How far ahead of now the shared hold-off deadline sits.
    fn hold_off_ms(&self, host: &str) -> u64 {
        self.bucket(host).1.saturating_sub(now_millis())
    }
}

/// Runs one blocking request against a fresh client and reports what happened.
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

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

async fn refusing(status: u16, retry_after: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/refused"))
        .respond_with(ResponseTemplate::new(status).insert_header("Retry-After", retry_after))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_writes_a_backoff_deadline_every_process_can_see() {
    let server = refusing(429, "7").await;
    let home = Home::new();
    let url = format!("{}/refused", server.uri());

    let error = off_thread(&home, move |client| {
        client.fetch_url(&url).err().map(|e| e.to_string())
    })
    .await
    .expect("a 429 must not read as success");

    assert!(error.contains("refused the request"), "{error}");

    // The point of the file: a 429 answered to this process holds every other
    // process off too, instead of letting them walk into the same wall.
    let ahead = home.hold_off_ms(&server.address().ip().to_string());
    assert!(
        (5_000..=7_000).contains(&ahead),
        "expected the header's 7s, got {ahead}ms"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_after_is_read_from_the_header_and_not_from_the_body() {
    let server = MockServer::start().await;
    // The body says one second and the header says thirty. An implementation
    // that reads the body honours neither the host nor its own default.
    Mock::given(method("GET"))
        .and(path("/refused"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("Retry-After", "30")
                .set_body_string(r#"{"retry_after": 1, "message": "slow down"}"#),
        )
        .mount(&server)
        .await;

    let home = Home::new();
    let url = format!("{}/refused", server.uri());
    off_thread(&home, move |client| client.fetch_url(&url).is_ok()).await;

    let ahead = home.hold_off_ms(&server.address().ip().to_string());
    assert!(ahead > 25_000, "the header asked for 30s; got {ahead}ms");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_uncapped_retry_after_cannot_hold_a_run_hostage() {
    let server = refusing(429, "86400").await;
    let home = Home::new();
    let url = format!("{}/refused", server.uri());

    off_thread(&home, move |client| client.fetch_url(&url).is_ok()).await;

    // A day is what the host asked for. One url must not be able to stop a
    // whole run, so the ceiling wins.
    let ahead = home.hold_off_ms(&server.address().ip().to_string());
    assert!(ahead <= 60_000, "expected the 60s cap, got {ahead}ms");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_without_a_header_still_holds_callers_off() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/refused"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;

    let home = Home::new();
    let url = format!("{}/refused", server.uri());
    off_thread(&home, move |client| client.fetch_url(&url).is_ok()).await;

    // No Retry-After is not permission to carry straight on.
    let ahead = home.hold_off_ms(&server.address().ip().to_string());
    assert!(
        ahead > 0,
        "a refusal with no header must still slow every caller"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pacing_clock_is_stamped_when_a_request_returns() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(400)))
        .mount(&server)
        .await;

    let home = Home::new();
    let url = format!("{}/slow", server.uri());
    let before = now_millis();

    off_thread(&home, move |client| {
        client.fetch_url(&url).map(|_| ()).unwrap()
    })
    .await;

    // Stamped on return, not on dispatch: a call that spent its time inside
    // the request must not hand the next caller a free pass.
    let (last_returned, _) = home.bucket(&server.address().ip().to_string());
    assert!(
        last_returned >= before + 350,
        "stamped at dispatch: {last_returned} vs {before}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_body_that_is_not_utf8_still_leaves_a_digest_behind() {
    // 0xFF is not a valid UTF-8 byte anywhere.
    let bytes = vec![0x3C, 0x68, 0x74, 0x6D, 0x6C, 0x3E, 0xFF, 0xFE];

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/latin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
        .mount(&server)
        .await;

    let home = Home::new();
    let url = format!("{}/latin", server.uri());

    let (decoded_failed, raw) = off_thread(&home, {
        let url = url.clone();
        move |client| {
            // The decoding path refuses it, which is right for the XML lanes.
            let decoded = client.fetch(client.get(&url)).is_err();
            // The byte path takes it.
            let raw = client.fetch_bytes(client.get(&url)).unwrap();
            (decoded, (raw.bytes, raw.blob))
        }
    })
    .await;

    assert!(decoded_failed, "a non-UTF-8 body must not decode silently");
    assert_eq!(raw.0, bytes);
    assert_eq!(
        raw.1,
        BlobId::of(&bytes),
        "the digest is of exactly what arrived"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_blob_is_frozen_before_anything_tries_to_read_it() {
    let bytes = vec![0x00, 0xFF, 0x01, 0xFE];

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/binary"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
        .mount(&server)
        .await;

    let home = Home::new();
    let url = format!("{}/binary", server.uri());

    // Fails on the decode, after the bytes have already been written.
    off_thread(&home, move |client| client.fetch(client.get(&url)).is_ok()).await;

    let held = BlobStore::new(&home.blobs)
        .get(&BlobId::of(&bytes))
        .unwrap();

    // The whole premise: a response no reading can turn into text is still
    // citable, because the bytes were frozen before anyone tried to read them.
    assert_eq!(held, bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_error_status_names_the_url_that_produced_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gone"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let home = Home::new();
    let url = format!("{}/gone", server.uri());

    let chain = off_thread(&home, {
        let url = url.clone();
        move |client| client.fetch_url(&url).err().map(|e| format!("{e:#}"))
    })
    .await
    .expect("a 404 must not read as success");

    assert!(
        chain.contains(&url),
        "the failing address must be in the chain: {chain}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn provenance_records_the_address_actually_dialled() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let home = Home::new();
    let url = format!("{}/search", server.uri());

    let recorded = off_thread(&home, move |client| {
        client
            .fetch(client.get(&url).query(&[("q", "burgerlijk")]))
            .unwrap()
            .url
    })
    .await;

    // Taken from the built request rather than from the caller, so the query
    // string travels with the provenance instead of being lost.
    assert!(recorded.contains("q=burgerlijk"), "{recorded}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replay_fetch_stops_at_a_redirect_instead_of_taking_it() {
    // The archive replays a captured 301 as a real 301. Following it fetches a
    // different capture, whose bytes then cannot match the digest recorded for
    // the one that was asked for — so a sound capture fails verification.
    // Measured at 30% of a 400-address sweep before the policy was split.
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/capture"))
        .respond_with(ResponseTemplate::new(301).insert_header("Location", "/elsewhere"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/elsewhere"))
        .respond_with(ResponseTemplate::new(200).set_body_string("a different capture"))
        .mount(&server)
        .await;

    let home = Home::new();
    let url = format!("{}/capture", server.uri());

    let followed = off_thread(&home, {
        let url = url.clone();
        move |client| client.fetch_bytes(client.get(&url)).map(|r| r.bytes).ok()
    })
    .await;

    // The ordinary fetch takes the redirect, which is right for every other lane.
    assert_eq!(followed.as_deref(), Some(&b"a different capture"[..]));

    let stopped = off_thread(&home, move |client| {
        client
            .fetch_bytes_unredirected(client.get(&url))
            .map(|r| r.bytes)
            .ok()
    })
    .await
    .expect("a captured redirect is a capture, not a failure");

    // The redirect's own payload, which for a captured 301 is empty. That is
    // exactly what the index recorded a digest of, so verification passes
    // instead of comparing against a page from somewhere else entirely.
    assert!(
        stopped.is_empty(),
        "got {} bytes of someone else's capture",
        stopped.len()
    );
    assert_eq!(
        bron::sources::wayback::digest_of(&stopped),
        "3I42H3S6NNFQ2MSVX7XZKYAYSCX5QBYJ",
        "the empty-payload digest the index carries for a captured redirect"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_that_refuses_the_connection_still_slows_every_caller() {
    // The archive's other way of saying no. It does not answer 429; it drops
    // the connection, and a caller that only recognises 429 reads that as the
    // service being down and keeps dialling. Measured: 171 consecutive
    // requests refused this way in one run, none of them pausing anything.
    // Port 1 is privileged and nothing serves it, so the connection is
    // refused rather than answered.
    let home = Home::new();
    let host = "127.0.0.1".to_string();
    let url = "http://127.0.0.1:1/gone".to_string();

    let error = off_thread(&home, move |client| {
        client.fetch_url(&url).err().map(|e| format!("{e:#}"))
    })
    .await
    .expect("an unreachable host is a failure");

    assert!(error.contains("holding off"), "{error}");

    let ahead = home.hold_off_ms(&host);
    assert!(
        ahead > 25_000,
        "a refused connection must pause the run, got {ahead}ms"
    );
}
