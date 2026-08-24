//! Small helpers over roxmltree.
//!
//! The overheid.nl feeds mix namespaced SRU envelopes with un-namespaced
//! document bodies, and several elements carry a text node next to a
//! `<meta-data>` block. These helpers make that distinction explicit at the
//! call site, because picking the wrong one silently yields metadata soup.
//!
//! Names and namespaces are `&'static str`: every one is a constant, and saying
//! so keeps a single lifetime on the returned iterators.

use roxmltree::Node;

pub type Elem<'a> = Node<'a, 'a>;

/// First descendant with this namespace and local name.
pub fn find<'a>(node: Elem<'a>, namespace: &'static str, name: &'static str) -> Option<Elem<'a>> {
    node.descendants()
        .find(|n| n.tag_name().name() == name && n.tag_name().namespace() == Some(namespace))
}

/// Every descendant with this namespace and local name.
pub fn find_all<'a>(
    node: Elem<'a>,
    namespace: &'static str,
    name: &'static str,
) -> impl Iterator<Item = Elem<'a>> + 'a {
    node.descendants()
        .filter(move |n| n.tag_name().name() == name && n.tag_name().namespace() == Some(namespace))
}

pub fn find_all_local<'a>(
    node: Elem<'a>,
    name: &'static str,
) -> impl Iterator<Item = Elem<'a>> + 'a {
    node.descendants()
        .filter(move |n| n.tag_name().name() == name)
}

/// True when the node sits anywhere inside a `<meta-data>` block.
///
/// BWB documents repeat `<al>` inside the provenance metadata attached to every
/// article and every lid. Collecting text without this check splices citation
/// bookkeeping into statutory text: Boek 6 carries 1.476 such blocks against
/// 340 articles.
pub fn in_metadata(node: Elem) -> bool {
    node.ancestors().any(|a| a.tag_name().name() == "meta-data")
}

/// Descendants with this local name that are not buried in a `<meta-data>`
/// block.
pub fn find_all_content<'a>(
    node: Elem<'a>,
    name: &'static str,
) -> impl Iterator<Item = Elem<'a>> + 'a {
    find_all_local(node, name).filter(|n| !in_metadata(*n))
}

/// Text of every descendant text node, joined. The equivalent of itertext.
pub fn inner_text(node: Elem) -> Option<String> {
    collect(node.descendants().filter(|n| n.is_text()))
}

/// Namespaced descendant's full inner text, the common SRU read.
pub fn text_of(node: Elem, namespace: &'static str, name: &'static str) -> Option<String> {
    find(node, namespace, name).and_then(inner_text)
}

fn collect<'a>(nodes: impl Iterator<Item = Elem<'a>>) -> Option<String> {
    let mut text = String::new();
    for node in nodes {
        text.push_str(node.text().unwrap_or_default());
    }

    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed.len() == text.len() {
        Some(text)
    } else {
        Some(trimmed.to_string())
    }
}

/// Collapses the runs of whitespace that XML indentation leaves in extracted
/// prose, keeping the newlines the caller inserted between blocks.
///
/// Split on ASCII whitespace rather than on Unicode's, because the two are not
/// the same question. Indentation is spaces, tabs and newlines; a non-breaking
/// space is something the publisher typed, and `€\u{a0}38.883` means the amount
/// is not to be broken across a line. Rust counts it as whitespace, so
/// `split_whitespace` would quietly rewrite it to a plain space and the text
/// would no longer be the text that was published. The service keeps it, and a
/// provision that differs between the two halves is the drift this project
/// keeps having to close.
pub fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());

    for line in text.split('\n') {
        let start = out.len();
        for word in line.split_ascii_whitespace() {
            if out.len() > start {
                out.push(' ');
            }
            out.push_str(word);
        }
        if out.len() > start {
            out.push('\n');
        }
    }

    while out.ends_with('\n') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> roxmltree::Document<'_> {
        roxmltree::Document::parse(xml).unwrap()
    }

    #[test]
    fn inner_text_collects_children() {
        let doc = parse("<al>Verbintenissen <nadruk>kunnen</nadruk> ontstaan.</al>");
        assert_eq!(
            inner_text(doc.root_element()).as_deref(),
            Some("Verbintenissen kunnen ontstaan.")
        );
    }

    #[test]
    fn descendants_are_found_at_every_depth() {
        let doc = parse("<artikel><lid><lid>nested</lid></lid></artikel>");
        assert_eq!(find_all_local(doc.root_element(), "lid").count(), 2);
    }

    #[test]
    fn metadata_subtrees_are_excluded_from_content() {
        let doc = parse("<artikel><al>text</al><meta-data><al>noise</al></meta-data></artikel>");
        let found: Vec<_> = find_all_content(doc.root_element(), "al")
            .filter_map(inner_text)
            .collect();
        assert_eq!(found, vec!["text"]);
    }

    #[test]
    fn tidy_collapses_indentation_but_keeps_paragraphs() {
        assert_eq!(tidy("  een   twee \n\n  drie  "), "een twee\ndrie");
        assert_eq!(tidy(""), "");
        assert_eq!(tidy("   "), "");
        assert_eq!(tidy("a\nb\nc"), "a\nb\nc");
    }
}
