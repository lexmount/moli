// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Context for layout.

use super::FontContext;
use super::context::LayoutContext;
use super::style::{Brush, StyleProperty, TextStyle};

use super::layout::Layout;

use alloc::string::String;
use core::ops::{Bound, Range, RangeBounds};
use parlance::{BaseDirection, BidiLevel};
use parley_engine::break_overrides::LineBreakOverrideFn;

use crate::inline_box::{InlineBox, LayoutInlineBox};
use crate::resolve::{ResolvedStyle, StyleRun};
use crate::{InlineBoxBidi, InlineBoxKind};

#[derive(Clone, Copy)]
pub(crate) struct BuilderOptions<'a> {
    scale: f32,
    quantize: bool,
    base_direction: BaseDirection,
    line_break_override: Option<&'a LineBreakOverrideFn>,
}

impl BuilderOptions<'_> {
    pub(crate) fn new(scale: f32, quantize: bool) -> Self {
        Self {
            scale,
            quantize,
            base_direction: BaseDirection::Auto,
            line_break_override: None,
        }
    }
}

/// Builder for constructing a text layout with ranged attributes.
///
/// This builder does not collapse whitespace. For whitespace collapsing, use the [`TreeBuilder`]
#[must_use]
pub struct RangedBuilder<'a, B: Brush> {
    pub(crate) options: BuilderOptions<'a>,
    pub(crate) lcx: &'a mut LayoutContext<B>,
    pub(crate) fcx: &'a mut FontContext,
}

impl<'b, B: Brush> RangedBuilder<'b, B> {
    pub fn push_default<'a>(&mut self, property: impl Into<StyleProperty<'a, B>>) {
        let resolved =
            self.lcx
                .rcx
                .resolve_property(self.fcx, &property.into(), self.options.scale);
        self.lcx.ranged_style_builder.push_default(resolved);
    }

    pub fn push<'a>(
        &mut self,
        property: impl Into<StyleProperty<'a, B>>,
        range: impl RangeBounds<usize>,
    ) {
        let resolved =
            self.lcx
                .rcx
                .resolve_property(self.fcx, &property.into(), self.options.scale);
        self.lcx.ranged_style_builder.push(resolved, range);
    }

    /// Adds a box at the source-text offset specified by [`InlineBox::index`].
    ///
    /// Building the layout panics if the offset is not a character boundary within
    /// the source text, including its end. Boxes can be pushed in any order; boxes
    /// at the same offset retain their insertion order before bidi reordering.
    pub fn push_inline_box(&mut self, inline_box: InlineBox) {
        self.push_inline_box_with_bidi(inline_box, InlineBoxBidi::Neutral);
    }

    /// Adds a box with explicit bidi participation at its source-text offset.
    ///
    /// The anchor rules are the same as [`Self::push_inline_box`]. Use inherited
    /// levels only for transparent structural markers, not real inline objects.
    pub fn push_inline_box_with_bidi(&mut self, inline_box: InlineBox, bidi: InlineBoxBidi) {
        self.lcx.inline_boxes.push(LayoutInlineBox {
            inline_box,
            bidi,
            bidi_level: BidiLevel::new(0),
            parent_style_index: 0,
            baseline_offset: 0.,
        });
    }

    /// Sets the paragraph's base direction.
    ///
    /// The default is [`BaseDirection::Auto`], which infers the direction from the text.
    pub fn set_base_direction(&mut self, base_direction: BaseDirection) {
        self.options.base_direction = base_direction;
    }

    /// Set the callback which will be called as a first provider of line breaking decisions.
    ///
    /// See [`LineBreakOverrideFn`] for more details.
    pub fn set_line_break_override(&mut self, overrides: Option<&'b LineBreakOverrideFn>) {
        self.options.line_break_override = overrides;
    }

    pub fn build_into(self, layout: &mut Layout<B>, text: impl AsRef<str>) {
        // Apply RangedStyleBuilder styles directly to style-table/style-run state.
        self.lcx
            .ranged_style_builder
            .finish(&mut self.lcx.style_table, &mut self.lcx.style_runs);

        // Call generic layout builder method
        build_into_layout(layout, text.as_ref(), self.lcx, self.fcx, self.options);
    }

    pub fn build(self, text: impl AsRef<str>) -> Layout<B> {
        let mut layout = Layout::default();
        self.build_into(&mut layout, text);
        layout
    }
}

