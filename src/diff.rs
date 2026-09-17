//! Comparing two renditions of a law.
//!
//! Articles are matched on `stam-id`, upstream's own identity for an article
//! across renditions. Numbers repeat within a law and structural paths move
//! when a law is restructured; the stam-id does neither, so a match here is an
//! identity rather than a guess.
//!
//! [`Article`] is this module's own shape rather than the one the service
//! sends. The rule that decides what changed must not move when the wire does,
//! and `content` is what bron prints rather than a field that happens to sit on
//! a transport type.
//!
//! Nothing that cannot be matched is quietly dropped. An article with no
//! identity, and a stam-id the register used twice inside one rendition, are
//! both counted and reported, because a comparison that leaves them out of its
//! own totals answers 'nothing changed' where the answer is 'this could not be
//! compared'.
//!
//! Nothing in this module decides what a change means. Whether an amendment
//! reaches conduct from before it is a question of transitional law, and the
//! act that made the change is named so a reader can go and settle it.

use std::collections::{HashMap, HashSet};

/// One article, in the terms a comparison is made in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Article {
    /// Upstream's identity for this article across renditions.
    pub stam_id: Option<String>,
    pub number: String,
    pub label: Option<String>,
    pub path: Option<String>,
    /// What bron prints for this article.
    pub content: String,
    /// Upstream's vocabulary, unmapped: `nieuwe-regeling`, `wijziging`,
    /// `nieuw`, `vervallen`.
    pub effect: Option<String>,
}

/// What happened to one article between two renditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Unchanged,
    /// The text differs.
    Amended,
    /// Present in the later rendition only.
    Added,
    /// Marked `vervallen` in the later rendition and not in the earlier one.
    ///
    /// The register keeps a repealed article in every rendition after the one
    /// that repealed it, so the earlier side has to be read too. Without it a
    /// repeal from 1989 is reported as a change in every window since.
    Repealed,
    /// Present in the earlier rendition only, and gone without being marked.
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
///
/// Each side is the position of the article in the slice it came from, so the
/// caller keeps whatever it built these from: a span, a raw slice, the act that
/// produced the text. Copying those through here would make this module carry a
/// shape it does not read.
#[derive(Debug, Clone)]
pub struct ArticleChange {
    pub change: Change,
    pub number: String,
    pub label: Option<String>,
    pub path: Option<String>,
    /// Where it sits in the earlier rendition. Absent when it did not exist yet.
    pub before: Option<usize>,
    /// Where it sits in the later one. Absent when it is gone.
    pub after: Option<usize>,
}

/// Articles carrying no `stam-id`, counted once per rendition they stand in.
///
/// Not one total across the two, because nothing identifies them: whether the
/// one in the earlier rendition is the one in the later is exactly what a
/// missing stam-id refuses to say, and matching them on a number or a path
/// would be the guess this module exists to avoid. So the count is of sides,
/// and it says so.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Unidentified {
    pub before: u32,
    pub after: u32,
}

impl Unidentified {
    pub fn any(self) -> bool {
        self.before > 0 || self.after > 0
    }
}

/// A comparison, and what it could not place.
#[derive(Debug, Default)]
pub struct Comparison {
    pub changes: Vec<ArticleChange>,
    pub unidentified: Unidentified,
    /// stam-ids the register used more than once inside one rendition. Every
    /// article under one of them is left out, on both sides.
    pub repeated: Vec<String>,
}

impl Comparison {
    /// Whether anything was left out of the comparison.
    pub fn is_whole(&self) -> bool {
        !self.unidentified.any() && self.repeated.is_empty()
    }
}

