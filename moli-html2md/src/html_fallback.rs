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

#[derive(Clone, Copy)]
enum TextMode {
    Normal,
    Code,
    Pre,
}

enum Task<Id> {
    Node(Id, usize, TextMode),
    Siblings(Option<Id>, usize, TextMode),
    MathSiblings(Option<Id>, usize, TextMode),
    Close(String),
    EndTable,
    Fallback(String, usize),
    Literal(&'static str),
}

pub(crate) struct FormattingContext {
    pub(crate) in_link: bool,
    pub(crate) wrapped_block: bool,
}

/// HTML fallback preserves structural content and referenced navigation targets.
/// Keep content markup only; scripts, styles, event handlers and unrelated
/// presentation attributes are never copied.
pub(crate) fn render<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    depth: usize,
    options: &crate::Options,
    anchor_targets: &crate::anchors::Targets,
    emitted_anchors: &mut HashSet<String>,
    formatting: FormattingContext,
) -> crate::content::Html {
    let mut in_link = formatting.in_link;
    let limit = options.max_depth;
    let default_language = options.default_code_language.as_deref();
    let inline_root = matches!(dom.node_kind(root), NodeKind::Element("code"));
    // Formatted blocks receive an outer HTML wrapper. Numeric references preserve
    // source newlines without blank lines ending that CommonMark HTML block.
    let keep_source_lines =
        !formatting.wrapped_block && matches!(dom.node_kind(root), NodeKind::Element("pre"));
    let mut text_escaper = TextEscaper {
        inline_root,
        keep_newlines: keep_source_lines,
        preformatted_code: options.preformatted_code,
        trailing_cr: false,
    };
    let mut output = String::new();
    let mut content = crate::content::Content::default();
    let mut table_modes = Vec::new();
    let mut tasks = vec![Task::Node(root, depth, TextMode::Normal)];
    while let Some(task) = tasks.pop() {
        match task {
            Task::Literal(value) => output.push_str(value),
            Task::EndTable => {
                table_modes.pop();
            }
            Task::Fallback(label, before) => {
                if content.mark() == before {
                    content.text(&label);
                    output.push(' ');
                    text_escaper.write(&label, &mut output, TextMode::Normal);
                    output.push(' ');
                }
            }
            Task::Close(tag) => {
                if tag == "a" {
                    in_link = false;
                }
                if matches!(tag.as_str(), "table" | "thead" | "tbody" | "tfoot" | "tr") {
                    begin_html_line(&mut output);
                }
                output.push_str("</");
                output.push_str(&tag);
                output.push('>');
            }
            Task::Siblings(None, _, _) | Task::MathSiblings(None, _, _) => {}
            Task::Siblings(Some(node), depth, mode) => {
                tasks.push(Task::Siblings(dom.next_sibling(node), depth, mode));
                tasks.push(Task::Node(node, depth, mode));
            }
            Task::MathSiblings(Some(node), depth, mode) => {
                tasks.push(Task::MathSiblings(dom.next_sibling(node), depth, mode));
                if !crate::mathml::is_hidden_alternative(dom, node) {
                    tasks.push(Task::Node(node, depth, mode));
                }
            }
            Task::Node(node, depth, mode) => {
                if depth >= limit || crate::visibility::nonrendered_serialized_state(dom, node) {
                    continue;
                }
                let code = !matches!(mode, TextMode::Normal);
                if code && crate::content::is_code_control(dom, node) {
                    continue;
                }
                if let NodeKind::Element(tag) = dom.node_kind(node) {
                    let owns_boundaries = allowed(tag) && crate::structure::owns_boundaries(tag);
                    if !code && dom.has_block_layout(node) && !owns_boundaries {
                        output.push_str("<div>");
                        tasks.push(Task::Close("div".to_owned()));
                    } else if !code && dom.has_text_boundary(node) && !owns_boundaries {
                        output.push(' ');
                        tasks.push(Task::Literal(" "));
                    }
                }
                let mode = match dom.node_kind(node) {
                    NodeKind::Element("pre") => TextMode::Pre,
                    NodeKind::Element("code") if matches!(mode, TextMode::Normal) => {
                        text_escaper.trailing_cr = false;
                        TextMode::Code
                    }
                    _ => mode,
                };
                let code = !matches!(mode, TextMode::Normal);
                let mut write_text = |text: &str, output: &mut String| {
                    text_escaper.write(text, output, mode);
                };
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
                    NodeKind::Text(text) => {
                        if !text.is_empty()
                            && matches!(mode, TextMode::Code)
                            && options.preformatted_code
                        {
                            content.object();
                        } else {
                            content.text(text);
                        }
                        write_text(text, &mut output);
                    }
                    NodeKind::Element(
                        "head" | "title" | "script" | "style" | "noscript" | "template",
                    )
                    | NodeKind::Other => {}
                    NodeKind::Element("math") => {
                        // MathML contributes content without going through the
                        // text escaper, so a later LF cannot complete a prior CR.
                        text_escaper.trailing_cr = false;
                        content.object();
                        for id in crate::anchors::within(dom, node, limit - depth, anchor_targets) {
                            if emitted_anchors.insert(id.clone()) {
                                output.push_str(&crate::anchors::markup(&id));
                            }
                        }
                        output.push_str(&crate::mathml::render(dom, node, limit - depth))
                    }
                    NodeKind::Element("input") => {
                        if let Some(value) = crate::form::input_text(dom, node) {
                            content.text(value);
                            output.push(' ');
                            write_text(value, &mut output);
                            output.push(' ');
                        }
                    }
                    NodeKind::Element("textarea") => {
                        if let Some(value) = dom.control_value(node) {
                            content.text(value);
                            write_text(value, &mut output);
                        } else {
                            tasks.push(Task::Siblings(dom.first_child(node), depth + 1, mode));
                        }
                    }
                    NodeKind::Element(tag) => {
                        if tag == "table" {
                            table_modes.push(crate::table::is_presentational(dom, node));
                            tasks.push(Task::EndTable);
                        }
                        if tag == "img" {
                            let label = crate::content::image_label(dom, node);
                            let targets: Vec<_> = anchor_targets
                                .targets(dom, node)
                                .into_iter()
                                .filter(|id| emitted_anchors.insert((*id).to_owned()))
                                .collect();
                            if let Some(src) = crate::media::source(dom, node) {
                                content.object();
                                for id in targets.iter().skip(1) {
                                    output.push_str(&crate::anchors::markup(id));
                                }
                                output.push_str("<img src=\"");
                                escape_attribute(src, &mut output);
                                output.push_str("\" alt=\"");
                                escape_attribute(label, &mut output);
                                output.push('"');
                                if let Some(id) = targets.first() {
                                    output.push_str(" id=\"");
                                    escape_attribute(id, &mut output);
                                    output.push('"');
                                }
                                if let Some(title) = dom.attribute(node, "title") {
                                    output.push_str(" title=\"");
                                    escape_attribute(title, &mut output);
                                    output.push('"');
                                }
                                output.push('>');
                            } else {
                                content.text(label);
                                for id in targets {
                                    output.push_str(&crate::anchors::markup(id));
                                }
                                write_text(label, &mut output);
                            }
                            continue;
                        }
                        if matches!(tag, "iframe" | "video" | "audio") {
                            let urls = crate::media::sources(dom, node, tag, depth, limit);
                            let poster = crate::media::poster(dom, node, tag);
                            if !urls.is_empty() || poster.is_some() {
                                content.object();
                                let block = tag == "audio" && !code;
                                output.push_str(if block { "<div>" } else { " " });
                                if let Some(poster) = poster {
                                    output.push_str("<img src=\"");
                                    escape_attribute(poster, &mut output);
                                    output.push_str("\" alt=\"");
                                    escape_attribute(
                                        &crate::media::poster_label(dom, node),
                                        &mut output,
                                    );
                                    output.push_str("\"> ");
                                }
                                let has_source = !urls.is_empty();
                                for url in urls {
                                    if in_link {
                                        write_text(
                                            crate::media::label(dom, node, tag),
                                            &mut output,
                                        );
                                        output.push_str(" (");
                                        write_text(url, &mut output);
                                        output.push_str(") ");
                                    } else {
                                        output.push_str("<a href=\"");
                                        escape_attribute(url, &mut output);
                                        output.push_str("\">");
                                        write_text(
                                            crate::media::label(dom, node, tag),
                                            &mut output,
                                        );
                                        output.push_str("</a> ");
                                    }
                                }
                                if block {
                                    output.push_str("</div>");
                                }
                                if has_source {
                                    continue;
                                }
                            }
                        }
                        let expanded = table_modes.last() == Some(&true)
                            && matches!(
                                tag,
                                "table"
                                    | "thead"
                                    | "tbody"
                                    | "tfoot"
                                    | "tr"
                                    | "td"
                                    | "th"
                                    | "caption"
                                    | "colgroup"
                                    | "col"
                            );
                        // Placeholder anchors expand to non-link targets, so
                        // generated descendants can keep their own links.
                        let link_href = if tag == "a" {
                            crate::media::link_href(dom, node)
                        } else {
                            None
                        };
                        let retained = if expanded && matches!(tag, "colgroup" | "col") {
                            None
                        } else if expanded {
                            Some("div")
                        } else if allowed(tag)
                            && (tag != "a" || (!in_link && link_href.is_some()))
                            && (tag != "span"
                                || !anchor_targets.targets(dom, node).is_empty()
                                || dom.attribute(node, "title").is_some())
                        {
                            Some(tag)
                        } else {
                            None
                        };
                        if let Some(tag) = retained {
                            if matches!(tag, "br" | "hr") {
                                content.object();
                            }
                            in_link |= tag == "a";
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
                                    | "colgroup"
                                    | "col"
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
                                "multiple", "disabled", "open", "span",
                            ] {
                                if expanded
                                    && matches!(name, "rowspan" | "colspan" | "scope" | "headers")
                                {
                                    continue;
                                }
                                let image_source = if name == "src" && tag == "img" {
                                    crate::media::source(dom, node)
                                } else {
                                    None
                                };
                                let value = if name == "href" && tag == "a" {
                                    link_href
                                } else if name == "src" && tag == "img" {
                                    image_source
                                } else {
                                    dom.attribute(node, name)
                                };
                                if let Some(value) = value {
                                    if (name == "href"
                                        && tag != "a"
                                        && !crate::media::safe_link_url(value))
                                        || (name == "src" && !crate::media::safe_url(value, true))
                                    {
                                        continue;
                                    }
                                    output.push(' ');
                                    output.push_str(name);
                                    output.push_str("=\"");
                                    escape_attribute(value, &mut output);
                                    output.push('"');
                                }
                            }
                            if let Some(id) = targets.first() {
                                output.push_str(" id=\"");
                                escape_attribute(id, &mut output);
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
                                escape_attribute(language, &mut output);
                                output.push('"');
                            }
                            output.push('>');
                            // The HTML parser discards one initial LF after pre.
                            // Supply that LF separately from the actual source.
                            if tag == "pre" {
                                output.push('\n');
                            }
                            if matches!(tag, "br" | "hr" | "img" | "col") {
                                continue;
                            }
                            tasks.push(Task::Close(tag.to_owned()));
                        } else {
                            // Supported tags can still be expanded, for example
                            // SVG links nested inside an author's outer link.
                            for id in anchor_targets.targets(dom, node) {
                                if emitted_anchors.insert(id.to_owned()) {
                                    output.push_str(&crate::anchors::markup(id));
                                }
                            }
                        }
                        if crate::mathml::primary_alternative(dom, node, limit - depth).is_some() {
                            for id in
                                crate::anchors::within(dom, node, limit - depth, anchor_targets)
                            {
                                if emitted_anchors.insert(id.clone()) {
                                    output.push_str(&crate::anchors::markup(&id));
                                }
                            }
                            tasks.push(Task::MathSiblings(dom.first_child(node), depth + 1, mode));
                        } else {
                            if matches!(mode, TextMode::Normal)
                                && let Some(label) =
                                    crate::content::fallback_text(dom, node, limit - depth)
                            {
                                tasks.push(Task::Fallback(label, content.mark()));
                            }
                            tasks.push(Task::Siblings(dom.first_child(node), depth + 1, mode));
                        }
                    }
                    NodeKind::Document => {
                        tasks.push(Task::Siblings(dom.first_child(node), depth + 1, mode))
                    }
                }
            }
        }
    }
    crate::content::Html {
        markup: output,
        has_content: content.mark() != 0,
    }
}

