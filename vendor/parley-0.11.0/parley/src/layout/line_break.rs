// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Greedy line breaking.

use alloc::vec::Vec;

#[cfg(feature = "libm")]
#[allow(unused_imports)]
use core_maths::CoreFloat;
use parlance::BidiLevel;

use crate::layout::data::{AlignedSubtreeOffset, run_box_metrics};
use crate::layout::spacing::{EffectiveSpacing, Justification, is_word_separator};
use crate::layout::style_metrics::{
    BoxMetrics, InlineBoxPlacement, StyleMetrics, inline_box_placement,
};
use crate::layout::whitespace::atom_hanging_advance;
use crate::layout::{
    BreakReason, Layout, LayoutData, LayoutItem, LayoutItemKind, LineData, LineItemData,
    LineMetrics, Run,
};
use crate::style::Brush;
use crate::{
    BaselineShift, InlineBox, InlineBoxKind, OverflowWrap, TextWrapMode, VerticalAlign,
    WhiteSpaceCollapse,
};

use core::ops::Range;
use parley_engine::Atom;
use parley_engine::shape::{Character, Whitespace};
use smallvec::SmallVec;

#[derive(Default)]
struct LineLayout {
    lines: Vec<LineData>,
    line_items: Vec<LineItemData>,
    aligned_subtree_offsets: Vec<AlignedSubtreeOffset>,
}

impl LineLayout {
    fn swap<B: Brush>(&mut self, layout: &mut LayoutData<B>) {
        core::mem::swap(&mut self.lines, &mut layout.lines);
        core::mem::swap(&mut self.line_items, &mut layout.line_items);
        core::mem::swap(
            &mut self.aligned_subtree_offsets,
            &mut layout.aligned_subtree_offsets,
        );
    }

    /// Drops the lines from `len` onwards, along with their aligned subtree offsets.
    fn truncate(&mut self, len: usize) {
        self.lines.truncate(len);
        let offsets = self
            .lines
            .last()
            .map_or(0, |line| line.aligned_subtree_offsets.end as usize);
        self.aligned_subtree_offsets.truncate(offsets);
    }
}

#[derive(Clone, Default)]
struct LineState {
    x: f32,
    text_advance: super::text_advance::TextAdvance,
    items: Range<usize>,
    /// The line's shaped clusters, as a range into [`parley_engine::ShapedText::shaped_clusters`].
    /// The bounds are atom-aligned.
    clusters: Range<u32>,
    box_metrics: LineBoxMetrics,
    /// This is set to true if we encounter something on the line (either a glyph or an inline box)
    /// that is taller than the `line_max_height`. When in this state `break_next` should yield control
    /// flow to the caller to handle the constraint violation.
    ///
    /// This never happens when calling `break_all_lines` as it never sets `line_max_height`, and it defaults to `f32::MAX`.
    max_height_exceeded: bool,

    /// We lag the text-wrap-mode by one cluster due to line-breaking boundaries only
    /// being triggered on the cluster after the linebreak.
    text_wrap_mode: TextWrapMode,

    /// The number of word separators appended to the line so far.
    ///
    /// These are the line's justification opportunities, before subtracting those that end up in
    /// the line's hanging whitespace.
    num_word_separators: u32,
}

impl LineState {
    /// Reset the per-line running state in preparation for building a new line.
    fn reset(&mut self) {
        self.x = 0.0;
        self.text_advance = super::text_advance::TextAdvance::default();
        self.box_metrics.reset();
        self.num_word_separators = 0;
    }
}

/// The metrics of a line box.
///
/// Note a line box is distinct from an *inline* box. A line box is a single line within the layout,
/// and can hold multiple text runs and inline boxes.
///
/// Following CSS 2.2 § 10.8 (line height calculations in "Visual formatting model details"), line
/// boxes are sized to fit the line's inline content. Span boxes and inline boxes with a
/// parent-relative `vertical-align` are first aligned to each other relative to the baseline of
/// their parent span; the boxes aligned to each other in this way form an
/// [independent aligned subtree], rooted at the root span box or at a box with
/// `vertical-align: top | bottom`. Each subtree's extents are tracked relative to its own
/// baseline: the root aligned subtree's in [`Self::root`], those rooted at a `top`/`bottom` span
/// in [`SubtreeHistory`]. The subtrees are only positioned against each other once the line is
/// complete (see `BreakLines::finish_line`).
/// See <https://www.w3.org/TR/CSS22/visudet.html#line-height>.
///
/// [independent aligned subtree]: crate::layout::style_metrics#aligned-subtrees
#[derive(Clone, Copy, Debug)]
struct LineBoxMetrics {
    /// Extents of the root aligned subtree.
    root: SubtreeExtents,
    /// Height of the tallest aligned subtree rooted at a `top`/`bottom` span (see
    /// [`SubtreeHistory`]).
    non_root_height: f32,
    /// Height of the tallest `vertical-align: top` inline box, which is positioned against the
    /// line box rather than a baseline.
    line_relative_top_height: f32,
    /// Height of the tallest `vertical-align: bottom` inline box.
    line_relative_bottom_height: f32,
    /// Whether the line has any text or non-empty in-flow inline box. A line with only empty
    /// inline boxes is treated as having zero height (an "invisible" line box).
    has_content: bool,
    /// The layout item and style of the last text atom added, whose boxes are already on the line.
    last_text: (usize, u16),
}

/// Extents of one [independent aligned subtree] on the current line, measured from the subtree's
/// own baseline.
///
/// [independent aligned subtree]: crate::layout::style_metrics#aligned-subtrees
#[derive(Clone, Copy, Debug, PartialEq)]
struct SubtreeExtents {
    /// Style index of the subtree root: `0` for the root span box, otherwise a style with
    /// `vertical-align: top | bottom`.
    root: u16,
    /// The line-height expanded extents.
    ///
    /// The extents are in block flow direction; i.e., for horizontal text, these are vertical, and
    /// for vertical text, these are horizontal.
    line_box: Extents,
    /// The content extents.
    ///
    /// This covers, roughly, the glyphs and inline boxes. This does not take into account
    /// typographic leading, but only the typographic ascent and descent. In case of negative
    /// leading, this can be larger than [`Self::line_box`].
    ///
    /// Like [`Self::line_box`], these are in block flow direction.
    content_box: Extents,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Extents {
    /// The space over the baseline.
    over: f32,
    /// The space under the baseline.
    under: f32,
}

impl Default for Extents {
    fn default() -> Self {
        // Content with small line heights and/or with inline boxes sitting entirely above or
        // below the baseline can cause the baseline to fall outside the line box; i.e., extents
        // can be negative. Hence, we initialize the space over and under the baseline to negative
        // infinity. Lines with no contributing content resolve to zero via [`Self::or_zero`],
        // keeping the sentinel internal.
        Self {
            over: f32::NEG_INFINITY,
            under: f32::NEG_INFINITY,
        }
    }
}

impl Extents {
    /// Resolve unset (default) extents to zero.
    fn or_zero(self) -> Self {
        if self.over == f32::NEG_INFINITY && self.under == f32::NEG_INFINITY {
            Self {
                over: 0.,
                under: 0.,
            }
        } else {
            self
        }
    }

