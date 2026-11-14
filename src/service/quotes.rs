//! Checking quotations against the texts they cite, through the service.
//!
//! The service reads each quotation again in the unit it cites, through the
//! routes `law show` and `case show` read, and answers a verdict per
//! quotation. bron sends the quotations as typed and prints the verdicts.

use serde::Deserialize;

use crate::error::Outcome;
use crate::transport::Client;

use super::Service;
use super::client::Answered;
use super::model::Span;

/// One verdict per quotation, in the order asked.
#[derive(Debug, Deserialize)]
pub struct QuoteVerdicts {
    pub quotes: Vec<QuoteVerdict>,
}

/// What the service found of one quotation. `verdict` is read as a string, so
/// a verdict this build has no name for is printed by name rather than
/// refused.
#[derive(Debug, Deserialize)]
pub struct QuoteVerdict {
    /// `quote_found`, `quote_mismatch` or `citation_unresolved`.
    pub verdict: String,
    /// The quotation as it was asked, every name it was asked with.
    pub cited: serde_json::Value,
    pub rendition: Option<String>,
    pub blob: Option<String>,
    pub as_of: Option<String>,
    pub as_known_on: Option<String>,
    /// The unit that holds the quotation, where it was found.
    pub unit: Option<QuotedUnit>,
    /// Another unit of the article or the ruling that holds it, where the one
    /// cited does not.
    pub found_elsewhere: Option<QuotedUnit>,
    /// Where the quotation departs from the unit it cites.
    pub divergence: Option<Divergence>,
    /// The failure the route answered, where the citation did not resolve.
    pub error: Option<CitationFailure>,
    #[serde(default)]
    pub caveats: Vec<serde_json::Value>,
}

/// An article, a lid, or a paragraph of a ruling.
#[derive(Debug, Deserialize)]
pub struct QuotedUnit {
    /// Of an article or a lid, as the register prints it.
    pub label: Option<String>,
    /// Of a paragraph of a ruling.
    pub ordinal: Option<u32>,
    pub span: Option<Span>,
}

#[derive(Debug, Deserialize)]
pub struct Divergence {
    pub unit: QuotedUnit,
    pub differences: Vec<Difference>,
}

/// The unit's words where the quotation has other words; either is empty
/// where the other has words the one lacks.
#[derive(Debug, Deserialize)]
pub struct Difference {
    pub text: String,
    pub quoted: String,
}

#[derive(Debug, Deserialize)]
pub struct CitationFailure {
    pub message: String,
}

impl Service {
    /// Each quotation read again in the text it cites. `quotes` is sent as
    /// typed: the service judges how many, and what each names.
    pub fn quotes(
        &self,
        client: &Client,
        quotes: &serde_json::Value,
    ) -> Outcome<Answered<QuoteVerdicts>> {
        let url = self.route(&["quotes"])?;

        self.read(
            client,
            client
                .post(&url)
                .json(&serde_json::json!({ "quotes": quotes })),
        )
    }
}
