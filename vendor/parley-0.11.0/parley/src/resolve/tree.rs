// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hierarchical tree based style application.
use alloc::{string::String, vec::Vec};

use crate::style::{TextWrapMode, WhiteSpaceCollapse};

use super::{Brush, ResolvedProperty, ResolvedStyle, StyleRun};

#[derive(Debug, Clone)]
struct StyleTreeNode<B: Brush> {
    parent: Option<usize>,
    style: ResolvedStyle<B>,
    style_id: Option<u16>,
    /// The style's id when it is used for whitespace that wrapping is allowed after, which differs
    /// from `style_id` only for spans that disable wrapping.
    wrappable_style_id: Option<u16>,
}

/// A collapsible whitespace sequence that has not been committed yet.
#[derive(Debug, Clone, Copy)]
struct PendingWhitespace {
    /// The span the sequence started in, which the collapsed space is attributed to.
    span: usize,
    /// Whether any of the collapsed whitespace came from a span that allows wrapping, in which
    /// case the collapsed space is a soft wrap opportunity.
    wrappable: bool,
}

/// Whether `c` is a segment break, i.e. a character which is a forced line break when preserved.
fn is_segment_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// Builder for constructing a tree of styles
#[derive(Clone)]
pub(crate) struct TreeStyleBuilder<B: Brush> {
    tree: Vec<StyleTreeNode<B>>,
    style_table: Vec<ResolvedStyle<B>>,
    style_runs: Vec<StyleRun>,
    text: String,
    uncommitted_text: String,
    current_span: usize,
    /// The span that a not-yet-committed collapsible whitespace sequence belongs to.
    ///
    /// Collapsible whitespace is only committed once it is known to be followed by content in the
    /// same inline formatting context, so that it can collapse across span and inline box
    /// boundaries and be removed at the end of the text.
    pending_whitespace: Option<PendingWhitespace>,
    /// Whether the most recently pushed item is an inline box, in which case pending collapsible
    /// whitespace is not at the start of the inline formatting context.
    last_item_is_inline_box: bool,
}

impl<B: Brush> Default for TreeStyleBuilder<B> {
    fn default() -> Self {
        Self {
            tree: Vec::new(),
            style_table: Vec::new(),
            style_runs: Vec::new(),
            text: String::new(),
            uncommitted_text: String::new(),
            current_span: usize::MAX,
            pending_whitespace: None,
            last_item_is_inline_box: false,
        }
    }
}

impl<B: Brush> TreeStyleBuilder<B> {
    /// The style of the span that text is currently being pushed into.
    fn current_style(&self) -> ResolvedStyle<B> {
        self.tree[self.current_span].style.clone()
    }

    /// Prepares the builder for accepting a tree of styles and text.
    ///
    /// The provided `root_style` is the default style applied to all text unless overridden.
    pub(crate) fn begin(&mut self, root_style: ResolvedStyle<B>) {
        self.tree.clear();
        self.style_table.clear();
        self.style_runs.clear();
        self.text.clear();
        self.uncommitted_text.clear();
        self.pending_whitespace = None;
        self.last_item_is_inline_box = false;

        // The root style is always materialised at index 0 so that it can act as the parent of
        // every other style (and as the strut for otherwise empty lines).
        let mut root_style = root_style;
        root_style.parent = 0;
        self.style_table.push(root_style.clone());
        self.tree.push(StyleTreeNode {
            parent: None,
            style: root_style,
            style_id: Some(0),
            wrappable_style_id: None,
        });
        self.current_span = 0;
    }

    /// Records that an inline box has been pushed, so that following collapsible whitespace is not
    /// treated as whitespace at the start of the inline formatting context.
    pub(crate) fn set_last_item_is_inline_box(&mut self) {
        self.last_item_is_inline_box = true;
    }

