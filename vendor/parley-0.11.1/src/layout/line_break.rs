// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Greedy line breaking.

use alloc::vec::Vec;

#[cfg(feature = "libm")]
#[allow(unused_imports)]
use core_maths::CoreFloat;

use crate::analysis::cluster::Whitespace;
use crate::analysis::Boundary;
use crate::data::ClusterData;
use crate::layout::{
    BreakReason, Layout, LayoutData, LayoutItem, LayoutItemKind, LineData, LineItemData,
    LineMetrics, Run,
};
use crate::style::Brush;
use crate::{InlineBoxKind, OverflowWrap, TextWrapMode};

use super::text_advance::{shaping_cluster_end, TextAdvance};
use core::ops::Range;

#[derive(Default)]
struct LineLayout {
    lines: Vec<LineData>,
    line_items: Vec<LineItemData>,
}

impl LineLayout {
    fn swap<B: Brush>(&mut self, layout: &mut LayoutData<B>) {
        core::mem::swap(&mut self.lines, &mut layout.lines);
        core::mem::swap(&mut self.line_items, &mut layout.line_items);
    }
}

#[derive(Clone, Default)]
struct LineState {
    advance: TextAdvance,
    items: Range<usize>,
    clusters: Range<usize>,
    num_spaces: usize,
    /// Of the line currently being built, the maximum line height seen so far.
    /// This represents a lower-bound on the eventual line height of the line.
    running_line_height: f32,
    /// This is set to true if we encounter something on the line (either a glyph or an inline box)
    /// that is taller than the `line_max_height`. When in this state `break_next` should yield control
    /// flow to the caller to handle the constraint violation.
    ///
    /// This never happens when calling `break_all_lines` as it never sets `line_max_height`, and it defaults to `f32::MAX`.
    max_height_exceeded: bool,

    /// We lag the text-wrap-mode by one cluster due to line-breaking boundaries only
    /// being triggered on the cluster after the linebreak.
    text_wrap_mode: TextWrapMode,
}

#[derive(Clone, Default)]
struct PrevBoundaryState {
    item_idx: usize,
    run_idx: usize,
    cluster_idx: usize,
    state: LineState,
}

/// Reason that the line breaker has yielded control flow
#[derive(Clone, Debug)]
pub enum YieldData {
    /// Control flow was yielded because a line break occurred.
    /// The `reason` field of the [`LineBreakData`] contains a specific reason about what caused a
    /// line break at this location.
    LineBreak(LineBreakData),
    /// Control flow was yielded because content on the line caused the line to exceed the max height
    ///
    /// The caller is responsible for finding a new location for the line with a greater available height
    /// adjusting the line geometry to the new position and resuming iteration.
    ///
    /// Note: that by default no max height is set (and one is not required for laying out text into
    /// rectangular regions), so you will only encounter this if you explicitly set a max height
    /// using `BreakLine::set_line_max_height`.
    MaxHeightExceeded(MaxHeightBreakData),
    /// Control flow was yielded because an inline box with kind [`InlineBoxKind::CustomOutOfFlow`]
    /// was encountered.
    ///
    /// Parley does not position these boxes itself. The caller is responsible for
    /// placing the box (e.g. via a caller-owned algorithm), adjusting the line geometry through
    /// [`BreakerState`], and then resuming iteration.
    InlineBoxBreak(BoxBreakData),
}

#[derive(Clone, Debug)]
/// Information about a line break
pub struct LineBreakData {
    /// The reason for the line break (see [`BreakReason`] for details)
    pub reason: BreakReason,
    /// The computed advance (width) of the line
    pub advance: f32,
    /// The computed height of the line
    pub line_height: f32,
    /// The position of the top of the line
    pub line_y_start: f64,
    /// The position of the bottom of the line
    pub line_y_end: f64,
}

#[derive(Clone, Debug)]
/// Information about a "max height break" (where control flow has been yielded due to the
/// line's configured max height being exceeded by content by laid out into the line).
pub struct MaxHeightBreakData {
    /// The current advance of the in-progress line
    pub advance: f32,
    /// The current line height of the in-progress line
    pub line_height: f32,
}

#[derive(Clone, Debug)]
/// Information about a "box break" (where control flow has been yielded due to an inline box
/// with kind [`InlineBoxKind::CustomOutOfFlow`] being encountered during layout.
pub struct BoxBreakData {
    /// The user-supplied ID for the inline box
    pub inline_box_id: u64,
    /// The index of the inline box within `Layout::inline_boxes()`
    pub inline_box_index: usize,
    /// The current advance of the line (up to but *not* including the `CustomOutOfFlow` box)
    pub advance: f32,
}

#[derive(Clone)]
/// The mutable state of the line breaker.
///
/// This is exposed so that callers using [`BreakLines`] directly can inspect and
/// adjust line geometry between calls to [`BreakLines::break_next`].
///
/// A `BreakerState` can be cloned and later passed to [`BreakLines::revert_to`]
/// to retry layout from a saved checkpoint.
pub struct BreakerState {
    /// The number of items that have been processed (used to revert state)
    items: usize,
    /// The number of lines that have been processed (used to revert state)
    lines: usize,

    /// Iteration state: the current item (within the layout)
    item_idx: usize,
    /// Iteration state: the current run (within the layout)
    run_idx: usize,
    /// Iteration state: the current cluster (within the layout)
    cluster_idx: usize,

    /// The x coordinate of the left/start of the current line
    line_x: f32,
    /// The y coordinate of the top/start of the current line
    /// Use of f64 here is important. f32 causes test failures due to accumulated error
    line_y: f64,

    /// The max advance of the entire layout.
    layout_max_advance: f32,
    /// The max advance (max width) of the current line. This must be <= the `layout_max_advance`.
    line_max_advance: f32,
    /// The max height available to the current line.
    line_max_height: f32,

    /// The state of the current line
    line: LineState,

    // Saved breaker states for reverting to a previously encountered line-breaking opportunity
    /// Saved breaker state for the last non-emergency line-breaking opportunity
    prev_boundary: Option<PrevBoundaryState>,
    /// Saved breaker state for the last emergency line-breaking opportunity
    emergency_boundary: Option<PrevBoundaryState>,
}

