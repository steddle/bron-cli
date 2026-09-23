//! Reading a kamerstukdossier through the bron service.
//!
//! The service answers a dossier the act behind a text it holds names: the
//! acts that name it, and its papers oldest first, each with the kind its own
//! title states and, on an amendement or a motie, what the Tweede Kamer
//! decided. One call, read the way `publication show` is.

use chrono::NaiveDate;
use serde::Deserialize;

use crate::error::Outcome;
use crate::sources::Client;

use super::Service;
use super::client::Answered;

/// One kamerstukdossier, its acts and its papers.
#[derive(Debug, Deserialize)]
pub struct DossierPapers {
    /// The number as the register writes it, e.g. `34608`.
    pub dossier: String,
    /// The bill as its papers name it. Absent where no paper states one.
    pub title: Option<String>,
    pub acts: Vec<DossierAct>,
    pub papers: Vec<DossierPaper>,
}

/// An act whose held text names the dossier as its own.
#[derive(Debug, Deserialize)]
pub struct DossierAct {
    /// As `stb-2019-130`.
    pub act: String,
    /// The regulations whose held texts it produced or which it established.
    pub regulations: Vec<DossierRegulation>,
}

#[derive(Debug, Deserialize)]
pub struct DossierRegulation {
    pub regulation_id: String,
    pub title: Option<String>,
}

/// One kamerstuk of the dossier.
#[derive(Debug, Deserialize)]
pub struct DossierPaper {
    /// As `kst-34608-12`; `publication show` answers it.
    pub id: String,
    /// Its number in the dossier, `12`, or a letter for the Eerste Kamer.
    pub number: String,
    /// Its kind, as the service reads it off the paper's own title:
    /// `memorie_van_toelichting`, `amendement`, `motie`. Kept as the string
    /// sent, so a kind this build has never heard of crosses untouched.
    /// Absent where the title names none.
    pub kind: Option<String>,
    /// The paper's own words for itself.
    pub title: Option<String>,
    pub published_on: Option<NaiveDate>,
    /// False where the service read the paper from the publisher for this
    /// answer and put it on its queue.
    pub held: bool,
    /// On an amendement or a motie, every besluit the Tweede Kamer's latest
    /// answer states; empty where it states none. Absent on other kinds.
    pub fates: Option<Vec<Fate>>,
}

/// One besluit of the Tweede Kamer on a paper.
#[derive(Debug, Deserialize)]
pub struct Fate {
    /// As the Tweede Kamer words it: `Stemmen - aangenomen`.
    pub decision: String,
    /// The zaak it was taken on, as `2018Z24529`.
    pub zaak: String,
    /// SHA-256 of the frozen answer that states it.
    pub stated_in: String,
}

impl Service {
    /// One kamerstukdossier by its number. `kind`, `limit` and `offset` go to
    /// the service exactly as typed; the service says what it takes.
    pub fn dossier(
        &self,
        client: &Client,
        dossier: &str,
        kind: Option<&str>,
        limit: &str,
        offset: &str,
    ) -> Outcome<Answered<DossierPapers>> {
        let url = self.route(&["dossiers", dossier])?;

        let mut query = vec![("limit", limit.to_string()), ("offset", offset.to_string())];
        if let Some(kind) = kind {
            query.push(("kind", kind.to_string()));
        }

        self.read(client, client.get(&url).query(&query))
    }
}
