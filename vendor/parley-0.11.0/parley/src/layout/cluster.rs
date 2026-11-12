// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::layout::Style;
use crate::layout::data::BreakReason;
use crate::layout::layout::Layout;
use crate::layout::line::{Line, LineItem};
use crate::layout::run::Run;
use crate::style::Brush;

use core::ops::Range;
use parley_engine::shape::Character;
use parley_engine::{Atom, Glyph, Grapheme, shape::Whitespace};

/// Atomic unit of text.
///
/// This spans a grapheme intersected with a [`Run`], which can be multiple characters of source
/// text. See [UAX #29 § 3][uax-grapheme]. Grapheme edges are caret/selection/hit-testing edges.
///
/// Note a full grapheme as per UAX #29 § 3 can extend past a [`Run`]. In particular, it can extend
/// past a line boundary and a font boundary. This type does not currently model that.
///
/// [uax-grapheme]: https://www.unicode.org/reports/tr29/#Grapheme_Cluster_Boundaries
pub struct Cluster<'a, B: Brush> {
    pub(crate) run: Run<'a, B>,
    /// The atom containing this grapheme.
    pub(crate) atom: Atom<'a>,
    /// The grapheme.
    pub(crate) grapheme: Grapheme,
}

// `Cluster` is `Copy` and `Clone` regardless of `B`.
impl<B: Brush> Copy for Cluster<'_, B> {}
impl<B: Brush> Clone for Cluster<'_, B> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Defines the visual side of the cluster for hit testing.
///
/// See [`Cluster::from_point`].
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ClusterSide {
    /// Cluster was hit on the left half.
    Left,
    /// Cluster was hit on the right half.
    Right,
}

impl<'a, B: Brush> Cluster<'a, B> {
    /// Returns the cluster for the given layout and byte index in the source text.
    pub fn from_byte_index(layout: &'a Layout<B>, byte_index: usize) -> Option<Self> {
        let (_, line) = layout.line_for_byte_index(byte_index)?;
        line.runs()
            .find(|run| run.text_range().contains(&byte_index))?
            .cluster_at_text_byte(byte_index)
    }

    /// Returns the cluster and side which is at the specified position in the given layout. If no cluster is
    /// under the specified point then None will be returned.
    ///
    /// This is usually the expected behaviour when hit-testing clusters for "hover" or "click" functionality.
    pub fn from_point_exact(layout: &'a Layout<B>, x: f32, y: f32) -> Option<(Self, ClusterSide)> {
        Cluster::from_point_impl(layout, x, y, true)
    }

    /// Returns the cluster and side which is at the specified position in the given layout. If no cluster is
    /// under the specified point but the point is within the overall layout area then it will return the nearest.
    ///
    /// This is usually the expected behaviour when hit-testing clusers for text selection or caret positioning.
    pub fn from_point(layout: &'a Layout<B>, x: f32, y: f32) -> Option<(Self, ClusterSide)> {
        Cluster::from_point_impl(layout, x, y, false)
    }

    /// Returns the cluster and side for the given layout and point.
    fn from_point_impl(
        layout: &'a Layout<B>,
        x: f32,
        y: f32,
        exact: bool,
    ) -> Option<(Self, ClusterSide)> {
        let mut line_index = 0;
        if let Some((index, line)) = layout.line_for_offset(y) {
            line_index = index;
            let mut offset = line.metrics().offset + line.metrics().inline_min_coord;
            let last_run_index = line.len().saturating_sub(1);
            for item in line.items_nonpositioned() {
                match item {
                    LineItem::Run(run) => {
                        let is_last_run = run.index as usize == last_run_index;
                        let run_advance = run.advance();
                        if x > offset + run_advance && (exact || !is_last_run) {
                            offset += run_advance;
                            continue;
                        }
                        for cluster in run.visual_clusters() {
                            let is_last_cluster = is_last_run && cluster.is_line_right_end_of_run();
                            let cluster_advance = cluster.advance();
                            let edge = offset;
                            offset += cluster_advance;
                            if x > offset && (exact || !is_last_cluster) {
                                continue;
                            }
                            if x < edge && exact {
                                continue;
                            }
                            let side = if x <= edge + cluster_advance * 0.5 {
                                ClusterSide::Left
                            } else {
                                ClusterSide::Right
                            };
                            return Some((cluster, side));
                        }
                    }
                    LineItem::InlineBox(inline_box) => {
                        offset += inline_box.width;
                    }
                }
            }
        }
        if y <= 0.0 && !exact {
            // Fall back to the first cluster of the line's first run.
            let cluster = layout.get(line_index)?.item(0)?.run()?.clusters().next()?;
            Some((cluster, ClusterSide::Left))
        } else {
            None
        }
    }