impl Default for BreakerState {
    fn default() -> Self {
        Self {
            items: 0,
            lines: 0,
            item_idx: 0,
            run_idx: 0,
            cluster_idx: 0,
            line_x: 0.0,
            line_y: 0.0,
            layout_max_advance: 0.0,
            line_max_advance: 0.0,
            line_max_height: f32::MAX,
            line: LineState::default(),
            prev_boundary: None,
            emergency_boundary: None,
        }
    }
}

impl BreakerState {
    /// Add the cluster(s) currently being evaluated to the current line
    pub fn append_cluster_to_line(&mut self, next_x: f32, clusters_height: f32) {
        self.line.items.end = self.item_idx + 1;
        self.line.clusters.end = self.cluster_idx + 1;
        self.cluster_idx += 1;
        self.line.advance = TextAdvance::from_width(next_x);
        self.add_line_height(clusters_height);
    }

    fn append_text_cluster_to_line(&mut self, advance: TextAdvance, height: f32) {
        self.append_cluster_to_line(advance.value, height);
        self.line.advance = advance;
    }

    /// Add inline box to line
    pub fn append_inline_box_to_line(&mut self, next_x: f32, box_height: f32) {
        self.item_idx += 1;
        self.line.items.end += 1;
        self.line.advance = TextAdvance::from_width(next_x);
        self.add_line_height(box_height);
    }

    /// Store the current iteration state so that we can revert to it if we later want to take
    /// the line breaking opportunity at this point.
    fn mark_line_break_opportunity(&mut self) {
        self.prev_boundary = Some(PrevBoundaryState {
            item_idx: self.item_idx,
            run_idx: self.run_idx,
            cluster_idx: self.cluster_idx,
            state: self.line.clone(),
        });
    }

    /// Store the current iteration state so that we can revert to it if we later want to take
    /// an *emergency* line breaking opportunity at this point.
    fn mark_emergency_break_opportunity(&mut self) {
        self.emergency_boundary = Some(PrevBoundaryState {
            item_idx: self.item_idx,
            run_idx: self.run_idx,
            cluster_idx: self.cluster_idx,
            state: self.line.clone(),
        });
    }

    /// Revert boundary state to prev state
    fn reset_to(&mut self, prev_state: PrevBoundaryState) {
        self.item_idx = prev_state.item_idx;
        self.run_idx = prev_state.run_idx;
        self.cluster_idx = prev_state.cluster_idx;
        self.line = prev_state.state;
    }

    #[inline(always)]
    fn add_line_height(&mut self, height: f32) {
        self.line.running_line_height = self.line.running_line_height.max(height);
        self.line.max_height_exceeded = self.line.running_line_height > self.line_max_height;
    }

    /// Get the max-advance of the entire layout
    #[inline(always)]
    pub fn layout_max_advance(&self) -> f32 {
        self.layout_max_advance
    }
    /// Set the max-advance of the entire layout
    #[inline(always)]
    pub fn set_layout_max_advance(&mut self, advance: f32) {
        self.layout_max_advance = advance;
    }

    /// Get the max-advance of the current line
    #[inline(always)]
    pub fn line_max_advance(&self) -> f32 {
        self.line_max_advance
    }
    /// Set the max-advance of the current line
    #[inline(always)]
    pub fn set_line_max_advance(&mut self, advance: f32) {
        self.line_max_advance = advance;
    }

    /// Get the max-height of the current line
    #[inline(always)]
    pub fn line_max_height(&self) -> f32 {
        self.line_max_height
    }
    /// Set the max-height of the current line.
    #[inline(always)]
    pub fn set_line_max_height(&mut self, height: f32) {
        self.line_max_height = height;
    }

    /// Get the x-offset of the current line
    #[inline(always)]
    pub fn line_x(&self) -> f32 {
        self.line_x
    }
    /// Set the x-offset for the current line.
    #[inline(always)]
    pub fn set_line_x(&mut self, x: f32) {
        self.line_x = x;
    }

    /// Get the y-offset of the current line
    #[inline(always)]
    pub fn line_y(&self) -> f64 {
        self.line_y
    }
    /// Set the y-offset for the current line.
    #[inline(always)]
    pub fn set_line_y(&mut self, y: f64) {
        self.line_y = y;
    }
}

/// Line breaking support for a paragraph.
pub struct BreakLines<'a, B: Brush> {
    layout: &'a mut Layout<B>,
    lines: LineLayout,
    state: BreakerState,
    prev_state: Option<BreakerState>,
    done: bool,
}

impl<'a, B: Brush> BreakLines<'a, B> {
    pub(crate) fn new(layout: &'a mut Layout<B>) -> Self {
        layout.data.width = 0.;
        layout.data.height = 0.;
        let mut lines = LineLayout::default();
        lines.swap(&mut layout.data);
        lines.lines.clear();
        lines.line_items.clear();
        Self {
            layout,
            lines,
            state: BreakerState::default(),
            prev_state: None,
            done: false,
        }
    }

    /// Reset state when a line has been committed
    fn start_new_line(
        &mut self,
        reason: BreakReason,
        max_advance: f32,
        line_indent: f32,
    ) -> Option<YieldData> {
        commit_line(
            self.layout,
            &mut self.lines,
            &mut self.state.line,
            max_advance,
            reason,
            line_indent,
        );

        let line_height = self.state.line.running_line_height;
        let line_y_start = self.state.line_y;

        self.state.items = self.lines.line_items.len();
        self.state.lines = self.lines.lines.len();
        self.state.line.advance = TextAdvance::default();
        self.state.line.running_line_height = 0.;
        self.state.prev_boundary = None;
        self.state.emergency_boundary = None;

        self.finish_line(self.lines.lines.len() - 1, line_height);

        self.state.line_y += line_height as f64;

        Some(YieldData::LineBreak(
            self.last_line_data(reason, line_y_start),
        ))
    }

    #[inline(always)]
    fn last_line_data(&self, reason: BreakReason, line_y_start: f64) -> LineBreakData {
        let line = self.lines.lines.last().unwrap();
        LineBreakData {
            reason,
            advance: line.metrics.advance,
            line_height: line.size(),
            line_y_start,
            line_y_end: self.state.line_y,
        }
    }