    /// Grow to include a box extending `over` above and `under` below a baseline that is itself
    /// `baseline_offset` above the extents' baseline.
    #[inline(always)]
    fn add(&mut self, baseline_offset: f32, over: f32, under: f32) {
        self.over = self.over.max(baseline_offset + over);
        self.under = self.under.max(under - baseline_offset);
    }

    fn height(self) -> f32 {
        let this = self.or_zero();
        this.over + this.under
    }
}

impl SubtreeExtents {
    fn new(root: u16) -> Self {
        Self {
            root,
            line_box: Extents::default(),
            content_box: Extents::default(),
        }
    }

    /// Grow to include a box whose baseline is `baseline_offset` above the subtree's baseline.
    #[inline(always)]
    fn add(&mut self, baseline_offset: f32, metrics: BoxMetrics) {
        self.line_box
            .add(baseline_offset, metrics.over, metrics.under);
        self.content_box
            .add(baseline_offset, metrics.ascent, metrics.descent);
    }
}

/// The extents of every [independent aligned subtree] on the current line other than the root
/// one (which is in [`LineBoxMetrics::root`]), i.e. those rooted at a span with
/// `vertical-align: top | bottom`. Empty for lines without such spans.
///
/// Reverting to a saved line-breaking opportunity has to restore these extents to what they were
/// at that opportunity. So that saving an opportunity doesn't have to copy them, this is a
/// history: [saving](Self::save) freezes the entries pushed so far, and growing a subtree whose
/// entry is frozen pushes its new extents instead of overwriting the old ones. Hence:
///
/// - the current extents of a subtree are the *last* entry with its root, and
/// - [restoring](Self::restore) a save drops every entry pushed since, leaving the frozen
///   entries as they were at that save.
///
/// [independent aligned subtree]: crate::layout::style_metrics#aligned-subtrees
#[derive(Clone, Default)]
struct SubtreeHistory {
    entries: Vec<SubtreeExtents>,
    /// The first `frozen_count` entries may be needed by a save, so are never modified.
    frozen_count: usize,
}

impl SubtreeHistory {
    /// The current extents of every subtree with content on the line, in no particular order.
    fn current(&self) -> SmallVec<[SubtreeExtents; 4]> {
        let mut current = SmallVec::<[SubtreeExtents; 4]>::new();
        for entry in self.entries.iter().rev() {
            if current.iter().rev().all(|s| s.root != entry.root) {
                current.push(*entry);
            }
        }
        current
    }

    /// Grow the subtree rooted at `root` to include a box whose baseline is `baseline_offset`
    /// above the subtree's baseline. Returns the subtree's new extents.
    fn grow(&mut self, root: u16, baseline_offset: f32, metrics: BoxMetrics) -> SubtreeExtents {
        let index = self.entries.iter().rposition(|s| s.root == root);
        let old = index.map(|i| self.entries[i]);
        let mut new = old.unwrap_or(SubtreeExtents::new(root));
        new.add(baseline_offset, metrics);
        if old != Some(new) {
            match index {
                Some(i) if i >= self.frozen_count => self.entries[i] = new,
                _ => self.entries.push(new),
            }
        }
        new
    }

    /// Save the current extents of every subtree, returning the length of the history to pass
    /// to [`Self::restore`].
    fn save(&mut self) -> usize {
        self.frozen_count = self.entries.len();
        self.frozen_count
    }

    /// Restore the extents of every subtree to what they were at the save that returned `len`.
    /// Earlier saves can still be restored afterwards; later ones can't.
    fn restore(&mut self, len: usize) {
        self.entries.truncate(len);
        self.frozen_count = len;
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.frozen_count = 0;
    }
}

impl Default for LineBoxMetrics {
    /// An empty line containing only the root aligned subtree, without a strut.
    fn default() -> Self {
        Self {
            root: SubtreeExtents::new(0),
            non_root_height: 0.,
            line_relative_top_height: 0.,
            line_relative_bottom_height: 0.,
            has_content: false,
            last_text: (usize::MAX, 0),
        }
    }
}

impl LineBoxMetrics {
    /// Reset to an empty line.
    fn reset(&mut self) {
        *self = Self::default();
    }

    /// Add a box whose baseline is `baseline_offset` above the baseline of the aligned subtree
    /// rooted at `aligned_subtree_root`.
    #[inline]
    fn add_box(
        &mut self,
        aligned_subtree_root: u16,
        baseline_offset: f32,
        metrics: BoxMetrics,
        subtrees: &mut SubtreeHistory,
    ) {
        if aligned_subtree_root == 0 {
            self.root.add(baseline_offset, metrics);
        } else {
            let subtree = subtrees.grow(aligned_subtree_root, baseline_offset, metrics);
            self.non_root_height = self.non_root_height.max(subtree.line_box.height());
        }
    }

    /// The line height seen so far.
    #[inline]
    fn line_height(&self) -> f32 {
        self.root
            .line_box
            .height()
            .max(self.non_root_height)
            .max(self.line_relative_top_height)
            .max(self.line_relative_bottom_height)
    }

    /// Add the span box generated by `style_index`, and those of any of its ancestors that are
    /// not yet on the line.
    fn add_style(
        &mut self,
        style_index: u16,
        style_metrics: &[StyleMetrics],
        contributed: &mut Vec<u16>,
        subtrees: &mut SubtreeHistory,
    ) {
        let mut index = style_index;
        while !contributed.contains(&index) {
            contributed.push(index);
            let Some(metrics) = style_metrics.get(usize::from(index)) else {
                return;
            };
            let span_box = BoxMetrics {
                ascent: metrics.ascent,
                descent: metrics.descent,
                over: metrics.over,
                under: metrics.under,
            };
            self.add_box(
                metrics.aligned_subtree_root,
                metrics.baseline_offset,
                span_box,
                subtrees,
            );
            if index == 0 {
                return;
            }
            index = metrics.parent;
        }
    }

    /// Add the glyphs of a text atom of `style_index` in layout item `item_idx` and text run
    /// `run_idx`. The atom's characters are `characters`, the first of which has style
    /// `style_index`.
    ///
    /// This adds the span boxes of the atom's styles together with its ancestors. When the first
    /// style's line height is [`LineHeight::MetricsRelative`] (which corresponds to CSS
    /// `line-height: normal`), it also adds the run's box, which matters when the run was shaped
    /// with a fallback font whose metrics differ from the style's first available font, the font
    /// the span box is built from. See [`run_box_metrics`]
    ///
    /// [`LineHeight::MetricsRelative`]: crate::LineHeight::MetricsRelative
    #[inline]
    fn add_text<B: Brush>(
        &mut self,
        item_idx: usize,
        run_idx: usize,
        style_index: u16,
        characters: &[Character],
        data: &LayoutData<B>,
        contributed: &mut Vec<u16>,
        subtrees: &mut SubtreeHistory,
    ) {
        self.has_content = true;
        // Consecutive atoms almost always come from the same run and style, whose boxes are then
        // already on the line, so we can exit early. In case the run has atoms with mixed styles,
        // new boxes may still be added.
        if self.last_text == (item_idx, style_index) && !data.runs[run_idx].has_mixed_style_atoms {
            return;
        }
        self.add_text_boxes(
            item_idx,
            run_idx,
            style_index,
            characters,
            data,
            contributed,
            subtrees,
        );
    }

