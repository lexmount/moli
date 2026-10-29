use crate::{Dom, NodeKind};
use std::fmt::Write as _;

/// Math renderers often pair semantic MathML with an aria-hidden visual copy.
/// Recognize that relationship from markup, independently of library classes.
pub(crate) fn primary_alternative<D: Dom + ?Sized>(
    dom: &D,
    node: D::NodeId,
    limit: usize,
) -> Option<D::NodeId> {
    let mut primary = None;
    let mut alternative = false;
    let mut child = dom.first_child(node);
    while let Some(id) = child {
        child = dom.next_sibling(id);
        if ignorable(dom.node_kind(id)) {
            continue;
        }
        if is_hidden_alternative(dom, id) {
            alternative = true;
            continue;
        }
        if primary.is_some() {
            return None;
        }
        let mut candidate = id;
        let mut found = None;
        // Bound wrapper inspection per node; deeply nested ordinary prose must
        // not cause repeated whole-subtree scans.
        for _ in 1..limit.min(9) {
            if dom.node_kind(candidate) == NodeKind::Element("math") {
                // Retain the original branch, including links and styles
                // around the formula, for ordinary DOM traversal.
                found = Some(id);
                break;
            }
            let mut children = dom.first_child(candidate);
            let mut single = None;
            while let Some(id) = children {
                children = dom.next_sibling(id);
                if ignorable(dom.node_kind(id)) {
                    continue;
                }
                if single.is_some() {
                    return None;
                }
                single = Some(id);
            }
            candidate = single?;
        }
        primary = Some(found?);
    }
    if alternative { primary } else { None }
}

pub(crate) fn is_hidden_alternative<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> bool {
    matches!(dom.node_kind(node), NodeKind::Element(_))
        && dom
            .attribute(node, "aria-hidden")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn ignorable(kind: NodeKind<'_>) -> bool {
    matches!(kind, NodeKind::Other)
        || matches!(kind, NodeKind::Text(text) if text.trim().is_empty())
}

enum Task<Id> {
    Node(Id, usize),
    Sibling(Option<Id>, usize),
    Close(String),
}

/// Keep native mathematical structure instead of inventing a lossy plain-text
/// equation. Only mathematical markup and presentation attributes survive;
/// alternate encodings, HTML, scripts and event handlers cannot become active.
pub(crate) fn render<D: Dom + ?Sized>(dom: &D, root: D::NodeId, max_depth: usize) -> String {
    let mut output = String::new();
    let mut tasks = vec![Task::Node(root, 0)];
    while let Some(task) = tasks.pop() {
        match task {
            Task::Close(tag) => {
                output.push_str("</");
                output.push_str(&tag);
                output.push('>');
            }
            Task::Sibling(Some(node), depth) => {
                tasks.push(Task::Sibling(dom.next_sibling(node), depth));
                tasks.push(Task::Node(node, depth));
            }
            Task::Sibling(None, _) => {}
            Task::Node(node, depth) => {
                if depth >= max_depth {
                    continue;
                }
                match dom.node_kind(node) {
                    NodeKind::Text(text) => escape(text, &mut output, true),
                    NodeKind::Element("annotation" | "annotation-xml" | "script" | "style") => {}
                    NodeKind::Element(tag @ ("semantics" | "maction")) => {
                        // Presentation is the first child; subsequent annotation
                        // children are alternate representations, not more terms.
                        // maction likewise displays its first element, and its
                        // replacement must remain one argument even when empty.
                        if tag == "maction" {
                            output.push_str("<mrow>");
                            tasks.push(Task::Close("mrow".to_owned()));
                        }
                        let mut child = dom.first_child(node);
                        while let Some(id) = child {
                            if matches!(dom.node_kind(id), NodeKind::Element(_)) {
                                tasks.push(Task::Node(id, depth + 1));
                                break;
                            }
                            child = dom.next_sibling(id);
                        }
                    }
                    NodeKind::Element(tag) => {
                        // Unknown containers still occupy one child position in
                        // fractions, scripts and roots. Keep their grouping.
                        let tag = if allowed_tag(tag) { tag } else { "mrow" };
                        output.push('<');
                        output.push_str(tag);
                        for name in ATTRIBUTES {
                            if let Some(value) = dom.attribute(node, name) {
                                output.push(' ');
                                output.push_str(name);
                                output.push_str("=\"");
                                escape(value, &mut output, false);
                                output.push('"');
                            }
                        }
                        output.push('>');
                        tasks.push(Task::Close(tag.to_owned()));
                        tasks.push(Task::Sibling(dom.first_child(node), depth + 1));
                    }
                    NodeKind::Document => {
                        tasks.push(Task::Sibling(dom.first_child(node), depth + 1));
                    }
                    NodeKind::Other => {}
                }
            }
        }
    }
    output
}

fn allowed_tag(tag: &str) -> bool {
    matches!(
        tag,
        "math"
            | "mi"
            | "mn"
            | "mo"
            | "mtext"
            | "ms"
            | "mspace"
            | "mrow"
            | "mfrac"
            | "msqrt"
            | "mroot"
            | "mstyle"
            | "merror"
            | "mpadded"
            | "mphantom"
            | "mfenced"
            | "menclose"
            | "msub"
            | "msup"
            | "msubsup"
            | "munder"
            | "mover"
            | "munderover"
            | "mmultiscripts"
            | "mprescripts"
            | "none"
            | "mtable"
            | "mtr"
            | "mlabeledtr"
            | "mtd"
            | "maligngroup"
            | "malignmark"
    )
}

const ATTRIBUTES: &[&str] = &[
    "display",
    "mathvariant",
    "displaystyle",
    "scriptlevel",
    "linethickness",
    "bevelled",
    "open",
    "close",
    "separators",
    "notation",
    "accent",
    "accentunder",
    "stretchy",
    "symmetric",
    "largeop",
    "movablelimits",
    "form",
    "fence",
    "separator",
    "rowspan",
    "columnspan",
    "rowalign",
    "columnalign",
    "rowlines",
    "columnlines",
    "lquote",
    "rquote",
];

fn escape(text: &str, output: &mut String, markdown_text: bool) {
    for ch in text.chars() {
        match ch {
            '\\' | '*' | '_' | '`' | '~' | '[' | ']' | '$' if markdown_text => {
                write!(output, "&#{};", ch as u32).expect("writing to String");
            }
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            // Keep an inline HTML fragment on one Markdown line, even when the
            // source's pretty printing contains blank lines.
            '\n' | '\r' | '\t' => output.push(' '),
            _ => output.push(ch),
        }
    }
}
