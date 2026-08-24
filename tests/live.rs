//! The one thing no fixture can tell us.
//!
//! Everything else about this crate is provable offline: parsers run against
//! frozen bytes, and the transport runs against a mock that misbehaves on cue.
//! One claim is not. bron pins the CDX column set with `fl=` and asserts the
//! header row it gets back — and that contract is the archive's to change,
//! unilaterally, with no announcement and no local test that would notice.
//!
//! So this file holds exactly one test. It hits the real service.
//!
//!     cargo test --features live-tests --test live
//!
//! If it fails, the archive changed its answer and every stored capture field
//! is suspect until someone looks.
#![cfg(feature = "live-tests")]

use bron::sources::{Client, wayback};

#[test]
fn the_index_still_answers_the_columns_bron_pins() {
    let home = tempfile::tempdir().unwrap();
    let client = Client::with_buckets(home.path().join("buckets")).unwrap();

    // example.com is captured constantly and belongs to nobody, so asking for
    // it costs the archive nothing and tells us what we need.
    let query = wayback::IndexQuery {
        limit: Some(1),
        ..Default::default()
    };
    let found = wayback::captures(&client, "https://example.com", &query)
        .expect("the pinned columns no longer match what the index returns");

    let row = found
        .rows
        .first()
        .expect("the archive holds captures for example.com");

    // Every field bron stores, actually populated. A column that silently
    // started arriving empty would pass a header check and still be wrong.
    assert!(!row.id.urlkey().is_empty());
    assert!(!row.original.is_empty());
    assert!(row.digest.is_some(), "the index stopped supplying digests");
    assert!(row.mime_type.is_some());
}
