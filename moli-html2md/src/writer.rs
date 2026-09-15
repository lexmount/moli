// Formatting is delayed until visible content arrives. This keeps whitespace
// outside newly opened/closed marks and coalesces adjacent identical styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Style<'a> {
    Strong,
    Emphasis,
    Strike,
    Link {
        // Distinguish adjacent links even when their destinations match.
        serial: usize,
        href: &'a str,
        title: Option<&'a str>,
    },
}

struct OpenStyle<'a> {
    style: Style<'a>,
    start: usize,
    html: bool,
    has_closed_child: bool,
}

#[derive(Default)]
pub(crate) struct Writer<'a> {
    output: String,
    desired: Vec<Style<'a>>,
    emitted: Vec<OpenStyle<'a>>,
    space: bool,
    preserved_spaces: String,
    breaks: usize,
    code: Option<String>,
    line_digits: Option<usize>,
    heading: bool,
    single_line_attributes: bool,
    pub(crate) last_link: usize,
    // Block events in this output buffer, used when finalizing a list item.
    pub(crate) has_blocks: bool,
}

pub(crate) fn is_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}')
}

impl<'a> Writer<'a> {
    pub(crate) fn child(&self) -> Self {
        Self {
            desired: self.desired.clone(),
            single_line_attributes: self.single_line_attributes,
            ..Self::default()
        }
    }

    pub(crate) fn heading(&mut self) {
        self.heading = true;
        self.single_line_attributes = true;
    }

    pub(crate) fn push_style(&mut self, style: Style<'a>) -> bool {
        // Repeated emphasis has the same visible meaning. Nested links cannot
        // be represented in Markdown, so retain the outer destination.
        if self.desired.contains(&style)
            || matches!(style, Style::Link { .. })
                && self.desired.iter().any(|s| matches!(s, Style::Link { .. }))
        {
            return false;
        }
        self.flush_code();
        self.desired.push(style);
        true
    }

    pub(crate) fn pop_style(&mut self) {
        self.flush_code();
        self.desired.pop();
    }

    pub(crate) fn end_link(&mut self, serial: usize) {
        self.flush_code();
        if self.last_link < serial {
            // Empty anchors still carry a destination. Materialize the link
            // after its leading whitespace, even without a text event.
            self.prepare_inline('[');
        }
        self.pop_style();
    }

    pub(crate) fn text(&mut self, text: &str) {
        self.flush_code();
        for ch in text.chars() {
            if is_space(ch) {
                self.space = true;
                continue;
            }
            if ch.is_whitespace() {
                // NBSP and other Unicode spaces remain visible characters,
                // but Markdown emphasis delimiters cannot touch whitespace.
                if self.space {
                    self.preserved_spaces.push(' ');
                    self.space = false;
                }
                self.preserved_spaces.push(ch);
                continue;
            }
            self.prepare_inline(ch);
            let line_start = self.output.is_empty() || self.output.ends_with('\n');
            if line_start {
                self.line_digits = Some(0);
            }
            match ch {
                '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' => {
                    self.output.push('\\');
                    self.output.push(ch);
                }
                '#' if line_start || self.heading => {
                    self.output.push('\\');
                    self.output.push(ch);
                }
                '+' | '-' | '=' if line_start => {
                    self.output.push('\\');
                    self.output.push(ch);
                }
                '.' | ')' if self.line_digits.is_some_and(|digits| digits > 0) => {
                    self.output.push('\\');
                    self.output.push(ch);
                }
                '&' => self.output.push_str("&amp;"),
                _ => self.output.push(ch),
            }
            self.line_digits = if ch.is_ascii_digit() {
                self.line_digits.map(|n| n + 1)
            } else {
                None
            };
        }
    }

    pub(crate) fn code(&mut self, text: &str) {
        if text.starts_with(is_space) {
            self.flush_code();
            self.space = true;
        }
        let text = text.trim_matches(is_space);
        if !text.is_empty() {
            if self.code.is_none() {
                self.prepare_inline('`');
                self.code = Some(String::new());
            }
            let code = self.code.as_mut().expect("code buffer was initialized");
            let mut space = false;
            for ch in text.chars() {
                if is_space(ch) {
                    space = true;
                } else {
                    if space {
                        code.push(' ');
                        space = false;
                    }
                    code.push(ch);
                }
            }
        }
    }

    pub(crate) fn code_with_edges(&mut self, text: &str, preformatted: bool) {
        if preformatted {
            if !text.is_empty() {
                if self.code.is_none() {
                    self.prepare_inline('`');
                    self.code = Some(String::new());
                }
                self.code
                    .as_mut()
                    .expect("initialized code buffer")
                    .push_str(&text.replace("\r\n", " ").replace(['\n', '\r'], " "));
            }
            return;
        }
        let content = text.trim_matches(char::is_whitespace);
        if content.is_empty() {
            self.text(text);
            return;
        }
        let leading = text.len() - text.trim_start_matches(char::is_whitespace).len();
        let trailing = text.trim_end_matches(char::is_whitespace).len();
        if leading > 0 {
            self.text(&text[..leading]);
        }
        self.code(content);
        if trailing < text.len() {
            self.flush_code();
            self.text(&text[trailing..]);
        }
    }

