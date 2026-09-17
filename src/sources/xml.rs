//! Small helpers over roxmltree.
//!
//! Every element bron reads is namespaced, so a lookup names the namespace
//! beside the local name and a same-named element from another vocabulary
//! cannot answer it.
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

/// Text of every descendant text node, joined. The equivalent of itertext.
pub fn inner_text(node: Elem) -> Option<String> {
    let text: String = node
        .descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect();

    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Namespaced descendant's full inner text, the common SRU read.
pub fn text_of(node: Elem, namespace: &'static str, name: &'static str) -> Option<String> {
    find(node, namespace, name).and_then(inner_text)
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
    fn tidy_collapses_indentation_but_keeps_paragraphs() {
        assert_eq!(tidy("  een   twee \n\n  drie  "), "een twee\ndrie");
        assert_eq!(tidy(""), "");
        assert_eq!(tidy("   "), "");
        assert_eq!(tidy("a\nb\nc"), "a\nb\nc");
    }
}