    /// Applies white space processing to the buffered text and commits the result, leaving any
    /// trailing collapsible whitespace pending.
    pub(crate) fn commit_uncommitted_text(&mut self) {
        let uncommitted_text = core::mem::take(&mut self.uncommitted_text);
        if uncommitted_text.is_empty() {
            return;
        }

        let span = self.current_span;
        match self.tree[span].style.white_space_collapse {
            WhiteSpaceCollapse::Preserve | WhiteSpaceCollapse::BreakSpaces => {
                if uncommitted_text.starts_with(is_segment_break) {
                    // Pending whitespace is always from a `WhiteSpaceCollapse::Collapse` or
                    // `WhiteSpaceCollapse::PreserveBreaks` span, and following CSS Text 4 § 4.3.1 Rule 1
                    // must be removed if it immediately precedes a preserved segment break.
                    self.pending_whitespace = None;
                }
                self.flush_pending_whitespace();
                self.commit_text(span, &uncommitted_text);
            }
            mode @ (WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::PreserveBreaks) => {
                let mut rest = uncommitted_text.as_str();
                while !rest.is_empty() {
                    let whitespace_len =
                        rest.find(|c| !mode.is_collapsible(c)).unwrap_or(rest.len());
                    if whitespace_len > 0 {
                        // The collapsed space is attributed to the span the whitespace sequence
                        // started in, but is a wrap opportunity if any of the spans it collapses
                        // whitespace from allows wrapping.
                        let wrappable = self.tree[span].style.text_wrap_mode == TextWrapMode::Wrap;
                        let pending = self
                            .pending_whitespace
                            .get_or_insert(PendingWhitespace { span, wrappable });
                        pending.wrappable |= wrappable;
                        rest = &rest[whitespace_len..];
                        continue;
                    }

                    if rest.starts_with(is_segment_break) {
                        // Collapsible whitespace immediately preceding a segment break is removed.
                        // In `Collapse` mode ASCII `CR` and `LF` newlines are collapsible and never
                        // reach this point, so only `LS` and `PS` are treated as forced breaks. In
                        // `PreserveBreaks` mode `CR` and `LF` are not collapsible and are also
                        // preserved as forced breaks.
                        self.pending_whitespace = None;
                        let break_len = rest.chars().next().unwrap().len_utf8();
                        self.commit_text(span, &rest[..break_len]);
                        rest = &rest[break_len..];
                        continue;
                    }
                    let text_len = rest
                        .find(|c| mode.is_collapsible(c) || is_segment_break(c))
                        .unwrap_or(rest.len());
                    self.flush_pending_whitespace();
                    self.commit_text(span, &rest[..text_len]);
                    rest = &rest[text_len..];
                }
            }
        }
    }

    /// Resolves a pending collapsible whitespace sequence, committing a single space for it if it
    /// is followed by content that it can collapse into, and dropping it otherwise.
    pub(crate) fn flush_pending_whitespace(&mut self) {
        let Some(pending) = self.pending_whitespace.take() else {
            return;
        };

        // Whitespace at the start of the inline formatting context is removed, as is whitespace
        // immediately following a preserved segment break. Whitespace following a preserved space
        // or tab is retained, as only collapsible whitespace collapses.
        let is_at_start = self.text.is_empty() && !self.last_item_is_inline_box;
        if is_at_start || self.text.ends_with(is_segment_break) {
            return;
        }

        let style_index = if pending.wrappable {
            self.resolve_wrappable_style_id(pending.span)
        } else {
            self.resolve_style_id(pending.span)
        };
        self.commit_styled_text(style_index, " ");
    }

    /// The style table index of the span that text is currently being pushed into.
    pub(crate) fn resolve_current_style_id(&mut self) -> u16 {
        self.resolve_style_id(self.current_span)
    }

    /// Appends already white space processed `text` to the buffer, attributed to `span`.
    fn commit_text(&mut self, span: usize, text: &str) {
        let style_index = self.resolve_style_id(span);
        self.commit_styled_text(style_index, text);
    }

    /// Appends already white space processed `text` to the buffer with the given style.
    fn commit_styled_text(&mut self, style_index: u16, text: &str) {
        let start = self.text.len();
        self.text.push_str(text);
        match self.style_runs.last_mut() {
            Some(run) if run.style_index == style_index && run.range.end == start => {
                run.range.end = self.text.len();
            }
            _ => self.style_runs.push(StyleRun {
                style_index,
                range: start..self.text.len(),
            }),
        }
        self.last_item_is_inline_box = false;
    }

    /// The index of `span`'s style in the style table, adding it (and any ancestors that have not
    /// yet been added) to the table if necessary. Ancestors are added first so that a style's
    /// parent always has a lower index than the style itself.
    fn resolve_style_id(&mut self, span: usize) -> u16 {
        if let Some(style_id) = self.tree[span].style_id {
            return style_id;
        }
        let parent = self.tree[span]
            .parent
            .expect("root style is materialised in `begin`");
        let parent_id = self.resolve_style_id(parent);
        let mut style = self.tree[span].style.clone();
        style.parent = parent_id;
        let style_id = self.push_style(style);
        self.tree[span].style_id = Some(style_id);
        style_id
    }

    fn push_style(&mut self, style: ResolvedStyle<B>) -> u16 {
        let style_id = self.style_table.len();
        assert!(style_id <= u16::MAX as usize, "too many styles");
        self.style_table.push(style);
        style_id as u16
    }