/// Compares two renditions of one law, in document order of the later one.
///
/// Articles the later rendition does not carry are appended, so nothing that
/// existed on either side and could be identified is left out of the report.
pub fn compare(before: &[Article], after: &[Article]) -> Comparison {
    let (old, old_twice) = index(before);
    let (new, new_twice) = index(after);

    let ambiguous: HashSet<&str> = old_twice.union(&new_twice).copied().collect();
    let mut repeated: Vec<String> = ambiguous.iter().map(|id| (*id).to_string()).collect();
    repeated.sort();

    let unidentified = Unidentified {
        before: nameless(before),
        after: nameless(after),
    };

    let mut changes = Vec::new();

    for (index, article) in after.iter().enumerate() {
        let Some(key) = identity(article, &ambiguous) else {
            continue;
        };
        let was = old.get(key).copied();

        let change = match was.map(|i| &before[i]) {
            None => Change::Added,

            // Repealed in this window, not repealed at some point before it.
            // The register keeps a repealed article in every later rendition,
            // still carrying `vervallen`, so testing the new side alone counts
            // a standing repeal as a fresh one every time. BW2 between January
            // and July 2026 reported 69, among them article 31 through
            // stb-1989-541: an attribution that was right about the act and
            // wrong about the window.
            Some(previous)
                if article.effect.as_deref() == Some("vervallen")
                    && previous.effect.as_deref() != Some("vervallen") =>
            {
                Change::Repealed
            }

            Some(previous) if previous.content != article.content => Change::Amended,
            Some(_) => Change::Unchanged,
        };

        changes.push(shaped(change, article, was, Some(index)));
    }

    for (index, article) in before.iter().enumerate() {
        let Some(key) = identity(article, &ambiguous) else {
            continue;
        };
        if !new.contains_key(key) {
            changes.push(shaped(Change::Withdrawn, article, Some(index), None));
        }
    }

    Comparison {
        changes,
        unidentified,
        repeated,
    }
}

/// The stam-id an article can be matched on: its own, unless the register used
/// it more than once.
fn identity<'a>(article: &'a Article, ambiguous: &HashSet<&str>) -> Option<&'a str> {
    article
        .stam_id
        .as_deref()
        .filter(|id| !ambiguous.contains(id))
}

fn nameless(articles: &[Article]) -> u32 {
    articles.iter().filter(|a| a.stam_id.is_none()).count() as u32
}

/// How a change is named, which is how the side that still exists names it.
fn shaped(
    change: Change,
    shape: &Article,
    before: Option<usize>,
    after: Option<usize>,
) -> ArticleChange {
    ArticleChange {
        change,
        number: shape.number.clone(),
        label: shape.label.clone(),
        path: shape.path.clone(),
        before,
        after,
    }
}

