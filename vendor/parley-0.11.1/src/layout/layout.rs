// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::InlineBox;
use crate::layout::alignment::align;
use crate::layout::alignment::unjustify;
use crate::layout::data::LayoutData;
use crate::style::Brush;
use core::cmp::Ordering;
use core::fmt;

use crate::IndentOptions;
use crate::layout::{
    ContentWidths, Style, alignment::Alignment, alignment::AlignmentOptions, line::Line,
    line_break::BreakLines,
};

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

    /// Returns the scale factor provided when creating the layout.
    pub fn scale(&self) -> f32 {
        self.data.scale
    }

    /// Returns the style collection for the layout.
    pub fn styles(&self) -> &[Style<B>] {
        &self.data.styles
    }

    /// The `max_advance` that was used to line break the `Layout`
    pub fn layout_max_advance(&self) -> f32 {
        self.data.layout_max_advance
    }

    /// Returns the computed width of the layout excluding the width of trailing whitespace.
    pub fn width(&self) -> f32 {
        self.data.width
    }

    /// Returns the computed width of the layout including the width of trailing whitespace.
    pub fn full_width(&self) -> f32 {
        self.data.full_width
    }

    /// Calculates the lower and upper bounds on the width of the layout. These
    /// are recalculated every time this method is called.
    ///
    /// This method currently may not return the correct results for
    /// mixed-direction text.
    pub fn calculate_content_widths(&self) -> ContentWidths {
        self.data.calculate_content_widths()
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
        self.data.base_level & 1 != 0
    }

    pub fn inline_boxes(&self) -> &[InlineBox] {
        &self.data.inline_boxes
    }

    pub fn inline_boxes_mut(&mut self) -> &mut [InlineBox] {
        &mut self.data.inline_boxes
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
        unjustify(&mut self.data);
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
        unjustify(&mut self.data);
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