    pub(crate) fn image(&mut self, alt: &str, src: &str, title: Option<&str>) {
        self.flush_code();
        self.prepare_inline('!');
        self.output.push_str("![");
        // Image labels are literal text, not a second Markdown input.
        for ch in clean_attribute(alt) {
            match ch {
                '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>' => {
                    self.output.push('\\');
                    self.output.push(ch);
                }
                '&' => self.output.push_str("&amp;"),
                // Markdown image labels render soft breaks as spaces. Keep
                // that meaning when a heading flattens its content.
                '\n' if self.single_line_attributes => self.output.push(' '),
                _ => self.output.push(ch),
            }
        }
        self.output.push_str("](");
        destination(&mut self.output, src, title, self.single_line_attributes);
        self.output.push(')');
        self.line_digits = None;
    }

    pub(crate) fn boundary(&mut self, lines: usize) {
        self.has_blocks |= lines > 1;
        self.flush_code();
        self.close_to(0, None);
        self.flush_spaces(false);
        self.space = false;
        self.breaks = self.breaks.max(lines);
    }

    pub(crate) fn hard_break(&mut self) {
        self.flush_code();
        self.close_to(0, None);
        self.flush_spaces(false);
        self.flush_breaks();
        self.output.push_str("  \n");
        self.space = false;
        self.line_digits = Some(0);
    }

    pub(crate) fn block(&mut self, text: &str, before: usize, after: usize) {
        if text.is_empty() {
            self.boundary(before.max(after));
            return;
        }
        self.boundary(before);
        self.flush_breaks();
        self.output.push_str(text);
        self.line_digits = None;
        self.boundary(after);
    }

    pub(crate) fn finish(mut self) -> String {
        self.flush_code();
        self.close_to(0, None);
        self.flush_spaces(false);
        let end = self.output.trim_end_matches(is_space).len();
        self.output.truncate(end);
        self.output
    }

    fn prepare_inline(&mut self, next: char) {
        self.flush_breaks();
        let common = self
            .emitted
            .iter()
            .zip(&self.desired)
            .take_while(|(a, b)| a.style == **b)
            .count();
        // A new link emits '[' before its label. Include that character when
        // checking the boundaries of surrounding emphasis, even though style
        // emission is delayed until the first character of the label arrives.
        let link = self.desired[common..]
            .iter()
            .position(|style| matches!(style, Style::Link { .. }))
            .map(|index| common + index);
        let next_after_close = if self.space || !self.preserved_spaces.is_empty() {
            ' '
        } else if link.is_some() {
            '['
        } else {
            next
        };
        self.close_to(common, Some(next_after_close));
        self.flush_spaces(true);
        for (index, &style) in self.desired.iter().enumerate().skip(common) {
            let start = self.output.len();
            // An opening emphasis delimiter between a word and punctuation
            // cannot open a CommonMark span. Inline HTML preserves its meaning.
            let first = if link.is_some_and(|link| index < link) {
                '['
            } else {
                next
            };
            let html = is_punctuation(first)
                && self
                    .output
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
            match style {
                Style::Strong => self.output.push_str(if html { "<strong>" } else { "**" }),
                Style::Emphasis => self.output.push_str(if html { "<em>" } else { "*" }),
                Style::Strike => self.output.push_str(if html { "<del>" } else { "~~" }),
                Style::Link { serial, .. } => {
                    self.last_link = self.last_link.max(serial);
                    if self.output.ends_with('!') {
                        self.output.insert(self.output.len() - 1, '\\');
                    }
                    self.output.push('[');
                }
            }
            self.emitted.push(OpenStyle {
                style,
                start,
                html,
                has_closed_child: false,
            });
        }
    }