    #[inline(always)]
    fn max_height_break_data(&self, line_height: f32) -> Option<YieldData> {
        Some(YieldData::MaxHeightExceeded(MaxHeightBreakData {
            advance: self.state.line.advance.value,
            line_height,
        }))
    }

    #[inline(always)]
    pub fn state(&self) -> &BreakerState {
        &self.state
    }

    #[inline(always)]
    pub fn state_mut(&mut self) -> &mut BreakerState {
        &mut self.state
    }

    /// Returns the last committed line's metrics while line breaking is paused.
    ///
    /// Custom float layout can inspect the advance excluding hanging whitespace
    /// before accepting a line, or use [`Self::revert_to`] to try a wider slot.
    pub fn last_line_metrics(&self) -> Option<&LineMetrics> {
        self.lines.lines.last().map(|line| &line.metrics)
    }

    /// Set the max-advance of the previous line.
    ///
    /// This is an escape-hatch for allowing a custom width for
    /// alignment on each line, which is different to the breaking width.
    ///
    /// Should be used in combination with [`AlignmentOptions`](crate::AlignmentOptions::align_when_overflowing).
    ///
    /// This method changes a line's reported [`inline_max_coord`](LineMetrics::inline_max_coord), so
    /// if you use this method and read that value, you should be cautious.
    // This escape hatch has not been carefully evaluated for unexpected consequences.
    // It's current motivation is cases where Parley gives different line breaking results
    // than blink, for reasons which haven't been fully understood.
    #[doc(hidden)]
    pub fn set_prior_line_width(&mut self, advance: f32) {
        if let Some(line) = self.lines.lines.last_mut() {
            line.metrics.inline_max_coord = line.metrics.inline_min_coord + advance;
        }
    }

    /// Reverts the to an externally saved state.
    pub fn revert_to(&mut self, state: BreakerState) {
        self.state = state;
        self.lines.lines.truncate(self.state.lines);
        self.lines.line_items.truncate(self.state.items);
        self.done = false;
    }

    /// Reverts the last computed line, returning to the previous state.
    #[inline(always)]
    pub fn revert(&mut self) -> bool {
        if let Some(state) = self.prev_state.take() {
            self.revert_to(state);
            true
        } else {
            false
        }
    }

    /// Returns the y-coordinate of the top of the current line
    #[inline(always)]
    pub fn committed_y(&self) -> f64 {
        self.state.line_y
    }

    /// Returns true if all the text has been placed into lines.
    #[inline(always)]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Computes the next line in the paragraph. Returns the advance and size
    /// (width and height for horizontal layouts) of the line.
    #[inline(always)]
    pub fn break_next(&mut self) -> Option<YieldData> {
        self.break_next_line_or_box()
    }