/// Where each identity sits, and which identities were used more than once.
///
/// A map keeps the last of a repeated key and says nothing, which would report
/// one of two articles as unchanged and lose the other without a trace.
fn index(articles: &[Article]) -> (HashMap<&str, usize>, HashSet<&str>) {
    let mut at = HashMap::new();
    let mut twice = HashSet::new();

    for (index, article) in articles.iter().enumerate() {
        let Some(key) = article.stam_id.as_deref() else {
            continue;
        };
        if at.insert(key, index).is_some() {
            twice.insert(key);
        }
    }

    (at, twice)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn article(stam: &str, number: &str, text: &str, effect: &str) -> Article {
        Article {
            stam_id: Some(stam.into()),
            number: number.into(),
            label: Some(format!("Artikel {number}")),
            path: Some(format!("/Boek6/Artikel{number}")),
            content: text.into(),
            effect: Some(effect.into()),
        }
    }

    fn changed(comparison: &Comparison, number: &str) -> Change {
        comparison
            .changes
            .iter()
            .find(|c| c.number == number)
            .unwrap()
            .change
    }

    /// The register keeps a repealed article in every rendition after the one
    /// that repealed it, still carrying `vervallen` and still empty. Counting
    /// it again each time is an attribution that is right about the act and
    /// wrong about the window: BW2 between January and July 2026 reported 69
    /// repeals, among them article 31 through stb-1989-541.
    #[test]
    fn a_repeal_that_already_stood_is_not_a_change_in_this_window() {
        let before = [article("31", "31", "", "vervallen")];
        let after = [article("31", "31", "", "vervallen")];

        assert_eq!(changed(&compare(&before, &after), "31"), Change::Unchanged);
    }

    #[test]
    fn a_repeal_within_the_window_is_still_reported() {
        let before = [article("31", "31", "Geldt nog.", "wijziging")];
        let after = [article("31", "31", "", "vervallen")];

        assert_eq!(changed(&compare(&before, &after), "31"), Change::Repealed);
    }

    #[test]
    fn reports_every_kind_of_change() {
        let before = [
            article("1", "1", "Ongewijzigde tekst.", "nieuwe-regeling"),
            article("2", "2", "De oude tekst.", "nieuwe-regeling"),
            article("3", "3", "Verdwijnt zonder meer.", "nieuwe-regeling"),
            article("4", "4", "Wordt ingetrokken.", "nieuwe-regeling"),
        ];
        let after = [
            article("1", "1", "Ongewijzigde tekst.", "nieuwe-regeling"),
            article("2", "2", "De nieuwe tekst.", "wijziging"),
            article("4", "4", "", "vervallen"),
            article("5", "5", "Geheel nieuw.", "nieuw"),
        ];

        let comparison = compare(&before, &after);

        assert_eq!(changed(&comparison, "1"), Change::Unchanged);
        assert_eq!(changed(&comparison, "2"), Change::Amended);
        assert_eq!(changed(&comparison, "4"), Change::Repealed);
        assert_eq!(changed(&comparison, "5"), Change::Added);
        assert_eq!(changed(&comparison, "3"), Change::Withdrawn);
        assert!(comparison.is_whole());
    }

    #[test]
    fn each_side_points_back_at_the_article_it_came_from() {
        let before = [article("2", "2", "De oude tekst.", "nieuwe-regeling")];
        let after = [article("2", "2", "De nieuwe tekst.", "wijziging")];

        let comparison = compare(&before, &after);
        let amended = &comparison.changes[0];

        assert_eq!(before[amended.before.unwrap()].content, "De oude tekst.");
        assert_eq!(after[amended.after.unwrap()].content, "De nieuwe tekst.");
    }

    #[test]
    fn matching_is_on_identity_not_on_number() {
        // The same article renumbered is one amendment, not a removal plus an
        // addition. Matching on the printed number would report it as both.
        let before = [article("7", "196b", "Tekst.", "nieuwe-regeling")];
        let after = [article("7", "196c", "Tekst.", "wijziging")];

        let comparison = compare(&before, &after);

        assert_eq!(comparison.changes.len(), 1);
        assert_eq!(comparison.changes[0].change, Change::Unchanged);
        assert_eq!(comparison.changes[0].number, "196c");
    }

    /// Nothing says whether an article with no identity on one side is the same
    /// article on the other. It is left out, and left out is not the same as
    /// unchanged, so it is counted and the caller says so.
    #[test]
    fn an_article_without_an_identity_is_counted_rather_than_dropped() {
        let mut orphan = article("x", "9", "Tekst.", "nieuw");
        orphan.stam_id = None;

        let comparison = compare(&[], &[orphan]);

        assert!(comparison.changes.is_empty());
        assert_eq!(comparison.unidentified.after, 1);
        assert!(!comparison.is_whole());
    }

    /// One article standing in both renditions counts once in each, because
    /// that is the only thing that can be counted: a missing stam-id is exactly
    /// the register declining to say whether these two are one article. A
    /// single total would read as two articles, and a number here stands in for
    /// an adjective.
    #[test]
    fn an_article_with_no_identity_is_counted_in_each_rendition_it_stands_in() {
        let nameless = |text: &str| {
            let mut article = article("x", "9", text, "wijziging");
            article.stam_id = None;
            article
        };

        let comparison = compare(&[nameless("Oud.")], &[nameless("Nieuw.")]);

        assert_eq!(
            comparison.unidentified,
            Unidentified {
                before: 1,
                after: 1
            }
        );
    }

    /// A map keeps the last of a repeated key and says nothing. That would
    /// report one of two articles as unchanged and lose the other with no trace
    /// of either the loss or the ambiguity.
    #[test]
    fn an_identity_the_register_used_twice_is_named_rather_than_resolved() {
        let before = [article("7", "1", "Eerste.", "nieuwe-regeling")];
        let after = [
            article("7", "1", "Eerste.", "nieuwe-regeling"),
            article("7", "2", "Tweede.", "nieuw"),
        ];

        let comparison = compare(&before, &after);

        assert!(comparison.changes.is_empty());
        assert_eq!(comparison.repeated, vec!["7".to_string()]);
        assert!(!comparison.is_whole());
    }
}