    /// The part of [`Self::add_text`] for atoms whose boxes may not be on the line yet.
    ///
    /// Note we mark this `inline(never)`, to keep the common case fast.
    #[inline(never)]
    fn add_text_boxes<B: Brush>(
        &mut self,
        item_idx: usize,
        run_idx: usize,
        style_index: u16,
        characters: &[Character],
        data: &LayoutData<B>,
        contributed: &mut Vec<u16>,
        subtrees: &mut SubtreeHistory,
    ) {
        self.last_text = (item_idx, style_index);
        if contributed.last() != Some(&style_index) {
            self.add_style(style_index, &data.style_metrics, contributed, subtrees);
        }
        if data.runs[run_idx].has_mixed_style_atoms {
            // Add the spans of all the atom's other styles.
            for character in characters.iter().skip(1) {
                if character.style_index != style_index {
                    self.add_style(
                        character.style_index,
                        &data.style_metrics,
                        contributed,
                        subtrees,
                    );
                }
            }
        }
        let style = usize::from(style_index);
        let shaped_run = &data.shaped_text.runs()[run_idx];
        let Some(run_box) = data.runs[run_idx]
            .run_box
            .or_else(|| run_box_metrics(&data.styles[style], shaped_run, data.quantize))
        else {
            return;
        };
        let (baseline_offset, aligned_subtree_root) = data
            .style_metrics
            .get(style)
            .map_or((0., 0), |m| (m.baseline_offset, m.aligned_subtree_root));
        self.add_box(aligned_subtree_root, baseline_offset, run_box, subtrees);
    }

    /// Add an inline box extending `ascent` above and `descent` below a baseline that is
    /// `baseline_offset` above the baseline of the aligned subtree rooted at
    /// `aligned_subtree_root`.
    fn add_inline_box(
        &mut self,
        aligned_subtree_root: u16,
        baseline_offset: f32,
        ascent: f32,
        descent: f32,
        subtrees: &mut SubtreeHistory,
    ) {
        // Inline box extents are exact box sizes supplied by the caller, not font metrics;
        // rounding them would change the space the box reserves relative to its height.
        // Negative margins can make the reserved space negative; the box is still content.
        // Out-of-flow boxes have infinitely negative extents and are not.
        if (ascent != 0. || descent != 0.) && ascent.is_finite() && descent.is_finite() {
            self.has_content = true;
        }
        let inline_box = BoxMetrics {
            ascent,
            descent,
            over: ascent,
            under: descent,
        };
        self.add_box(aligned_subtree_root, baseline_offset, inline_box, subtrees);
    }

    /// Add an inline box with `vertical-align: top | bottom`, which only constrains the line
    /// box's height.
    fn add_line_relative_inline_box(&mut self, align: VerticalAlign, height: f32, quantize: bool) {
        let height = if quantize { height.round() } else { height };
        if height > 0. {
            self.has_content = true;
        }
        let slot = match align.shift {
            BaselineShift::Bottom => &mut self.line_relative_bottom_height,
            _ => &mut self.line_relative_top_height,
        };
        *slot = slot.max(height);
    }
}

#[derive(Clone)]
struct PrevBoundaryState {
    item_idx: usize,
    run_idx: usize,
    cluster_idx: u32,
    state: LineState,
    /// Length of [`BreakerState::contributed`] at this opportunity.
    contributed_len: usize,
    /// Length of [`BreakerState::subtrees`] at this opportunity, as [saved](SubtreeHistory::save).
    subtrees_len: usize,
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
    /// using [`BreakerState::set_line_max_height`].
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
    /// Iteration state: the current shaped cluster.
    ///
    /// This indexes into [`parley_engine::ShapedText::shaped_clusters`]. It's atom-aligned, pointing
    /// at the start of the next atom to consume.
    //
    // TODO: rename this `shaped_cluster_idx`
    cluster_idx: u32,

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
    /// Style indices whose span box has already been added to the current line (see
    /// [`LineBoxMetrics::add_style`]). Lives here rather than in [`LineState`] so that saving a
    /// line-breaking opportunity only records its length; reverting truncates it back.
    contributed: Vec<u16>,
    /// Extents of the aligned subtrees rooted at `top`/`bottom` spans on the current line. Like
    /// [`Self::contributed`], saving a line-breaking opportunity only records its length.
    subtrees: SubtreeHistory,

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
            contributed: Vec::new(),
            subtrees: SubtreeHistory::default(),
            prev_boundary: None,
            emergency_boundary: None,
        }
    }
}

impl BreakerState {
    /// Add the atom currently being evaluated to the current line.
    ///
    /// `style_index` is the style of the atom's first character. The span boxes of the atom's
    /// styles (and those of their ancestors) are added to the line too, as is the box of the atom's
    /// run, see [`LineBoxMetrics::add_text`]. `is_word_separator` is `true` iff the atom is a
    /// [word separator](`is_word_separator`), i.e., a justification opportunity.
    #[inline]
    fn append_atom_to_line<B: Brush>(
        &mut self,
        atom: &Atom<'_>,
        next_x: f32,
        style_index: u16,
        data: &LayoutData<B>,
        is_word_separator: bool,
    ) {
        self.line.items.end = self.item_idx + 1;
        let spacing = EffectiveSpacing::new(data.runs[self.run_idx].spacing, Justification::NONE);
        let allocation = data
            .text_item_quantization
            .as_ref()
            .map(|policy| policy.cluster(atom.shaped_clusters_range().start));
        let text_advance = self
            .line
            .text_advance
            .with_text(spacing.atom_advance(atom), allocation);
        self.line.clusters.end = atom.shaped_clusters_range().end;
        self.cluster_idx = atom.shaped_clusters_range().end;
        self.line.x = next_x;
        self.line.text_advance = super::text_advance::TextAdvance::from_width(next_x);
        self.line.text_advance = text_advance;
        self.line.num_word_separators += u32::from(is_word_separator);
        self.line.box_metrics.add_text(
            self.item_idx,
            self.run_idx,
            style_index,
            atom.characters(),
            data,
            &mut self.contributed,
            &mut self.subtrees,
        );
        self.update_max_height_exceeded();
    }

    /// Add an inline box to the line.
    ///
    /// `ascent` and `descent` are the distances the box extends above and below the line's root
    /// baseline respectively. A box with its bottom aligned to the baseline is simply one with a
    /// zero descent. The box grows the line only insofar as it extends beyond the text.
    pub fn append_inline_box_to_line(&mut self, next_x: f32, ascent: f32, descent: f32) {
        self.item_idx += 1;
        self.line.items.end += 1;
        self.line.x = next_x;
        self.line.text_advance = super::text_advance::TextAdvance::from_width(next_x);
        self.line
            .box_metrics
            .add_inline_box(0, 0., ascent, descent, &mut self.subtrees);
        self.update_max_height_exceeded();
    }

    /// Add an in-flow inline box to the line at its resolved `placement` (ignored for
    /// `vertical-align: top | bottom`, which only constrains the line box).
    fn append_aligned_inline_box_to_line(
        &mut self,
        next_x: f32,
        inline_box: &InlineBox,
        placement: InlineBoxPlacement,
        quantize: bool,
    ) {
        self.item_idx += 1;
        self.line.items.end += 1;
        self.line.x = next_x;
        self.line.text_advance = super::text_advance::TextAdvance::from_width(next_x);
        if inline_box.vertical_align.is_line_relative() {
            self.line.box_metrics.add_line_relative_inline_box(
                inline_box.vertical_align,
                inline_box.height,
                quantize,
            );
        } else {
            self.line.box_metrics.add_inline_box(
                placement.aligned_subtree_root,
                placement.baseline_offset,
                placement.ascent,
                placement.descent,
                &mut self.subtrees,
            );
        }
        self.update_max_height_exceeded();
    }