    /// Computes the next line in the paragraph. Returns the advance and size
    /// (width and height for horizontal layouts) of the line.
    fn break_next_line_or_box(&mut self) -> Option<YieldData> {
        assert!(
            self.state.layout_max_advance == f32::INFINITY
                || self.state.line_max_advance - self.state.layout_max_advance < 1.0
        );

        // Maintain iterator state
        if self.done {
            return None;
        }
        self.prev_state = Some(self.state.clone());

        // HACK: ignore max_advance for empty layouts
        // Prevents crash when width is too small (https://github.com/linebender/parley/issues/186)
        let max_advance =
            if self.layout.data.text_len == 0 && self.layout.data.inline_boxes.is_empty() {
                f32::MAX
            } else {
                self.state.line_max_advance
            };

        let line_indent = self.resolve_indent();

        let max_advance = max_advance - line_indent;

        // dbg!(&self.layout.items);

        // println!("\nBREAK NEXT");
        // dbg!(&self.state.line.items);

        // Iterate over remaining runs in the Layout
        let item_count = self.layout.data.items.len();
        while self.state.item_idx < item_count {
            let item = &self.layout.data.items[self.state.item_idx];

            // println!(
            //     "\nitem = {} {:?}. x: {}",
            //     self.state.item_idx, item.kind, self.state.line.advance.value
            // );
            // dbg!(&self.state.line.items);

            match item.kind {
                LayoutItemKind::InlineBox => {
                    let inline_box = &self.layout.data.inline_boxes[item.index];

                    let (width_contribution, height_contribution) = match inline_box.kind {
                        InlineBoxKind::InFlow => (inline_box.width, inline_box.height),
                        InlineBoxKind::OutOfFlow => (0.0, 0.0),
                        // If the box is a `CustomOutOfFlow` box then we yield control flow back to the caller.
                        // It is then the caller's responsibility to handle placement of the box.
                        InlineBoxKind::CustomOutOfFlow => {
                            return Some(YieldData::InlineBoxBreak(BoxBreakData {
                                inline_box_id: inline_box.id,
                                inline_box_index: item.index,
                                advance: self.state.line.advance.value,
                            }));
                        }
                    };

                    // Compute the x position of the content being currently processed
                    let next_x = self.state.line.advance.value + width_contribution;

                    // println!("BOX next_x: {}", next_x);

                    let box_will_be_appended =
                        next_x <= max_advance || self.state.line.advance.value == 0.0;
                    if height_contribution > self.state.line_max_height && box_will_be_appended {
                        return self.max_height_break_data(height_contribution);
                    }

                    // If the box fits on the current line (or we are at the start of the current line)
                    // then simply move on to the next item
                    if next_x <= max_advance || self.state.line.text_wrap_mode != TextWrapMode::Wrap
                    {
                        // println!("BOX FITS");

                        self.state
                            .append_inline_box_to_line(next_x, height_contribution);

                        // We can always line break after an inline box
                        self.state.mark_line_break_opportunity();
                    } else {
                        // If we're at the start of the line, this box will never fit, so consume it and accept the overflow.
                        let reason = if self.state.line.advance.value == 0.0 {
                            // println!("BOX EMERGENCY BREAK");
                            self.state
                                .append_inline_box_to_line(next_x, height_contribution);
                            BreakReason::Emergency
                        } else {
                            // println!("BOX BREAK");
                            BreakReason::Regular
                        };
                        return self.start_new_line(reason, max_advance, line_indent);
                    }
                }
                LayoutItemKind::TextRun => {
                    let run_idx = item.index;
                    let run_data = &self.layout.data.runs[run_idx];

                    let run = Run::new(self.layout, 0, 0, run_data, None);
                    let cluster_start = run_data.cluster_range.start;
                    let cluster_end = run_data.cluster_range.end;

                    // println!("TextRun ({:?})", &run_data.text_range);

                    // Iterate over remaining clusters in the Run
                    while self.state.cluster_idx < cluster_end {
                        let cluster = run.get(self.state.cluster_idx - cluster_start).unwrap();

                        // Retrieve metadata about the cluster
                        let is_ligature_continuation = if run.is_rtl() {
                            // The stored flags are visual; break opportunities
                            // are visited in logical order.
                            self.state.cluster_idx > cluster_start
                                && self.layout.data.clusters[self.state.cluster_idx - 1]
                                    .is_ligature_component()
                        } else {
                            cluster.is_ligature_continuation()
                        };
                        let whitespace = cluster.info().whitespace();
                        let is_newline = whitespace == Whitespace::Newline;
                        let is_space = whitespace.is_space_or_nbsp();
                        let boundary = cluster.info().boundary();
                        let line_height = run.metrics().line_height;
                        let max_height_exceeded = self.state.line.max_height_exceeded;
                        let style = &self.layout.data.styles[cluster.data.style_index as usize];

                        // Lag text_wrap_mode style by one cluster
                        let text_wrap_mode = self.state.line.text_wrap_mode;
                        self.state.line.text_wrap_mode = style.text_wrap_mode;

                        if boundary == Boundary::Line && text_wrap_mode == TextWrapMode::Wrap {
                            // We do not currently handle breaking within a ligature, so we ignore boundaries in such a position.
                            //
                            // We also don't record boundaries when the advance is 0. As we do not want overflowing content to cause extra consecutive
                            // line breaks. We should accept the overflowing fragment in that scenario.
                            if !is_ligature_continuation && self.state.line.advance.value != 0.0 {
                                self.state.mark_line_break_opportunity();
                                // break_opportunity = true;
                            }
                        } else if is_newline {
                            if max_height_exceeded {
                                return self.max_height_break_data(line_height);
                            }
                            self.state
                                .append_cluster_to_line(self.state.line.advance.value, line_height);
                            return self.start_new_line(
                                BreakReason::Explicit,
                                max_advance,
                                line_indent,
                            );
                        } else if
                        // This text can contribute "emergency" line breaks.
                        style.overflow_wrap != OverflowWrap::Normal && !is_ligature_continuation
                        && text_wrap_mode == TextWrapMode::Wrap
                        // If we're at the start of the line, this particular cluster will never fit, so it's not a valid emergency break opportunity.
                        && self.state.line.advance.value != 0.0
                        {
                            self.state.mark_emergency_break_opportunity();
                        }

                        // Consume a shaping cluster atomically, but account
                        // for each of its text items. Assigning the whole
                        // advance to the last component's group loses an
                        // earlier item's unrounded tail and disagrees with
                        // intrinsic width and positioned line-item geometry.
                        let end = shaping_cluster_end(
                            &self.layout.data.clusters,
                            self.state.cluster_idx,
                            cluster_end,
                            run.is_rtl(),
                        );
                        let next_advance = if let Some(policy) =
                            &self.layout.data.text_item_quantization
                        {
                            let mut next = self.state.line.advance;
                            for index in self.state.cluster_idx..end {
                                next = next.with_text(
                                    self.layout.data.clusters[index].advance,
                                    Some(policy.cluster(index)),
                                );
                            }
                            next
                        } else {
                            let advance = self.layout.data.clusters[self.state.cluster_idx..end]
                                .iter()
                                .map(|cluster| cluster.advance)
                                .sum();
                            self.state.line.advance.with_text(advance, None)
                        };
                        self.state.cluster_idx = end - 1;
                        let next_x = next_advance.value;

                        // println!("Cluster {} next_x: {}", self.state.cluster_idx, next_x);

                        // If the content fits (the x position does NOT exceed max_advance)
                        //
                        // We simply append the cluster(s) to the current line
                        if next_x <= max_advance {
                            if max_height_exceeded {
                                return self.max_height_break_data(line_height);
                            }
                            self.state
                                .append_text_cluster_to_line(next_advance, line_height);
                            if is_space {
                                self.state.line.num_spaces += 1;
                            }
                        }
                        // Else we attempt to line break:
                        //
                        // This will only succeed if there is an available line-break opportunity that has been marked earlier
                        // in the line. If there is no such line-breaking opportunity (such as if wrapping is disabled), then
                        // we fall back to appending the content to the line anyway.
                        else {
                            // Case: cluster is a space character (and wrapping is enabled)
                            //
                            // We hang any overflowing whitespace and then line-break.
                            if is_space && text_wrap_mode == TextWrapMode::Wrap {
                                if max_height_exceeded {
                                    return self.max_height_break_data(line_height);
                                }
                                self.state
                                    .append_text_cluster_to_line(next_advance, line_height);
                                return self.start_new_line(
                                    BreakReason::Regular,
                                    max_advance,
                                    line_indent,
                                );
                            }
                            // Case: we have previously encountered a REGULAR line-breaking opportunity in the current line
                            //
                            // We "take" the line-breaking opportunity by starting a new line and resetting our
                            // item/run/cluster iteration state back to how it was when the line-breaking opportunity was encountered
                            else if let Some(prev) = self.state.prev_boundary.take() {
                                self.state.reset_to(prev);
                                return self.start_new_line(
                                    BreakReason::Regular,
                                    max_advance,
                                    line_indent,
                                );
                            }
                            // Case: we have previously encountered an EMERGENCY line-breaking opportunity in the current line
                            //
                            // We "take" the line-breaking opportunity by starting a new line and resetting our
                            // item/run/cluster iteration state back to how it was when the line-breaking opportunity was encountered
                            else if let Some(prev_emergency) =
                                self.state.emergency_boundary.take()
                            {
                                self.state.reset_to(prev_emergency);
                                return self.start_new_line(
                                    BreakReason::Emergency,
                                    max_advance,
                                    line_indent,
                                );
                            }
                            // Case: no line-breaking opportunities available
                            //
                            // This can happen when wrapping is disabled (TextWrapMode::NoWrap) or when no wrapping opportunities
                            // (according to our `OverflowWrap` and `WordBreak` styles) have yet been encountered.
                            //
                            // We fall back to appending the content to the line.
                            else {
                                if max_height_exceeded {
                                    return self.max_height_break_data(line_height);
                                }
                                self.state
                                    .append_text_cluster_to_line(next_advance, line_height);
                            }
                        }
                    }
                    self.state.run_idx += 1;
                    self.state.item_idx += 1;
                }
            }
        }

        if self.state.line.items.end == 0 {
            self.state.line.items.end = 1;
        }
        self.done = true;
        self.start_new_line(BreakReason::None, max_advance, line_indent)
    }

