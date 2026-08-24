//! The parsers, against bytes that actually came off the wire.
//!
//! The unit tests in each module use hand-written XML, which proves the parser
//! handles the format as the author imagined it. This uses a document the
//! publisher really served, which is a different and better question.

use chrono::NaiveDate;

use bron::blob::BlobId;
use bron::ids::Ecli;
use bron::model::Direction;
use bron::sources::{Source, rechtspraak};

const URL: &str = "https://data.rechtspraak.nl/uitspraken/content?id=ECLI:NL:HR:2019:1278";

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);

    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn date(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

#[test]
fn parses_the_real_decision_with_its_relations() {
    let body = fixture("rechtspraak-content-ECLI-NL-HR-2019-1278.xml");
    let ecli: Ecli = "ECLI:NL:HR:2019:1278".parse().unwrap();
    let case =
        rechtspraak::parse(Source::new(BlobId::of(body.as_bytes()), &body, URL), &ecli).unwrap();

    assert_eq!(case.court.as_deref(), Some("Hoge Raad"));
    assert_eq!(case.decided_on, Some(date("2019-07-19")));
    assert!(case.full_text.is_some_and(|t| t.len() > 1_000));

    // One relation, pointing backwards at the conclusie. Which direction a
    // relation runs is the whole of what `case appeal` answers from.
    assert_eq!(case.relations.len(), 1);
    assert_eq!(case.relations[0].direction, Direction::Earlier);
    assert_eq!(case.relations[0].kind.as_deref(), Some("conclusie"));
    assert_eq!(case.relations[0].target.as_str(), "ECLI:NL:PHR:2019:496");
}