    /// Returns the line that contains the cluster.
    pub fn line(&self) -> Line<'a, B> {
        self.run.layout.get(self.run.line_index as usize).unwrap()
    }

    /// Returns the run that contains the cluster.
    pub fn run(&self) -> Run<'a, B> {
        self.run
    }

    /// Returns the path to reach the cluster from a layout.
    pub fn path(&self) -> ClusterPath {
        ClusterPath::new(
            self.run.line_index,
            self.run.index,
            self.grapheme.char_range().start,
        )
    }

    /// The first (logical) character of this cluster.
    pub(crate) fn first_character(&self) -> &'a Character {
        &self
            .run
            .full_slice()
            .characters_in(self.grapheme.char_range())[0]
    }

    /// Returns the range of text that defines the cluster.
    pub fn text_range(&self) -> Range<usize> {
        let range = self
            .run
            .full_slice()
            .text_byte_range(self.grapheme.char_range());
        if self.run.layout.data.text_item_quantization.is_some() {
            let item = self.run.text_range();
            range.start.max(item.start)..range.end.min(item.end)
        } else {
            range
        }
    }

    /// Returns the style of the character this cluster represents.
    ///
    /// All of the cluster's glyphs share this style.
    ///
    /// See also [`Self::style_index`].
    pub fn style(&self) -> &'a Style<B> {
        &self.run.layout.styles()[usize::from(self.style_index())]
    }

    /// Returns the style index of the character this cluster represents.
    ///
    /// All of the cluster's glyphs share this style.
    ///
    /// See also [`Self::style`].
    pub fn style_index(&self) -> u16 {
        self.first_character().style_index
    }

    /// Returns the advance of the cluster.
    ///
    /// If a shaped cluster crosses this grapheme cluster's boundaries (see
    /// [`Self::is_ligature_continuation`]), the shaped cluster's advance is split evenly over the
    /// clusters it overlaps.
    pub fn advance(&self) -> f32 {
        let spacing = self.run.line_spacing();
        if let Some(policy) = &self.run.layout.data.text_item_quantization {
            let slice = self.run.full_slice();
            let range = slice.text_byte_range(self.atom.char_range());
            let slot = policy.item_ends.partition_point(|end| *end <= range.start);
            let owner_end = policy.item_ends[slot];
            if owner_end < range.end {
                let owners = self
                    .atom
                    .graphemes_start()
                    .filter(|grapheme| {
                        slice.text_byte_range(grapheme.char_range()).start < owner_end
                    })
                    .count();
                let gap = spacing.atom_advance(&self.atom) - self.atom.advance();
                return (self.atom.advance() + gap) / owners as f32;
            }
        }
        spacing.grapheme_advance(&self.atom, self.grapheme, self.is_rtl())
    }

    /// Returns `true` if this is a right-to-left cluster.
    pub fn is_rtl(&self) -> bool {
        self.run.is_rtl()
    }

    /// Returns `true` if the cluster is the beginning of a ligature.
    pub fn is_ligature_start(&self) -> bool {
        self.grapheme.is_atom_start() && !self.grapheme.is_atom_end()
    }

    /// Returns `true` if the cluster is a ligature continuation.
    pub fn is_ligature_continuation(&self) -> bool {
        !self.grapheme.is_atom_start()
    }

    /// Returns `true` if the cluster is a word boundary.
    pub fn is_word_boundary(&self) -> bool {
        self.first_character().flags.is_word_boundary()
    }

    /// Returns `true` if the cluster is a soft line break.
    pub fn is_soft_line_break(&self) -> bool {
        self.is_end_of_line()
            && matches!(
                self.line().data.break_reason,
                BreakReason::Regular | BreakReason::Emergency
            )
    }

    /// Returns `true` if the cluster is a hard line break.
    pub fn is_hard_line_break(&self) -> bool {
        self.first_character().whitespace == Whitespace::Newline
    }

    /// Returns `true` if the cluster is a space or no-break space.
    pub fn is_space_or_nbsp(&self) -> bool {
        self.first_character().whitespace.is_space_or_nbsp()
    }

    /// Returns `true` if the cluster is an emoji sequence.
    pub fn is_emoji(&self) -> bool {
        self.first_character().flags.is_emoji()
    }

    /// Returns an iterator over the glyphs in the cluster.
    ///
    /// For our purposes, the glyphs of a shaped cluster belong to its first (logical) grapheme
    /// cluster: for a ligature, the ligature start yields all of the ligature's glyphs and the
    /// continuations yield none.
    pub fn glyphs(&self) -> impl Iterator<Item = Glyph> + Clone + use<'a, B> {
        self.grapheme
            .is_atom_start()
            .then(|| self.run.glyphs_in(self.atom.shaped_clusters_range()))
            .into_iter()
            .flatten()
    }

    /// Whether this cluster is at the line-left end of the run.
    fn is_line_left_end_of_run(&self) -> bool {
        let chars = self.run.line_slice().char_range();
        if self.is_rtl() {
            self.grapheme.char_range().end == chars.end
        } else {
            self.grapheme.char_range().start == chars.start
        }
    }

    /// Whether this cluster is at the line-right end of the run.
    fn is_line_right_end_of_run(&self) -> bool {
        let chars = self.run.line_slice().char_range();
        if self.is_rtl() {
            self.grapheme.char_range().start == chars.start
        } else {
            self.grapheme.char_range().end == chars.end
        }
    }

    /// Returns `true` if this cluster is at the beginning of a line.
    pub fn is_start_of_line(&self) -> bool {
        self.run.index == 0 && self.is_line_left_end_of_run()
    }

    /// Returns `true` if this cluster is at the end of a line.
    pub fn is_end_of_line(&self) -> bool {
        self.line().len().saturating_sub(1) == self.run.index as usize
            && self.is_line_right_end_of_run()
    }

    /// If the cluster as at the end of the line, returns the reason
    /// for the line break.
    pub fn is_line_break(&self) -> Option<BreakReason> {
        if self.is_end_of_line() {
            Some(self.line().data.break_reason)
        } else {
            None
        }
    }

    /// The cluster logically following this one within the same run, if any.
    fn next_logical_in_run(&self) -> Option<Self> {
        self.run
            .cluster_containing_char(self.grapheme.char_range().end)
    }

    /// The cluster logically preceding this one within the same run, if any.
    fn previous_logical_in_run(&self) -> Option<Self> {
        self.run
            .cluster_containing_char(self.grapheme.char_range().start.checked_sub(1)?)
    }

    /// Returns the cluster that follows this one in logical order.
    pub fn next_logical(&self) -> Option<Self> {
        if let Some(next) = self.next_logical_in_run() {
            // Fast path: next cluster is in the same run
            Some(next)
        } else {
            let index = self.text_range().end;
            if index >= self.run.layout.data.text_len {
                return None;
            }
            // We have to search for the cluster containing our end index
            Self::from_byte_index(self.run.layout, index)
        }
    }

    /// Returns the cluster that precedes this one in logical order.
    pub fn previous_logical(&self) -> Option<Self> {
        if let Some(previous) = self.previous_logical_in_run() {
            // Fast path: previous cluster is in the same run
            Some(previous)
        } else {
            Self::from_byte_index(self.run.layout, self.text_range().start.checked_sub(1)?)
        }
    }

    /// Returns the cluster that follows this one in visual order.
    pub fn next_visual(&self) -> Option<Self> {
        // Fast path: next visual cluster is in the same run
        let next = if self.is_rtl() {
            self.previous_logical_in_run()
        } else {
            self.next_logical_in_run()
        };
        if let Some(next) = next {
            Some(next)
        } else {
            // We just want to find the first line/run following this one that
            // contains any cluster.
            let layout = self.run.layout;
            let mut run_index = self.run.index as usize + 1;
            for line_index in self.run.line_index as usize..layout.len() {
                let line = layout.get(line_index)?;
                for run_index in run_index..line.len() {
                    if let Some(cluster) = line
                        .item(run_index)
                        .and_then(|item| item.run())
                        .and_then(|run| run.visual_clusters().next())
                    {
                        return Some(cluster);
                    }
                }
                // Restart at first run on next line
                run_index = 0;
            }
            None
        }
    }

    /// Returns the cluster that precedes this one in visual order.
    pub fn previous_visual(&self) -> Option<Self> {
        // Fast path: previous visual cluster is in the same run
        let previous = if self.is_rtl() {
            self.next_logical_in_run()
        } else {
            self.previous_logical_in_run()
        };
        if let Some(previous) = previous {
            Some(previous)
        } else {
            // To find the first line/run preceding this one that contains any cluster.
            let layout = self.run.layout;
            let mut run_index = Some(self.run.index as usize);
            for line_index in (0..=self.run.line_index as usize).rev() {
                let line = layout.get(line_index)?;
                let first_run = run_index.unwrap_or(line.len());
                for run_index in (0..first_run).rev() {
                    if let Some(cluster) = line
                        .item(run_index)
                        .and_then(|item| item.run())
                        .and_then(|run| run.visual_clusters_rev().next())
                    {
                        return Some(cluster);
                    }
                }
                run_index = None;
            }
            None
        }
    }

    /// Returns the next cluster that is marked as a word boundary.
    pub fn next_logical_word(&self) -> Option<Self> {
        let mut cluster = *self;
        while let Some(next) = cluster.next_logical() {
            if next.is_word_boundary() {
                return Some(next);
            }
            cluster = next;
        }
        None
    }

    /// Returns the next cluster that is marked as a word boundary.
    pub fn next_visual_word(&self) -> Option<Self> {
        let mut cluster = *self;
        while let Some(next) = cluster.next_visual() {
            if next.is_word_boundary() {
                return Some(next);
            }
            cluster = next;
        }
        None
    }

    /// Returns the previous cluster that is marked as a word boundary.
    pub fn previous_logical_word(&self) -> Option<Self> {
        let mut cluster = *self;
        while let Some(prev) = cluster.previous_logical() {
            if prev.is_word_boundary() {
                return Some(prev);
            }
            cluster = prev;
        }
        None
    }

    /// Returns the previous cluster that is marked as a word boundary.
    pub fn previous_visual_word(&self) -> Option<Self> {
        let mut cluster = *self;
        while let Some(prev) = cluster.previous_visual() {
            if prev.is_word_boundary() {
                return Some(prev);
            }
            cluster = prev;
        }
        None
    }

    /// Returns the visual inline-axis offset of this cluster from the line's minimum coordinate.
    ///
    /// The offset includes [`crate::LineMetrics::offset`], but not
    /// [`crate::LineMetrics::inline_min_coord`].
    ///
    /// The cost of this function is roughly linear in the number of clusters
    /// on the containing line. Use [`Line::positioned_visual_clusters`] to obtain
    /// all cluster offsets in a single pass.
    pub fn visual_offset(&self) -> Option<f32> {
        let line = self.line();
        let mut offset = line.metrics().offset;
        for run_index in 0..=self.run.index as usize {
            let item = line.item(run_index)?;
            match item {
                LineItem::Run(run) => {
                    if run_index != self.run.index as usize {
                        offset += run.advance();
                    } else {
                        for cluster in run.visual_clusters() {
                            if cluster.grapheme.char_range().start
                                == self.grapheme.char_range().start
                            {
                                break;
                            }
                            offset += cluster.advance();
                        }
                    }
                }
                LineItem::InlineBox(inline_box) => {
                    offset += inline_box.width;
                }
            }
        }
        Some(offset)
    }
}

