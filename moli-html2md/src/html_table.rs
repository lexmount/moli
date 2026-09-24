use crate::{Dom, NodeKind};

/// Markdown has no merged cells, nested tables, or block content inside cells.
/// Detect those structures before conversion so a table is traversed at most
/// twice, even when it contains thousands of nested tables.
pub(crate) fn needed<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    depth: usize,
    limit: usize,
) -> bool {
    let mut stack = vec![(dom.first_child(root), depth + 1)];
    while let Some((node, depth)) = stack.pop() {
        let Some(node) = node else {
            continue;
        };
        if depth >= limit {
            continue;
        }
        stack.push((dom.next_sibling(node), depth));
        if let NodeKind::Element(tag) = dom.node_kind(node) {
            if matches!(
                tag,
                "table"
                    | "pre"
                    | "blockquote"
                    | "ul"
                    | "ol"
                    | "li"
                    | "hr"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
            ) {
                return true;
            }
            if matches!(tag, "td" | "th")
                && ["rowspan", "colspan"].iter().any(|attr| {
                    dom.attribute(node, attr)
                        .is_some_and(|value| value.trim().parse::<u32>() != Ok(1))
                })
            {
                return true;
            }
        }
        stack.push((dom.first_child(node), depth + 1));
    }
    false
}

pub(crate) fn has_cells<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    depth: usize,
    limit: usize,
) -> bool {
    let mut stack = vec![(dom.first_child(root), depth + 1)];
    while let Some((node, depth)) = stack.pop() {
        let Some(node) = node else {
            continue;
        };
        if depth >= limit {
            continue;
        }
        if matches!(dom.node_kind(node), NodeKind::Element("th" | "td")) {
            return true;
        }
        stack.push((dom.next_sibling(node), depth));
        stack.push((dom.first_child(node), depth + 1));
    }
    false
}

enum Task<Id> {
    Node(Id, usize),
    Siblings(Option<Id>, usize),
    Close(String),
}

/// A compact HTML block is valid Markdown and preserves the original cell
/// associations. Keep content markup only; scripts, styles, event handlers
/// and unrelated presentation attributes are never copied.
pub(crate) fn render<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    depth: usize,
    limit: usize,
) -> String {
    let mut output = String::new();
    let mut tasks = vec![Task::Node(root, depth)];
    while let Some(task) = tasks.pop() {
        match task {
            Task::Close(tag) => {
                output.push_str("</");
                output.push_str(&tag);
                output.push('>');
            }
            Task::Siblings(None, _) => {}
            Task::Siblings(Some(node), depth) => {
                tasks.push(Task::Siblings(dom.next_sibling(node), depth));
                tasks.push(Task::Node(node, depth));
            }
            Task::Node(node, depth) => {
                if depth >= limit {
                    continue;
                }
                match dom.node_kind(node) {
                    NodeKind::Text(text) => escape(text, &mut output),
                    NodeKind::Element(
                        "head" | "title" | "script" | "style" | "noscript" | "template",
                    )
                    | NodeKind::Other => {}
                    NodeKind::Element("math") => {
                        output.push_str(&crate::mathml::render(dom, node, limit - depth))
                    }
                    NodeKind::Element("input") => {
                        if let Some(value) = crate::form::input_text(dom, node) {
                            escape(&value, &mut output);
                        }
                    }
                    NodeKind::Element(tag) => {
                        if crate::visibility::nonrendered_serialized_state(dom, node) {
                            continue;
                        }
                        if matches!(tag, "iframe" | "video" | "audio") {
                            let urls = crate::media::sources(dom, node, tag, depth, limit);
                            if !urls.is_empty() {
                                output.push_str("<div>");
                                if tag == "video"
                                    && let Some(poster) = dom
                                        .attribute(node, "poster")
                                        .filter(|src| crate::media::safe_url(src, true))
                                {
                                    output.push_str("<img src=\"");
                                    escape(poster, &mut output);
                                    output.push_str("\" alt=\"\">");
                                }
                                for url in urls
                                    .into_iter()
                                    .filter(|url| crate::media::safe_url(url, false))
                                {
                                    output.push_str("<a href=\"");
                                    escape(url, &mut output);
                                    output.push_str("\">");
                                    escape(crate::media::label(dom, node, tag), &mut output);
                                    output.push_str("</a><br>");
                                }
                                output.push_str("</div>");
                                continue;
                            }
                        }
                        let retained = if allowed(tag) {
                            Some(tag)
                        } else if dom.has_block_layout(node) {
                            Some("div")
                        } else {
                            None
                        };
                        if dom.has_block_layout(node)
                            && matches!(
                                retained,
                                Some(
                                    "a" | "span"
                                        | "strong"
                                        | "b"
                                        | "em"
                                        | "i"
                                        | "s"
                                        | "del"
                                        | "sup"
                                        | "sub"
                                        | "code"
                                        | "img"
                                        | "select"
                                        | "button"
                                )
                            )
                        {
                            output.push_str("<div>");
                            tasks.push(Task::Close("div".to_owned()));
                        }
                        if let Some(tag) = retained {
                            output.push('<');
                            output.push_str(tag);
                            for name in [
                                "rowspan", "colspan", "scope", "headers", "id", "href", "src",
                                "alt", "title", "start", "reversed", "value", "label", "selected",
                                "multiple", "disabled", "name",
                            ] {
                                let image_source = if name == "src" && tag == "img" {
                                    crate::media::source(dom, node)
                                } else {
                                    None
                                };
                                let value = if name == "src" && tag == "img" {
                                    image_source.as_deref()
                                } else {
                                    dom.attribute(node, name)
                                };
                                if let Some(value) = value {
                                    if matches!(name, "href" | "src")
                                        && !crate::media::safe_url(value, name == "src")
                                    {
                                        continue;
                                    }
                                    output.push(' ');
                                    output.push_str(name);
                                    output.push_str("=\"");
                                    escape(value, &mut output);
                                    output.push('"');
                                }
                            }
                            output.push('>');
                            if !matches!(tag, "br" | "hr" | "img") {
                                tasks.push(Task::Close(tag.to_owned()));
                            }
                        }
                        if let Some((math, relative_depth)) =
                            crate::mathml::primary_alternative(dom, node, limit - depth)
                        {
                            output.push_str(&crate::mathml::render(
                                dom,
                                math,
                                limit - depth - relative_depth,
                            ));
                        } else {
                            tasks.push(Task::Siblings(dom.first_child(node), depth + 1));
                        }
                    }
                    NodeKind::Document => {
                        tasks.push(Task::Siblings(dom.first_child(node), depth + 1))
                    }
                }
            }
        }
    }
    output
}

fn allowed(tag: &str) -> bool {
    matches!(
        tag,
        "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "caption"
            | "select"
            | "optgroup"
            | "option"
            | "button"
            | "p"
            | "div"
            | "span"
            | "a"
            | "img"
            | "br"
            | "hr"
            | "strong"
            | "b"
            | "em"
            | "i"
            | "s"
            | "del"
            | "sup"
            | "sub"
            | "code"
            | "pre"
            | "blockquote"
            | "ul"
            | "ol"
            | "li"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "dl"
            | "dt"
            | "dd"
            | "details"
            | "summary"
    )
}

fn escape(text: &str, output: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            // Raw HTML blocks end at an empty Markdown line. Entities preserve
            // preformatted line endings without prematurely ending the block.
            '\n' => output.push_str("&#10;"),
            '\r' => output.push_str("&#13;"),
            _ => output.push(ch),
        }
    }
}
