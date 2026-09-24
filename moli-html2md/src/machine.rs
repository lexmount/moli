use crate::output::Output;
use crate::table::Table;
use crate::visibility::nonrendered_serialized_state;
use crate::writer::{Style, Writer, longest_run};
use crate::{Dom, NodeKind, Options};
use std::collections::HashSet;

enum Task<'a, Id> {
    Visit(Id, usize),
    Children(Option<Id>, usize),
    Boundary,
    ChoiceBoundary,
    RestoreSvg(bool),
    PopStyle,
    EndLink(usize),
    EndQuote,
    EndHeading(usize),
    EndList(usize),
    EndItem(String),
    RawChildren(Option<Id>, usize, bool),
    EndRawBlock,
    EndCode,
    EndPre(Option<&'a str>),
    TableCell,
    EndTableCell,
}

struct List {
    output_depth: usize,
    ordered: bool,
    next: i64,
    loose: bool,
}

struct Machine<'a, D: Dom + ?Sized> {
    dom: &'a D,
    options: &'a Options,
    tasks: Vec<Task<'a, D::NodeId>>,
    writers: Vec<Writer<'a>>,
    lists: Vec<List>,
    tables: Vec<Table<D::NodeId>>,
    raw: String,
    serial: usize,
    in_svg: bool,
    anchor_targets: HashSet<String>,
    emitted_anchors: HashSet<String>,
}

pub(crate) fn convert<D: Dom + ?Sized>(dom: &D, root: D::NodeId, options: &Options) -> String {
    let mut machine = Machine {
        dom,
        options,
        tasks: vec![Task::Visit(root, 0)],
        writers: vec![Writer::default()],
        lists: Vec::new(),
        tables: Vec::new(),
        raw: String::new(),
        serial: 0,
        in_svg: false,
        anchor_targets: crate::anchors::referenced(dom, root, options.max_depth),
        emitted_anchors: HashSet::new(),
    };
    machine.run();
    machine.take_writer().into_string()
}

impl<'a, D: Dom + ?Sized> Machine<'a, D> {
    fn run(&mut self) {
        while let Some(task) = self.tasks.pop() {
            match task {
                Task::Visit(node, depth) => self.visit(node, depth),
                Task::Children(Some(node), depth) => {
                    // Keep one sibling continuation, not a vector of every
                    // child. Traversal storage is bounded by depth, not width.
                    self.tasks
                        .push(Task::Children(self.dom.next_sibling(node), depth));
                    self.tasks.push(Task::Visit(node, depth));
                }
                Task::Children(None, _) | Task::RawChildren(None, _, _) => {}
                Task::Boundary => self.writer().boundary(2),
                Task::ChoiceBoundary => self.writer().boundary(1),
                Task::RestoreSvg(previous) => self.in_svg = previous,
                Task::PopStyle => self.writer().pop_style(),
                Task::EndLink(serial) => self.writer().end_link(serial),
                Task::EndQuote => {
                    let content = self.take_writer().into_string();
                    let mut quote = String::new();
                    for line in content.lines() {
                        if !quote.is_empty() {
                            quote.push('\n');
                        }
                        quote.push('>');
                        if !line.is_empty() {
                            quote.push(' ');
                            quote.push_str(line);
                        }
                    }
                    self.writer().block(quote.into(), 2, 2);
                }
                Task::EndHeading(level) => {
                    let content = self.take_writer().into_string();
                    if !content.is_empty() {
                        let heading =
                            format!("{} {}", "#".repeat(level), content.replace('\n', " "));
                        self.writer().block(heading.into(), 2, 2);
                    } else {
                        self.writer().boundary(2);
                    }
                }
                Task::EndList(before) => {
                    self.lists.pop();
                    let content = self.take_writer();
                    let has_blocks = self.writer().has_blocks;
                    self.writer().block(content, before, 2);
                    if before == 1 {
                        // A nested list alone does not make its parent item loose.
                        self.writer().has_blocks = has_blocks;
                    }
                }
                Task::EndItem(marker) => {
                    let loose = self.writer().has_blocks;
                    let content = self.take_writer().into_string();
                    if content.is_empty() {
                        continue;
                    }
                    let mut item = String::new();
                    for (index, line) in content.lines().enumerate() {
                        if index == 0 {
                            item.push_str(&marker);
                        } else {
                            item.push('\n');
                            if !line.is_empty() {
                                for _ in 0..marker.len() {
                                    item.push(' ');
                                }
                            }
                        }
                        item.push_str(line);
                    }
                    let list = self.lists.last_mut().expect("list item belongs to a list");
                    let separation = if loose || list.loose { 2 } else { 1 };
                    list.loose |= loose;
                    self.writer().block(item.into(), separation, separation);
                }
                Task::RawChildren(Some(node), depth, inline) => self.raw_node(node, depth, inline),
                Task::EndRawBlock => {
                    if !self.raw.is_empty() && !self.raw.ends_with('\n') {
                        self.raw.push('\n');
                    }
                }
                Task::EndCode => {
                    let text = std::mem::take(&mut self.raw);
                    let preformatted = self.options.preformatted_code;
                    self.writer().code_with_edges(&text, preformatted);
                }
                Task::EndPre(language) => self.end_pre(language),
                Task::TableCell => self.table_cell(),
                Task::EndTableCell => {
                    let content = self.take_writer();
                    let table = self.tables.last_mut().expect("cell belongs to a table");
                    table.finish_cell(content);
                }
            }
        }
    }