    /// Computes the next line in the paragraph by character count.
    ///
    /// This method breaks lines based on the number of characters rather than advance width.
    /// Each text cluster (including whitespace and newlines) counts as 1 character.
    /// Each inline box also counts as 1 character.
    /// Ligature components each count separately (matching character count).
    ///
    /// Unlike `break_next`, this method does not respect normal line break opportunities and
    /// will break exactly when the character limit is reached. It does not break on newlines, for example.
    ///
    /// Inline boxes are supported and each contributes as 1 character.
    pub fn break_next_with_length(&mut self, max_chars: u32) -> Option<()> {
        if self.done {
            return None;
        }

        let line_indent = self.resolve_indent();

        // Track cluster count for this line
        let mut char_count: u32 = 0;

        let item_count = self.layout.data.items.len();
        while self.state.item_idx < item_count {
            let item = &self.layout.data.items[self.state.item_idx];

            match item.kind {
                LayoutItemKind::InlineBox => {
                    let inline_box = &self.layout.data.inline_boxes[item.index];

                    if inline_box.kind != InlineBoxKind::InFlow {
                        self.state
                            .append_inline_box_to_line(self.state.line.advance.value, 0.0);
                        continue;
                    }

                    // Check if adding this box would exceed the limit
                    if char_count >= max_chars && max_chars != 0 {
                        // Break before this box
                        self.start_new_line(BreakReason::Regular, f32::MAX, line_indent);
                        return Some(());
                    }

                    // Compute the x position for the line width tracking
                    let next_x = self.state.line.advance.value + inline_box.width;
                    self.state
                        .append_inline_box_to_line(next_x, inline_box.height);
                    char_count += 1;

                    // Check if we've reached the limit after adding this box
                    if char_count >= max_chars {
                        // Check if we've consumed all content (this is the last line).
                        let is_last_item = self.state.item_idx >= self.layout.data.items.len();
                        let break_reason = if is_last_item {
                            BreakReason::None
                        } else {
                            BreakReason::Regular
                        };

                        if break_reason == BreakReason::None {
                            self.done = true;
                        }
                        self.start_new_line(break_reason, f32::MAX, line_indent);
                        return Some(());
                    }
                }
                LayoutItemKind::TextRun => {
                    let run_idx = item.index;
                    let run_data = &self.layout.data.runs[run_idx];
                    let run = Run::new(self.layout, 0, 0, run_data, None);
                    let cluster_start = run_data.cluster_range.start;
                    let cluster_end = run_data.cluster_range.end;

                    while self.state.cluster_idx < cluster_end {
                        let cluster = run.get(self.state.cluster_idx - cluster_start).unwrap();

                        // Check if we should break before this cluster
                        if char_count >= max_chars && max_chars != 0 {
                            self.start_new_line(BreakReason::Regular, f32::MAX, line_indent);
                            return Some(());
                        }

                        let whitespace = cluster.info().whitespace();
                        let is_newline = whitespace == Whitespace::Newline;
                        let is_space = whitespace.is_space_or_nbsp();
                        let advance = cluster.advance();

                        // Compute the x position.
                        // Newlines don't contribute to line width (matching break_next behavior).
                        let next_advance = self.state.line.advance.with_text(
                            if is_newline { 0.0 } else { advance },
                            self.layout
                                .data
                                .text_item_quantization
                                .as_ref()
                                .map(|policy| policy.cluster(self.state.cluster_idx)),
                        );
                        let line_height = run.metrics().line_height;
                        self.state
                            .append_text_cluster_to_line(next_advance, line_height);
                        char_count += 1;

                        if is_space {
                            self.state.line.num_spaces += 1;
                        }

                        // Check if we've reached the limit after adding this cluster
                        if char_count >= max_chars {
                            // Determine the break reason:
                            // - BreakReason::None for the last line (end of content)
                            // - BreakReason::Explicit if this line ends with a newline
                            // - BreakReason::Regular for soft wraps
                            let is_last_cluster_of_run = self.state.cluster_idx >= cluster_end;
                            let is_last_item =
                                self.state.item_idx + 1 >= self.layout.data.items.len();
                            let break_reason = if is_last_cluster_of_run && is_last_item {
                                BreakReason::None
                            } else if is_newline {
                                BreakReason::Explicit
                            } else {
                                BreakReason::Regular
                            };

                            if break_reason == BreakReason::None {
                                self.done = true;
                            }
                            self.start_new_line(break_reason, f32::MAX, line_indent);
                            return Some(());
                        }
                    }
                    self.state.run_idx += 1;
                    self.state.item_idx += 1;
                }
            }
        }

        // Commit the final line (only reached if content remains after all break_next_with_length calls)
        if self.state.line.items.end == 0 {
            self.state.line.items.end = 1;
        }
        self.done = true;
        self.start_new_line(BreakReason::None, f32::MAX, line_indent);
        Some(())
    }

    /// Breaks all remaining lines with the specified maximum advance. This
    /// consumes the line breaker.
    pub fn break_remaining(mut self, max_advance: f32) {
        // println!("\nDEBUG ITEMS");
        // for item in &self.layout.items {
        //     match item.kind {
        //         LayoutItemKind::InlineBox => println!("{:?}", item.kind),
        //         LayoutItemKind::TextRun => {
        //             let run_data = &self.layout.runs[item.index];
        //             println!("{:?} ({:?})", item.kind, &run_data.text_range);
        //         }
        //     }
        // }

        // println!("\nBREAK ALL");
        self.state.layout_max_advance = max_advance;
        self.state.line_max_advance = max_advance;
        while self.break_next().is_some() {}
        self.finish();
    }

