// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::analysis::cluster::Whitespace;
use crate::layout::Style;
use crate::layout::data::BreakReason;
use crate::layout::data::ClusterData;
use crate::layout::glyph::Glyph;
use crate::layout::layout::Layout;
use crate::layout::line::{Line, LineItem};
use crate::layout::run::Run;
use crate::style::Brush;
use core::ops::Range;

/// Atomic unit of text.
#[derive(Copy, Clone)]
pub struct Cluster<'a, B: Brush> {
    pub(crate) path: ClusterPath,
    pub(crate) run: Run<'a, B>,
    pub(crate) data: &'a ClusterData,
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
    /// Returns the cluster for the given layout and byte index.
    pub fn from_byte_index(layout: &'a Layout<B>, byte_index: usize) -> Option<Self> {
        let mut path = ClusterPath::default();
        if let Some((line_index, line)) = layout.line_for_byte_index(byte_index) {
            path.line_index = line_index as u32;
            for run in line.runs() {
                path.run_index = run.index;
                if !run.text_range().contains(&byte_index) {
                    continue;
                }
                for (cluster_index, cluster) in run.clusters().enumerate() {
                    path.logical_index = cluster_index as u32;
                    if cluster.text_range().contains(&byte_index) {
                        return path.cluster(layout);
                    }
                }
            }
        }
        None
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
        let mut path = ClusterPath::default();
        if let Some((line_index, line)) = layout.line_for_offset(y) {
            path.line_index = line_index as u32;
            let mut offset = line.metrics().offset + line.metrics().inline_min_coord;
            let last_run_index = line.len().saturating_sub(1);
            for item in line.items_nonpositioned() {
                match item {
                    LineItem::Run(run) => {
                        let is_last_run = run.index as usize == last_run_index;
                        let run_advance = run.advance();
                        path.run_index = run.index;
                        path.logical_index = 0;
                        if x > offset + run_advance && (exact || !is_last_run) {
                            offset += run_advance;
                            continue;
                        }
                        let last_cluster_index = run.cluster_range().len().saturating_sub(1);
                        for (visual_index, cluster) in run.visual_clusters().enumerate() {
                            let is_last_cluster = is_last_run && visual_index == last_cluster_index;
                            path.logical_index =
                                run.visual_to_logical(visual_index).unwrap_or_default() as u32;
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
                            return Some((path.cluster(layout)?, side));
                        }
                    }
                    LineItem::InlineBox(inline_box) => {
                        offset += inline_box.width;
                    }
                }
            }
        }
        if y <= 0.0 && !exact {
            Some((path.cluster(layout)?, ClusterSide::Left))
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
        self.run.clone()
    }

    /// Returns the path that contains the set of indices to reach the cluster
    /// from a layout.
    pub fn path(&self) -> ClusterPath {
        self.path
    }

    /// Returns the range of text that defines the cluster.
    pub fn text_range(&self) -> Range<usize> {
        self.data.text_range(self.run.data)
    }

    /// Returns the first style that applies to the cluster. If the cluster contains multiple glyphs
    /// then this style may not apply to all glyphs in the cluster (see the `DIVERGENT_STYLES` flag)
    pub fn first_style(&self) -> &Style<B> {
        &self.run.layout.styles()[usize::from(self.data.style_index)]
    }

    /// Returns the advance of the cluster.
    pub fn advance(&self) -> f32 {
        self.data.advance
    }

    /// Returns `true` if this is a right-to-left cluster.
    pub fn is_rtl(&self) -> bool {
        self.run.is_rtl()
    }

    /// Returns `true` if the cluster is the beginning of a ligature.
    pub fn is_ligature_start(&self) -> bool {
        self.data.is_ligature_start()
    }

    /// Returns `true` if the cluster is a ligature continuation.
    pub fn is_ligature_continuation(&self) -> bool {
        self.data.is_ligature_component()
    }

    /// Returns `true` if the cluster is a word boundary.
    pub fn is_word_boundary(&self) -> bool {
        self.data.info.is_boundary()
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
        self.data.info.whitespace() == Whitespace::Newline
    }

    /// Returns `true` if the cluster is a space or no-break space.
    pub fn is_space_or_nbsp(&self) -> bool {
        self.data.info.whitespace().is_space_or_nbsp()
    }

    /// Returns `true` if the cluster is an emoji sequence.
    pub fn is_emoji(&self) -> bool {
        self.data.info.is_emoji()
    }

    /// Returns an iterator over the glyphs in the cluster.
    pub fn glyphs(&self) -> impl Iterator<Item = Glyph> + 'a + Clone {
        if self.data.glyph_len == 0xFF {
            GlyphIter::Single(Some(Glyph {
                id: self.data.glyph_offset,
                style_index: self.data.style_index,
                x: 0.,
                y: 0.,
                advance: self.data.advance,
            }))
        } else {
            let start = self.run.data.glyph_start + self.data.glyph_offset as usize;
            GlyphIter::Slice(
                self.run.layout.data.glyphs[start..start + self.data.glyph_len as usize].iter(),
            )
        }
    }

    /// Returns `true` if this cluster is at the beginning of a line.
    pub fn is_start_of_line(&self) -> bool {
        self.path.run_index == 0 && self.run.logical_to_visual(self.path.logical_index()) == Some(0)
    }

    /// Returns `true` if this cluster is at the end of a line.
    pub fn is_end_of_line(&self) -> bool {
        self.line().len().saturating_sub(1) == self.path.run_index()
            && self.run.logical_to_visual(self.path.logical_index())
                == Some(self.run.cluster_range().len().saturating_sub(1))
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

    /// Returns the cluster that follows this one in logical order.
    pub fn next_logical(&self) -> Option<Self> {
        if self.path.logical_index() + 1 < self.run.cluster_range().len() {
            // Fast path: next cluster is in the same run
            ClusterPath {
                line_index: self.path.line_index,
                run_index: self.path.run_index,
                logical_index: self.path.logical_index + 1,
            }
            .cluster(self.run.layout)
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
        if self.path.logical_index > 0 {
            // Fast path: previous cluster is in the same run
            ClusterPath {
                line_index: self.path.line_index,
                run_index: self.path.run_index,
                logical_index: self.path.logical_index - 1,
            }
            .cluster(self.run.layout)
        } else {
            Self::from_byte_index(self.run.layout, self.text_range().start.checked_sub(1)?)
        }
    }

    /// Returns the cluster that follows this one in visual order.
    pub fn next_visual(&self) -> Option<Self> {
        let layout = self.run.layout;
        let run = self.run.clone();
        let visual_index = run.logical_to_visual(self.path.logical_index())?;
        if let Some(cluster_index) = run.visual_to_logical(visual_index + 1) {
            // Fast path: next visual cluster is in the same run
            run.get(cluster_index)
        } else {
            // We just want to find the first line/run following this one that
            // contains any cluster.
            let mut run_index = self.path.run_index() + 1;
            for line_index in self.path.line_index()..layout.len() {
                let line = layout.get(line_index)?;
                for run_index in run_index..line.len() {
                    if let Some(run) = line.item(run_index).and_then(|item| item.run()) {
                        if !run.cluster_range().is_empty() {
                            return ClusterPath {
                                line_index: line_index as u32,
                                run_index: run_index as u32,
                                logical_index: run.visual_to_logical(0)? as u32,
                            }
                            .cluster(layout);
                        }
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
        let visual_index = self.run.logical_to_visual(self.path.logical_index())?;
        if let Some(cluster_index) = visual_index
            .checked_sub(1)
            .and_then(|visual_index| self.run.visual_to_logical(visual_index))
        {
            // Fast path: previous visual cluster is in the same run
            ClusterPath {
                line_index: self.path.line_index,
                run_index: self.path.run_index,
                logical_index: cluster_index as u32,
            }
            .cluster(self.run.layout)
        } else {
            // We just want to find the first line/run preceding this one that
            // contains any cluster.
            let layout = self.run.layout;
            let mut run_index = Some(self.path.run_index());
            for line_index in (0..=self.path.line_index()).rev() {
                let line = layout.get(line_index)?;
                let first_run = run_index.unwrap_or(line.len());
                for run_index in (0..first_run).rev() {
                    if let Some(run) = line.item(run_index).and_then(|item| item.run()) {
                        let range = run.cluster_range();
                        if !range.is_empty() {
                            return ClusterPath {
                                line_index: line_index as u32,
                                run_index: run_index as u32,
                                logical_index: run.visual_to_logical(range.len() - 1)? as u32,
                            }
                            .cluster(layout);
                        }
                    }
                }
                run_index = None;
            }
            None
        }
    }

    /// Returns the next cluster that is marked as a word boundary.
    pub fn next_logical_word(&self) -> Option<Self> {
        let mut cluster = self.clone();
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
        let mut cluster = self.clone();
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
        let mut cluster = self.clone();
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
        let mut cluster = self.clone();
        while let Some(prev) = cluster.previous_visual() {
            if prev.is_word_boundary() {
                return Some(prev);
            }
            cluster = prev;
        }
        None
    }

    /// Returns the visual offset of this cluster along direction of text flow.
    ///
    /// This cost of this function is roughly linear in the number of clusters
    /// on the containing line.
    pub fn visual_offset(&self) -> Option<f32> {
        let line = self.path.line(self.run.layout)?;
        let mut offset = line.metrics().offset;
        for run_index in 0..=self.path.run_index() {
            let item = line.item(run_index)?;
            match item {
                LineItem::Run(run) => {
                    if run_index != self.path.run_index() {
                        offset += run.advance();
                    } else {
                        let visual_index = run.logical_to_visual(self.path.logical_index())?;
                        for cluster in run.visual_clusters().take(visual_index) {
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

    pub(crate) fn info(&self) -> &super::data::ClusterInfo {
        &self.data.info
    }

    /// Returns the text length of the cluster in bytes.
    ///
    /// This is only used for tests, and is *not* part of the public API.
    #[doc(hidden)]
    pub fn text_len(&self) -> u8 {
        self.data.text_len
    }

    /// Returns this cluster's original character.
    ///
    /// This is only used for tests, and is *not* part of the public API.
    #[doc(hidden)]
    pub fn source_char(&self) -> char {
        self.data.info.source_char()
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

/// Index based path to a cluster.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Default, Debug)]
pub struct ClusterPath {
    pub(crate) line_index: u32,
    pub(crate) run_index: u32,
    pub(crate) logical_index: u32,
}

impl ClusterPath {
    pub(crate) fn new(line_index: u32, run_index: u32, logical_index: u32) -> Self {
        Self {
            line_index,
            run_index,
            logical_index,
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

    /// Returns the logical index of the cluster within the owning run.
    pub fn logical_index(&self) -> usize {
        self.logical_index as usize
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
        self.run(layout)?.get(self.logical_index())
    }
}

#[derive(Clone)]
enum GlyphIter<'a> {
    Single(Option<Glyph>),
    Slice(core::slice::Iter<'a, Glyph>),
}

impl Iterator for GlyphIter<'_> {
    type Item = Glyph;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Single(glyph) => glyph.take(),
            Self::Slice(iter) => {
                let glyph = *iter.next()?;
                Some(glyph)
            }
        }
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