    fn writer(&mut self) -> &mut Writer<'a> {
        self.writers.last_mut().expect("conversion has an output")
    }

    fn capture(&mut self) {
        let writer = self.writer().child();
        self.writers.push(writer);
    }

    fn take_writer(&mut self) -> Output {
        let writer = self.writers.pop().expect("capture has an output");
        if let Some(parent) = self.writers.last_mut() {
            parent.last_link = parent.last_link.max(writer.last_link);
        }
        writer.finish()
    }

    fn children(&mut self, node: D::NodeId, depth: usize) {
        if depth < self.options.max_depth {
            self.tasks
                .push(Task::Children(self.dom.first_child(node), depth));
        }
    }

    fn style(&mut self, style: Style<'a>) {
        if self.writer().push_style(style) {
            self.tasks.push(Task::PopStyle);
        }
    }

    fn visit(&mut self, node: D::NodeId, depth: usize) {
        if depth >= self.options.max_depth {
            return;
        }
        if self.dom.has_block_layout(node) {
            self.writer().boundary(2);
            self.tasks.push(Task::Boundary);
        } else if self.dom.has_text_boundary(node) {
            self.writer().boundary(1);
            self.tasks.push(Task::ChoiceBoundary);
        }
        let tag = match self.dom.node_kind(node) {
            NodeKind::Document => {
                self.children(node, depth + 1);
                return;
            }
            NodeKind::Text(text) => {
                self.writer().text(text);
                return;
            }
            NodeKind::Other => return,
            NodeKind::Element(tag) => tag,
        };
        if nonrendered_serialized_state(self.dom, node) {
            return;
        }
        let tooltip = self
            .dom
            .attribute(node, "data-toggle")
            .is_some_and(|value| value.eq_ignore_ascii_case("tooltip"));
        if tooltip
            && let Some(value) = self
                .dom
                .attribute(node, "data-original-title")
                .filter(|value| !value.trim().is_empty())
        {
            self.writer().text(value);
            return;
        }
        if tooltip {
            self.writer().boundary(1);
            self.tasks.push(Task::ChoiceBoundary);
        }
        if tag == "i"
            && self.dom.first_child(node).is_none()
            && self.dom.attribute(node, "class").is_some_and(|class| {
                class.split_ascii_whitespace().any(|token| {
                    let token = token.to_ascii_lowercase();
                    matches!(
                        token.as_str(),
                        "check" | "fa-check" | "icon-check" | "bi-check"
                    )
                })
            })
        {
            self.writer().text("✓");
            return;
        }
        if tag != "table"
            && !matches!(
                tag,
                "head" | "script" | "style" | "noscript" | "template" | "title"
            )
        {
            for attribute in ["id", "name"] {
                if attribute == "name" && tag != "a" {
                    continue;
                }
                if let Some(id) = self.dom.attribute(node, attribute)
                    && self.anchor_targets.contains(&crate::anchors::normalize(id))
                {
                    let id = crate::anchors::normalize(id);
                    if self.emitted_anchors.insert(id.clone()) {
                        self.writer().inline_html(&crate::anchors::markup(&id));
                    }
                }
            }
        }
        match tag {
            "math" => {
                let markup = crate::mathml::render(self.dom, node, self.options.max_depth - depth);
                self.writer().inline_html(&markup);
                return;
            }
            "head" | "script" | "style" | "noscript" | "template" => return,
            "title" if !self.in_svg => return,
            "svg" | "foreignObject" | "foreignobject" => {
                self.tasks.push(Task::RestoreSvg(self.in_svg));
                self.in_svg = tag == "svg";
            }
            "br" => {
                self.writer().hard_break();
                return;
            }
            "hr" => {
                self.writer().block("---".into(), 2, 2);
                return;
            }
            "select" | "optgroup" | "option" | "button" | "time" => {
                self.writer().boundary(1);
                if matches!(tag, "optgroup" | "option")
                    && let Some(label) = self.dom.attribute(node, "label")
                {
                    self.writer().text(label);
                    self.writer().boundary(1);
                    if tag == "option" {
                        return;
                    }
                }
                self.tasks.push(Task::ChoiceBoundary);
            }
            "strong" | "b" => self.style(Style::Strong),
            "em" | "i" => self.style(Style::Emphasis),
            "del" | "s" | "strike" => self.style(Style::Strike),
            "sup" => self.style(Style::Superscript),
            "sub" => self.style(Style::Subscript),
            "a" => {
                if let Some(href) = self
                    .dom
                    .attribute(node, "href")
                    .filter(|href| !href.is_empty() && crate::media::safe_url(href, false))
                {
                    self.serial += 1;
                    let serial = self.serial;
                    let style = Style::Link {
                        serial,
                        href,
                        title: self.dom.attribute(node, "title"),
                    };
                    if self.writer().push_style(style) {
                        self.tasks.push(Task::EndLink(serial));
                    }
                }
            }
            "img" => {
                let alt = self.dom.attribute(node, "alt").unwrap_or_default();
                let title = self.dom.attribute(node, "title");
                if let Some(src) = crate::media::source(self.dom, node) {
                    self.writer().image(alt, &src, title);
                } else {
                    // A failed or absent resource must not erase its textual
                    // alternative, especially when it is a link's only label.
                    self.writer().text(alt);
                }
                return;
            }
            "input" => {
                if let Some(value) = crate::form::input_text(self.dom, node) {
                    self.writer().text(&value);
                }
                return;
            }
            "iframe" | "video" | "audio" => {
                if self.media(node, tag, depth) {
                    return;
                }
                if is_block(tag) {
                    self.writer().boundary(2);
                    self.tasks.push(Task::Boundary);
                }
            }
            "code" | "pre" => {
                if subtree_has_link(self.dom, node, self.options.max_depth - depth) {
                    let html =
                        crate::html_table::render(self.dom, node, depth, self.options.max_depth);
                    if tag == "pre" {
                        self.writer().block(html.into(), 2, 2);
                    } else {
                        self.writer().inline_html(&html);
                    }
                    return;
                }
                self.raw.clear();
                let end = if tag == "pre" {
                    Task::EndPre(self.language(node))
                } else {
                    Task::EndCode
                };
                self.tasks.push(end);
                if depth + 1 < self.options.max_depth {
                    self.tasks.push(Task::RawChildren(
                        self.dom.first_child(node),
                        depth + 1,
                        tag == "code",
                    ));
                }
                return;
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let remaining = self.options.max_depth - depth;
                if !subtree_has_content(self.dom, node, remaining) {
                    self.writer().boundary(2);
                    return;
                }
                if subtree_has_nested_heading(self.dom, node, remaining) {
                    self.writer().boundary(2);
                    self.tasks.push(Task::Boundary);
                    self.children(node, depth + 1);
                    return;
                }
                if subtree_has_block(self.dom, node, remaining) {
                    let html =
                        crate::html_table::render(self.dom, node, depth, self.options.max_depth);
                    self.writer().block(html.into(), 2, 2);
                    return;
                }
                self.capture();
                self.writer().heading();
                self.tasks
                    .push(Task::EndHeading((tag.as_bytes()[1] - b'0') as usize));
            }
            "blockquote" => {
                self.capture();
                self.tasks.push(Task::EndQuote);
            }
            _ if quote_container(self.dom.attribute(node, "class")) => {
                self.capture();
                self.tasks.push(Task::EndQuote);
            }
            "ul" | "ol" => self.start_list(node, tag == "ol"),
            "li" => self.start_item(),
            "table" => {
                let presentation = self.dom.attribute(node, "role").is_some_and(|role| {
                    matches!(
                        role.trim().to_ascii_lowercase().as_str(),
                        "presentation" | "none"
                    )
                });
                if (!presentation
                    && crate::html_table::needed(self.dom, node, depth, self.options.max_depth))
                    || (!self.anchor_targets.is_empty()
                        && crate::anchors::contains(
                            self.dom,
                            node,
                            self.options.max_depth - depth,
                            &self.anchor_targets,
                        ))
                {
                    let html =
                        crate::html_table::render(self.dom, node, depth, self.options.max_depth);
                    self.writer().block(html.into(), 2, 2);
                    return;
                }
                if let Some(mut table) =
                    Table::from_dom(self.dom, node, depth, self.options.max_depth)
                {
                    let captions = table.take_captions();
                    self.tables.push(table);
                    self.tasks.push(Task::TableCell);
                    for caption in captions.into_iter().rev() {
                        self.tasks.push(Task::Visit(caption.node, caption.depth));
                    }
                    return;
                }
                if !presentation
                    && crate::html_table::has_cells(self.dom, node, depth, self.options.max_depth)
                {
                    let html =
                        crate::html_table::render(self.dom, node, depth, self.options.max_depth);
                    self.writer().block(html.into(), 2, 2);
                    return;
                }
                self.writer().boundary(2);
                self.tasks.push(Task::Boundary);
            }
            _ if is_block(tag) => {
                self.writer().boundary(2);
                self.tasks.push(Task::Boundary);
            }
            _ => {}
        }
        if let Some((math, relative_depth)) =
            crate::mathml::primary_alternative(self.dom, node, self.options.max_depth - depth)
        {
            let markup = crate::mathml::render(
                self.dom,
                math,
                self.options.max_depth - depth - relative_depth,
            );
            self.writer().inline_html(&markup);
            return;
        }
        if self.dom.first_child(node).is_none()
            && let Some(label) = self
                .dom
                .attribute(node, "aria-label")
                .filter(|label| !label.trim().is_empty())
        {
            self.writer().text(label);
            return;
        }
        self.children(node, depth + 1);
    }

    fn media(&mut self, node: D::NodeId, tag: &str, depth: usize) -> bool {
        let urls = crate::media::sources(self.dom, node, tag, depth, self.options.max_depth);
        if urls.is_empty() {
            return false;
        }
        self.writer().boundary(2);
        if tag == "video"
            && let Some(poster) = self
                .dom
                .attribute(node, "poster")
                .filter(|src| !src.is_empty())
        {
            self.writer().image("", poster, None);
            self.writer().boundary(1);
        }
        let label = crate::media::label(self.dom, node, tag);
        for href in urls {
            self.serial += 1;
            let serial = self.serial;
            let opened = self.writer().push_style(Style::Link {
                serial,
                href,
                title: None,
            });
            self.writer().text(label);
            if opened {
                self.writer().end_link(serial);
            }
            self.writer().boundary(1);
        }
        self.writer().boundary(2);
        true
    }

    fn table_cell(&mut self) {
        let table = self.tables.last_mut().expect("table conversion is active");
        if let Some(cell) = table.next_cell() {
            self.capture();
            self.tasks.push(Task::TableCell);
            self.tasks.push(Task::EndTableCell);
            self.children(cell.node, cell.depth + 1);
        } else {
            let table = self.tables.pop().expect("table conversion is active");
            self.writer().block(table.finish(), 2, 2);
        }
    }

    fn start_list(&mut self, node: D::NodeId, ordered: bool) {
        let before = if self
            .lists
            .last()
            .is_some_and(|list| self.writers.len() == list.output_depth + 1)
        {
            1
        } else {
            2
        };
        let next = self
            .dom
            .attribute(node, "start")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1);
        self.capture();
        self.lists.push(List {
            output_depth: self.writers.len(),
            ordered,
            next,
            loose: false,
        });
        self.tasks.push(Task::EndList(before));
    }

    fn start_item(&mut self) {
        let Some(list) = self
            .lists
            .last_mut()
            .filter(|list| list.output_depth == self.writers.len())
        else {
            self.writer().boundary(2);
            self.tasks.push(Task::Boundary);
            return;
        };
        let marker = if list.ordered {
            let marker = format!("{}. ", list.next);
            list.next = list.next.saturating_add(1);
            marker
        } else {
            "- ".to_owned()
        };
        self.capture();
        self.tasks.push(Task::EndItem(marker));
    }

    fn language(&self, node: D::NodeId) -> Option<&'a str> {
        let mut child = self.dom.first_child(node);
        while let Some(id) = child {
            if self.dom.node_kind(id) == NodeKind::Element("code")
                && let Some(language) = class_language(self.dom.attribute(id, "class"))
            {
                return Some(language);
            }
            child = self.dom.next_sibling(id);
        }
        class_language(self.dom.attribute(node, "class"))
            .or(self.options.default_code_language.as_deref())
    }

    fn raw_node(&mut self, node: D::NodeId, depth: usize, inline: bool) {
        self.tasks.push(Task::RawChildren(
            self.dom.next_sibling(node),
            depth,
            inline,
        ));
        if depth >= self.options.max_depth {
            return;
        }
        if nonrendered_serialized_state(self.dom, node) {
            return;
        }
        if self.dom.attribute(node, "class").is_some_and(|class| {
            class.split_ascii_whitespace().any(|token| {
                let token = token.to_ascii_lowercase();
                token.contains("copy") && (token.contains("btn") || token.contains("button"))
            })
        }) {
            return;
        }
        match self.dom.node_kind(node) {
            NodeKind::Text(text) => self.raw.push_str(text),
            NodeKind::Element("script" | "style" | "template" | "noscript") => {}
            // Inline code normalizes line endings to spaces. Keep explicit
            // breaks without converting descendant formatting into Markdown.
            NodeKind::Element("br") => self.raw.push('\n'),
            NodeKind::Document | NodeKind::Element(_) if depth + 1 < self.options.max_depth => {
                if !inline
                    && matches!(self.dom.node_kind(node), NodeKind::Element(tag) if is_structural_block(tag))
                {
                    if !self.raw.is_empty() && !self.raw.ends_with('\n') {
                        self.raw.push('\n');
                    }
                    self.tasks.push(Task::EndRawBlock);
                }
                self.tasks.push(Task::RawChildren(
                    self.dom.first_child(node),
                    depth + 1,
                    inline,
                ));
            }
            _ => {}
        }
    }

    fn end_pre(&mut self, language: Option<&str>) {
        let text = std::mem::take(&mut self.raw);
        if text.trim().is_empty() {
            self.writer().boundary(2);
            return;
        }
        // CommonMark code blocks normalize CRLF/CR and require a closing fence
        // on its own line. Preserve all other spaces and blank lines verbatim.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        // The info string occupies one line. Tilde fences also allow language
        // names containing backticks without changing the name itself.
        let language = language
            .unwrap_or_default()
            .split(['\r', '\n'])
            .next()
            .unwrap();
        let marker = if language.contains('`') { '~' } else { '`' };
        let fence = marker
            .to_string()
            .repeat(3.max(longest_run(&text, marker) + 1));
        let mut block = fence.clone();
        for ch in language.chars() {
            match ch {
                '\\' => block.push_str("\\\\"),
                '&' => block.push_str("&amp;"),
                _ => block.push(ch),
            }
        }
        block.push('\n');
        block.push_str(&text);
        if !text.is_empty() && !text.ends_with('\n') {
            block.push('\n');
        }
        block.push_str(&fence);
        self.writer().block(block.into(), 2, 2);
    }
}

