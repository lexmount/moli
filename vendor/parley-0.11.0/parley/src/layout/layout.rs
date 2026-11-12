// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::IndentOptions;
use crate::InlineBox;
use crate::layout::alignment::align;
use crate::layout::data::LayoutData;
use crate::layout::{
    ContentWidths, SpanMetrics, Style, alignment::Alignment, alignment::AlignmentOptions,
    line::Line, line_break::BreakLines,
};
use crate::style::Brush;

use core::cmp::Ordering;
use core::fmt;

/// Text layout.
///
/// The [`Debug`] implementation prints a compact summary by default.
/// The alternate form (`{:#?}`) formats the full underlying data.
///
/// [`Debug`]: core::fmt::Debug
#[derive(Clone)]
pub struct Layout<B: Brush> {
    pub(crate) data: LayoutData<B>,
}

impl<B: Brush> fmt::Debug for Layout<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            f.debug_struct("Layout").field("data", &self.data).finish()
        } else {
            f.debug_struct("Layout")
                .field("text_len", &self.data.text_len)
                .field("width", &self.data.width)
                .field("height", &self.data.height)
                .field("lines", &self.data.lines.len())
                .field("runs", &self.data.runs.len())
                .field("styles", &self.data.styles.len())
                .finish_non_exhaustive()
        }
    }
}

impl<B: Brush> Layout<B> {
    /// Creates an empty layout.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the layout while retaining capacity.
    ///
    /// This clears underlying strong references to fonts, which would otherwise
    /// prevent those fonts from being freed until dropping or reusing this layout.
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Returns the scale factor provided when creating the layout.
    pub fn scale(&self) -> f32 {
        self.data.scale
    }

    /// Returns the style collection for the layout.
    pub fn styles(&self) -> &[Style<B>] {
        &self.data.styles
    }

    /// The inherent metrics of the root span (in CSS terms, that is the metrics of the
    /// [root inline box](https://www.w3.org/TR/css-inline-3/#root-inline-box)).
    ///
    /// These are available once the layout is built, and will be `None` otherwise.
    /// The metric values come from the first available font and font size resolved
    /// from [`push_default`](crate::RangedBuilder::push_default).
    ///
    /// The motivating use case is placing bullet list markers - Blink uses the root
    /// span's ascent and descent for their location and size.
    pub fn root_span_metrics(&self) -> Option<SpanMetrics> {
        self.data.style_metrics.first().map(|it| SpanMetrics {
            ascent: it.ascent,
            descent: it.descent,
            x_height: it.x_height,
        })
    }

    /// Returns the length of the source text, in bytes.
    pub fn text_len(&self) -> usize {
        self.data.text_len
    }

    /// Returns the alignment that was applied to the layout by [`Layout::align`].
    ///
    /// Returns `None` if the layout has not been aligned since it was last built.
    pub fn alignment(&self) -> Option<Alignment> {
        self.data.alignment
    }

    /// The `max_advance` that was used to line break the `Layout`
    pub fn layout_max_advance(&self) -> f32 {
        self.data.layout_max_advance
    }

    /// Returns the computed width of the layout excluding the width of hanging whitespace.
    pub fn width(&self) -> f32 {
        self.data.width
    }

    /// Returns the computed width of the layout including the width of trailing whitespace.
    pub fn full_width(&self) -> f32 {
        self.data.full_width
    }

    /// Returns the y-offset of the baseline of the first line in the layout.
    /// Returns `None` if the layout is empty.
    pub fn first_baseline(&self) -> Option<f32> {
        self.lines().next().map(|line| line.metrics().baseline)
    }

    /// Returns the y-offset of the baseline of the last line in the layout.
    /// Returns `None` if the layout is empty.
    pub fn last_baseline(&self) -> Option<f32> {
        self.lines().next_back().map(|line| line.metrics().baseline)
    }

    /// Calculates the lower and upper bounds on the width of the layout. These
    /// are recalculated every time this method is called.
    ///
    /// This method currently may not return the correct results for
    /// mixed-direction text.
    pub fn calculate_content_widths(&self) -> ContentWidths {
        self.data.calculate_content_widths()
    }