    /// Consumes the line breaker and finalizes all line computations.
    pub fn finish(mut self) {
        if self.layout.data.text_len == 0 {
            if let Some(line) = self.lines.line_items.first_mut() {
                line.text_range = 0..0;
                line.cluster_range = 0..0;
            }
        }
    }

    #[inline]
    fn resolve_indent(&self) -> f32 {
        let should_indent = {
            let is_scope_line = if self.layout.data.indent_options.each_line {
                self.lines.lines.is_empty()
                    || self.lines.lines.last().map(|l| l.break_reason)
                        == Some(BreakReason::Explicit)
            } else {
                self.lines.lines.is_empty()
            };
            is_scope_line ^ self.layout.data.indent_options.hanging
        };

        if should_indent {
            self.layout.data.indent_amount
        } else {
            0.0
        }
    }

    fn finish_line(&mut self, line_idx: usize, line_height: f32) {
        let prev_line_metrics = match line_idx {
            0 => None,
            idx => Some(self.lines.lines[idx - 1].metrics),
        };
        let line = &mut self.lines.lines[line_idx];

        // Reset metrics for line
        line.metrics.ascent = 0.;
        line.metrics.descent = 0.;
        line.metrics.leading = 0.;
        line.metrics.offset = 0.;
        line.text_range.start = usize::MAX;

        line.metrics.line_height = line_height;

        if line.item_range.is_empty() {
            line.text_range = self.layout.data.text_len..self.layout.data.text_len;
        }
        // Compute metrics for the line, but ignore trailing whitespace.
        let mut have_metrics = false;
        let mut needs_reorder = false;
        for line_item in self.lines.line_items[line.item_range.clone()]
            .iter_mut()
            .rev()
        {
            match line_item.kind {
                LayoutItemKind::InlineBox => {
                    let item = &self.layout.data.inline_boxes[line_item.index];

                    // Advance is already computed in "commit line" for items
                    if item.kind == InlineBoxKind::InFlow {
                        // Default vertical alignment is to align the bottom of boxes with the text baseline.
                        // This is equivalent to the entire height of the box being "ascent"
                        line.metrics.ascent = line.metrics.ascent.max(item.height);

                        // Mark us as having seen non-whitespace content on this line
                        have_metrics = true;
                    }
                }
                LayoutItemKind::TextRun => {
                    line_item.compute_whitespace_properties(&self.layout.data);

                    // Compute the text range for the line
                    // Q: Can we not simplify this computation by assuming that items are in order?
                    line.text_range.end = line.text_range.end.max(line_item.text_range.end);
                    line.text_range.start = line.text_range.start.min(line_item.text_range.start);

                    // Mark line as needing bidi re-ordering if it contains any runs with non-zero bidi level
                    // (zero is the default level, so this is equivalent to marking lines that have multiple levels)
                    if line_item.bidi_level != 0 {
                        needs_reorder = true;
                    }

                    // Compute the run's advance by summing the advances of its constituent clusters
                    line_item.advance = self.layout.data.clusters[line_item.cluster_range.clone()]
                        .iter()
                        .map(|c| c.advance)
                        .sum();

                    // Ignore trailing whitespace for metrics computation
                    // (we are iterating backwards so trailing whitespace comes first)
                    if !have_metrics && line_item.is_whitespace {
                        continue;
                    }

                    // Compute the run's vertical metrics
                    let run = &self.layout.data.runs[line_item.index];
                    line.metrics.ascent = line.metrics.ascent.max(run.metrics.ascent);
                    line.metrics.descent = line.metrics.descent.max(run.metrics.descent);

                    // Mark us as having seen non-whitespace content on this line
                    have_metrics = true;
                }
            }
        }

        // Reorder the items within the line (if required). Reordering is required if the line contains
        // a mix of bidi levels (a mix of LTR and RTL text)
        let item_count = line.item_range.end - line.item_range.start;
        if needs_reorder && item_count > 1 {
            reorder_line_items(&mut self.lines.line_items[line.item_range.clone()]);
        }

        if let Some(policy) = &self.layout.data.text_item_quantization {
            let items = &mut self.lines.line_items[line.item_range.clone()];
            let mut pending: Option<(usize, usize, f32)> = None;
            for index in 0..items.len() {
                let item = &items[index];
                let group = item
                    .is_text_run()
                    .then(|| policy.cluster_groups.get(item.cluster_range.start).copied())
                    .flatten();
                if let Some((previous, last, raw)) = pending.take() {
                    if group == Some(previous) {
                        pending = Some((previous, index, raw + item.advance));
                        continue;
                    }
                    items[last].advance += policy.round(raw) - raw;
                }
                pending = group.map(|group| (group, index, items[index].advance));
            }
            if let Some((_, last, raw)) = pending {
                items[last].advance += policy.round(raw) - raw;
            }
        }

        // Compute size of line's trailing whitespace. "Trailing" is considered the right edge
        // for LTR text and the left edge for RTL text.
        let run = if self.layout.is_rtl() {
            self.lines.line_items[line.item_range.clone()].first()
        } else {
            self.lines.line_items[line.item_range.clone()].last()
        };
        line.metrics.trailing_whitespace = run
            .filter(|item| item.is_text_run() && item.has_trailing_whitespace)
            .map(|run| {
                fn whitespace_advance<'c, I: Iterator<Item = &'c ClusterData>>(clusters: I) -> f32 {
                    clusters
                        .take_while(|cluster| cluster.info.whitespace() != Whitespace::None)
                        .map(|cluster| cluster.advance)
                        .sum()
                }

                let clusters = &self.layout.data.clusters[run.cluster_range.clone()];
                if run.is_rtl() {
                    whitespace_advance(clusters.iter())
                } else {
                    whitespace_advance(clusters.iter().rev())
                }
            })
            .unwrap_or(0.0);

        if let Some(policy) = &self.layout.data.text_item_quantization {
            if let Some(run) = run.filter(|run| run.is_text_run() && !run.cluster_range.is_empty())
            {
                let group = policy.cluster_groups[run.cluster_range.start];
                let raw = self.lines.line_items[line.item_range.clone()]
                    .iter()
                    .filter(|item| {
                        item.is_text_run()
                            && !item.cluster_range.is_empty()
                            && policy.cluster_groups[item.cluster_range.start] == group
                    })
                    .flat_map(|item| &self.layout.data.clusters[item.cluster_range.clone()])
                    .map(|cluster| cluster.advance)
                    .sum::<f32>();
                line.metrics.trailing_whitespace =
                    policy.round(raw) - policy.round(raw - line.metrics.trailing_whitespace);
            }
        }

        if !have_metrics {
            // Line consisting entirely of whitespace?
            if !line.item_range.is_empty() {
                let line_item = &self.lines.line_items[line.item_range.start];
                if line_item.is_text_run() {
                    let run = &self.layout.data.runs[line_item.index];
                    line.metrics.ascent = run.metrics.ascent;
                    line.metrics.descent = run.metrics.descent;
                }
            } else if let Some(metrics) = prev_line_metrics {
                // HACK: copy metrics from previous line if we don't have
                // any; this should only occur for an empty line following
                // a newline at the end of a layout
                line.metrics = metrics;
                // If we have no items on this line, it must be the last (empty)
                // line in a layout following a newline. Commit an empty run so
                // that AccessKit has a node with which to identify the visual
                // cursor position
                if let Some((index, run)) = self
                    .layout
                    .data
                    .runs
                    .iter()
                    .enumerate()
                    .rfind(|(_, run)| !run.text_range.is_empty())
                {
                    let run_index = self.lines.line_items.len();
                    let cluster = run.cluster_range.end;
                    let text = run.text_range.end;
                    self.lines.line_items.push(LineItemData {
                        kind: LayoutItemKind::TextRun,
                        index,
                        bidi_level: 0,
                        advance: 0.,
                        is_whitespace: false,
                        has_trailing_whitespace: false,
                        cluster_range: cluster..cluster,
                        text_range: text..text,
                    });
                    line.item_range = run_index..run_index + 1;
                }
            }
        }

        line.metrics.leading =
            line.metrics.line_height - (line.metrics.ascent + line.metrics.descent);

        // Whether metrics should be quantized to pixel boundaries
        let quantize = self.layout.data.quantize;

        let (ascent, descent) = if quantize {
            // We mimic Chrome in rounding ascent and descent separately,
            // before calculating the rest.
            // See lines_integral_line_height_ascent_descent_rounding() for more details.
            (line.metrics.ascent.round(), line.metrics.descent.round())
        } else {
            (line.metrics.ascent, line.metrics.descent)
        };

        let (leading_above, leading_below) = if quantize {
            // Calculate leading using the rounded ascent and descent.
            let leading = line.metrics.line_height - (ascent + descent);
            // We mimic Chrome in giving 'below' the larger leading half.
            // Although the comment in Chromium's NGLineHeightMetrics::AddLeading function
            // in ng_line_height_metrics.cc claims it's for legacy test compatibility.
            // So we might want to think about giving 'above' the larger half instead.
            let above = (leading * 0.5).floor();
            let below = leading.round() - above;
            (above, below)
        } else {
            (line.metrics.leading * 0.5, line.metrics.leading * 0.5)
        };

        let y = self.state.line_y;
        line.metrics.baseline =
            ascent + leading_above + if quantize { y.round() as f32 } else { y as f32 };

        // Small line heights will cause leading to be negative.
        // Negative leadings are correct for baseline calculation, but not for min/max coords.
        // We clamp leading to zero for the purposes of min/max coords,
        // which in turn clamps the selection box minimum height to ascent + descent.
        line.metrics.block_min_coord = line.metrics.baseline - ascent - leading_above.max(0.);
        line.metrics.block_max_coord = line.metrics.baseline + descent + leading_below.max(0.);

        // let max_advance = if self.state.line_max_advance < f32::MAX {
        //     self.state.line_max_advance
        // } else {
        //     line.metrics.advance - line.metrics.trailing_whitespace
        // };

        line.metrics.inline_min_coord = self.state.line_x;
        line.metrics.inline_max_coord = self.state.line_x + self.state.line_max_advance;
    }
}