/// Builder for constructing a text layout from a style table and
/// indexed style runs.
///
/// This builder does not collapse whitespace. For whitespace collapsing, use the [`TreeBuilder`]
#[must_use]
pub struct StyleRunBuilder<'a, B: Brush> {
    pub(crate) options: BuilderOptions<'a>,
    pub(crate) len: usize,
    pub(crate) lcx: &'a mut LayoutContext<B>,
    pub(crate) fcx: &'a mut FontContext,
    pub(crate) cursor: usize,
}

impl<'b, B: Brush> StyleRunBuilder<'b, B> {
    /// Reserves additional capacity for styles and runs.
    ///
    /// This is an optional optimization for callers that know counts
    /// up front; call it before pushing styles and runs to reduce
    /// reallocations.
    pub fn reserve(&mut self, additional_styles: usize, additional_runs: usize) {
        self.lcx.style_table.reserve(additional_styles);
        self.lcx.style_runs.reserve(additional_runs);
    }

    /// Adds a fully-specified style to the shared style table and
    /// returns its index.
    pub fn push_style<'family, 'settings>(
        &mut self,
        style: TextStyle<'family, 'settings, B>,
    ) -> u16 {
        let resolved = self
            .lcx
            .rcx
            .resolve_entire_style_set(self.fcx, &style, self.options.scale);
        let style_index = self.lcx.style_table.len();
        assert!(style_index <= u16::MAX as usize, "too many styles");
        self.lcx.style_table.push(resolved);
        style_index as u16
    }

    /// Adds a style run referencing an entry from the style table.
    ///
    /// Runs must be contiguous and non-overlapping, and must cover
    /// `0..text.len()` once all runs have been added.
    pub fn push_style_run(&mut self, style_index: u16, range: impl RangeBounds<usize>) {
        let range = resolve_range(range, self.len);
        assert!(
            range.start == self.cursor,
            "StyleRunBuilder expects contiguous non-overlapping runs"
        );
        assert!(
            range.start <= range.end,
            "StyleRunBuilder expects ordered ranges"
        );
        assert!(
            (style_index as usize) < self.lcx.style_table.len(),
            "StyleRunBuilder expects style indices that were previously added via push_style"
        );
        self.lcx.style_runs.push(StyleRun {
            style_index,
            range: range.clone(),
        });
        self.cursor = range.end;
    }

    /// Adds a box at the source-text offset specified by [`InlineBox::index`].
    ///
    /// Building the layout panics if the offset is not a character boundary within
    /// the source text, including its end. Boxes can be pushed in any order; boxes
    /// at the same offset retain their insertion order before bidi reordering.
    pub fn push_inline_box(&mut self, inline_box: InlineBox) {
        self.push_inline_box_with_bidi(inline_box, InlineBoxBidi::Neutral);
    }

    /// Adds a box with explicit bidi participation at its source-text offset.
    ///
    /// The anchor rules are the same as [`Self::push_inline_box`]. Use inherited
    /// levels only for transparent structural markers, not real inline objects.
    pub fn push_inline_box_with_bidi(&mut self, inline_box: InlineBox, bidi: InlineBoxBidi) {
        self.lcx.inline_boxes.push(LayoutInlineBox {
            inline_box,
            bidi,
            bidi_level: BidiLevel::new(0),
            parent_style_index: 0,
            baseline_offset: 0.,
        });
    }

    /// Sets the paragraph's base direction.
    ///
    /// The default is [`BaseDirection::Auto`], which infers the direction from the text.
    pub fn set_base_direction(&mut self, base_direction: BaseDirection) {
        self.options.base_direction = base_direction;
    }