fn allowed(tag: &str) -> bool {
    crate::structure::is_structural_block(tag)
        || matches!(
            tag,
            "select"
                | "colgroup"
                | "col"
                | "optgroup"
                | "option"
                | "button"
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
                | "strike"
                | "del"
                | "sup"
                | "sub"
                | "code"
        )
}

pub(crate) fn escape_attribute(text: &str, output: &mut String) {
    for ch in text.chars() {
        if ch == '|' {
            // Keep raw HTML attributes independent of GFM pipe escaping.
            output.push_str("&#124;");
        } else {
            escape_character(ch, output);
        }
    }
}

fn escape_character(ch: char, output: &mut String) {
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

struct TextEscaper {
    inline_root: bool,
    keep_newlines: bool,
    preformatted_code: bool,
    trailing_cr: bool,
}

impl TextEscaper {
    fn write(&mut self, text: &str, output: &mut String, mode: TextMode) {
        let code = !matches!(mode, TextMode::Normal);
        let normalize_newlines = matches!(mode, TextMode::Code) && self.preformatted_code;
        for ch in text.chars() {
            let previous_cr = self.trailing_cr;
            self.trailing_cr = normalize_newlines && ch == '\r';
            // Treat CRLF as one line ending, including across inline wrappers.
            if normalize_newlines && ch == '\n' && previous_cr {
                continue;
            }
            let ch = if normalize_newlines && matches!(ch, '\r' | '\n') {
                ' '
            } else {
                ch
            };
            if !code && matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
                if !output.ends_with([' ', '\n']) {
                    output.push(' ');
                }
                continue;
            }
            if self.keep_newlines && code && ch == '\n' {
                output.push(ch);
            } else if code && self.inline_root && ch.is_ascii_punctuation() {
                write!(output, "&#{};", ch as u32).expect("writing to String");
            } else {
                escape_character(ch, output);
            }
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