fn quote_container(class: Option<&str>) -> bool {
    class.is_some_and(|class| {
        class.split_ascii_whitespace().any(|token| {
            matches!(
                token.to_ascii_lowercase().as_str(),
                "quote" | "quoteblock" | "quote-block" | "quoted-text"
            )
        })
    })
}

fn subtree_has_content<D: Dom + ?Sized>(dom: &D, root: D::NodeId, limit: usize) -> bool {
    let mut pending = vec![(root, 0)];
    while let Some((node, depth)) = pending.pop() {
        match dom.node_kind(node) {
            NodeKind::Text(text) if !text.trim_matches(char::is_whitespace).is_empty() => {
                return true;
            }
            NodeKind::Element("img")
                if dom
                    .attribute(node, "alt")
                    .is_some_and(|alt| !alt.trim().is_empty()) =>
            {
                return true;
            }
            _ => {}
        }
        if depth + 1 >= limit {
            continue;
        }
        let mut child = dom.first_child(node);
        while let Some(id) = child {
            pending.push((id, depth + 1));
            child = dom.next_sibling(id);
        }
    }
    false
}

fn subtree_has_nested_heading<D: Dom + ?Sized>(dom: &D, root: D::NodeId, limit: usize) -> bool {
    let mut pending = Vec::new();
    let mut child = dom.first_child(root);
    while let Some(id) = child {
        pending.push((id, 1));
        child = dom.next_sibling(id);
    }
    while let Some((node, depth)) = pending.pop() {
        if matches!(
            dom.node_kind(node),
            NodeKind::Element("h1" | "h2" | "h3" | "h4" | "h5" | "h6")
        ) {
            return true;
        }
        if depth + 1 >= limit {
            continue;
        }
        let mut child = dom.first_child(node);
        while let Some(id) = child {
            pending.push((id, depth + 1));
            child = dom.next_sibling(id);
        }
    }
    false
}