    /// Set the callback which will be called as a first provider of line breaking decisions.
    ///
    /// See [`LineBreakOverrideFn`] for more details.
    pub fn set_line_break_override(&mut self, overrides: Option<&'b LineBreakOverrideFn>) {
        self.options.line_break_override = overrides;
    }

    pub fn build_into(self, layout: &mut Layout<B>, text: impl AsRef<str>) {
        assert!(
            self.cursor == self.len,
            "StyleRunBuilder requires runs that cover the full text"
        );
        if self.lcx.style_runs.is_empty() {
            // This is reachable for empty text. In this case, the layout still needs a style and an
            // empty style run, e.g. to size a cursor.
            if self.lcx.style_table.is_empty() {
                let style = self.lcx.rcx.resolve_entire_style_set(
                    self.fcx,
                    &TextStyle::default(),
                    self.options.scale,
                );
                self.lcx.style_table.push(style);
            }
            self.lcx.style_runs.push(StyleRun {
                style_index: 0,
                range: 0..0,
            });
        }
        build_into_layout(layout, text.as_ref(), self.lcx, self.fcx, self.options);
    }

    pub fn build(self, text: impl AsRef<str>) -> Layout<B> {
        let mut layout = Layout::default();
        self.build_into(&mut layout, text);
        layout
    }
}

/// Builder for constructing a text layout with a tree of attributes.
#[must_use]
pub struct TreeBuilder<'a, B: Brush> {
    pub(crate) options: BuilderOptions<'a>,
    pub(crate) lcx: &'a mut LayoutContext<B>,
    pub(crate) fcx: &'a mut FontContext,
}

impl<'b, B: Brush> TreeBuilder<'b, B> {
    pub fn push_style_span(&mut self, style: TextStyle<'_, '_, B>) {
        let resolved = self
            .lcx
            .rcx
            .resolve_entire_style_set(self.fcx, &style, self.options.scale);
        self.lcx.tree_style_builder.push_style_span(resolved);
    }

    pub fn push_style_modification_span<'s, 'iter>(
        &mut self,
        properties: impl IntoIterator<Item = &'iter StyleProperty<'s, B>>,
    ) where
        's: 'iter,
        B: 'iter,
    {
        self.lcx
            .tree_style_builder
            .push_style_modification_span(properties.into_iter().map(|p| {
                self.lcx
                    .rcx
                    .resolve_property(self.fcx, p, self.options.scale)
            }));
    }

    pub fn pop_style_span(&mut self) {
        self.lcx.tree_style_builder.pop_style_span();
    }

    pub fn push_text(&mut self, text: &str) {
        self.lcx.tree_style_builder.push_text(text);
    }

    /// Adds a box after the text pushed so far.
    ///
    /// The supplied [`InlineBox::index`] is ignored. Its offset is computed from
    /// the committed text after whitespace handling.
    pub fn push_inline_box(&mut self, inline_box: InlineBox) {
        self.push_inline_box_with_bidi(inline_box, InlineBoxBidi::Neutral);
    }

    /// Adds a box with explicit bidi participation after the text pushed so far.
    ///
    /// Like [`Self::push_inline_box`], this ignores the supplied source offset
    /// and computes it after whitespace handling.
    pub fn push_inline_box_with_bidi(&mut self, mut inline_box: InlineBox, bidi: InlineBoxBidi) {
        self.lcx.tree_style_builder.commit_uncommitted_text();

        if inline_box.kind == InlineBoxKind::InFlow {
            self.lcx.tree_style_builder.flush_pending_whitespace();
            self.lcx.tree_style_builder.set_last_item_is_inline_box();
        }

        // TODO: arrange type better here to factor out the index
        inline_box.index = self.lcx.tree_style_builder.committed_text_len();
        let parent_style_index = self.lcx.tree_style_builder.resolve_current_style_id();
        self.lcx.inline_boxes.push(LayoutInlineBox {
            inline_box,
            bidi,
            bidi_level: BidiLevel::new(0),
            parent_style_index,
            baseline_offset: 0.,
        });
    }

    /// Sets the paragraph's base direction.
    ///
    /// The default is [`BaseDirection::Auto`], which infers the direction from the text.
    pub fn set_base_direction(&mut self, base_direction: BaseDirection) {
        self.options.base_direction = base_direction;
    }