    /// Store the current iteration state so that we can revert to it if we later want to take
    /// the line breaking opportunity at this point.
    fn mark_line_break_opportunity(&mut self) {
        self.prev_boundary = Some(PrevBoundaryState {
            item_idx: self.item_idx,
            run_idx: self.run_idx,
            cluster_idx: self.cluster_idx,
            state: self.line.clone(),
            contributed_len: self.contributed.len(),
            subtrees_len: self.subtrees.save(),
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
            contributed_len: self.contributed.len(),
            subtrees_len: self.subtrees.save(),
        });
    }

    /// Revert boundary state to prev state
    fn reset_to(&mut self, prev_state: PrevBoundaryState) {
        self.item_idx = prev_state.item_idx;
        self.run_idx = prev_state.run_idx;
        self.cluster_idx = prev_state.cluster_idx;
        self.line = prev_state.state;
        self.contributed.truncate(prev_state.contributed_len);
        self.subtrees.restore(prev_state.subtrees_len);
    }

    #[inline(always)]
    fn update_max_height_exceeded(&mut self) {
        self.line.max_height_exceeded = self.line_max_height != f32::MAX
            && self.line.box_metrics.line_height() > self.line_max_height;
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
        self.update_max_height_exceeded();
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
        lines.aligned_subtree_offsets.clear();
        let mut this = Self {
            layout,
            lines,
            state: BreakerState::default(),
            prev_state: None,
            done: false,
        };
        this.reset_line();
        this
    }

    /// Reset the per-line running state in preparation for building a new line.
    ///
    /// The line starts out containing the root span box (the "strut", CSS 2 §10.8).
    fn reset_line(&mut self) {
        let state = &mut self.state;
        state.line.reset();
        state.contributed.clear();
        state.subtrees.clear();
        state.line.box_metrics.add_style(
            0,
            &self.layout.data.style_metrics,
            &mut state.contributed,
            &mut state.subtrees,
        );
        state.update_max_height_exceeded();
    }