fn subtree_has_link<D: Dom + ?Sized>(dom: &D, root: D::NodeId, limit: usize) -> bool {
    let mut pending = vec![(dom.first_child(root), 1)];
    while let Some((node, depth)) = pending.pop() {
        let Some(node) = node else {
            continue;
        };
        if depth >= limit {
            continue;
        }
        pending.push((dom.next_sibling(node), depth));
        if matches!(dom.node_kind(node), NodeKind::Element("a"))
            && dom
                .attribute(node, "href")
                .is_some_and(|href| !href.is_empty() && crate::media::safe_url(href, false))
        {
            return true;
        }
        pending.push((dom.first_child(node), depth + 1));
    }
    false
}

fn subtree_has_block<D: Dom + ?Sized>(dom: &D, root: D::NodeId, limit: usize) -> bool {
    let mut pending = vec![(dom.first_child(root), 1)];
    while let Some((node, depth)) = pending.pop() {
        let Some(node) = node else {
            continue;
        };
        if depth >= limit {
            continue;
        }
        pending.push((dom.next_sibling(node), depth));
        if matches!(dom.node_kind(node), NodeKind::Element(tag) if is_structural_block(tag)) {
            return true;
        }
        pending.push((dom.first_child(node), depth + 1));
    }
    false
}

fn class_language(class: Option<&str>) -> Option<&str> {
    class?
        .split_ascii_whitespace()
        .find_map(|part| part.strip_prefix("language-").filter(|s| !s.is_empty()))
}

fn is_block(tag: &str) -> bool {
    is_structural_block(tag)
        || matches!(
            tag,
            "audio" | "canvas" | "frameset" | "isindex" | "noframes" | "output"
        )
}

fn is_structural_block(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "body"
            | "caption"
            | "center"
            | "dd"
            | "details"
            | "dialog"
            | "dir"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "html"
            | "legend"
            | "li"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "summary"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
            | "ul"
    )
}