    /// Set the callback which will be called as a first provider of line breaking decisions.
    ///
    /// See [`LineBreakOverrideFn`] for more details.
    pub fn set_line_break_override(&mut self, overrides: Option<&'b LineBreakOverrideFn>) {
        self.options.line_break_override = overrides;
    }

    #[inline]
    pub fn build_into(self, layout: &mut Layout<B>) -> String {
        // Apply TreeStyleBuilder styles to LayoutContext.
        let text = self
            .lcx
            .tree_style_builder
            .finish(&mut self.lcx.style_table, &mut self.lcx.style_runs);

        // Call generic layout builder method
        build_into_layout(layout, &text, self.lcx, self.fcx, self.options);

        text
    }

    #[inline]
    pub fn build(self) -> (Layout<B>, String) {
        let mut layout = Layout::default();
        let text = self.build_into(&mut layout);
        (layout, text)
    }
}

fn build_into_layout<B: Brush>(
    layout: &mut Layout<B>,
    text: &str,
    lcx: &mut LayoutContext<B>,
    fcx: &mut FontContext,
    options: BuilderOptions<'_>,
) {
    if text.is_empty() && lcx.style_runs.is_empty() {
        lcx.style_table.push(ResolvedStyle::default());
        lcx.style_runs.push(StyleRun {
            style_index: 0,
            range: 0..0,
        });
    }
    assert!(
        !lcx.style_runs.is_empty(),
        "at least one style run is required"
    );

    // Sort the inline boxes before bidi analysis and shaping.
    // Keep this stable so contiguous boxes participate in insertion order.
    // TODO: consider requiring source order, as `TreeBuilder` already does.
    lcx.inline_boxes.sort_by_key(|b| b.inline_box.index);

    crate::analysis::analyze_text(
        lcx,
        text,
        options.base_direction,
        options.line_break_override,
    );

    layout.data.clear();
    layout.data.scale = options.scale;
    layout.data.quantize = options.quantize;
    layout.data.base_level = lcx.analysis.paragraph_level();
    layout.data.text_len = text.len();

    // Characters not covered by any style run (e.g. the space substituted for empty text during
    // analysis) use the root style.
    lcx.char_style_indices.clear();
    lcx.char_style_indices
        .resize(lcx.analysis.char_info().len(), 0);
    let mut char_index = 0;
    for style_run in &lcx.style_runs {
        for _ in text[style_run.range.clone()].chars() {
            lcx.char_style_indices[char_index] = style_run.style_index;
            char_index += 1;
        }
    }

    // Copy the visual styles into the layout
    layout
        .data
        .styles
        .extend(lcx.style_table.iter().map(|s| s.as_layout_style()));

    {
        super::shape::shape_text(
            &lcx.rcx,
            fcx,
            &lcx.style_table,
            &lcx.inline_boxes,
            &lcx.analysis,
            &lcx.char_style_indices,
            &mut lcx.scx,
            text,
            layout,
            &lcx.analysis_data_sources,
        );
    }

    // After shaping, so that the metrics of fonts used for shaping can be reused.
    crate::layout::style_metrics::resolve_style_metrics(
        &lcx.rcx,
        fcx,
        &lcx.style_table,
        &layout.data.shaped_text,
        options.quantize,
        &mut layout.data.style_metrics,
    );

    // Move inline boxes into the layout
    layout.data.inline_boxes.clear();
    core::mem::swap(&mut layout.data.inline_boxes, &mut lcx.inline_boxes);
}

fn resolve_range(range: impl RangeBounds<usize>, len: usize) -> Range<usize> {
    let start = match range.start_bound() {
        Bound::Unbounded => 0,
        Bound::Included(n) => *n,
        Bound::Excluded(n) => *n + 1,
    };
    let end = match range.end_bound() {
        Bound::Unbounded => len,
        Bound::Included(n) => *n + 1,
        Bound::Excluded(n) => *n,
    };
    start.min(len)..end.min(len)
}