impl<B: Brush> Drop for BreakLines<'_, B> {
    fn drop(&mut self) {
        // Compute the overall width and height of the entire layout
        // The "width" excludes trailing whitespace. The "full_width" includes it.
        let mut layout_width = 0_f32;
        let mut layout_full_width = 0_f32;
        let mut height = 0_f64; // f32 causes test failures due to accumulated error
        for line in &mut self.lines.lines {
            let indent_extra = line.indent.max(0.0);
            let line_max = line.metrics.inline_min_coord + line.metrics.advance + indent_extra;
            layout_full_width = layout_full_width.max(line_max);
            layout_width = layout_width.max(line_max - line.metrics.trailing_whitespace);
            height += line.metrics.line_height as f64;
        }

        // If laying out with infinite width constraint, then set all lines' "max_width"
        // to the measured width of the longest line.
        if self.state.layout_max_advance >= f32::MAX {
            for line in &mut self.lines.lines {
                if line.metrics.inline_max_coord >= f32::MAX {
                    line.metrics.inline_max_coord = layout_width;
                }
            }
        }

        // Don't include the last line's line_height in the layout's height if the last line is empty
        if let Some(last_line) = self.lines.lines.last() {
            if last_line.item_range.is_empty() {
                height -= last_line.metrics.line_height as f64;
            }
        }

        // Save the computed widths/height to the layout
        self.layout.data.width = layout_width;
        self.layout.data.full_width = layout_full_width;
        self.layout.data.height = height as f32;
        self.layout.data.layout_max_advance = self.state.layout_max_advance;

        // for (i, line) in self.lines.lines.iter().enumerate() {
        //     println!("LINE {i} (h:{})", line.metrics.line_height);
        //     for item_idx in line.item_range.clone() {
        //         let item = &self.lines.line_items[item_idx];
        //         println!("  ITEM {:?} ({})", item.kind, item.advance);
        //     }
        // }

        // Save the computed lines to the layout
        self.lines.swap(&mut self.layout.data);
    }
}

