use crate::{Dom, NodeKind, output::Output, writer::is_space};

pub(crate) struct Cell<Id> {
    pub(crate) node: Id,
    pub(crate) depth: usize,
}

pub(crate) struct Table<Id> {
    captions: Vec<Cell<Id>>,
    cells: std::vec::IntoIter<Cell<Id>>,
    content: Vec<Output>,
    in_cell: bool,
    rows: Vec<usize>,
    separators: Vec<&'static str>,
    has_header: bool,
}

impl<Id: Copy> Table<Id> {
    /// Inspect only the table's sections, rows and cells. Cell descendants are
    /// converted by the main task stack, so nested tables never recurse or
    /// repeatedly scan their descendants to decide how to render an ancestor.
    pub(crate) fn from_dom<D: Dom<NodeId = Id> + ?Sized>(
        dom: &D,
        node: Id,
        depth: usize,
        max_depth: usize,
    ) -> Option<Self> {
        if dom.attribute(node, "role").is_some_and(|role| {
            let role = role.trim_matches(is_space);
            role.eq_ignore_ascii_case("presentation") || role.eq_ignore_ascii_case("none")
        }) {
            return None;
        }
        let mut table = Self {
            captions: Vec::new(),
            cells: Vec::new().into_iter(),
            content: Vec::new(),
            in_cell: false,
            rows: Vec::new(),
            separators: Vec::new(),
            has_header: false,
        };
        let mut cells = Vec::new();
        let mut child = dom.first_child(node);
        while let Some(node) = child.filter(|_| depth + 1 < max_depth) {
            child = dom.next_sibling(node);
            match dom.node_kind(node) {
                NodeKind::Element("caption") if table.rows.is_empty() => {
                    table.captions.push(Cell {
                        node,
                        depth: depth + 1,
                    });
                }
                NodeKind::Element("tr") => {
                    table.add_row(dom, node, depth + 1, max_depth, "table", &mut cells)?;
                }
                NodeKind::Element(tag @ ("thead" | "tbody" | "tfoot")) => {
                    let mut row = dom.first_child(node);
                    while let Some(node) = row.filter(|_| depth + 2 < max_depth) {
                        row = dom.next_sibling(node);
                        match dom.node_kind(node) {
                            NodeKind::Element("tr") => {
                                table.add_row(dom, node, depth + 2, max_depth, tag, &mut cells)?
                            }
                            kind if ignorable(kind) => {}
                            _ => return None,
                        }
                    }
                }
                NodeKind::Element("colgroup" | "col") => {}
                kind if ignorable(kind) => {}
                _ => return None,
            }
        }
        if table.rows.is_empty() {
            return None;
        }
        table.cells = cells.into_iter();
        Some(table)
    }

    fn add_row<D: Dom<NodeId = Id> + ?Sized>(
        &mut self,
        dom: &D,
        node: Id,
        depth: usize,
        max_depth: usize,
        section: &str,
        cells: &mut Vec<Cell<Id>>,
    ) -> Option<()> {
        let start = cells.len();
        let mut all_headers = true;
        let mut child = dom.first_child(node);
        while let Some(node) = child.filter(|_| depth + 1 < max_depth) {
            child = dom.next_sibling(node);
            match dom.node_kind(node) {
                NodeKind::Element(tag @ ("th" | "td")) => {
                    // GFM has no cell spans. Never use their values to allocate
                    // a grid or silently shift the following cells left.
                    if ["colspan", "rowspan"].iter().any(|name| {
                        dom.attribute(node, name)
                            .is_some_and(|span| span.trim().parse::<u32>() != Ok(1))
                    }) {
                        return None;
                    }
                    all_headers &= tag == "th";
                    cells.push(Cell {
                        node,
                        depth: depth + 1,
                    });
                    // The first cell encountered in each column supplies its
                    // alignment. A synthetic header can grow to fit later rows.
                    if !self.has_header && cells.len() - start > self.separators.len() {
                        let align = dom.attribute(node, "align").unwrap_or_default().trim();
                        self.separators.push(if align.eq_ignore_ascii_case("left") {
                            ":---"
                        } else if align.eq_ignore_ascii_case("right") {
                            "---:"
                        } else if align.eq_ignore_ascii_case("center") {
                            ":---:"
                        } else {
                            "---"
                        });
                    }
                }
                kind if ignorable(kind) => {}
                _ => return None,
            }
        }
        let count = cells.len() - start;
        if count == 0 {
            return Some(());
        }
        if self.rows.is_empty() {
            // Recognize explicit headings as in Turndown's GFM plugin. When
            // absent, synthesize empty headings without consuming a data row.
            self.has_header = section == "thead" || (section != "tfoot" && all_headers);
        } else if section == "thead" || (self.has_header && count > self.separators.len()) {
            // GFM supports a single heading row.
            // Markdown parsers discard cells beyond the header's width.
            return None;
        }
        self.rows.push(count);
        Some(())
    }

    pub(crate) fn take_captions(&mut self) -> Vec<Cell<Id>> {
        std::mem::take(&mut self.captions)
    }

    pub(crate) fn next_cell(&mut self) -> Option<Cell<Id>> {
        debug_assert!(
            !self.in_cell,
            "finish the current cell before starting another"
        );
        let cell = self.cells.next()?;
        self.in_cell = true;
        Some(cell)
    }

    pub(crate) fn finish_cell(&mut self, content: Output) {
        debug_assert!(self.in_cell, "cell conversion must be active");
        self.content.push(content);
        self.in_cell = false;
    }

    pub(crate) fn finish(self) -> Output {
        if self.content.iter().all(Output::is_empty) {
            return Output::default();
        }
        let mut output = String::new();
        if !self.has_header {
            output.push('|');
            for _ in &self.separators {
                output.push_str("  |");
            }
            separator_row(&mut output, &self.separators);
        }
        let mut content = self.content.into_iter();
        for (row, count) in self.rows.into_iter().enumerate() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push('|');
            // GFM supplies missing trailing cells. Keeping short rows short
            // avoids expanding sparse input to rows * header width bytes.
            for _ in 0..count {
                output.push(' ');
                let cell = content
                    .next()
                    .expect("every table cell was converted")
                    .into_string();
                for (line, text) in cell.split('\n').enumerate() {
                    if line > 0 {
                        output.push_str("<br>");
                    }
                    for ch in text.trim_end_matches(is_space).chars() {
                        if ch == '|' {
                            // GFM removes this escape before parsing inline
                            // content, including code spans and link URLs.
                            output.push('\\');
                        }
                        output.push(ch);
                    }
                }
                output.push_str(" |");
            }
            if row == 0 && self.has_header {
                separator_row(&mut output, &self.separators);
            }
        }
        output.into()
    }
}

fn separator_row(output: &mut String, separators: &[&str]) {
    output.push_str("\n|");
    for separator in separators {
        output.push(' ');
        output.push_str(separator);
        output.push_str(" |");
    }
}

fn ignorable(kind: NodeKind<'_>) -> bool {
    match kind {
        NodeKind::Other | NodeKind::Element("script" | "style" | "noscript" | "template") => true,
        NodeKind::Text(text) => text.chars().all(is_space),
        _ => false,
    }
}