    /// Allocate caller-supplied UTF-8 text items on a horizontal grid.
    /// Configure after shaping and before line breaking. Each partial item
    /// rounds upwards; fallback fonts retain one allocation item. Shared glyphs
    /// belong to their first logical character. Canvas/SVG keep the default
    /// unquantized path. Reconfiguration never changes the shaped data.
    pub fn set_text_item_quantization(&mut self, quantum: f32, item_ends: &[usize]) {
        assert!(quantum.is_finite() && quantum > 0.0);
        assert!(item_ends.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(item_ends.last().copied().unwrap_or(0) == self.data.text_len);
        assert!(
            self.data.lines.is_empty(),
            "configure allocation before line breaking"
        );
        if self.data.text_len == 0 {
            // Empty layouts retain a synthetic run for line and caret metrics,
            // but there are no caller-owned text items to allocate.
            self.data.text_item_quantization = None;
            return;
        }
        let mut groups = alloc::vec![0; self.data.shaped_text.shaped_clusters().len()];
        let mut previous = None;
        let mut group = 0;
        for item in &self.data.items {
            if item.kind == super::data::LayoutItemKind::InlineBox {
                previous = None;
                continue;
            }
            let slice = self.data.shaped_text.run_slice(item.index as u32);
            for atom in slice.atoms_start() {
                let offset = slice.text_byte_range(atom.char_range()).start;
                let key = (
                    item_ends.partition_point(|end| *end <= offset),
                    item.bidi_level,
                );
                if previous != Some(key) {
                    group += 1;
                    previous = Some(key);
                }
                for index in atom.shaped_clusters_range() {
                    groups[index as usize] = group;
                }
            }
        }
        self.data.text_item_quantization = Some(super::text_advance::TextItemQuantization {
            quantum,
            item_ends: item_ends.to_vec(),
            cluster_groups: groups,
        });
    }

    /// Returns the height of the layout.
    pub fn height(&self) -> f32 {
        self.data.height
    }

    /// Returns the number of lines in the layout.
    pub fn len(&self) -> usize {
        self.data.lines.len()
    }

    /// Returns `true` if the layout is empty.
    pub fn is_empty(&self) -> bool {
        self.data.lines.is_empty()
    }

    /// Returns the line at the specified index.
    ///
    /// Returns `None` if the index is out of bounds, i.e. if it's
    /// not less than [`self.len()`](Self::len).
    pub fn get(&self, index: usize) -> Option<Line<'_, B>> {
        Some(Line {
            index: index as u32,
            layout: self,
            data: self.data.lines.get(index)?,
        })
    }

    /// Returns `true` if the dominant direction of the layout is right-to-left.
    pub fn is_rtl(&self) -> bool {
        self.data.base_level.is_rtl()
    }