    /// The index of `span`'s style with wrapping enabled, adding it to the table if necessary.
    fn resolve_wrappable_style_id(&mut self, span: usize) -> u16 {
        if self.tree[span].style.text_wrap_mode == TextWrapMode::Wrap {
            return self.resolve_style_id(span);
        }
        if let Some(style_id) = self.tree[span].wrappable_style_id {
            return style_id;
        }
        // Materialise the span itself first so the wrappable variant shares its parent.
        let span_id = self.resolve_style_id(span);
        let mut style = self.style_table[usize::from(span_id)].clone();
        style.text_wrap_mode = TextWrapMode::Wrap;
        let style_id = self.push_style(style);
        self.tree[span].wrappable_style_id = Some(style_id);
        style_id
    }

    /// The length in bytes of the text committed so far, excluding buffered text.
    pub(crate) fn committed_text_len(&self) -> usize {
        self.text.len()
    }

    /// Begins a child span with the given style, which subsequent text is attributed to.
    pub(crate) fn push_style_span(&mut self, style: ResolvedStyle<B>) {
        self.commit_uncommitted_text();

        self.tree.push(StyleTreeNode {
            parent: Some(self.current_span),
            style,
            style_id: None,
            wrappable_style_id: None,
        });
        self.current_span = self.tree.len() - 1;
    }

    /// Begins a child span with the current style modified by the given properties.
    pub(crate) fn push_style_modification_span(
        &mut self,
        properties: impl Iterator<Item = ResolvedProperty<B>>,
    ) {
        let mut style = self.current_style();
        for prop in properties {
            style.apply(prop);
        }
        self.push_style_span(style);
    }

    /// Ends the current span, returning to its parent.
    pub(crate) fn pop_style_span(&mut self) {
        self.commit_uncommitted_text();

        self.current_span = self.tree[self.current_span]
            .parent
            .expect("Popped root style");
    }

    /// Buffers text in the current span, to be white space processed when it is committed.
    pub(crate) fn push_text(&mut self, text: &str) {
        self.uncommitted_text.push_str(text);
    }

