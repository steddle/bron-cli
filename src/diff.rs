//! Comparing two versions of a law.
//!
//! Articles are matched on `stam-id`, upstream's own identity for an article
//! across versions. Numbers repeat within a law and structural paths move when
//! a law is restructured; the stam-id does neither, so a match here is an
//! identity rather than a guess.
//!
//! Nothing in this module decides what a change means. Whether an amendment
//! reaches conduct from before it is a question of transitional law, and the
//! act that made the change is named so a reader can go and settle it.

use std::collections::HashMap;

use crate::sources::{FetchedArticle, FetchedLaw};

/// What happened to one article between two versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Unchanged,
    /// The text differs.
    Amended,
    /// Present in the later version only.
    Added,
    /// Marked `vervallen` in the later version.
    Repealed,
    /// Present in the earlier version only, and gone without being marked.
    ///
    /// Distinct from [`Change::Repealed`]: upstream said nothing, so all that
    /// can be reported is that it is no longer there.
    Withdrawn,
}

impl Change {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Amended => "amended",
            Self::Added => "added",
            Self::Repealed => "repealed",
            Self::Withdrawn => "withdrawn",
        }
    }

    pub fn is_change(self) -> bool {
        self != Self::Unchanged
    }
}

/// One article, as it stood on both sides of the comparison.
#[derive(Debug, Clone)]
pub struct ArticleChange {
    pub change: Change,
    pub number: String,
    pub label: Option<String>,
    pub path: Option<String>,
    /// The article as it was. Absent when it did not exist yet.
    pub before: Option<FetchedArticle>,
    /// The article as it is. Absent when it is gone.
    pub after: Option<FetchedArticle>,
}

impl ArticleChange {
    fn of(change: Change, before: Option<&FetchedArticle>, after: Option<&FetchedArticle>) -> Self {
        let shape = after.or(before).expect("a change has at least one side");
        Self {
            change,
            number: shape.number.clone(),
            label: shape.label.clone(),
            path: shape.path.clone(),
            before: before.cloned(),
            after: after.cloned(),
        }
    }
}

/// Compares two versions of one law, in document order of the later version.
///
/// Articles the later version does not carry are appended, so nothing that
/// existed on either side is left out of the report.
pub fn compare(before: &FetchedLaw, after: &FetchedLaw) -> Vec<ArticleChange> {
    let old: HashMap<&str, &FetchedArticle> = index(before);
    let new: HashMap<&str, &FetchedArticle> = index(after);

    let mut changes = Vec::new();

    for article in &after.articles {
        let Some(key) = article.stam_id.as_deref() else {
            continue;
        };
        let was = old.get(key).copied();

        let change = match was {
            None => Change::Added,
            Some(_) if article.effect.as_deref() == Some("vervallen") => Change::Repealed,
            Some(previous) if previous.content != article.content => Change::Amended,
            Some(_) => Change::Unchanged,
        };

        changes.push(ArticleChange::of(change, was, Some(article)));
    }

    for article in &before.articles {
        let Some(key) = article.stam_id.as_deref() else {
            continue;
        };
        if !new.contains_key(key) {
            changes.push(ArticleChange::of(Change::Withdrawn, Some(article), None));
        }
    }

    changes
}

fn index(law: &FetchedLaw) -> HashMap<&str, &FetchedArticle> {
    law.articles
        .iter()
        .filter_map(|a| Some((a.stam_id.as_deref()?, a)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::BlobId;
    use crate::model::{Span, Validity};
    use crate::sources::Provenance;

    fn article(stam: &str, number: &str, text: &str, effect: &str) -> FetchedArticle {
        FetchedArticle {
            stam_id: Some(stam.into()),
            number: number.into(),
            label: Some(format!("Artikel {number}")),
            title: None,
            path: Some(format!("/Boek6/Artikel{number}")),
            content: text.into(),
            span: Span {
                start: 0,
                end: text.len() as u32,
            },
            leden: Vec::new(),
            amended_by: None,
            effect: Some(effect.into()),
            in_force_from: None,
            tables: 0,
            jci: None,
            illustrations: 0,
            versie_id: Some(format!("v-{stam}")),
            status: Some("goed".into()),
        }
    }

    fn law(version: &str, articles: Vec<FetchedArticle>) -> FetchedLaw {
        FetchedLaw {
            id: "BWBR0005289".parse().unwrap(),
            title: "Burgerlijk Wetboek Boek 6".into(),
            law_type: Some("wet".into()),
            version: version.into(),
            in_force: Validity {
                from: "2020-01-01".parse().unwrap(),
                until: None,
            },
            visible: None,
            upstream_hash: None,
            provenance: Provenance {
                blob: BlobId::of(version.as_bytes()),
                url: "https://example.invalid/x.xml".into(),
                byte_len: 1,
            },
            articles,
            skipped_articles: 0,
        }
    }

    #[test]
    fn reports_every_kind_of_change() {
        let before = law(
            "2015",
            vec![
                article("1", "1", "Ongewijzigde tekst.", "nieuwe-regeling"),
                article("2", "2", "De oude tekst.", "nieuwe-regeling"),
                article("3", "3", "Verdwijnt zonder meer.", "nieuwe-regeling"),
                article("4", "4", "Wordt ingetrokken.", "nieuwe-regeling"),
            ],
        );
        let after = law(
            "2023",
            vec![
                article("1", "1", "Ongewijzigde tekst.", "nieuwe-regeling"),
                article("2", "2", "De nieuwe tekst.", "wijziging"),
                article("4", "4", "", "vervallen"),
                article("5", "5", "Geheel nieuw.", "nieuw"),
            ],
        );

        let changes = compare(&before, &after);
        let by_number = |n: &str| changes.iter().find(|c| c.number == n).unwrap().change;

        assert_eq!(by_number("1"), Change::Unchanged);
        assert_eq!(by_number("2"), Change::Amended);
        assert_eq!(by_number("4"), Change::Repealed);
        assert_eq!(by_number("5"), Change::Added);
        assert_eq!(by_number("3"), Change::Withdrawn);
    }

    #[test]
    fn an_amendment_carries_both_texts() {
        let before = law(
            "2015",
            vec![article("2", "2", "De oude tekst.", "nieuwe-regeling")],
        );
        let after = law(
            "2023",
            vec![article("2", "2", "De nieuwe tekst.", "wijziging")],
        );

        let changes = compare(&before, &after);
        let amended = &changes[0];

        assert_eq!(amended.before.as_ref().unwrap().content, "De oude tekst.");
        assert_eq!(amended.after.as_ref().unwrap().content, "De nieuwe tekst.");
    }

    #[test]
    fn matching_is_on_identity_not_on_number() {
        // The same article renumbered is one amendment, not a removal plus an
        // addition. Matching on the printed number would report it as both.
        let before = law(
            "2015",
            vec![article("7", "196b", "Tekst.", "nieuwe-regeling")],
        );
        let after = law("2023", vec![article("7", "196c", "Tekst.", "wijziging")]);

        let changes = compare(&before, &after);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change, Change::Unchanged);
        assert_eq!(changes[0].number, "196c");
    }

    #[test]
    fn an_article_without_an_identity_is_left_out_rather_than_guessed_at() {
        let mut orphan = article("x", "9", "Tekst.", "nieuw");
        orphan.stam_id = None;

        let changes = compare(&law("2015", vec![]), &law("2023", vec![orphan]));
        assert!(changes.is_empty());
    }
}