    /// Iterator over inline boxes in text index order.
    pub fn inline_boxes(&self) -> impl ExactSizeIterator<Item = &InlineBox> + '_ {
        self.data.inline_boxes.iter().map(|b| &b.inline_box)
    }

    /// Mutable iterator over the inline boxes in text index order.
    pub fn inline_boxes_mut(&mut self) -> impl ExactSizeIterator<Item = &mut InlineBox> + '_ {
        self.data.inline_boxes.iter_mut().map(|b| &mut b.inline_box)
    }

    /// Replace a line's visual item order with a permutation of its current items.
    ///
    /// Call after line breaking and before alignment. Hosts can use this to place
    /// transparent decoration boxes at visual fragment edges after bidi reorder.
    /// Text runs must retain their relative order; source ranges, advances and
    /// line membership are preserved. Breaking lines again resets this order.
    ///
    /// Panics if the line does not exist, the indices are not a permutation, or
    /// text runs are reordered.
    pub fn reorder_line_items(&mut self, line_index: usize, order: &[usize]) {
        let range = self.data.lines[line_index].item_range.clone();
        assert_eq!(order.len(), range.len(), "invalid visual item count");
        let items = &mut self.data.line_items[range];
        let mut seen = alloc::vec![false; items.len()];
        let mut previous_text = None;
        for &index in order {
            assert!(
                index < items.len() && !seen[index],
                "invalid visual permutation"
            );
            seen[index] = true;
            if items[index].kind == super::data::LayoutItemKind::TextRun {
                assert!(
                    previous_text.is_none_or(|previous| previous < index),
                    "text order must be preserved"
                );
                previous_text = Some(index);
            }
        }
        let reordered = order
            .iter()
            .map(|&index| items[index].clone())
            .collect::<alloc::vec::Vec<_>>();
        items.clone_from_slice(&reordered);
    }

    /// Returns an iterator over the lines in the layout.
    pub fn lines(
        &self,
    ) -> impl ExactSizeIterator<Item = Line<'_, B>> + DoubleEndedIterator + '_ + Clone {
        self.data
            .lines
            .iter()
            .enumerate()
            .map(move |(index, data)| Line {
                index: index as u32,
                layout: self,
                data,
            })
    }

    /// Sets the text-indent for the layout.
    ///
    /// The indent is applied as a margin on the start edge of indented lines, reducing the
    /// available width for line breaking and offsetting content during alignment. Negative
    /// values cause the line to protrude beyond the start edge.
    ///
    /// This must be called before [`Layout::break_all_lines`] or [`Layout::break_lines`],
    /// and before [`Layout::align`].
    pub fn set_text_indent(&mut self, amount: f32, options: IndentOptions) {
        self.data.indent_amount = amount;
        self.data.indent_options = options;
    }

    /// Returns line breaker to compute lines for the layout.
    pub fn break_lines(&mut self) -> BreakLines<'_, B> {
        BreakLines::new(self)
    }

    /// Breaks all lines with the specified maximum advance.
    pub fn break_all_lines(&mut self, max_advance: Option<f32>) {
        self.break_lines()
            .break_remaining(max_advance.unwrap_or(f32::MAX));
    }

    /// Apply alignment to the layout.
    ///
    /// You must perform line breaking prior to aligning, through [`Layout::break_lines`] or
    /// [`Layout::break_all_lines`].
    ///
    /// If a finite `max_advance` is supplied to `Layout::break_all_lines` then that width will be applied
    /// relative to that width. Otherwise alignment will be applied relative to the width of the
    /// longest line as computed by line breaking.
    ///
    /// If line-specific `offset` and `max_advance` are set using the advanced methods on the `BreakLines`
    /// struct then each line will be aligned individually within its line box.
    pub fn align(&mut self, alignment: Alignment, options: AlignmentOptions) {
        align(&mut self.data, alignment, options);
    }

    /// Returns the index and `Line` object for the line containing the
    /// given byte `index` in the source text.
    pub(crate) fn line_for_byte_index(&self, index: usize) -> Option<(usize, Line<'_, B>)> {
        let line_index = self
            .data
            .lines
            .binary_search_by(|line| {
                if index < line.text_range.start {
                    Ordering::Greater
                } else if index >= line.text_range.end {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            })
            .ok()?;
        Some((line_index, self.get(line_index)?))
    }

    /// Returns the index and `Line` object for the line containing the
    /// given `offset`.
    ///
    /// The offset is specified in the direction orthogonal to line direction.
    /// For horizontal text, this is a vertical or y offset. If the offset is
    /// on a line boundary, it is considered to be contained by the later line.
    pub(crate) fn line_for_offset(&self, offset: f32) -> Option<(usize, Line<'_, B>)> {
        if offset < 0.0 {
            return Some((0, self.get(0)?));
        }
        let maybe_line_index = self.data.lines.binary_search_by(|line| {
            if offset < line.metrics.block_min_coord {
                Ordering::Greater
            } else if offset >= line.metrics.block_max_coord {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        });
        let line_index = match maybe_line_index {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        Some((line_index, self.get(line_index)?))
    }
}

impl<B: Brush> Default for Layout<B> {
    fn default() -> Self {
        Self {
            data: LayoutData::default(),
        }
    }
}