/// Determines how a cursor attaches to a cluster.
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
pub enum Affinity {
    /// Cursor is attached to the character that is logically following in the
    /// text stream.
    #[default]
    Downstream = 0,
    /// Cursor is attached to the character that is logically preceding in the
    /// text stream.
    Upstream = 1,
}

impl Affinity {
    #[must_use]
    pub fn invert(&self) -> Self {
        match self {
            Self::Downstream => Self::Upstream,
            Self::Upstream => Self::Downstream,
        }
    }
}

/// Index-based path to a cluster.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Default, Debug)]
pub struct ClusterPath {
    pub(crate) line_index: u32,
    pub(crate) run_index: u32,
    /// The index of the cluster's first logical character.
    pub(crate) char_index: u32,
}

impl ClusterPath {
    /// Creates a path from the index of a line within a layout, the index of a
    /// run within that line, and the index of a grapheme cluster within that run.
    ///
    /// The path is not validated against any layout: use [`Self::cluster`] to
    /// resolve it.
    pub fn new(line_index: u32, run_index: u32, char_index: u32) -> Self {
        Self {
            line_index,
            run_index,
            char_index,
        }
    }

    /// Returns the index of the line containing this cluster.
    pub fn line_index(&self) -> usize {
        self.line_index as usize
    }