    fn flush_spaces(&mut self, trailing_ascii: bool) {
        if !self.preserved_spaces.is_empty() {
            // Materialize any block boundary before preserving Unicode space.
            self.flush_breaks();
            let text = if self.output.is_empty() || self.output.ends_with('\n') {
                self.preserved_spaces.trim_start_matches(' ')
            } else {
                &self.preserved_spaces
            };
            self.output.push_str(text);
            self.preserved_spaces.clear();
            self.line_digits = None;
        }
        if trailing_ascii && self.space && !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push(' ');
            self.line_digits = None;
        }
        self.space = false;
    }

    fn close_to(&mut self, count: usize, next: Option<char>) {
        let mut closing_run = None;
        let closed_child = self.emitted.len() > count;
        while self.emitted.len() > count {
            let opened = self.emitted.pop().expect("nonempty style stack");
            let delimiters = match opened.style {
                Style::Strong => Some(("**", "<strong>", "</strong>")),
                Style::Emphasis => Some(("*", "<em>", "</em>")),
                Style::Strike => Some(("~~", "<del>", "</del>")),
                Style::Link { href, title, .. } => {
                    self.output.push_str("](");
                    destination(&mut self.output, href, title, self.single_line_attributes);
                    self.output.push(')');
                    closing_run = None;
                    None
                }
            };
            if let Some((marker, open, close)) = delimiters {
                // Nested strong/emphasis closes with a single run of '*'. Its
                // left edge is the character before the run, not a delimiter
                // just emitted for the inner style. Links, HTML and a different
                // marker end that run and retain their own punctuation edge.
                let marker_byte = marker.as_bytes()[0];
                let preceding = match closing_run {
                    Some((previous, preceding))
                        if previous == marker_byte && !opened.has_closed_child =>
                    {
                        preceding
                    }
                    _ => self.output.chars().next_back(),
                };
                let closing_needs_html = next.is_some_and(char::is_alphanumeric)
                    && preceding.is_some_and(is_punctuation);
                if opened.html || closing_needs_html {
                    if !opened.html {
                        // Only the converter's output changes. Remaining open
                        // ancestors precede this offset, so their offsets stay valid.
                        self.output
                            .replace_range(opened.start..opened.start + marker.len(), open);
                    }
                    self.output.push_str(close);
                    closing_run = None;
                } else {
                    self.output.push_str(marker);
                    closing_run = Some((marker_byte, preceding));
                }
            }
        }
        if closed_child {
            // Earlier child delimiters can pair with a later sibling before
            // the parent's closer is reached: ***a**b**c***d loses emphasis.
            // A child rewritten as HTML can also change the opening edge.
            // Keep the conservative edge check for those interrupted spans.
            for opened in &mut self.emitted {
                opened.has_closed_child = true;
            }
        }
    }

    fn flush_breaks(&mut self) {
        if self.breaks > 0 {
            if !self.output.is_empty() {
                let existing = self.output.len() - self.output.trim_end_matches('\n').len();
                for _ in existing..self.breaks {
                    self.output.push('\n');
                }
            }
            self.line_digits = Some(0);
            self.breaks = 0;
        }
    }

    fn flush_code(&mut self) {
        if let Some(code) = self.code.take() {
            let fence = "`".repeat(longest_run(&code, '`') + 1);
            let padding = code.starts_with('`')
                || code.ends_with('`')
                || code.starts_with(' ') && code.ends_with(' ') && code.chars().any(|ch| ch != ' ');
            self.output.push_str(&fence);
            if padding {
                self.output.push(' ');
            }
            self.output.push_str(&code);
            if padding {
                self.output.push(' ');
            }
            self.output.push_str(&fence);
            self.line_digits = None;
        }
    }
}

fn is_punctuation(ch: char) -> bool {
    // Conservatively include Unicode symbols and combining marks; choosing
    // inline HTML for these edges is preferable to emitting literal delimiters.
    !ch.is_alphanumeric() && !ch.is_whitespace()
}

pub(crate) fn longest_run(text: &str, marker: char) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in text.chars() {
        if ch == marker {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn destination(output: &mut String, href: &str, title: Option<&str>, single_line: bool) {
    for ch in href.chars() {
        match ch {
            '\\' | '(' | ')' => {
                output.push('\\');
                output.push(ch);
            }
            ' ' => output.push_str("%20"),
            '\t' => output.push_str("%09"),
            '\r' => output.push_str("%0D"),
            '\n' => output.push_str("%0A"),
            '<' => output.push_str("%3C"),
            '>' => output.push_str("%3E"),
            '&' => output.push_str("&amp;"),
            _ => output.push(ch),
        }
    }
    if let Some(title) = title.filter(|title| !title.is_empty()) {
        output.push_str(" \"");
        for ch in clean_attribute(title) {
            match ch {
                '\\' | '"' => {
                    output.push('\\');
                    output.push(ch);
                }
                '&' => output.push_str("&amp;"),
                // Protect title newlines from heading line flattening.
                '\n' if single_line => output.push_str("&#10;"),
                _ => output.push(ch),
            }
        }
        output.push('"');
    }
}

// Attribute newlines are significant in labels and titles; indentation after a
// newline is not. Collapse each newline/whitespace run without flattening it.
fn clean_attribute(value: &str) -> impl Iterator<Item = char> + '_ {
    let mut after_newline = false;
    value.chars().filter(move |&ch| {
        if after_newline && ch.is_whitespace() {
            return false;
        }
        after_newline = ch == '\n';
        true
    })
}