    /// Computes style table + style runs and returns the final text buffer.
    pub(crate) fn finish(
        &mut self,
        style_table: &mut Vec<ResolvedStyle<B>>,
        style_runs: &mut Vec<StyleRun>,
    ) -> String {
        while self.tree[self.current_span].parent.is_some() {
            self.pop_style_span();
        }

        self.commit_uncommitted_text();

        style_table.clear();
        style_runs.clear();
        style_table.extend_from_slice(&self.style_table);
        style_runs.extend_from_slice(&self.style_runs);

        if style_runs.is_empty() {
            // If there's no text, the layout still needs the root style, e.g., to size a cursor.
            style_runs.push(StyleRun {
                style_index: 0,
                range: 0..0,
            });
        }

        core::mem::take(&mut self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use core::ops::Range;

    #[test]
    fn preserve_breaks_collapses_across_text_and_style_boundaries() {
        for (input, expected) in [
            ("a \t b\n  c", "a b\nc"),
            (" \ta \t\r \t\n \tb \t", "a\r\nb"),
            ("\n \t\n \t", "\n\n"),
            (" \t", ""),
            ("a \u{2028} \tb \u{2029} c", "a\u{2028}b\u{2029}c"),
            ("a \u{00a0} \u{3000} b", "a \u{00a0} \u{3000} b"),
            ("a\u{000b}\u{000c}\u{0085}b", "a\u{000b}\u{000c}\u{0085}b"),
        ] {
            for split in (0..=input.len()).filter(|&i| input.is_char_boundary(i)) {
                let mut builder = TreeStyleBuilder::<u32>::default();
                builder.begin(ResolvedStyle {
                    white_space_collapse: WhiteSpaceCollapse::PreserveBreaks,
                    ..ResolvedStyle::default()
                });
                builder.push_text(&input[..split]);
                builder.push_style_modification_span([ResolvedProperty::FontSize(20.)].into_iter());
                builder.push_text(&input[split..]);
                builder.pop_style_span();
                let text = builder.finish(&mut Vec::new(), &mut Vec::new());
                assert_eq!(text, expected, "{input:?} split at {split}");
            }
        }
    }

    #[test]
    fn preserve_breaks_keeps_pending_whitespace_provenance_across_modes() {
        let mut builder = TreeStyleBuilder::<u32>::default();
        builder.begin(ResolvedStyle {
            white_space_collapse: WhiteSpaceCollapse::Collapse,
            ..ResolvedStyle::default()
        });
        builder.push_style_modification_span(
            [
                ResolvedProperty::Brush(1),
                ResolvedProperty::TextWrapMode(TextWrapMode::NoWrap),
            ]
            .into_iter(),
        );
        builder.push_text("a ");
        builder.pop_style_span();
        builder.push_style_modification_span(
            [ResolvedProperty::WhiteSpaceCollapse(
                WhiteSpaceCollapse::PreserveBreaks,
            )]
            .into_iter(),
        );
        builder.push_text("\tb \t\n \tc");

        let mut styles = Vec::new();
        let mut runs = Vec::new();
        let text = builder.finish(&mut styles, &mut runs);
        assert_eq!(text, "a b\nc");
        let space = runs.iter().find(|run| run.range.contains(&1)).unwrap();
        let style = &styles[space.style_index as usize];
        assert_eq!(style.brush, 1);
        assert_eq!(style.text_wrap_mode, TextWrapMode::Wrap);
        assert_eq!(style.white_space_collapse, WhiteSpaceCollapse::Collapse);
    }

    #[test]
    fn collapses_ascii_whitespace_without_trimming_non_ascii_whitespace() {
        let mut builder = TreeStyleBuilder::<u32>::default();
        builder.begin(ResolvedStyle {
            white_space_collapse: WhiteSpaceCollapse::Collapse,
            ..ResolvedStyle::default()
        });
        builder.push_text(" \u{00a0}text\u{00a0} ");

        let mut style_table = Vec::new();
        let mut style_runs = Vec::new();
        let text = builder.finish(&mut style_table, &mut style_runs);

        assert_eq!(text, "\u{00a0}text\u{00a0}");
    }

    #[test]
    fn collapsible_whitespace_at_span_end_is_committed_with_the_span_style() {
        let mut builder = TreeStyleBuilder::<u32>::default();
        builder.begin(ResolvedStyle {
            white_space_collapse: WhiteSpaceCollapse::Collapse,
            ..ResolvedStyle::default()
        });
        builder.push_style_modification_span([ResolvedProperty::FontSize(20.)].into_iter());
        builder.push_text("A ");
        builder.pop_style_span();
        builder.push_text("B");

        let mut style_table = Vec::new();
        let mut style_runs = Vec::new();
        let text = builder.finish(&mut style_table, &mut style_runs);

        assert_eq!(text, "A B");
        assert_eq!(style_runs.len(), 2);
        assert_eq!(style_runs[0].style_index, 1);
        assert_eq!(style_runs[0].range, Range { start: 0, end: 2 });
        assert_eq!(style_runs[1].style_index, 0);
        assert_eq!(style_runs[1].range, Range { start: 2, end: 3 });
    }

    #[test]
    fn collapsible_whitespace_before_preserved_text_is_committed() {
        let mut builder = TreeStyleBuilder::<u32>::default();
        builder.begin(ResolvedStyle {
            white_space_collapse: WhiteSpaceCollapse::Collapse,
            ..ResolvedStyle::default()
        });
        builder.push_text("A ");
        builder.push_style_modification_span(
            [ResolvedProperty::WhiteSpaceCollapse(
                WhiteSpaceCollapse::Preserve,
            )]
            .into_iter(),
        );
        builder.push_text("  B  ");
        builder.pop_style_span();
        builder.push_text("  C");

        let mut style_table = Vec::new();
        let mut style_runs = Vec::new();
        let text = builder.finish(&mut style_table, &mut style_runs);

        assert_eq!(text, "A   B   C");
    }

    #[test]
    fn collapsible_whitespace_around_a_preserved_segment_break_is_removed() {
        let mut builder = TreeStyleBuilder::<u32>::default();
        builder.begin(ResolvedStyle {
            white_space_collapse: WhiteSpaceCollapse::Collapse,
            ..ResolvedStyle::default()
        });
        builder.push_text("A  ");
        builder.push_style_modification_span(
            [ResolvedProperty::WhiteSpaceCollapse(
                WhiteSpaceCollapse::Preserve,
            )]
            .into_iter(),
        );
        builder.push_text("\n");
        builder.pop_style_span();
        builder.push_text("  B");

        let mut style_table = Vec::new();
        let mut style_runs = Vec::new();
        let text = builder.finish(&mut style_table, &mut style_runs);

        assert_eq!(text, "A\nB");
    }

    #[test]
    fn reuses_style_id_when_returning_to_parent_span() {
        let mut builder = TreeStyleBuilder::<u32>::default();
        builder.begin(ResolvedStyle::default());
        builder.push_text("A");
        builder.push_style_modification_span([ResolvedProperty::FontSize(20.)].into_iter());
        builder.push_text("B");
        builder.pop_style_span();
        builder.push_text("C");

        let mut style_table = Vec::new();
        let mut style_runs = Vec::new();
        let text = builder.finish(&mut style_table, &mut style_runs);

        assert_eq!(text, "ABC");
        assert_eq!(style_table.len(), 2);
        assert_eq!(style_runs.len(), 3);
        assert_eq!(style_runs[0].style_index, 0);
        assert_eq!(style_runs[1].style_index, 1);
        assert_eq!(style_runs[2].style_index, 0);
        assert_eq!(style_runs[0].range, Range { start: 0, end: 1 });
        assert_eq!(style_runs[1].range, Range { start: 1, end: 2 });
        assert_eq!(style_runs[2].range, Range { start: 2, end: 3 });
    }

    #[test]
    fn reuses_root_style_id_across_multiple_pop_return_cycles() {
        let mut builder = TreeStyleBuilder::<u32>::default();
        builder.begin(ResolvedStyle::default());
        builder.push_text("A");
        builder.push_style_modification_span([ResolvedProperty::FontSize(20.)].into_iter());
        builder.push_text("B");
        builder.pop_style_span();
        builder.push_text("C");
        builder.push_style_modification_span([ResolvedProperty::LetterSpacing(1.)].into_iter());
        builder.push_text("D");
        builder.pop_style_span();
        builder.push_text("E");

        let mut style_table = Vec::new();
        let mut style_runs = Vec::new();
        let text = builder.finish(&mut style_table, &mut style_runs);

        assert_eq!(text, "ABCDE");
        assert_eq!(style_table.len(), 3);
        assert_eq!(style_runs.len(), 5);
        assert_eq!(style_runs[0].style_index, 0);
        assert_eq!(style_runs[1].style_index, 1);
        assert_eq!(style_runs[2].style_index, 0);
        assert_eq!(style_runs[3].style_index, 2);
        assert_eq!(style_runs[4].style_index, 0);
    }

    #[test]
    fn materialises_ancestor_spans_without_text() {
        let mut builder = TreeStyleBuilder::<u32>::default();
        builder.begin(ResolvedStyle::default());
        builder.push_style_modification_span([ResolvedProperty::FontSize(40.)].into_iter());
        builder.push_style_modification_span([ResolvedProperty::FontSize(20.)].into_iter());
        builder.push_text("B");
        builder.pop_style_span();
        builder.pop_style_span();

        let mut style_table = Vec::new();
        let mut style_runs = Vec::new();
        let text = builder.finish(&mut style_table, &mut style_runs);

        assert_eq!(text, "B");
        assert_eq!(style_table.len(), 3);
        assert_eq!(style_table[0].parent, 0);
        assert_eq!(style_table[1].parent, 0);
        assert_eq!(style_table[1].font_size, 40.);
        assert_eq!(style_table[2].parent, 1);
        assert_eq!(style_table[2].font_size, 20.);
        assert_eq!(style_runs.len(), 1);
        assert_eq!(style_runs[0].style_index, 2);
    }

    #[test]
    fn empty_text_uses_root_style() {
        let mut builder = TreeStyleBuilder::<u32>::default();
        builder.begin(ResolvedStyle::default());

        let mut style_table = Vec::new();
        let mut style_runs = Vec::new();
        builder.finish(&mut style_table, &mut style_runs);

        assert_eq!(style_table.len(), 1);
        assert_eq!(style_runs.len(), 1);
        assert_eq!(style_runs[0].style_index, 0);
        assert_eq!(style_runs[0].range, Range { start: 0, end: 0 });
    }

    #[test]
    fn reuses_parent_and_root_style_ids_after_nested_pop() {
        let mut builder = TreeStyleBuilder::<u32>::default();
        builder.begin(ResolvedStyle::default());
        builder.push_text("R");
        builder.push_style_modification_span([ResolvedProperty::FontSize(20.)].into_iter());
        builder.push_text("A");
        builder.push_style_modification_span([ResolvedProperty::LetterSpacing(1.)].into_iter());
        builder.push_text("B");
        builder.pop_style_span();
        builder.push_text("C");
        builder.pop_style_span();
        builder.push_text("D");

        let mut style_table = Vec::new();
        let mut style_runs = Vec::new();
        let text = builder.finish(&mut style_table, &mut style_runs);

        assert_eq!(text, "RABCD");
        assert_eq!(style_table.len(), 3);
        assert_eq!(style_runs.len(), 5);
        assert_eq!(style_runs[0].style_index, 0);
        assert_eq!(style_runs[1].style_index, 1);
        assert_eq!(style_runs[2].style_index, 2);
        assert_eq!(style_runs[3].style_index, 1);
        assert_eq!(style_runs[4].style_index, 0);
    }
}