    /// Returns the index of the run (within the owning line) containing this
    /// cluster.
    pub fn run_index(&self) -> usize {
        self.run_index as usize
    }

    /// Returns the line for this path and the specified layout.
    pub fn line<'a, B: Brush>(&self, layout: &'a Layout<B>) -> Option<Line<'a, B>> {
        layout.get(self.line_index())
    }

    /// Returns the run for this path and the specified layout.
    pub fn run<'a, B: Brush>(&self, layout: &'a Layout<B>) -> Option<Run<'a, B>> {
        self.line(layout)?.item(self.run_index())?.run()
    }

    /// Returns the cluster for this path and the specified layout.
    pub fn cluster<'a, B: Brush>(&self, layout: &'a Layout<B>) -> Option<Cluster<'a, B>> {
        self.run(layout)?.cluster_containing_char(self.char_index)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Alignment, AlignmentOptions, Cluster, FontContext, Layout, LayoutContext,
        PositionedLayoutItem, StyleProperty,
    };

    type Brush = ();

    fn create_unaligned_layout() -> Layout<Brush> {
        let mut layout_ctx = LayoutContext::new();
        // TODO: Use a test font
        let mut font_ctx = FontContext::new();
        let text = "Parley exists";
        let mut builder = layout_ctx.ranged_builder(&mut font_ctx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(10.));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout
    }

    fn cluster_from_position_with_alignment(alignment: Alignment) {
        let mut layout = create_unaligned_layout();
        layout.align(alignment, AlignmentOptions::default());
        assert_eq!(
            layout.len(),
            1,
            "Text doesn't contain any newlines, and there's no max advance"
        );
        let line = layout.get(0).unwrap();

        let mut test_count = 0;
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                unreachable!("No inline boxes set up");
            };
            for glyph in run.positioned_glyphs() {
                test_count += 1;
                let cluster = Cluster::from_point(&layout, glyph.x + 0.1, glyph.y).unwrap();
                assert_eq!(cluster.0.glyphs().next().unwrap().id, glyph.id);
            }
        }
        assert!(test_count > 5);
    }

    #[test]
    fn cluster_from_position_start_alignment() {
        cluster_from_position_with_alignment(Alignment::Start);
    }
    #[test]
    fn cluster_from_position_center_alignment() {
        cluster_from_position_with_alignment(Alignment::Center);
    }
    #[test]
    fn cluster_from_position_end_alignment() {
        cluster_from_position_with_alignment(Alignment::End);
    }
    #[test]
    fn cluster_from_position_justified_alignment() {
        cluster_from_position_with_alignment(Alignment::Justify);
    }
}
