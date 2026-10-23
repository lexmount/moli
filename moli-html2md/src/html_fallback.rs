use std::collections::HashSet;
use std::fmt::Write as _;

use crate::{Dom, NodeKind};

/// Markdown has no merged cells, nested tables, or block content inside cells.
/// Each structural pre-scan stops at nested tables; nested scopes are disjoint.
/// Conversion and target discovery keep their own bounded linear traversals.
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
                        .is_some_and(|value| crate::table::has_cell_span(attr, value))
                })
            {
                return true;
            }
        }
        stack.push((dom.first_child(node), depth + 1));
    }
    false
}

/// Preserve this table's authored relationships, even without header cells.
/// Spans require structural output; they do not prove whether a table is layout.
/// Inspect only this table's own cells, not nested tables.
pub(crate) fn requires_table_structure<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    depth: usize,
    limit: usize,
) -> bool {
    if dom.attribute(root, "role").is_some_and(|role| {
        matches!(
            role.trim_matches(crate::writer::is_space)
                .to_ascii_lowercase()
                .as_str(),
            "table" | "grid" | "treegrid"
        )
    }) {
        return true;
    }
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
            if tag == "table" {
                continue;
            }
            if matches!(tag, "th" | "thead" | "caption") {
                return true;
            }
            if matches!(tag, "td" | "th")
                && (dom
                    .attribute(node, "scope")
                    .is_some_and(|value| !value.trim().is_empty())
                    || dom
                        .attribute(node, "headers")
                        .is_some_and(|value| !value.trim().is_empty())
                    || ["rowspan", "colspan"].into_iter().any(|name| {
                        dom.attribute(node, name)
                            .is_some_and(|value| crate::table::has_cell_span(name, value))
                    }))
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
    Node(Id, usize, bool),
    Siblings(Option<Id>, usize, bool),
    Close(String),
    Fallback(String, usize),
    Literal(&'static str),
}

/// HTML fallback preserves structural content and referenced navigation targets.
/// Keep content markup only; scripts, styles, event handlers and unrelated
/// presentation attributes are never copied.
pub(crate) fn render<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    depth: usize,
    limit: usize,
    anchor_targets: &crate::anchors::Targets,
    default_language: Option<&str>,
    emitted_anchors: &mut HashSet<String>,
) -> String {
    let inline_root = matches!(dom.node_kind(root), NodeKind::Element("code"));
    let keep_source_lines = matches!(dom.node_kind(root), NodeKind::Element("pre"));
    let mut output = String::new();
    let mut tasks = vec![Task::Node(root, depth, false)];
    while let Some(task) = tasks.pop() {
        match task {
            Task::Literal(value) => output.push_str(value),
            Task::Fallback(label, before) => {
                if output[before..]
                    .trim_matches(crate::writer::is_space)
                    .is_empty()
                {
                    output.push(' ');
                    escape(&label, &mut output);
                    output.push(' ');
                }
            }
            Task::Close(tag) => {
                if matches!(tag.as_str(), "table" | "thead" | "tbody" | "tfoot" | "tr") {
                    begin_html_line(&mut output);
                }
                output.push_str("</");
                output.push_str(&tag);
                output.push('>');
            }
            Task::Siblings(None, _, _) => {}
            Task::Siblings(Some(node), depth, code) => {
                tasks.push(Task::Siblings(dom.next_sibling(node), depth, code));
                tasks.push(Task::Node(node, depth, code));
            }
            Task::Node(node, depth, code) => {
                if depth >= limit {
                    continue;
                }
                if code && crate::content::is_code_control(dom, node) {
                    continue;
                }
                if let NodeKind::Element(tag) = dom.node_kind(node) {
                    if !code
                        && dom.has_block_layout(node)
                        && !has_block_tag(tag)
                        && !crate::machine::owns_boundaries(tag)
                    {
                        output.push_str("<div>");
                        tasks.push(Task::Close("div".to_owned()));
                    } else if !code
                        && dom.has_text_boundary(node)
                        && !crate::machine::owns_boundaries(tag)
                    {
                        output.push(' ');
                        tasks.push(Task::Literal(" "));
                    }
                }
                let code = code || matches!(dom.node_kind(node), NodeKind::Element("pre" | "code"));
                if let NodeKind::Element(tag) = dom.node_kind(node)
                    && !allowed(tag)
                {
                    for id in anchor_targets.targets(dom, node) {
                        if emitted_anchors.insert(id.to_owned()) {
                            output.push_str(&crate::anchors::markup(id));
                        }
                    }
                }
                match dom.node_kind(node) {
                    NodeKind::Text(text) => escape_text(
                        text,
                        &mut output,
                        code,
                        keep_source_lines && code,
                        inline_root,
                    ),
                    NodeKind::Element(
                        "head" | "title" | "script" | "style" | "noscript" | "template",
                    )
                    | NodeKind::Other => {}
                    NodeKind::Element("math") => {
                        for id in crate::anchors::within(dom, node, limit - depth, anchor_targets) {
                            if emitted_anchors.insert(id.clone()) {
                                output.push_str(&crate::anchors::markup(&id));
                            }
                        }
                        output.push_str(&crate::mathml::render(dom, node, limit - depth))
                    }
                    NodeKind::Element("input") => {
                        if let Some(value) = crate::form::input_text(dom, node) {
                            escape(&value, &mut output);
                        }
                    }
                    NodeKind::Element("textarea") => {
                        if let Some(value) = crate::form::textarea_text(dom, node) {
                            escape(&value, &mut output);
                        } else {
                            tasks.push(Task::Siblings(dom.first_child(node), depth + 1, code));
                        }
                    }
                    NodeKind::Element(tag) => {
                        if crate::visibility::nonrendered_serialized_state(dom, node) {
                            continue;
                        }
                        if tag == "img" {
                            let label = crate::content::image_label(dom, node);
                            let targets: Vec<_> = anchor_targets
                                .targets(dom, node)
                                .into_iter()
                                .filter(|id| emitted_anchors.insert((*id).to_owned()))
                                .collect();
                            if let Some(src) = crate::media::source(dom, node) {
                                for id in targets.iter().skip(1) {
                                    output.push_str(&crate::anchors::markup(id));
                                }
                                output.push_str("<img src=\"");
                                escape(&src, &mut output);
                                output.push_str("\" alt=\"");
                                escape(&label, &mut output);
                                output.push('"');
                                if let Some(id) = targets.first() {
                                    output.push_str(" id=\"");
                                    escape(id, &mut output);
                                    output.push('"');
                                }
                                if let Some(title) = dom.attribute(node, "title") {
                                    output.push_str(" title=\"");
                                    escape(title, &mut output);
                                    output.push('"');
                                }
                                output.push('>');
                            } else {
                                for id in targets {
                                    output.push_str(&crate::anchors::markup(id));
                                }
                                escape(&label, &mut output);
                            }
                            continue;
                        }
                        if matches!(tag, "iframe" | "video" | "audio") {
                            let urls = crate::media::sources(dom, node, tag, depth, limit);
                            let poster = crate::media::poster(dom, node, tag);
                            if !urls.is_empty() || poster.is_some() {
                                let block = tag == "audio";
                                output.push_str(if block { "<div>" } else { " " });
                                if let Some(poster) = poster {
                                    output.push_str("<img src=\"");
                                    escape(poster, &mut output);
                                    output.push_str("\" alt=\"");
                                    escape(&crate::media::poster_label(dom, node), &mut output);
                                    output.push_str("\"> ");
                                }
                                let has_source = !urls.is_empty();
                                for url in urls {
                                    output.push_str("<a href=\"");
                                    escape(url, &mut output);
                                    output.push_str("\">");
                                    escape(crate::media::label(dom, node, tag), &mut output);
                                    output.push_str("</a> ");
                                }
                                if block {
                                    output.push_str("</div>");
                                }
                                if has_source {
                                    continue;
                                }
                            }
                        }
                        let retained = if allowed(tag)
                            && (tag != "span"
                                || !anchor_targets.targets(dom, node).is_empty()
                                || dom.attribute(node, "title").is_some())
                        {
                            Some(tag)
                        } else {
                            None
                        };
                        if let Some(tag) = retained {
                            if matches!(
                                tag,
                                "table"
                                    | "thead"
                                    | "tbody"
                                    | "tfoot"
                                    | "tr"
                                    | "td"
                                    | "th"
                                    | "caption"
                            ) {
                                begin_html_line(&mut output);
                            }
                            let targets: Vec<_> = anchor_targets
                                .targets(dom, node)
                                .into_iter()
                                .filter(|id| emitted_anchors.insert((*id).to_owned()))
                                .collect();
                            for id in targets.iter().skip(1) {
                                output.push_str(&crate::anchors::markup(id));
                            }
                            output.push('<');
                            output.push_str(tag);
                            for name in [
                                "rowspan", "colspan", "scope", "headers", "href", "src", "alt",
                                "title", "start", "reversed", "value", "label", "selected",
                                "multiple", "disabled",
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
                                    if (name == "href" && !crate::media::safe_link_url(value))
                                        || (name == "src" && !crate::media::safe_url(value, true))
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
                            if let Some(id) = targets.first() {
                                output.push_str(" id=\"");
                                escape(id, &mut output);
                                output.push('"');
                            }
                            let language = if tag == "pre" {
                                crate::content::code_language(dom, node, default_language)
                            } else if tag == "code" {
                                crate::content::class_language(dom.attribute(node, "class"))
                            } else {
                                None
                            };
                            if let Some(language) =
                                language.and_then(|value| value.split_ascii_whitespace().next())
                            {
                                output.push_str(" class=\"language-");
                                escape(language, &mut output);
                                output.push('"');
                            }
                            output.push('>');
                            // The HTML parser discards one initial LF after pre.
                            // Supply that LF separately from the actual source.
                            if tag == "pre" {
                                output.push('\n');
                            }
                            if !matches!(tag, "br" | "hr" | "img") {
                                tasks.push(Task::Close(tag.to_owned()));
                            }
                        }
                        if let Some((math, relative_depth)) =
                            crate::mathml::primary_alternative(dom, node, limit - depth)
                        {
                            for id in
                                crate::anchors::within(dom, node, limit - depth, anchor_targets)
                            {
                                if emitted_anchors.insert(id.clone()) {
                                    output.push_str(&crate::anchors::markup(&id));
                                }
                            }
                            output.push_str(&crate::mathml::render(
                                dom,
                                math,
                                limit - depth - relative_depth,
                            ));
                        } else {
                            if let Some(label) = crate::content::fallback_text(dom, node) {
                                tasks.push(Task::Fallback(label, output.len()));
                            }
                            tasks.push(Task::Siblings(dom.first_child(node), depth + 1, code));
                        }
                    }
                    NodeKind::Document => {
                        tasks.push(Task::Siblings(dom.first_child(node), depth + 1, code))
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

fn has_block_tag(tag: &str) -> bool {
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
            | "p"
            | "div"
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
            | "hr"
    )
}

fn escape_text(
    text: &str,
    output: &mut String,
    code: bool,
    keep_newlines: bool,
    inline_root: bool,
) {
    for ch in text.chars() {
        if !code && matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
            if !output.ends_with([' ', '\n']) {
                output.push(' ');
            }
            continue;
        }
        if keep_newlines && matches!(ch, '\n' | '\r') {
            output.push(ch);
        } else if code
            && inline_root
            && matches!(ch, '\\' | '*' | '_' | '`' | '~' | '[' | ']' | '$')
        {
            write!(output, "&#{};", ch as u32).expect("writing to String");
        } else {
            let mut bytes = [0; 4];
            escape(ch.encode_utf8(&mut bytes), output);
        }
    }
}

fn begin_html_line(output: &mut String) {
    while output.ends_with(' ') {
        output.pop();
    }
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
}
