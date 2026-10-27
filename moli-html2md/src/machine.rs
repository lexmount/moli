use crate::output::Output;
use crate::structure::{is_block, is_structural_block, owns_boundaries};
use crate::table::Table;
use crate::visibility::nonrendered_serialized_state;
use crate::writer::{Style, Writer, longest_run};
use crate::{Dom, NodeKind, Options};
use std::collections::HashSet;

enum Task<'a, Id> {
    Visit(Id, usize),
    Children(Option<Id>, usize),
    MathChildren(Option<Id>, usize),
    Boundary,
    LayoutBoundary,
    ChoiceBoundary,
    InlineSpace,
    Fallback(String, usize),
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
    anchor_targets: crate::anchors::Targets,
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
                Task::MathChildren(Some(node), depth) => {
                    self.tasks
                        .push(Task::MathChildren(self.dom.next_sibling(node), depth));
                    // The recognized group contains one semantic branch,
                    // whitespace and hidden copies. Keep text in DOM order.
                    if !crate::mathml::is_hidden_alternative(self.dom, node) {
                        self.tasks.push(Task::Visit(node, depth));
                    }
                }
                Task::Children(None, _)
                | Task::MathChildren(None, _)
                | Task::RawChildren(None, _, _) => {}
                Task::Boundary => self.writer().boundary(2),
                Task::LayoutBoundary => self.writer().layout_boundary(),
                Task::InlineSpace => self.writer().text(" "),
                Task::ChoiceBoundary => self.writer().boundary(1),
                Task::Fallback(label, before) => {
                    if self.writer().content_mark() == before {
                        self.writer().text(" ");
                        self.writer().text(&label);
                        self.writer().text(" ");
                    }
                }
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
                    self.writer().append_block(quote.into(), 2, 2);
                }
                Task::EndHeading(level) => {
                    let content = self.take_writer().into_string();
                    if !content.is_empty() {
                        let heading = format!(
                            "{} {}",
                            "#".repeat(level),
                            content
                                .lines()
                                .map(|line| line.trim_matches(crate::writer::is_space))
                                .filter(|line| !line.is_empty())
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                        self.writer().append_block(heading.into(), 2, 2);
                    } else {
                        self.writer().boundary(2);
                    }
                }
                Task::EndList(before) => {
                    self.lists.pop();
                    let content = self.take_writer();
                    let has_blocks = self.writer().has_blocks;
                    self.writer().append_block(content, before, 2);
                    if before == 1 {
                        // A nested list at the end keeps its parent item tight;
                        // following content can still realize a blank-line boundary.
                        self.writer().has_blocks = has_blocks;
                    }
                }
                Task::EndItem(marker) => {
                    self.writer().materialize_pending();
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
                    self.writer()
                        .append_block(item.into(), separation, separation);
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
        let mut writer = self.writers.pop().expect("capture has an output");
        writer.materialize_pending();
        if let Some(parent) = self.writers.last_mut() {
            parent.merge_child(&writer);
        }
        writer.finish()
    }

    fn html(&mut self, node: D::NodeId, depth: usize, block: bool) {
        let formatting = self.writer().html_context(block);
        let html = crate::html_fallback::render(
            self.dom,
            node,
            depth,
            self.options,
            &self.anchor_targets,
            &mut self.emitted_anchors,
            formatting,
        );
        self.writer().html(html, block);
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
        let owns_boundaries =
            matches!(self.dom.node_kind(node), NodeKind::Element(tag) if owns_boundaries(tag));
        if !owns_boundaries && self.dom.has_block_layout(node) {
            self.writer().layout_boundary();
            self.tasks.push(Task::LayoutBoundary);
        } else if !owns_boundaries && self.dom.has_text_boundary(node) {
            self.writer().text(" ");
            self.tasks.push(Task::InlineSpace);
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
        if tag != "table"
            && !matches!(
                tag,
                "head"
                    | "script"
                    | "style"
                    | "noscript"
                    | "template"
                    | "title"
                    | "pre"
                    | "code"
                    | "li"
                    | "blockquote"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
            )
        {
            for id in self.anchor_targets.targets(self.dom, node) {
                if self.emitted_anchors.insert(id.to_owned()) {
                    self.writer().anchor(id);
                }
            }
        }
        match tag {
            "math" => {
                self.emit_anchors_within(node, self.options.max_depth - depth);
                let markup = crate::mathml::render(self.dom, node, self.options.max_depth - depth);
                self.writer().inline_html(&markup);
                return;
            }
            "head" | "script" | "style" | "noscript" | "template" => return,
            "title" => return,
            "br" => {
                self.writer().hard_break();
                return;
            }
            "hr" => {
                self.writer().block("---".into(), 2, 2);
                return;
            }
            "select" | "optgroup" | "option" => {
                self.writer().boundary(1);
                if matches!(tag, "optgroup" | "option")
                    && let Some(label) = self
                        .dom
                        .attribute(node, "label")
                        .filter(|label| !label.is_empty())
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
                if let Some(href) = crate::media::link_href(self.dom, node) {
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
                let alt = crate::content::image_label(self.dom, node);
                let title = self.dom.attribute(node, "title");
                if let Some(src) = crate::media::source(self.dom, node) {
                    self.writer().image(alt, src, title);
                } else {
                    // A failed or absent resource must not erase its textual
                    // alternative, especially when it is a link's only label.
                    self.writer().text(alt);
                }
                return;
            }
            "input" => {
                if let Some(value) = crate::form::input_text(self.dom, node) {
                    self.writer().text(" ");
                    self.writer().text(value);
                    self.writer().text(" ");
                }
                return;
            }
            "textarea" => {
                if let Some(value) = self.dom.control_value(node) {
                    self.writer().text(value);
                } else {
                    self.children(node, depth + 1);
                }
                return;
            }
            "iframe" | "video" | "audio" if self.media(node, tag, depth) => return,
            "code" | "pre" => {
                let remaining = self.options.max_depth - depth;
                if subtree_has_link(self.dom, node, remaining)
                    || (!self.anchor_targets.is_empty()
                        && crate::anchors::contains(
                            self.dom,
                            node,
                            remaining,
                            &self.anchor_targets,
                        ))
                {
                    self.html(node, depth, tag == "pre");
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
                if subtree_has_nested_heading(self.dom, node, remaining) {
                    self.writer().boundary(2);
                    self.emit_own_targets(node);
                    self.tasks.push(Task::Boundary);
                    self.children(node, depth + 1);
                    return;
                }
                if subtree_has_block(self.dom, node, remaining) {
                    self.html(node, depth, true);
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
            "ul" | "ol" => self.start_list(node, tag == "ol"),
            "li" => self.start_item(),
            "table" => {
                let presentation = crate::table::is_presentational(self.dom, node);
                let structural_table = !presentation
                    && crate::html_fallback::requires_table_structure(
                        self.dom,
                        node,
                        depth,
                        self.options.max_depth,
                    );
                let complex_table =
                    crate::html_fallback::needed(self.dom, node, depth, self.options.max_depth);
                if structural_table
                    && (complex_table
                        || (!self.anchor_targets.is_empty()
                            && crate::anchors::contains(
                                self.dom,
                                node,
                                self.options.max_depth - depth,
                                &self.anchor_targets,
                            )))
                {
                    self.html(node, depth, true);
                    return;
                }
                self.emit_own_targets(node);
                if !complex_table
                    && let Some(mut table) =
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
                if structural_table
                    && crate::html_fallback::has_cells(
                        self.dom,
                        node,
                        depth,
                        self.options.max_depth,
                    )
                {
                    self.html(node, depth, true);
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
        if matches!(
            tag,
            "li" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        ) {
            self.emit_own_targets(node);
        }
        if crate::mathml::primary_alternative(self.dom, node, self.options.max_depth - depth)
            .is_some()
        {
            self.emit_anchors_within(node, self.options.max_depth - depth);
            self.tasks
                .push(Task::MathChildren(self.dom.first_child(node), depth + 1));
            return;
        }
        if let Some(label) =
            crate::content::fallback_text(self.dom, node, self.options.max_depth - depth)
        {
            let before = self.writer().content_mark();
            self.tasks.push(Task::Fallback(label, before));
        }
        self.children(node, depth + 1);
    }

    fn emit_anchors_within(&mut self, node: D::NodeId, remaining: usize) {
        for id in crate::anchors::within(self.dom, node, remaining, &self.anchor_targets) {
            if self.emitted_anchors.insert(id.clone()) {
                self.writer().anchor(&id);
            }
        }
    }

    fn emit_own_targets(&mut self, node: D::NodeId) {
        for id in self.anchor_targets.targets(self.dom, node) {
            if self.emitted_anchors.insert(id.to_owned()) {
                self.writer().anchor(id);
            }
        }
    }

    fn media(&mut self, node: D::NodeId, tag: &str, depth: usize) -> bool {
        let urls = crate::media::sources(self.dom, node, tag, depth, self.options.max_depth);
        let poster = crate::media::poster(self.dom, node, tag);
        if urls.is_empty() && poster.is_none() {
            return false;
        }
        let block = is_block(tag);
        if block {
            self.writer().boundary(2);
        } else {
            self.writer().text(" ");
        }
        let label = crate::media::label(self.dom, node, tag);
        if let Some(poster) = poster {
            self.writer()
                .image(&format!("{label} poster"), poster, None);
            self.writer().text(" ");
        }
        let has_source = !urls.is_empty();
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
            } else {
                // Markdown cannot nest a media link inside a surrounding link.
                // Retain its destination as text rather than silently losing it.
                self.writer().text(" (");
                self.writer().text(href);
                self.writer().text(")");
            }
            self.writer().text(" ");
        }
        if block {
            self.writer().boundary(2);
        } else {
            self.writer().text(" ");
        }
        has_source
    }

    fn table_cell(&mut self) {
        let table = self.tables.last_mut().expect("table conversion is active");
        let mut targets = Vec::new();
        while let Some(node) = table.next_target() {
            targets.push(node);
        }
        if let Some(cell) = table.next_cell() {
            self.capture();
            for node in targets {
                self.emit_own_targets(node);
            }
            if let Some(row) = cell.row {
                self.emit_own_targets(row);
            }
            self.emit_own_targets(cell.node);
            self.tasks.push(Task::TableCell);
            self.tasks.push(Task::EndTableCell);
            self.children(cell.node, cell.depth + 1);
        } else {
            let table = self.tables.pop().expect("table conversion is active");
            self.writer().append_block(table.finish(), 2, 2);
            for node in targets {
                self.emit_own_targets(node);
            }
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
        crate::content::code_language(
            self.dom,
            node,
            self.options.default_code_language.as_deref(),
        )
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
        if crate::content::is_code_control(self.dom, node) {
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
                .is_some_and(|href| !href.is_empty() && crate::media::safe_link_url(href))
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
        if matches!(
            dom.node_kind(node),
            NodeKind::Element("table" | "pre" | "blockquote" | "ul" | "ol" | "hr")
        ) {
            return true;
        }
        pending.push((dom.first_child(node), depth + 1));
    }
    false
}