    /// Add the layout's inline box `index` to the current line. Out-of-flow boxes contribute
    /// nothing to the line's metrics.
    fn append_layout_inline_box(&mut self, index: usize, next_x: f32) {
        let quantize = self.layout.data.quantize;
        let layout_box = &mut self.layout.data.inline_boxes[index];
        let inline_box = &layout_box.inline_box;
        if inline_box.kind == InlineBoxKind::InFlow {
            let placement = inline_box_placement(
                inline_box,
                layout_box.parent_style_index,
                &self.layout.data.style_metrics,
                quantize,
            );
            layout_box.baseline_offset = placement.baseline_offset;
            self.state
                .append_aligned_inline_box_to_line(next_x, inline_box, placement, quantize);
        } else {
            self.state
                .append_inline_box_to_line(next_x, f32::NEG_INFINITY, f32::NEG_INFINITY);
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

        // A line containing only empty inline boxes (and no text) is an invisible line box with
        // zero height; the strut only applies once the line has content. The trailing empty line
        // after a final newline has no items and keeps the strut so the cursor has a position.
        let invisible = !self.state.line.box_metrics.has_content
            && !self.lines.lines.last().unwrap().item_range.is_empty();
        let line_height = if invisible {
            0.
        } else {
            self.state.line.box_metrics.line_height()
        };
        let line_y_start = self.state.line_y;

        self.state.items = self.lines.line_items.len();
        self.state.lines = self.lines.lines.len();
        self.state.prev_boundary = None;
        self.state.emergency_boundary = None;

        // `finish_line` reads the line's accumulated vertical metrics from `self.state.line`, so
        // it must run before we reset the per-line running state. It may grow the line (e.g. the
        // trailing line after a final newline), so use the returned height.
        let line_height = self.finish_line(self.lines.lines.len() - 1, line_height, invisible);
        self.reset_line();

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
            advance: self.state.line.x,
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
        self.lines.truncate(self.state.lines);
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
    pub fn break_next(&mut self) -> Option<YieldData> {
        assert!(
            self.state.layout_max_advance == f32::INFINITY
                || self.state.line_max_advance - self.state.layout_max_advance < 1.0
        );

        // Maintain iterator state
        if self.done {
            return None;
        }
        match &mut self.prev_state {
            Some(prev_state) => prev_state.clone_from(&self.state),
            None => self.prev_state = Some(self.state.clone()),
        }

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

        // Iterate over remaining runs in the Layout
        let item_count = self.layout.data.items.len();
        while self.state.item_idx < item_count {
            let item = &self.layout.data.items[self.state.item_idx];

            match item.kind {
                LayoutItemKind::InlineBox => {
                    let inline_box = &self.layout.data.inline_boxes[item.index].inline_box;

                    // In-flow boxes are aligned relative to their containing style's span box
                    // (see `append_aligned_inline_box_to_line`). Out-of-flow boxes are not in-flow
                    // content: unlike an atomic inline they neither contribute to the line's
                    // extents nor introduce a line break opportunity, so they are appended to the
                    // line and otherwise skipped.
                    let (width_contribution, height_contribution) = match inline_box.kind {
                        InlineBoxKind::InFlow => (inline_box.width, inline_box.height),
                        InlineBoxKind::OutOfFlow => {
                            self.append_layout_inline_box(item.index, self.state.line.x);
                            continue;
                        }
                        // If the box is a `CustomOutOfFlow` box then we yield control flow back to the caller.
                        // It is then the caller's responsibility to handle placement of the box.
                        InlineBoxKind::CustomOutOfFlow => {
                            return Some(YieldData::InlineBoxBreak(BoxBreakData {
                                inline_box_id: inline_box.id,
                                inline_box_index: item.index,
                                advance: self.state.line.x,
                            }));
                        }
                    };

                    // Compute the x position of the content being currently processed
                    let next_x = self.state.line.x + width_contribution;

                    let box_will_be_appended = next_x <= max_advance || self.state.line.x == 0.0;
                    if height_contribution > self.state.line_max_height && box_will_be_appended {
                        return self.max_height_break_data(height_contribution);
                    }

                    // If the box fits on the current line (or we are at the start of the current line)
                    // then simply move on to the next item
                    if next_x <= max_advance || self.state.line.text_wrap_mode != TextWrapMode::Wrap
                    {
                        self.append_layout_inline_box(item.index, next_x);

                        // There is a soft wrap opportunity after an inline box, unless wrapping
                        // is disabled
                        if self.state.line.text_wrap_mode == TextWrapMode::Wrap {
                            self.state.mark_line_break_opportunity();
                        }
                    } else {
                        // If we're at the start of the line, this box will never fit, so consume it and accept the overflow.
                        let reason = if self.state.line.x == 0.0 {
                            self.append_layout_inline_box(item.index, next_x);
                            BreakReason::Emergency
                        } else {
                            BreakReason::Regular
                        };
                        return self.start_new_line(reason, max_advance, line_indent);
                    }
                }
                LayoutItemKind::TextRun => {
                    let run_idx = item.index;
                    let run = Run::new(self.layout, 0, 0, run_idx, None);
                    let slice = run.full_slice();

                    // Additional spacing to apply between atoms.
                    let spacing = EffectiveSpacing::new(run.data.spacing, Justification::NONE);

                    let line_height = run.data.line_height;

                    // Iterate over the remaining atoms in the Run
                    for atom in slice.atoms_from(self.state.cluster_idx) {
                        // Retrieve metadata about the atom
                        let first_character = &atom.characters()[0];
                        let whitespace = first_character.whitespace;
                        let is_newline = whitespace == Whitespace::Newline;
                        // Whether this atom is a justification opportunity.
                        let is_separator = is_word_separator(whitespace);
                        let is_soft_wrap_opportunity =
                            first_character.flags.is_soft_wrap_opportunity();
                        let max_height_exceeded = self.state.line.max_height_exceeded;
                        let style_index = first_character.style_index;
                        let style = &self.layout.data.styles[style_index as usize];

                        // Lag text_wrap_mode style by one atom
                        let text_wrap_mode = self.state.line.text_wrap_mode;
                        self.state.line.text_wrap_mode = style.text_wrap_mode;

                        // Take mandatory line breaks before checking for line wrapping.
                        if is_newline {
                            if max_height_exceeded {
                                return self.max_height_break_data(line_height);
                            }

                            self.state.append_atom_to_line(
                                &atom,
                                self.state.line.x,
                                style_index,
                                &self.layout.data,
                                is_separator,
                            );

                            return self.start_new_line(
                                BreakReason::Explicit,
                                max_advance,
                                line_indent,
                            );
                        } else if is_soft_wrap_opportunity && text_wrap_mode == TextWrapMode::Wrap {
                            // We don't record boundaries when the advance is 0. As we do not want overflowing content to cause extra consecutive
                            // line breaks. We should accept the overflowing fragment in that scenario.
                            if self.state.line.x != 0.0 {
                                self.state.mark_line_break_opportunity();
                                // break_opportunity = true;
                            }
                        } else if
                        // This text can contribute "emergency" line breaks.
                        style.overflow_wrap != OverflowWrap::Normal
                        && text_wrap_mode == TextWrapMode::Wrap
                        // If we're at the start of the line, this particular atom will never fit, so it's not a valid emergency break opportunity.
                        && self.state.line.x != 0.0
                        {
                            self.state.mark_emergency_break_opportunity();
                        }

                        // Breaking an atom requires reshaping, which we don't do here, so it is
                        // consumed as a whole (this includes all clusters of a ligature).
                        let advance = spacing.atom_advance(&atom);

                        // Compute the x position of the content being currently processed
                        let allocation = self
                            .layout
                            .data
                            .text_item_quantization
                            .as_ref()
                            .map(|policy| policy.cluster(atom.shaped_clusters_range().start));
                        let next_advance =
                            self.state.line.text_advance.with_text(advance, allocation);
                        let next_x = next_advance.value;

                        // If the content fits (the x position does NOT exceed max_advance)
                        //
                        // We simply append the atom to the current line
                        if next_x <= max_advance {
                            if max_height_exceeded {
                                return self.max_height_break_data(line_height);
                            }
                            self.state.append_atom_to_line(
                                &atom,
                                next_x,
                                style_index,
                                &self.layout.data,
                                is_separator,
                            );
                        }
                        // Else we attempt to line break:
                        //
                        // This will only succeed if there is an available line-break opportunity that has been marked earlier
                        // in the line. If there is no such line-breaking opportunity (such as if wrapping is disabled), then
                        // we fall back to appending the content to the line anyway.
                        else {
                            // Case: the atom fits once its hanging whitespace is excluded.
                            //
                            // We append it to the line, with the whitespace hanging past the end.
                            let (hanging, all_hang) = atom_hanging_advance(
                                slice,
                                &atom,
                                &self.layout.data.styles,
                                spacing,
                                item.bidi_level.is_rtl(),
                            );
                            if all_hang
                                || next_advance.without_trailing_space(hanging) <= max_advance
                            {
                                if max_height_exceeded {
                                    return self.max_height_break_data(line_height);
                                }
                                self.state.append_atom_to_line(
                                    &atom,
                                    next_x,
                                    style_index,
                                    &self.layout.data,
                                    is_separator,
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
                                self.state.append_atom_to_line(
                                    &atom,
                                    next_x,
                                    style_index,
                                    &self.layout.data,
                                    is_separator,
                                );
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
    /// Each character of text (including whitespace and newlines) counts as 1.
    /// Each inline box also counts as 1 character.
    ///
    /// Unlike `break_next`, this method does not respect normal line break opportunities and
    /// will break when the character limit is reached. It does not break on newlines, for example.
    ///
    /// Breaks do fall on atom boundaries, however: when the limit is reached inside an atom (e.g. a
    /// ligature or a multi-character grapheme), the whole atom is placed on the line before
    /// breaking.
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
                    let inline_box = &self.layout.data.inline_boxes[item.index].inline_box;

                    if inline_box.kind != InlineBoxKind::InFlow {
                        self.append_layout_inline_box(item.index, self.state.line.x);
                        continue;
                    }

                    // Check if adding this box would exceed the limit
                    if char_count >= max_chars && max_chars != 0 {
                        // Break before this box
                        self.start_new_line(BreakReason::Regular, f32::MAX, line_indent);
                        return Some(());
                    }

                    // Compute the x position for the line width tracking
                    let next_x = self.state.line.x + inline_box.width;
                    self.append_layout_inline_box(item.index, next_x);
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
                    let shaped_run = &self.layout.data.shaped_text.runs()[run_idx];
                    let run = Run::new(self.layout, 0, 0, run_idx, None);
                    let slice = run.full_slice();
                    let spacing = run.line_spacing();
                    let cluster_end = shaped_run.shaped_clusters_range.end;

                    for atom in slice.atoms_from(self.state.cluster_idx) {
                        // Check if we should break before this atom
                        if char_count >= max_chars && max_chars != 0 {
                            self.start_new_line(BreakReason::Regular, f32::MAX, line_indent);
                            return Some(());
                        }

                        let first_character = &atom.characters()[0];
                        let whitespace = first_character.whitespace;
                        let is_newline = whitespace == Whitespace::Newline;
                        let is_separator = is_word_separator(whitespace);
                        let advance = spacing.atom_advance(&atom);

                        // Compute the x position.
                        // Newlines don't contribute to line width (matching break_next behavior).
                        let next_x = if is_newline {
                            self.state.line.x
                        } else {
                            self.state
                                .line
                                .text_advance
                                .with_text(
                                    advance,
                                    self.layout.data.text_item_quantization.as_ref().map(
                                        |policy| policy.cluster(atom.shaped_clusters_range().start),
                                    ),
                                )
                                .value
                        };
                        self.state.append_atom_to_line(
                            &atom,
                            next_x,
                            first_character.style_index,
                            &self.layout.data,
                            is_separator,
                        );
                        char_count += atom.char_range().len() as u32;

                        // Check if we've reached the limit after adding this atom
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
        self.state.layout_max_advance = max_advance;
        self.state.line_max_advance = max_advance;
        while let Some(yield_data) = self.break_next() {
            // When `break_next` encounters a `CustomOutOfFlow`, it yields a `YieldData::InlineBoxBreak`
            // without advancing the item iteration past the box. This allows embedders to implement custom
            // placement logic to place the box. However, it means that we must also handle placing the box
            // and advancing the iteration here to avoid an infinite loop.
            //
            // So we place the box as a zero-sized out-of-flow box to guarantee progress.
            if let YieldData::InlineBoxBreak(_) = yield_data {
                self.state
                    .append_inline_box_to_line(self.state.line.x, 0.0, 0.0);
            }
        }
        self.finish();
    }

    /// Consumes the line breaker and finalizes all line computations.
    pub fn finish(mut self) {
        if self.layout.data.text_len == 0
            && let Some(line) = self.lines.line_items.first_mut()
        {
            line.text_range = 0..0;
            line.shaped_cluster_range = 0..0;
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

    fn finish_line(&mut self, line_idx: usize, line_height: f32, invisible: bool) -> f32 {
        let line = &mut self.lines.lines[line_idx];

        // Reset metrics for line
        line.metrics.offset = 0.;

        line.metrics.line_height = line_height;

        if line.item_range.is_empty() {
            line.text_range = self.layout.data.text_len..self.layout.data.text_len;
        }

        // Whether metrics should be quantized to pixel boundaries
        let quantize = self.layout.data.quantize;

        if line.item_range.is_empty() && line_idx > 0 {
            // If we have no items on this line, it must be the last (empty)
            // line in a layout following a newline. Commit an empty run so
            // that consumers, such as accessibility integrations, have
            // something with which to identify the visual cursor position.
            if let Some((index, run)) = self
                .layout
                .data
                .shaped_text
                .runs()
                .iter()
                .enumerate()
                .rfind(|(_, run)| !run.range.byte_range.is_empty())
            {
                let run_index = self.lines.line_items.len();
                let cluster = run.shaped_clusters_range.end;
                let text = run.range.byte_range.end;
                // The empty run carries the style of the newline that produced this line, so the
                // line is sized by that style's span box (and its ancestors), not just the strut.
                let style_index = self.layout.data.shaped_text.shaped_clusters()
                    [cluster as usize - 1]
                    .style_index;
                self.state.line.box_metrics.add_text(
                    index,
                    index,
                    style_index,
                    &[],
                    &self.layout.data,
                    &mut self.state.contributed,
                    &mut self.state.subtrees,
                );
                line.metrics.line_height = self.state.line.box_metrics.line_height();
                self.lines.line_items.push(LineItemData {
                    kind: LayoutItemKind::TextRun,
                    index,
                    bidi_level: BidiLevel::new(0),
                    shaped_cluster_range: cluster..cluster,
                    text_range: text..text,
                    allocated_advance: None,
                });
                line.item_range = run_index..run_index + 1;
            }
        }

        // Position the independent aligned subtrees against each other (CSS 2.2 §10.8.1).
        //
        // The root aligned subtree determines the line's baseline. Those rooted at a `top`/`bottom`
        // span sit flush with the line box's top/bottom edge. If taller than the root aligned
        // subtree then top-aligned subtrees grow the line box downwards and bottom-aligned subtrees
        // grow it upwards.
        let box_metrics = &self.state.line.box_metrics;
        let (mut line_box_extents, mut content_box_extents) = if invisible {
            (Extents::default().or_zero(), Extents::default().or_zero())
        } else {
            (
                box_metrics.root.line_box.or_zero(),
                box_metrics.root.content_box.or_zero(),
            )
        };

        let subtrees = self.state.subtrees.current();
        let mut top_height = box_metrics.line_relative_top_height;
        let mut bottom_height = box_metrics.line_relative_bottom_height;
        for subtree in &subtrees {
            let height = subtree.line_box.height();
            match self.layout.data.styles[usize::from(subtree.root)]
                .vertical_align
                .shift
            {
                BaselineShift::Bottom => bottom_height = bottom_height.max(height),
                _ => top_height = top_height.max(height),
            }
        }
        let root_height = line_box_extents.over + line_box_extents.under;
        if top_height > root_height {
            line_box_extents.under += top_height - root_height;
        }
        let root_height = line_box_extents.over + line_box_extents.under;
        if bottom_height > root_height {
            line_box_extents.over += bottom_height - root_height;
        }

        let offsets = &mut self.lines.aligned_subtree_offsets;
        line.aligned_subtree_offsets.start = offsets.len() as u32;
        for subtree in &subtrees {
            let extents = subtree.line_box.or_zero();
            let offset = match self.layout.data.styles[usize::from(subtree.root)]
                .vertical_align
                .shift
            {
                BaselineShift::Bottom => extents.under - line_box_extents.under,
                _ => line_box_extents.over - extents.over,
            };
            let offset = if quantize { offset.round() } else { offset };
            offsets.push(AlignedSubtreeOffset {
                root: subtree.root,
                baseline_offset: offset,
            });
            let content = subtree.content_box.or_zero();
            content_box_extents.add(offset, content.over, content.under);
        }
        line.aligned_subtree_offsets.end = offsets.len() as u32;
        if box_metrics.line_relative_top_height > 0. {
            content_box_extents.add(
                0.,
                line_box_extents.over,
                box_metrics.line_relative_top_height - line_box_extents.over,
            );
        }
        if box_metrics.line_relative_bottom_height > 0. {
            content_box_extents.add(
                0.,
                box_metrics.line_relative_bottom_height - line_box_extents.under,
                line_box_extents.under,
            );
        }

        let top = if quantize {
            self.state.line_y.round() as f32
        } else {
            self.state.line_y as f32
        };
        line.metrics.baseline = top + line_box_extents.over;
        line.metrics.block_min_coord = top;
        line.metrics.block_max_coord = if quantize {
            // TODO: perhaps this should be something like the following, to ensure quantized line
            // boxes tile exactly without gaps or overlap. However, that would cause some
            // line height asserts to fail in the tests.
            // ```
            // (self.state.line_y as f32 + self.state.line.box_metrics.line_height()).round()
            // ```
            (line.metrics.baseline + line_box_extents.under).round()
        } else {
            line.metrics.baseline + line_box_extents.under
        };
        line.metrics.content_block_min_coord = line.metrics.baseline - content_box_extents.over;
        line.metrics.content_block_max_coord = line.metrics.baseline + content_box_extents.under;

        line.metrics.inline_min_coord = self.state.line_x;
        line.metrics.inline_max_coord = self.state.line_x + self.state.line_max_advance;
        line.metrics.line_height
    }
}

impl<B: Brush> Drop for BreakLines<'_, B> {
    fn drop(&mut self) {
        // Compute the overall width and height of the entire layout
        // The "width" excludes hanging whitespace. The "full_width" includes it.
        let mut layout_width = 0_f32;
        let mut layout_full_width = 0_f32;
        let mut height = 0_f64; // f32 causes test failures due to accumulated error
        for line in &self.lines.lines {
            let indent_extra = line.indent.max(0.0);
            let line_max = line.metrics.inline_min_coord + line.metrics.advance + indent_extra;
            layout_full_width = layout_full_width.max(line_max);
            layout_width = layout_width.max(line_max - line.metrics.hanging_advance);
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
        if let Some(last_line) = self.lines.lines.last()
            && last_line.item_range.is_empty()
        {
            height -= last_line.metrics.line_height as f64;
        }

        // Save the computed widths/height to the layout
        self.layout.data.width = layout_width;
        self.layout.data.full_width = layout_full_width;
        self.layout.data.height = height as f32;
        self.layout.data.layout_max_advance = self.state.layout_max_advance;

        // Save the computed lines to the layout
        self.lines.swap(&mut self.layout.data);
    }
}

#[expect(clippy::cast_possible_truncation, reason = "deferred")]
fn commit_line<B: Brush>(
    layout: &Layout<B>,
    lines: &mut LineLayout,
    state: &mut LineState,
    max_advance: f32,
    break_reason: BreakReason,
    line_indent: f32,
) {
    let shaped_text = &layout.data.shaped_text;
    let shaped_clusters = shaped_text.shaped_clusters();

    // Ensure that the cluster and item endpoints are within range
    state.clusters.end = state.clusters.end.min(shaped_clusters.len() as u32);
    state.items.end = state.items.end.min(layout.data.items.len());

    let start_item_idx = lines.line_items.len();

    let items_to_commit = &layout.data.items[state.items.clone()];

    // Compute first and last run index
    let is_text_run = |item: &LayoutItem| item.kind == LayoutItemKind::TextRun;
    let first_run_pos = items_to_commit.iter().position(is_text_run).unwrap_or(0);
    let last_run_pos = items_to_commit.iter().rposition(is_text_run).unwrap_or(0);

    // Iterate over the items to commit
    let mut last_item_kind = LayoutItemKind::TextRun;
    let mut committed_text_run = false;
    // The line's source text range, as the union of the ranges of the text runs committed to it.
    let mut text_start = usize::MAX;
    let mut text_end = 0;
    // Mark the line as needing bidi reordering if it contains any item with a non-zero bidi level.
    let mut needs_reorder = false;
    for (i, item) in items_to_commit.iter().enumerate() {
        match item.kind {
            LayoutItemKind::InlineBox => {
                needs_reorder |= item.bidi_level != BidiLevel::new(0);
                lines.line_items.push(LineItemData {
                    kind: LayoutItemKind::InlineBox,
                    index: item.index,
                    bidi_level: item.bidi_level,

                    // These properties are ignored for inline boxes. So we just put a dummy value.
                    shaped_cluster_range: 0..0,
                    text_range: 0..0,
                    allocated_advance: None,
                });

                last_item_kind = item.kind;
            }
            LayoutItemKind::TextRun => {
                let shaped_run = &shaped_text.runs()[item.index];

                // Compute cluster range
                // The first and last ranges have overrides to account for line-breaks within runs
                let mut cluster_range = shaped_run.shaped_clusters_range.clone();
                if i == first_run_pos {
                    cluster_range.start = state.clusters.start;
                }
                if i == last_run_pos {
                    cluster_range.end = state.clusters.end;
                }

                if cluster_range.start >= shaped_run.shaped_clusters_range.end {
                    continue;
                }

                last_item_kind = item.kind;
                committed_text_run = true;

                // Map the cluster range to the source-text range. Line boundaries are always
                // aligned to `Atom`s, i.e., line bounds are always grapheme bounds.
                let slice = shaped_text.run_slice(item.index as u32);
                let item_text_range = if cluster_range.is_empty() {
                    let char_pos = shaped_clusters[cluster_range.start as usize]
                        .chars_range()
                        .start;
                    let text_pos = slice.text_byte_at(char_pos);
                    text_pos..text_pos
                } else {
                    let char_range = shaped_clusters[cluster_range.start as usize]
                        .chars_range()
                        .start
                        ..shaped_clusters[cluster_range.end as usize - 1]
                            .chars_range()
                            .end;
                    slice.text_byte_range(char_range)
                };

                // Compute the text range for the line
                // Q: Can we not simplify this computation by assuming that items are in order?
                text_start = text_start.min(item_text_range.start);
                text_end = text_end.max(item_text_range.end);
                needs_reorder |= shaped_run.bidi_level != BidiLevel::new(0);

                lines.line_items.push(LineItemData {
                    kind: LayoutItemKind::TextRun,
                    index: item.index,
                    bidi_level: shaped_run.bidi_level,
                    shaped_cluster_range: cluster_range,
                    text_range: item_text_range,
                    allocated_advance: None,
                });
            }
        }
    }
    // let end_run_idx = lines.line_items.last().map(|item| item.index).unwrap_or(0);
    if let Some(policy) = &layout.data.text_item_quantization {
        let split = policy.split_line_items(&layout.data, &lines.line_items[start_item_idx..]);
        lines.line_items.truncate(start_item_idx);
        lines.line_items.extend(split);
    }
    let end_item_idx = lines.line_items.len();

    // Trailing whitespace under `WhiteSpaceCollapse::Preserve` directly preceding a forced break
    // hangs conditionally (CSS Text 4 § 4.3.2): only the part that doesn't fit hangs. Whitespace
    // that fits, stays inside the line box, and counts towards alignment and the layout width. The
    // end of the layout is considered to be a forced line break as well (CSS Text 4 § 5). Other
    // trailing whitespace hangs unconditionally.
    //
    // When a line wrapped (i.e., not because of a forced line break), whitespace always hangs
    // unconditionally. That's equivalent to considering that whitespace to be overflowing fully.
    // Hence an `overflow` of `f32::INFINITY` can be used to model that case.
    let overflow = match break_reason {
        BreakReason::Regular | BreakReason::Emergency => f32::INFINITY,
        BreakReason::Explicit | BreakReason::None => state.x - max_advance,
    };
    let (hanging_advance, justification_end_cluster, hanging_opportunities) = hanging_whitespace(
        layout,
        &lines.line_items[start_item_idx..end_item_idx],
        overflow,
    );
    let hanging_advance = if layout.data.text_item_quantization.is_some() {
        state.x - state.text_advance.without_trailing_space(hanging_advance)
    } else {
        hanging_advance
    };
    // The word separators counted as the line was built include the line's hanging whitespace, and
    // hanging whitespace is not stretched by justification.
    let num_justification_opportunities = state.num_word_separators - hanging_opportunities;

    // Reorder the line items according to their resolved bidi levels.
    if needs_reorder && end_item_idx - start_item_idx > 1 {
        reorder_line_items(&mut lines.line_items[start_item_idx..end_item_idx]);
    }

    lines.lines.push(LineData {
        item_range: start_item_idx..end_item_idx,
        text_range: text_start..text_end,
        max_advance,
        break_reason,
        num_justification_opportunities,
        justification: Justification {
            justification_end_cluster,
            ..Justification::NONE
        },
        indent: line_indent,
        metrics: LineMetrics {
            advance: state.x,
            hanging_advance,
            ..Default::default()
        },
        aligned_subtree_offsets: 0..0,
    });

    // Reset state for the new line
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
}

/// Returns the advance of the whitespace hanging past the end of the line made up of `line_items`,
/// the index one past its logically last shaped cluster that's eligible for justification (see
/// [`Justification::justification_end_cluster`]), and the number of justification opportunities
/// that are no longer eligible, as they are at or beyond that cluster.
///
/// `line_items` must be in logical order.
///
/// `overflow` is the advance by which the line's content overflows the available width (including
/// all of its trailing whitespace). Following CSS Text 4 § 9.2, conditionally hanging whitespace
/// only hangs as far as it overflows the available width. Pass [`f32::INFINITY`] to hang
/// conditional whitespace in full.
///
/// See the module-level docs of [`crate::layout::whitespace`] for an explanation of how
/// conditionally and unconditionally hanging whitespace are determined; in short, we can treat the
/// trailing whitespace as being a suffix of conditionally hanging whitespace preceded by
/// unconditionally hanging whitespace (and either can be empty). If the conditionally hanging
/// suffix hangs in full, the unconditionally hanging whitespace hangs in full.
//
// Note: This runs once per line, but it called in the line breaker's per-atom loop. The compiler
// sometimes decides to inline it, which (as of writing) makes the line breaker 5-10% slower.
#[inline(never)]
fn hanging_whitespace<B: Brush>(
    layout: &Layout<B>,
    line_items: &[LineItemData],
    overflow: f32,
) -> (f32, u32, u32) {
    let mut hanging_whitespace_advance = 0.;
    // Atoms with shaped clusters before this index may be stretched by justification.
    let mut justification_end_cluster = u32::MAX;
    let mut hanging_opportunities = 0;
    // Following CSS Text 4 § 4.3.2, a sequence of whitespace with `WhiteSpaceCollapse::Preserve`
    // directly preceding a forced line break hangs conditionally. All other whitespace hangs
    // unconditionally. Note that, in particular, a sequence of `WhiteSpaceCollapse::Preserve`
    // preceding a sequence of unconditionally hanging whitespace, also hangs unconditionally.
    //
    // Conditionally hanging whitespace hangs only as far as it overflows.
    //
    // Following CSS Text 4 § 9.2, unconditionally hanging whitespace before conditionally hanging
    // whitespace, hangs only if the conditionally hanging whitespace hangs in full.
    let mut in_conditional_suffix = true;
    let mut conditional_advance: Option<f32> = None;

    'items: for line_item in line_items.iter().rev() {
        match line_item.kind {
            LayoutItemKind::InlineBox => {
                let item = &layout.data.inline_boxes[line_item.index].inline_box;

                // Inline boxes don't hang.
                if item.kind == InlineBoxKind::InFlow {
                    break;
                }
            }
            LayoutItemKind::TextRun => {
                // Trailing whitespace can only hang if it ends up at the line's end edge after bidi
                // reordering. We don't currently apply UAX #9 L1 (resetting trailing whitespace to
                // paragraph level), so only logically-last items that match the paragraph level are
                // guaranteed to be at that edge.
                if line_item.bidi_level != layout.data.base_level {
                    break;
                }

                let effective_spacing = EffectiveSpacing::new(
                    layout.data.runs[line_item.index].spacing,
                    Justification::NONE,
                );
                let slice = layout
                    .data
                    .shaped_text
                    .run_slice(line_item.index as u32)
                    .narrow(line_item.shaped_cluster_range.clone());

                for atom in slice.atoms_end().rev() {
                    let (hanging, all_hang) = atom_hanging_advance(
                        slice,
                        &atom,
                        &layout.data.styles,
                        effective_spacing,
                        line_item.is_rtl(),
                    );
                    let whitespace = atom.characters()[0].whitespace;
                    if in_conditional_suffix && whitespace != Whitespace::Newline {
                        // The atom hangs from its logical end, so use the last cluster's style to
                        // decide whether it hangs conditionally.
                        let last_cluster = atom.shaped_clusters().last().unwrap();
                        if layout.data.styles[last_cluster.style_index as usize]
                            .white_space_collapse
                            == WhiteSpaceCollapse::Preserve
                        {
                            *conditional_advance.get_or_insert(0.) += hanging;
                        } else {
                            in_conditional_suffix = false;
                            // A conditionally hanging atom that fits (including one with a
                            // non-positive advance) doesn't hang, and so whitespace before it isn't
                            // at the line's end.
                            if conditional_advance
                                .is_some_and(|advance| advance <= 0. || advance > overflow)
                            {
                                break 'items;
                            }
                        }
                    }
                    hanging_whitespace_advance += hanging;
                    // Justification can't stretch within an atom, so it stops at the start of the
                    // last atom that hangs in its entirety or only partially.
                    justification_end_cluster = atom.shaped_clusters_range().start;
                    if is_word_separator(whitespace) {
                        hanging_opportunities += 1;
                    }
                    if !all_hang {
                        break 'items;
                    }
                }
            }
        }
    }

    if let Some(advance) = conditional_advance {
        hanging_whitespace_advance -= advance - advance.min(overflow).max(0.);
    }

    (
        hanging_whitespace_advance,
        justification_end_cluster,
        hanging_opportunities,
    )
}

/// Reorder items within line according to the bidi levels of the items
fn reorder_line_items(runs: &mut [LineItemData]) {
    let run_count = runs.len();

    // Find the max level and the min *odd* level
    let mut max_level = 0;
    let mut lowest_odd_level = 255;
    for run in runs.iter() {
        let level = run.bidi_level;
        let is_odd = level.to_u8() & 1 != 0;

        // Update max level
        if level.to_u8() > max_level {
            max_level = level.to_u8();
        }

        // Update min odd level
        if is_odd && level.to_u8() < lowest_odd_level {
            lowest_odd_level = level.to_u8();
        }
    }

    // Iterate over bidi levels
    for level in (lowest_odd_level..=max_level).rev() {
        // Iterate over text runs
        let mut i = 0;
        while i < run_count {
            if runs[i].bidi_level.to_u8() >= level {
                let mut end = i + 1;
                while end < run_count && runs[end].bidi_level.to_u8() >= level {
                    end += 1;
                }

                runs[i..end].reverse();
                i = end;
            }
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BoxMetrics, BreakerState};

    #[test]
    fn subtree_extents_restored_at_break_opportunities() {
        let metrics = BoxMetrics {
            ascent: 6.,
            descent: 2.,
            over: 6.,
            under: 2.,
        };
        for emergency in [false, true] {
            let mut state = BreakerState::default();
            let add_box = |state: &mut BreakerState, baseline_offset| {
                state
                    .line
                    .box_metrics
                    .add_box(1, baseline_offset, metrics, &mut state.subtrees);
            };
            let subtree_height =
                |state: &BreakerState| state.subtrees.current()[0].line_box.height();

            add_box(&mut state, 0.);
            add_box(&mut state, 1.);
            state.mark_line_break_opportunity();
            add_box(&mut state, 4.);
            state.mark_emergency_break_opportunity();
            add_box(&mut state, -4.);
            assert_eq!(state.line.box_metrics.line_height(), 16.);
            assert_eq!(subtree_height(&state), 16.);
            assert_eq!(state.subtrees.current().len(), 1);

            let boundary = if emergency {
                state.emergency_boundary.take().unwrap()
            } else {
                state.prev_boundary.take().unwrap()
            };
            state.reset_to(boundary);
            let expected = if emergency { 12. } else { 9. };
            assert_eq!(state.line.box_metrics.line_height(), expected);
            assert_eq!(subtree_height(&state), expected);
            assert_eq!(state.subtrees.current().len(), 1);
        }
    }
}