fn commit_line<B: Brush>(
    layout: &Layout<B>,
    lines: &mut LineLayout,
    state: &mut LineState,
    max_advance: f32,
    break_reason: BreakReason,
    line_indent: f32,
) -> bool {
    // Ensure that the cluster and item endpoints are within range
    state.clusters.end = state.clusters.end.min(layout.data.clusters.len());
    state.items.end = state.items.end.min(layout.data.items.len());

    let start_item_idx = lines.line_items.len();
    // let start_run_idx = lines.line_items.last().map(|item| item.index).unwrap_or(0);

    let items_to_commit = &layout.data.items[state.items.clone()];

    // Compute first and last run index
    let is_text_run = |item: &LayoutItem| item.kind == LayoutItemKind::TextRun;
    let first_run_pos = items_to_commit.iter().position(is_text_run).unwrap_or(0);
    let last_run_pos = items_to_commit.iter().rposition(is_text_run).unwrap_or(0);

    // Iterate over the items to commit
    // println!("\nCOMMIT LINE");
    let mut last_item_kind = LayoutItemKind::TextRun;
    let mut committed_text_run = false;
    for (i, item) in items_to_commit.iter().enumerate() {
        // println!("i = {} index = {} {:?}", i, item.index, item.kind);

        match item.kind {
            LayoutItemKind::InlineBox => {
                let inline_box = &layout.data.inline_boxes[item.index];

                lines.line_items.push(LineItemData {
                    kind: LayoutItemKind::InlineBox,
                    index: item.index,
                    bidi_level: item.bidi_level,
                    advance: inline_box.width,

                    // These properties are ignored for inline boxes. So we just put a dummy value.
                    is_whitespace: false,
                    has_trailing_whitespace: false,
                    cluster_range: 0..0,
                    text_range: 0..0,
                });

                last_item_kind = item.kind;
            }
            LayoutItemKind::TextRun => {
                let run_data = &layout.data.runs[item.index];

                // Compute cluster range
                // The first and last ranges have overrides to account for line-breaks within runs
                let mut cluster_range = run_data.cluster_range.clone();
                if i == first_run_pos {
                    cluster_range.start = state.clusters.start;
                }
                if i == last_run_pos {
                    cluster_range.end = state.clusters.end;
                }

                if cluster_range.start >= run_data.cluster_range.end {
                    // println!("INVALID CLUSTER");
                    // dbg!(&run_data.text_range);
                    // dbg!(cluster_range);
                    continue;
                }

                last_item_kind = item.kind;
                committed_text_run = true;

                // Push run to line
                let run = Run::new(layout, 0, 0, run_data, None);
                let mut start = cluster_range.start;
                while start < cluster_range.end {
                    let end = layout.data.text_item_quantization.as_ref().map_or(
                        cluster_range.end,
                        |policy| {
                            (start + 1..cluster_range.end)
                                .find(|index| {
                                    policy.cluster_groups[*index] != policy.cluster_groups[start]
                                })
                                .unwrap_or(cluster_range.end)
                        },
                    );
                    let first_cluster = run.get(start - run_data.cluster_range.start).unwrap();
                    let last_cluster = run.get(end - run_data.cluster_range.start - 1).unwrap();
                    let text_range =
                        first_cluster.text_range().start..last_cluster.text_range().end;

                    lines.line_items.push(LineItemData {
                        kind: LayoutItemKind::TextRun,
                        index: item.index,
                        bidi_level: run_data.bidi_level,
                        advance: 0.,
                        is_whitespace: false,
                        has_trailing_whitespace: false,
                        cluster_range: start..end,
                        text_range,
                    });
                    start = end;
                }
            }
        }
    }
    // let end_run_idx = lines.line_items.last().map(|item| item.index).unwrap_or(0);
    let end_item_idx = lines.line_items.len();

    // Exclude the trailing space from justification space count.
    // Only subtract if the line actually ends with a space — with
    // WordBreak::BreakAll, regular breaks can land between non-space
    // characters, in which case there is no trailing space to exclude.
    let mut num_spaces = state.num_spaces;
    if break_reason == BreakReason::Regular
        && state.clusters.start < state.clusters.end
        && layout.data.clusters[state.clusters.end - 1]
            .info
            .whitespace()
            .is_space_or_nbsp()
    {
        num_spaces = num_spaces.saturating_sub(1);
    }

    lines.lines.push(LineData {
        item_range: start_item_idx..end_item_idx,
        max_advance,
        break_reason,
        num_spaces,
        indent: line_indent,
        metrics: LineMetrics {
            advance: state.advance.value,
            ..Default::default()
        },
        ..Default::default()
    });

    // Reset state for the new line
    state.num_spaces = 0;
    if committed_text_run {
        state.clusters.start = state.clusters.end;
    }

    state.items.start = match last_item_kind {
        // For text runs, the first item of line N+1 needs to be the SAME as
        // the last item for line N. This is because the item (if it a text run
        // may be split across the two lines with some clusters in line N and some
        // in line N+1). The item is later filtered out (see `continue` in loop above)
        // if there are not actually any clusters in line N+1.
        LayoutItemKind::TextRun => state.items.end.saturating_sub(1),
        // Inline boxes cannot be spread across multiple lines, so we should set
        // the first item of line N+1 to be the item AFTER the last item in line N.
        LayoutItemKind::InlineBox => state.items.end,
    };

    true
}

/// Reorder items within line according to the bidi levels of the items
fn reorder_line_items(runs: &mut [LineItemData]) {
    let run_count = runs.len();

    // Find the max level and the min *odd* level
    let mut max_level = 0;
    let mut lowest_odd_level = 255;
    for run in runs.iter() {
        let level = run.bidi_level;
        let is_odd = level & 1 != 0;

        // Update max level
        if level > max_level {
            max_level = level;
        }

        // Update min odd level
        if is_odd && level < lowest_odd_level {
            lowest_odd_level = level;
        }
    }

    // Iterate over bidi levels
    for level in (lowest_odd_level..=max_level).rev() {
        // Iterate over text runs
        let mut i = 0;
        while i < run_count {
            if runs[i].bidi_level >= level {
                let mut end = i + 1;
                while end < run_count && runs[end].bidi_level >= level {
                    end += 1;
                }

                let mut j = i;
                let mut k = end - 1;
                while j < k {
                    runs.swap(j, k);
                    j += 1;
                    k -= 1;
                }

                i = end;
            }
            i += 1;
        }
    }
}
