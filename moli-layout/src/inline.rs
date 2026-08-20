// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The one-Parley-tree-per-IFC shape follows DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8, especially
// `layout/construct.rs::build_inline_layout_into`. Moli deliberately
// keeps the item stream, source mapping, and Parley layout pass-local.
// Relative positioning of atomic inline boxes additionally follows Blitz
// commit 4a9be930accc971675d5730e4fde3cfa13c3b57e.

use std::{collections::BTreeMap, fmt::Debug, hash::Hash, ops::Range};

use parley::{BreakReason, InlineBox, InlineBoxKind, Layout, PositionedLayoutItem, TextStyle};
use taffy::{
    Direction, FontBaseline, LogicalBoxStrut, LogicalOffset, LogicalSize, MaybeResolve as _, Point,
    Rect, Size, WritingDirection, WritingMode,
};

use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutPhysicalAxis, LayoutWorld, PaintColor, PaintEdgeSizes,
    PaintRect,
    style::{
        InlineDirection, InlineTextTransform, InlineUnicodeBidi, InlineVerticalAlign,
        InlineWhiteSpaceCollapse, LayoutInlineAlignment,
    },
    stylo_to_parley::TextBrush,
    text::{DocumentLayoutServices, InlineFontMetrics},
};

/// A rectangle in the IFC owner's flow-relative coordinate space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FlowRelativeRect {
    pub(crate) inline_offset: f32,
    pub(crate) block_offset: f32,
    pub(crate) inline_size: f32,
    pub(crate) block_size: f32,
}

impl FlowRelativeRect {
    pub(crate) const fn new(
        inline_offset: f32,
        block_offset: f32,
        inline_size: f32,
        block_size: f32,
    ) -> Self {
        Self {
            inline_offset,
            block_offset,
            inline_size,
            block_size,
        }
    }

    fn line_relative_rect(self, child: Self) -> LineRelativeRect {
        LineRelativeRect::new(
            child.inline_offset - self.inline_offset,
            child.block_offset - self.block_offset,
            child.inline_size,
            child.block_size,
        )
    }
}

/// Coordinates of an item relative to its containing line box.
///
/// Parley has already performed bidi reordering, so `inline_offset` is visual
/// rather than CSS `inline-start`. `line_over_offset` is measured from the
/// line-over side. These differ from flow-relative block coordinates in
/// `vertical-lr`, where lines progress left-to-right but line-over is right.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LineRelativeRect {
    pub(crate) inline_offset: f32,
    pub(crate) line_over_offset: f32,
    pub(crate) inline_size: f32,
    pub(crate) block_size: f32,
}

impl LineRelativeRect {
    pub(crate) const fn new(
        inline_offset: f32,
        line_over_offset: f32,
        inline_size: f32,
        block_size: f32,
    ) -> Self {
        Self {
            inline_offset,
            line_over_offset,
            inline_size,
            block_size,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LineRelativeOffset {
    pub(crate) inline_offset: f32,
    pub(crate) line_over_offset: f32,
}

impl LineRelativeOffset {
    pub(crate) const fn new(inline_offset: f32, line_over_offset: f32) -> Self {
        Self {
            inline_offset,
            line_over_offset,
        }
    }
}

/// Converts flow-relative lines and their visual line-relative children at
/// the physical fragment boundary.
///
/// The containing element's `direction` is intentionally absent here: Parley
/// has already resolved bidi ordering and alignment. Re-applying it would
/// mirror RTL content a second time. The writing mode remains necessary to
/// select physical axes and the direction of block progression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InlineCoordinateSpace {
    writing_mode: WritingMode,
}

impl InlineCoordinateSpace {
    pub(crate) const fn new(writing_mode: WritingMode) -> Self {
        Self { writing_mode }
    }

    pub(crate) const fn physical_inline_axis(self) -> LayoutPhysicalAxis {
        if self.writing_mode.is_horizontal() {
            LayoutPhysicalAxis::Horizontal
        } else {
            LayoutPhysicalAxis::Vertical
        }
    }

    pub(crate) fn to_logical_size<T>(self, size: Size<T>) -> LogicalSize<T> {
        self.writing_mode.to_logical(size)
    }

    pub(crate) fn to_physical_size<T>(self, size: LogicalSize<T>) -> Size<T> {
        self.writing_mode.to_physical(size)
    }

    pub(crate) fn to_line_relative_box_strut<T: Copy>(self, edges: Rect<T>) -> LogicalBoxStrut<T> {
        self.line_writing_direction().to_logical_box_strut(edges)
    }

    pub(crate) fn to_physical_flow_point(
        self,
        offset: LogicalOffset<f32>,
        inner_size: Size<f32>,
        outer_size: Size<f32>,
    ) -> Point<f32> {
        self.flow_writing_direction()
            .converter(outer_size)
            .to_physical_point(offset, inner_size)
    }

    pub(crate) fn to_physical_flow_rect(
        self,
        rect: FlowRelativeRect,
        outer_size: Size<f32>,
    ) -> PaintRect {
        let size = self.to_physical_size(LogicalSize {
            inline_size: rect.inline_size,
            block_size: rect.block_size,
        });
        let point = self.to_physical_flow_point(
            LogicalOffset {
                inline_offset: rect.inline_offset,
                block_offset: rect.block_offset,
            },
            size,
            outer_size,
        );
        PaintRect::new(point.x, point.y, size.width, size.height)
    }

    pub(crate) fn to_physical_line_rect(
        self,
        line: FlowRelativeRect,
        child: LineRelativeRect,
        outer_size: Size<f32>,
    ) -> PaintRect {
        let physical_line = self.to_physical_flow_rect(line, outer_size);
        let child_size = self.to_physical_size(LogicalSize {
            inline_size: child.inline_size,
            block_size: child.block_size,
        });
        let child_offset = self.line_writing_direction().converter(Size {
            width: physical_line.width,
            height: physical_line.height,
        });
        let child_offset = child_offset.to_physical_point(
            LogicalOffset {
                inline_offset: child.inline_offset,
                block_offset: child.line_over_offset,
            },
            child_size,
        );
        PaintRect::new(
            physical_line.x + child_offset.x,
            physical_line.y + child_offset.y,
            child_size.width,
            child_size.height,
        )
    }

    pub(crate) fn to_physical_line_point(
        self,
        line: FlowRelativeRect,
        child: LineRelativeOffset,
        inner_size: Size<f32>,
        outer_size: Size<f32>,
    ) -> Point<f32> {
        let physical_line = self.to_physical_flow_rect(line, outer_size);
        let child_offset = self
            .line_writing_direction()
            .converter(Size {
                width: physical_line.width,
                height: physical_line.height,
            })
            .to_physical_point(
                LogicalOffset {
                    inline_offset: child.inline_offset,
                    block_offset: child.line_over_offset,
                },
                inner_size,
            );
        Point {
            x: physical_line.x + child_offset.x,
            y: physical_line.y + child_offset.y,
        }
    }

    pub(crate) fn to_physical_line_baseline(
        self,
        line: &InlineLinePlacement,
        outer_size: Size<f32>,
    ) -> Point<Option<f32>> {
        let point = self.to_physical_line_point(
            line.rect,
            LineRelativeOffset::new(0.0, line.baseline - line.rect.block_offset),
            Size::ZERO,
            outer_size,
        );
        if self.writing_mode.is_horizontal() {
            Point {
                x: None,
                y: Some(point.y),
            }
        } else {
            Point {
                x: Some(point.x),
                y: None,
            }
        }
    }

    pub(crate) fn to_physical_line_block_baseline(
        self,
        block_offset: Option<f32>,
        outer_size: Size<f32>,
    ) -> Point<Option<f32>> {
        let Some(block_offset) = block_offset else {
            return Point::NONE;
        };
        let point = self
            .line_writing_direction()
            .converter(outer_size)
            .to_physical_point(
                LogicalOffset {
                    inline_offset: 0.0,
                    block_offset,
                },
                Size::ZERO,
            );
        if self.writing_mode.is_horizontal() {
            Point {
                x: None,
                y: Some(point.y),
            }
        } else {
            Point {
                x: Some(point.x),
                y: None,
            }
        }
    }

    pub(crate) fn to_line_block_baseline(
        self,
        baseline: Point<Option<f32>>,
        fragment_size: Size<f32>,
    ) -> Option<f32> {
        let physical = if self.writing_mode.is_horizontal() {
            Point {
                x: 0.0,
                y: baseline.y?,
            }
        } else {
            Point {
                x: baseline.x?,
                y: 0.0,
            }
        };
        Some(
            self.line_writing_direction()
                .converter(fragment_size)
                .to_logical_point(physical, Size::ZERO)
                .block_offset,
        )
    }

    const fn flow_writing_direction(self) -> WritingDirection {
        WritingDirection::new(self.writing_mode, Direction::Ltr)
    }

    const fn line_writing_direction(self) -> WritingDirection {
        let writing_mode = match self.writing_mode {
            // CSS line-relative directions invert vertical-lr's flow-relative
            // block axis. This is Blink's ToLineWritingMode().
            WritingMode::VerticalLr => WritingMode::VerticalRl,
            writing_mode => writing_mode,
        };
        WritingDirection::new(writing_mode, Direction::Ltr)
    }
}

/// Resolve the relative inset applied after Parley has positioned an atomic
/// inline box. Taffy cannot do this itself because atomic IFC children are
/// represented as Parley inline objects and their final locations are written
/// back after line layout.
pub(crate) fn relative_atomic_inset_offset(
    style: &taffy::Style<style::Atom>,
    containing_block_size: Size<f32>,
    writing_direction: WritingDirection,
) -> Point<f32> {
    let inset = taffy::Rect {
        left: style.inset.left.maybe_resolve(
            containing_block_size.width,
            crate::style::resolve_stylo_calc_value,
        ),
        right: style.inset.right.maybe_resolve(
            containing_block_size.width,
            crate::style::resolve_stylo_calc_value,
        ),
        top: style.inset.top.maybe_resolve(
            containing_block_size.height,
            crate::style::resolve_stylo_calc_value,
        ),
        bottom: style.inset.bottom.maybe_resolve(
            containing_block_size.height,
            crate::style::resolve_stylo_calc_value,
        ),
    };
    flow_relative_inset_offset(inset, writing_direction)
}

fn flow_relative_inset_offset(
    inset: Rect<Option<f32>>,
    writing_direction: WritingDirection,
) -> Point<f32> {
    let logical = writing_direction.to_logical_box_strut(inset);
    let offset = LogicalOffset {
        inline_offset: logical
            .inline_start
            .or_else(|| logical.inline_end.map(|value| -value))
            .unwrap_or(0.0),
        block_offset: logical
            .block_start
            .or_else(|| logical.block_end.map(|value| -value))
            .unwrap_or(0.0),
    };
    writing_direction
        .converter(Size::ZERO)
        .to_physical_point(offset, Size::ZERO)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InlineSourceMapEntry {
    pub(crate) output_range: Range<usize>,
    pub(crate) box_id: LayoutBoxId,
    pub(crate) source_byte_range: Range<usize>,
    pub(crate) source_utf16_range: Range<usize>,
    pub(crate) is_forced_line_break: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct InlineTextUnit {
    pub(crate) output_range: Range<usize>,
    pub(crate) style_box: LayoutBoxId,
    pub(crate) ancestors: Vec<LayoutBoxId>,
    pub(crate) sources: Vec<SourceOrigin>,
    kind: InlineTextUnitKind,
}

/// Semantic identity retained for one unit in Parley's shared text stream.
///
/// A DOM `<br>` and a preserved newline both shape as U+000A, but only the
/// former owns element geometry. Keeping that distinction beside the stream
/// is the analogue of Blink's forced-line-break `InlineItem`: line breaking
/// remains Parley's job while fragment provenance remains the browser's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineTextUnitKind {
    Text,
    Control,
    ForcedLineBreak { element_box: Option<LayoutBoxId> },
}

impl InlineTextUnitKind {
    const fn is_control(self) -> bool {
        matches!(self, Self::Control)
    }

    const fn element_line_break_box(self) -> Option<LayoutBoxId> {
        match self {
            Self::ForcedLineBreak {
                element_box: Some(box_id),
            } => Some(box_id),
            Self::Text | Self::Control | Self::ForcedLineBreak { .. } => None,
        }
    }

    const fn is_forced_line_break(self) -> bool {
        matches!(self, Self::ForcedLineBreak { .. })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SourceOrigin {
    pub(crate) box_id: LayoutBoxId,
    pub(crate) byte_range: Range<usize>,
    pub(crate) utf16_range: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InlineObjectRole {
    Atomic,
    Float,
    OutOfFlow,
    StartEdge,
    EndEdge,
}

#[derive(Clone, Debug)]
pub(crate) struct InlineObject {
    pub(crate) box_id: LayoutBoxId,
    pub(crate) role: InlineObjectRole,
    pub(crate) ancestors: Vec<LayoutBoxId>,
    /// The object's own computed `vertical-align`. Structural ancestor shifts
    /// are applied by the per-line inline box-state tree.
    pub(crate) vertical_align: InlineVerticalAlign,
}

/// Pass-owned metadata for one non-atomic inline box flattened into Parley.
///
/// Parley owns shaping and inline-axis breaking, while this hierarchy restores
/// the box states required by CSS line layout. It mirrors Blink's
/// `InlineBoxState`: every inline keeps its own font strut, parent, and
/// `vertical-align` instead of composing all ancestors onto each glyph run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InlineStructuralBox {
    pub(crate) box_id: LayoutBoxId,
    pub(crate) parent: LayoutBoxId,
    pub(crate) vertical_align: InlineVerticalAlign,
    pub(crate) strut: Option<InlineStrutMetrics>,
    pub(crate) include_used_font_metrics: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct InlineFormattingContext {
    pub(crate) root_style: LayoutBoxId,
    /// Whether Parley quantizes inline metrics to device-independent pixels.
    /// Callers that resume the line breaker must use the same policy.
    pub(crate) quantize: bool,
    /// Baseline protocol of the IFC owner. This is independent from Parley's
    /// horizontal shaping coordinates and controls line-box synthesis.
    pub(crate) font_baseline: FontBaseline,
    pub(crate) unbroken: Layout<TextBrush>,
    pub(crate) laid_out: Option<Layout<TextBrush>>,
    pub(crate) text_units: Vec<InlineTextUnit>,
    pub(crate) source_map: Vec<InlineSourceMapEntry>,
    pub(crate) selection: Option<InlineSelection>,
    pub(crate) objects: Vec<InlineObject>,
    /// Primary-font metrics indexed by Parley's style index. Glyph runs may
    /// use fallback fonts, but their CSSOM rectangles and text-edge alignment
    /// retain these primary metrics. Only `line-height: normal` additionally
    /// unites the used font's metrics into the enclosing line box.
    pub(crate) font_metrics: Vec<Option<InlineFontMetrics>>,
    /// The IFC owner's primary-font strut used while reconstructing CSS line
    /// baselines. Fallback glyph fonts must not replace its line height or
    /// x-height.
    pub(crate) parent_strut: Option<InlineStrutMetrics>,
    pub(crate) root_includes_used_font_metrics: bool,
    /// Direct structural parent of each shaped style. Including this identity
    /// in style deduplication prevents glyph runs from crossing a box-state
    /// boundary even when their paint/font properties are otherwise equal.
    pub(crate) style_parents: Vec<LayoutBoxId>,
    pub(crate) structural_boxes: Vec<InlineStructuralBox>,
    pub(crate) line_placements: Vec<InlineLinePlacement>,
    pub(crate) fragments: InlineFragments,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct InlineStrutMetrics {
    line_ascent: f32,
    line_descent: f32,
    text_ascent: f32,
    text_descent: f32,
    x_height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InlineSelection {
    Range(Range<usize>),
    Caret { offset: usize, color: PaintColor },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InlineLinePlacement {
    pub(crate) line_index: usize,
    pub(crate) rect: FlowRelativeRect,
    pub(crate) baseline: f32,
    /// CSS phantom line boxes retain positions for their inline descendants,
    /// but do not contribute height, baselines, or block margin-collapse
    /// barriers.
    pub(crate) phantom: bool,
    content_offset: f32,
    item_offsets: Vec<f32>,
    glyph_offsets: Vec<InlineGlyphOffset>,
    box_block_placements: Vec<InlineBoxBlockPlacement>,
}

impl InlineLinePlacement {
    pub(crate) fn item_offset(&self, item_index: usize) -> f32 {
        self.item_offsets
            .get(item_index)
            .copied()
            .unwrap_or_default()
    }

    fn glyph_offset(&self, run_index: usize, style_index: usize) -> f32 {
        self.glyph_offsets
            .iter()
            .find(|offset| offset.run_index == run_index && offset.style_index == style_index)
            .map_or(self.content_offset, |offset| offset.offset)
    }

    pub(crate) fn translate_block_axis(&mut self, offset: f32) {
        self.rect.block_offset += offset;
        self.baseline += offset;
        self.content_offset += offset;
        for item_offset in &mut self.item_offsets {
            *item_offset += offset;
        }
        for glyph_offset in &mut self.glyph_offsets {
            glyph_offset.offset += offset;
        }
        for box_placement in &mut self.box_block_placements {
            box_placement.top += offset;
        }
    }
}

pub(crate) fn flow_relative_line_rect(
    line: &parley::Line<'_, TextBrush>,
    placement: Option<&InlineLinePlacement>,
) -> FlowRelativeRect {
    placement.map_or_else(
        || {
            let metrics = line.metrics();
            FlowRelativeRect::new(
                metrics.inline_min_coord + metrics.offset,
                metrics.block_min_coord,
                metrics.advance,
                (metrics.block_max_coord - metrics.block_min_coord).max(0.0),
            )
        },
        |placement| placement.rect,
    )
}

fn line_box_rect_without_hanging(
    mut rect: FlowRelativeRect,
    trailing_whitespace: f32,
    is_rtl: bool,
) -> (FlowRelativeRect, bool) {
    let hanging = if trailing_whitespace.is_finite() {
        trailing_whitespace.clamp(0.0, rect.inline_size.max(0.0))
    } else {
        0.0
    };
    if is_rtl {
        rect.inline_offset += hanging;
    }
    rect.inline_size = (rect.inline_size - hanging).max(0.0);
    (rect, hanging > 0.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct InlineGlyphOffset {
    run_index: usize,
    style_index: usize,
    offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct InlineBoxBlockPlacement {
    box_id: LayoutBoxId,
    top: f32,
    height: f32,
}

impl InlineFormattingContext {
    pub(crate) fn object(&self, id: u64) -> Option<&InlineObject> {
        usize::try_from(id)
            .ok()
            .and_then(|index| self.objects.get(index))
    }

    fn style_parent(&self, index: usize) -> LayoutBoxId {
        self.style_parents
            .get(index)
            .copied()
            .unwrap_or(self.root_style)
    }

    fn box_includes_used_font_metrics(&self, box_id: LayoutBoxId) -> bool {
        if box_id == self.root_style {
            return self.root_includes_used_font_metrics;
        }
        self.structural_box(box_id)
            .is_some_and(|state| state.include_used_font_metrics)
    }

    fn structural_box(&self, id: LayoutBoxId) -> Option<&InlineStructuralBox> {
        self.structural_boxes
            .iter()
            .find(|state| state.box_id == id)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InlineFragments {
    pub(crate) lines: Vec<InlineLineFragment>,
    pub(crate) text: Vec<InlineSourceFragment>,
    pub(crate) boxes: Vec<InlineBoxFragment>,
    pub(crate) line_breaks: Vec<InlineLineBreakFragment>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LineRelativeFragments {
    lines: Vec<LineRelativeLineFragment>,
    text: Vec<LineRelativeSourceFragment>,
    boxes: Vec<LineRelativeBoxFragment>,
    line_breaks: Vec<LineRelativeLineBreakFragment>,
}

impl LineRelativeFragments {
    pub(crate) fn translate_block_axis(&mut self, offset: f32) {
        for line in &mut self.lines {
            line.used_rect.block_offset += offset;
        }
    }

    pub(crate) fn into_physical(
        self,
        coordinates: InlineCoordinateSpace,
        content_box_size: Size<f32>,
    ) -> InlineFragments {
        let Self {
            lines,
            text,
            boxes,
            line_breaks,
        } = self;
        let flow_lines = lines.iter().map(|line| line.used_rect).collect::<Vec<_>>();
        let physical_lines = lines
            .iter()
            .map(|line| coordinates.to_physical_flow_rect(line.used_rect, content_box_size))
            .collect::<Vec<_>>();
        InlineFragments {
            lines: lines
                .into_iter()
                .zip(physical_lines)
                .map(|(line, rect)| InlineLineFragment {
                    line_index: line.line_index,
                    used_rect: rect,
                    has_hanging: line.has_hanging,
                    inline_axis: coordinates.physical_inline_axis(),
                })
                .collect(),
            text: text
                .into_iter()
                .map(|text| InlineSourceFragment {
                    line_index: text.line_index,
                    box_id: text.box_id,
                    source_byte_range: text.source_byte_range,
                    source_utf16_range: text.source_utf16_range,
                    is_forced_line_break: text.is_forced_line_break,
                    inline_axis: coordinates.physical_inline_axis(),
                    rtl: text.rtl,
                    rect: coordinates.to_physical_line_rect(
                        flow_lines[text.line_index],
                        text.rect,
                        content_box_size,
                    ),
                })
                .collect(),
            boxes: boxes
                .into_iter()
                .map(|inline_box| InlineBoxFragment {
                    line_index: inline_box.line_index,
                    box_id: inline_box.box_id,
                    rect: coordinates.to_physical_line_rect(
                        flow_lines[inline_box.line_index],
                        inline_box.rect,
                        content_box_size,
                    ),
                    has_start_edge: inline_box.has_start_edge,
                    has_end_edge: inline_box.has_end_edge,
                })
                .collect(),
            line_breaks: line_breaks
                .into_iter()
                .map(|line_break| InlineLineBreakFragment {
                    line_index: line_break.line_index,
                    box_id: line_break.box_id,
                    rect: coordinates.to_physical_line_rect(
                        flow_lines[line_break.line_index],
                        line_break.rect,
                        content_box_size,
                    ),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineLineFragment {
    pub(crate) line_index: usize,
    /// The actual line-box fragment. Hanging trailing white space is outside
    /// this rect, while its text/inline child fragments retain their geometry.
    pub(crate) used_rect: PaintRect,
    has_hanging: bool,
    inline_axis: LayoutPhysicalAxis,
}

impl InlineLineFragment {
    /// Mirrors Blink's `AdjustOverflowForHanging`: the full text and inline
    /// box geometry remains available for paint and CSSOM, while scrollable
    /// overflow is clipped only along the line's inline axis.
    pub(crate) fn adjust_scrollable_overflow(self, mut overflow: PaintRect) -> PaintRect {
        if !self.has_hanging {
            return overflow;
        }
        match self.inline_axis {
            LayoutPhysicalAxis::Horizontal => {
                let line_start = self.used_rect.x;
                let line_end = self.used_rect.right();
                if overflow.x < line_start {
                    // Blink moves an overflow rect whose start edge is in the
                    // hanging area; it does not shrink that edge in place.
                    overflow.x = line_start;
                }
                if overflow.right() > line_end {
                    overflow.width = (line_end - overflow.x).max(0.0);
                }
            }
            LayoutPhysicalAxis::Vertical => {
                let line_start = self.used_rect.y;
                let line_end = self.used_rect.bottom();
                if overflow.y < line_start {
                    overflow.y = line_start;
                }
                if overflow.bottom() > line_end {
                    overflow.height = (line_end - overflow.y).max(0.0);
                }
            }
        }
        overflow
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LineRelativeLineFragment {
    line_index: usize,
    /// Used line-box geometry, excluding hanging trailing white space.
    used_rect: FlowRelativeRect,
    has_hanging: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InlineSourceFragment {
    pub(crate) line_index: usize,
    pub(crate) box_id: LayoutBoxId,
    pub(crate) source_byte_range: Range<usize>,
    pub(crate) source_utf16_range: Range<usize>,
    pub(crate) is_forced_line_break: bool,
    pub(crate) inline_axis: LayoutPhysicalAxis,
    pub(crate) rtl: bool,
    pub(crate) rect: PaintRect,
}

#[derive(Clone, Debug, PartialEq)]
struct LineRelativeSourceFragment {
    line_index: usize,
    box_id: LayoutBoxId,
    source_byte_range: Range<usize>,
    source_utf16_range: Range<usize>,
    is_forced_line_break: bool,
    rtl: bool,
    rect: LineRelativeRect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineLineBreakFragment {
    pub(crate) line_index: usize,
    pub(crate) box_id: LayoutBoxId,
    pub(crate) rect: PaintRect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LineRelativeLineBreakFragment {
    line_index: usize,
    box_id: LayoutBoxId,
    rect: LineRelativeRect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineBoxFragment {
    pub(crate) line_index: usize,
    pub(crate) box_id: LayoutBoxId,
    pub(crate) rect: PaintRect,
    pub(crate) has_start_edge: bool,
    pub(crate) has_end_edge: bool,
}

/// Physical box-model geometry for one fragment of a non-atomic inline box.
///
/// Parley includes the logical inline-axis edge contributions in its advance,
/// while the line box supplies only the structural box's block-axis font
/// extent. Resolving both facts here keeps paint, CSSOM geometry, hit testing,
/// and positioned containing blocks on one box-model definition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineFragmentBoxGeometry {
    pub(crate) margin_rect: PaintRect,
    pub(crate) border_rect: PaintRect,
    pub(crate) padding_rect: PaintRect,
    pub(crate) content_rect: PaintRect,
    pub(crate) border_widths: PaintEdgeSizes,
    pub(crate) padding_widths: PaintEdgeSizes,
}

pub(crate) fn inline_fragment_box_geometry(
    fragment: &InlineBoxFragment,
    writing_direction: WritingDirection,
    margin: Rect<f32>,
    padding: Rect<f32>,
    border: Rect<f32>,
) -> InlineFragmentBoxGeometry {
    let logical_margin = writing_direction.to_logical_box_strut(margin);
    let logical_padding = writing_direction.to_logical_box_strut(padding);
    let logical_border = writing_direction.to_logical_box_strut(border);

    let included_inline_margin = writing_direction.to_physical_box_strut(LogicalBoxStrut {
        inline_start: if fragment.has_start_edge {
            logical_margin.inline_start
        } else {
            0.0
        },
        inline_end: if fragment.has_end_edge {
            logical_margin.inline_end
        } else {
            0.0
        },
        block_start: 0.0,
        block_end: 0.0,
    });
    let block_expansion = writing_direction.to_physical_box_strut(LogicalBoxStrut {
        inline_start: 0.0,
        inline_end: 0.0,
        block_start: (logical_padding.block_start + logical_border.block_start).max(0.0),
        block_end: (logical_padding.block_end + logical_border.block_end).max(0.0),
    });
    let border_rect = PaintRect::new(
        fragment.rect.x + included_inline_margin.left - block_expansion.left,
        fragment.rect.y + included_inline_margin.top - block_expansion.top,
        (fragment.rect.width - included_inline_margin.left - included_inline_margin.right
            + block_expansion.left
            + block_expansion.right)
            .max(0.0),
        (fragment.rect.height - included_inline_margin.top - included_inline_margin.bottom
            + block_expansion.top
            + block_expansion.bottom)
            .max(0.0),
    );

    let painted_border = writing_direction.to_physical_box_strut(LogicalBoxStrut {
        inline_start: fragment
            .has_start_edge
            .then_some(logical_border.inline_start),
        inline_end: fragment.has_end_edge.then_some(logical_border.inline_end),
        block_start: Some(logical_border.block_start),
        block_end: Some(logical_border.block_end),
    });
    let painted_padding = writing_direction.to_physical_box_strut(LogicalBoxStrut {
        inline_start: fragment
            .has_start_edge
            .then_some(logical_padding.inline_start),
        inline_end: fragment.has_end_edge.then_some(logical_padding.inline_end),
        block_start: Some(logical_padding.block_start),
        block_end: Some(logical_padding.block_end),
    });
    let border_widths = nonnegative_physical_edge_sizes(painted_border);
    let padding_widths = nonnegative_physical_edge_sizes(painted_padding);
    let padding_rect = inset_physical_rect(border_rect, border_widths);
    let content_rect = inset_physical_rect(padding_rect, padding_widths);

    let fragment_margin = writing_direction.to_physical_box_strut(LogicalBoxStrut {
        inline_start: fragment
            .has_start_edge
            .then_some(logical_margin.inline_start),
        inline_end: fragment.has_end_edge.then_some(logical_margin.inline_end),
        block_start: Some(logical_margin.block_start),
        block_end: Some(logical_margin.block_end),
    });
    let margin_rect =
        expand_physical_rect(border_rect, signed_physical_edge_sizes(fragment_margin));

    InlineFragmentBoxGeometry {
        margin_rect,
        border_rect,
        padding_rect,
        content_rect,
        border_widths,
        padding_widths,
    }
}

fn nonnegative_physical_edge_sizes(edges: Rect<Option<f32>>) -> PaintEdgeSizes {
    PaintEdgeSizes::new(
        edges.top.unwrap_or(0.0).max(0.0),
        edges.right.unwrap_or(0.0).max(0.0),
        edges.bottom.unwrap_or(0.0).max(0.0),
        edges.left.unwrap_or(0.0).max(0.0),
    )
}

fn signed_physical_edge_sizes(edges: Rect<Option<f32>>) -> PaintEdgeSizes {
    PaintEdgeSizes::new(
        edges.top.unwrap_or(0.0),
        edges.right.unwrap_or(0.0),
        edges.bottom.unwrap_or(0.0),
        edges.left.unwrap_or(0.0),
    )
}

fn inset_physical_rect(rect: PaintRect, edges: PaintEdgeSizes) -> PaintRect {
    PaintRect::new(
        rect.x + edges.left,
        rect.y + edges.top,
        (rect.width - edges.left - edges.right).max(0.0),
        (rect.height - edges.top - edges.bottom).max(0.0),
    )
}

fn expand_physical_rect(rect: PaintRect, edges: PaintEdgeSizes) -> PaintRect {
    PaintRect::new(
        rect.x - edges.left,
        rect.y - edges.top,
        (rect.width + edges.left + edges.right).max(0.0),
        (rect.height + edges.top + edges.bottom).max(0.0),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LineRelativeBoxFragment {
    line_index: usize,
    box_id: LayoutBoxId,
    rect: LineRelativeRect,
    has_start_edge: bool,
    has_end_edge: bool,
}

pub(crate) fn build_inline_fragments(
    context: &InlineFormattingContext,
    layout: &Layout<TextBrush>,
    line_placements: &[InlineLinePlacement],
) -> LineRelativeFragments {
    let mut fragments = LineRelativeFragments::default();
    let mut box_fragments = BTreeMap::<(usize, usize), FragmentAccumulator>::new();
    let mut source_fragments = BTreeMap::<SourceFragmentKey, FragmentAccumulator>::new();
    let mut line_break_fragments = BTreeMap::<(usize, usize), FragmentAccumulator>::new();

    for (line_index, line) in layout.lines().enumerate() {
        let metrics = line.metrics();
        let placement = line_placements
            .get(line_index)
            .filter(|placement| placement.line_index == line_index);
        let line_rect = flow_relative_line_rect(&line, placement);
        let (used_line_rect, has_hanging) =
            line_box_rect_without_hanging(line_rect, metrics.trailing_whitespace, layout.is_rtl());
        fragments.lines.push(LineRelativeLineFragment {
            line_index,
            used_rect: used_line_rect,
            has_hanging,
        });
        if let Some(placement) = placement {
            for box_placement in &placement.box_block_placements {
                box_fragments
                    .entry((box_placement.box_id.index(), line_index))
                    .or_default()
                    .include_block_axis(box_placement.top, box_placement.height);
            }
        }

        for run in line.runs() {
            let run_metrics = run.font_metrics();
            for cluster in run.visual_clusters() {
                let range = cluster.text_range();
                let style_index = usize::from(cluster.style_index());
                let vertical_offset = placement.map_or(0.0, |placement| {
                    placement.glyph_offset(run.index(), style_index)
                });
                // CSSOM text quads use the typographic font box. CSS
                // `line-height` and its leading enlarge the containing line
                // box, but not LayoutText/Range geometry. This matches
                // Blink's InlineBoxState::text_top/text_height contract.
                let font_metrics = context
                    .font_metrics
                    .get(style_index)
                    .copied()
                    .flatten()
                    .map(|metrics| inline_strut_metrics(metrics, true, context.font_baseline));
                let ascent = font_metrics.map_or(run_metrics.ascent, |metrics| metrics.text_ascent);
                let descent =
                    font_metrics.map_or(run_metrics.descent, |metrics| metrics.text_descent);
                let rect = FlowRelativeRect::new(
                    metrics.inline_min_coord + cluster.visual_offset().unwrap_or(metrics.offset),
                    metrics.baseline - ascent + vertical_offset,
                    cluster.advance().max(0.0),
                    (ascent + descent).max(0.0),
                );
                for unit in context
                    .text_units
                    .iter()
                    .filter(|unit| ranges_overlap(&unit.output_range, &range))
                {
                    for ancestor in &unit.ancestors {
                        box_fragments
                            .entry((ancestor.index(), line_index))
                            .or_default()
                            .include_inline_axis(rect.inline_offset, rect.inline_size);
                    }
                    if let Some(box_id) = unit.kind.element_line_break_box() {
                        line_break_fragments
                            .entry((box_id.index(), line_index))
                            .or_default()
                            .include(rect);
                    }
                }
                for source in context
                    .source_map
                    .iter()
                    .filter(|source| ranges_overlap(&source.output_range, &range))
                {
                    source_fragments
                        .entry(SourceFragmentKey {
                            box_index: source.box_id.index(),
                            source_byte_start: source.source_byte_range.start,
                            source_byte_end: source.source_byte_range.end,
                            source_utf16_start: source.source_utf16_range.start,
                            source_utf16_end: source.source_utf16_range.end,
                            is_forced_line_break: source.is_forced_line_break,
                            line_index,
                            rtl: cluster.is_rtl(),
                        })
                        .or_default()
                        .include(rect);
                }
            }
        }

        for (item_index, item) in line.items().enumerate() {
            let PositionedLayoutItem::InlineBox(positioned) = item else {
                continue;
            };
            let Some(object) = context.object(positioned.id) else {
                continue;
            };
            let rect = (object.role == InlineObjectRole::Atomic).then(|| {
                FlowRelativeRect::new(
                    positioned.x,
                    positioned.y
                        + placement.map_or(0.0, |placement| placement.item_offset(item_index)),
                    positioned.width.max(0.0),
                    positioned.height.max(0.0),
                )
            });
            for ancestor in &object.ancestors {
                let accumulator = box_fragments
                    .entry((ancestor.index(), line_index))
                    .or_default();
                if let Some(rect) = rect {
                    accumulator.include_inline_axis(rect.inline_offset, rect.inline_size);
                } else if matches!(
                    object.role,
                    InlineObjectRole::StartEdge | InlineObjectRole::EndEdge
                ) {
                    accumulator.include_inline_axis(positioned.x, positioned.width);
                }
            }
            match object.role {
                InlineObjectRole::StartEdge | InlineObjectRole::EndEdge => {
                    let accumulator = box_fragments
                        .entry((object.box_id.index(), line_index))
                        .or_default();
                    accumulator.include_inline_axis(positioned.x, positioned.width);
                    accumulator.has_start_edge |= object.role == InlineObjectRole::StartEdge;
                    accumulator.has_end_edge |= object.role == InlineObjectRole::EndEdge;
                }
                InlineObjectRole::Atomic
                | InlineObjectRole::Float
                | InlineObjectRole::OutOfFlow => {}
            }
        }
    }

    fragments.boxes = box_fragments
        .into_iter()
        .filter_map(|((box_index, line_index), accumulator)| {
            let line_rect = fragments.lines.get(line_index)?.used_rect;
            Some(LineRelativeBoxFragment {
                line_index,
                box_id: LayoutBoxId::from_index(box_index),
                rect: line_rect.line_relative_rect(accumulator.rect(line_rect)?),
                has_start_edge: accumulator.has_start_edge,
                has_end_edge: accumulator.has_end_edge,
            })
        })
        .collect();
    fragments.text = source_fragments
        .into_iter()
        .filter_map(|(key, accumulator)| {
            let line_rect = fragments.lines.get(key.line_index)?.used_rect;
            Some(LineRelativeSourceFragment {
                line_index: key.line_index,
                box_id: LayoutBoxId::from_index(key.box_index),
                source_byte_range: key.source_byte_start..key.source_byte_end,
                source_utf16_range: key.source_utf16_start..key.source_utf16_end,
                is_forced_line_break: key.is_forced_line_break,
                rtl: key.rtl,
                rect: line_rect.line_relative_rect(accumulator.rect(line_rect)?),
            })
        })
        .collect();
    fragments.line_breaks = line_break_fragments
        .into_iter()
        .filter_map(|((box_index, line_index), accumulator)| {
            let line_rect = fragments.lines.get(line_index)?.used_rect;
            Some(LineRelativeLineBreakFragment {
                line_index,
                box_id: LayoutBoxId::from_index(box_index),
                rect: line_rect.line_relative_rect(accumulator.rect(line_rect)?),
            })
        })
        .collect();
    fragments
}

/// Builds the pass-local vertical placement sidecar that Parley does not
/// provide for CSS `vertical-align`. The sidecar leaves Parley's shaped data
/// immutable and applies the same offsets to glyph projection, atomic boxes,
/// out-of-flow static positions, and fragment geometry.
pub(crate) fn build_inline_line_placements(
    context: &InlineFormattingContext,
    layout: &Layout<TextBrush>,
    atomic_baseline_ascents: &[Option<f32>],
    structural_edge_contributions: &[bool],
) -> (Vec<InlineLinePlacement>, f32) {
    let mut placements = Vec::with_capacity(layout.lines().len());
    let mut preceding_adjustment = 0.0;
    let mut unadjusted_line_top = 0.0;

    for (line_index, line) in layout.lines().enumerate() {
        let metrics = line.metrics();
        let raw_top = unadjusted_line_top;
        let raw_bottom = raw_top + metrics.line_height.max(0.0);
        let mut geometries = line
            .items()
            .map(|item| match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    let run = glyph_run.run();
                    let run_metrics = run.font_metrics();
                    let paint = glyph_run.style().brush.paint;
                    let style_index = usize::from(glyph_run.style_index());
                    let structural_parent = context.style_parent(style_index);
                    let primary_strut = context
                        .font_metrics
                        .get(style_index)
                        .copied()
                        .flatten()
                        .map(|metrics| inline_strut_metrics(metrics, true, context.font_baseline));
                    let bounds = glyph_line_bounds(
                        primary_strut,
                        run_metrics,
                        run.line_height(),
                        context.box_includes_used_font_metrics(structural_parent),
                        context.font_baseline,
                    );
                    InlineItemVerticalGeometry {
                        bounds,
                        initial_top: glyph_run.baseline() + bounds.top,
                        structural_parent,
                        edge_box: None,
                        vertical_align: InlineVerticalAlign::default(),
                        contributes_to_line: paint,
                        creates_line: paint,
                        glyph_key: paint.then_some((run.index(), style_index)),
                        anchor: LineVerticalAnchor::Root,
                        relative_offset: 0.0,
                    }
                }
                PositionedLayoutItem::InlineBox(positioned) => {
                    let object = context.object(positioned.id);
                    let object_index = usize::try_from(positioned.id).ok();
                    let internal_baseline_ascent = object
                        .filter(|object| object.role == InlineObjectRole::Atomic)
                        .and(object_index)
                        .and_then(|index| atomic_baseline_ascents.get(index).copied().flatten());
                    let is_atomic =
                        object.is_some_and(|object| object.role == InlineObjectRole::Atomic);
                    let baseline_ascent = internal_baseline_ascent
                        .or_else(|| {
                            is_atomic.then(|| {
                                synthesized_font_ascent(context.font_baseline, positioned.height)
                            })
                        })
                        .unwrap_or_default();
                    InlineItemVerticalGeometry {
                        bounds: if is_atomic {
                            InlineVerticalBounds {
                                top: -baseline_ascent,
                                bottom: positioned.height - baseline_ascent,
                            }
                        } else {
                            InlineVerticalBounds::ZERO
                        },
                        initial_top: positioned.y,
                        structural_parent: object
                            .and_then(|object| object.ancestors.last().copied())
                            .unwrap_or(context.root_style),
                        edge_box: object.and_then(|object| {
                            matches!(
                                object.role,
                                InlineObjectRole::StartEdge | InlineObjectRole::EndEdge
                            )
                            .then_some(object.box_id)
                        }),
                        vertical_align: if is_atomic {
                            object
                                .map(|object| object.vertical_align)
                                .unwrap_or_default()
                        } else {
                            InlineVerticalAlign::default()
                        },
                        contributes_to_line: is_atomic,
                        creates_line: object.is_some_and(|object| match object.role {
                            InlineObjectRole::Atomic => true,
                            InlineObjectRole::StartEdge | InlineObjectRole::EndEdge => object_index
                                .and_then(|index| structural_edge_contributions.get(index))
                                .copied()
                                .unwrap_or(false),
                            InlineObjectRole::Float | InlineObjectRole::OutOfFlow => false,
                        }),
                        glyph_key: None,
                        anchor: LineVerticalAnchor::Root,
                        relative_offset: 0.0,
                    }
                }
            })
            .collect::<Vec<_>>();
        let phantom = css_line_is_phantom(
            line.break_reason(),
            geometries.iter().any(|geometry| geometry.creates_line),
        );
        let mut states = build_line_inline_box_states(context, line.text_range(), &geometries);
        let mut state_indices = BTreeMap::new();
        for (index, state) in states.iter().enumerate() {
            state_indices.insert(state.box_id.index(), index);
        }
        for state in &mut states {
            state.parent = state_indices.get(&state.parent_box.index()).copied();
            state.anchor = state
                .parent
                .map_or(LineVerticalAnchor::Root, LineVerticalAnchor::State);
        }
        for geometry in &mut geometries {
            geometry.anchor = geometry.edge_box.map_or_else(
                || {
                    state_indices
                        .get(&geometry.structural_parent.index())
                        .copied()
                        .map_or(LineVerticalAnchor::Root, LineVerticalAnchor::State)
                },
                |box_id| {
                    state_indices
                        .get(&box_id.index())
                        .copied()
                        .map_or(LineVerticalAnchor::Root, LineVerticalAnchor::State)
                },
            );
        }

        let fallback_root_bounds = InlineVerticalBounds {
            top: metrics.block_min_coord - metrics.baseline,
            bottom: metrics.block_max_coord - metrics.baseline,
        };
        let mut root_bounds = (!phantom).then(|| {
            context
                .parent_strut
                .map_or(fallback_root_bounds, InlineVerticalBounds::from_strut)
        });
        for state in &mut states {
            state.metrics = (!phantom)
                .then_some(state.strut)
                .flatten()
                .map(InlineVerticalBounds::from_strut);
        }

        // One pending list per structural target plus one for the root line
        // box. Top/bottom descendants are resolved only after the target's
        // other aligned descendants have established its subtree metrics.
        let root_pending_index = states.len();
        let mut pending = vec![Vec::<PendingLineAlignment>::new(); states.len() + 1];

        for (item_index, geometry) in geometries.iter_mut().enumerate() {
            if !geometry.contributes_to_line {
                continue;
            }
            let parent = match geometry.anchor {
                LineVerticalAnchor::State(index) => Some(index),
                LineVerticalAnchor::Root => None,
            };
            if matches!(
                geometry.vertical_align.kind,
                LayoutInlineAlignment::Top | LayoutInlineAlignment::Bottom
            ) {
                let target =
                    nearest_top_or_bottom_target(&states, parent).unwrap_or(root_pending_index);
                pending[target].push(PendingLineAlignment {
                    member: PendingLineMember::Item(item_index),
                    bounds: geometry.bounds,
                    vertical_align: geometry.vertical_align,
                });
                continue;
            }
            let offset = non_edge_vertical_offset(
                geometry.vertical_align,
                alignment_reference(context, &states, parent),
                geometry.bounds,
            );
            geometry.relative_offset = offset;
            include_in_parent(
                geometry.bounds.shifted(offset),
                parent,
                &mut states,
                &mut root_bounds,
            );
        }

        let mut state_order = (0..states.len()).collect::<Vec<_>>();
        state_order.sort_by_key(|index| std::cmp::Reverse(states[*index].depth));
        for state_index in state_order.iter().copied() {
            let target_pending = std::mem::take(&mut pending[state_index]);
            let mut target_metrics = states[state_index].metrics.take();
            resolve_pending_alignments(
                target_pending,
                LineVerticalAnchor::State(state_index),
                &mut target_metrics,
                &mut states,
                &mut geometries,
            );
            states[state_index].metrics = target_metrics;

            let Some(state_bounds) = states[state_index].metrics else {
                continue;
            };
            let parent = states[state_index].parent;
            let vertical_align = states[state_index].vertical_align;
            if matches!(
                vertical_align.kind,
                LayoutInlineAlignment::Top | LayoutInlineAlignment::Bottom
            ) {
                let target =
                    nearest_top_or_bottom_target(&states, parent).unwrap_or(root_pending_index);
                pending[target].push(PendingLineAlignment {
                    member: PendingLineMember::State(state_index),
                    bounds: state_bounds,
                    vertical_align,
                });
                continue;
            }
            let offset = non_edge_vertical_offset(
                vertical_align,
                alignment_reference(context, &states, parent),
                state_bounds,
            );
            states[state_index].relative_offset = offset;
            include_in_parent(
                state_bounds.shifted(offset),
                parent,
                &mut states,
                &mut root_bounds,
            );
        }

        resolve_pending_alignments(
            std::mem::take(&mut pending[root_pending_index]),
            LineVerticalAnchor::Root,
            &mut root_bounds,
            &mut states,
            &mut geometries,
        );

        let bounds = if phantom {
            InlineVerticalBounds::ZERO
        } else {
            root_bounds.unwrap_or(fallback_root_bounds)
        };
        let line_height = bounds.height();
        let root_baseline = raw_top + preceding_adjustment - bounds.top;

        let mut ascending_states = (0..states.len()).collect::<Vec<_>>();
        ascending_states.sort_by_key(|index| states[*index].depth);
        for state_index in ascending_states {
            states[state_index].global_offset = states[state_index].relative_offset
                + anchor_global_offset(states[state_index].anchor, &states);
        }
        let item_offsets = geometries
            .iter()
            .map(|geometry| {
                let desired_top = root_baseline
                    + anchor_global_offset(geometry.anchor, &states)
                    + geometry.relative_offset
                    + geometry.bounds.top;
                desired_top - geometry.initial_top
            })
            .collect::<Vec<_>>();
        let glyph_offsets = geometries
            .iter()
            .zip(&item_offsets)
            .filter_map(|(geometry, offset)| {
                let (run_index, style_index) = geometry.glyph_key?;
                Some(InlineGlyphOffset {
                    run_index,
                    style_index,
                    offset: *offset,
                })
            })
            .collect();
        let box_block_placements = states
            .iter()
            .filter_map(|state| {
                let strut = state.strut?;
                let baseline = root_baseline + state.global_offset;
                Some(InlineBoxBlockPlacement {
                    box_id: state.box_id,
                    top: baseline - strut.text_ascent,
                    height: (strut.text_ascent + strut.text_descent).max(0.0),
                })
            })
            .collect();
        placements.push(InlineLinePlacement {
            line_index,
            rect: FlowRelativeRect::new(
                metrics.inline_min_coord + metrics.offset,
                raw_top + preceding_adjustment,
                metrics.advance,
                line_height,
            ),
            baseline: root_baseline,
            phantom,
            content_offset: root_baseline - metrics.baseline,
            item_offsets,
            glyph_offsets,
            box_block_placements,
        });
        preceding_adjustment += line_height - (raw_bottom - raw_top);
        unadjusted_line_top += metrics.line_height.max(0.0);
    }

    (placements, preceding_adjustment)
}

#[derive(Clone, Copy, Debug)]
struct InlineItemVerticalGeometry {
    /// Line-height bounds relative to this item's own alignment baseline.
    bounds: InlineVerticalBounds,
    /// Parley's original block-start coordinate for converting the resolved
    /// baseline back into an item delta.
    initial_top: f32,
    structural_parent: LayoutBoxId,
    /// Structural edges track their own box baseline rather than their parent.
    edge_box: Option<LayoutBoxId>,
    vertical_align: InlineVerticalAlign,
    /// Whether this item supplies block-axis geometry to the line.
    contributes_to_line: bool,
    /// Whether this item prevents the line from being a CSS phantom line box.
    /// Structural inline edges with non-zero inline-axis decorations create a
    /// line without themselves affecting its block-axis height.
    creates_line: bool,
    glyph_key: Option<(usize, usize)>,
    anchor: LineVerticalAnchor,
    relative_offset: f32,
}

#[derive(Clone, Copy, Debug)]
struct LineInlineBoxState {
    box_id: LayoutBoxId,
    parent_box: LayoutBoxId,
    parent: Option<usize>,
    depth: usize,
    vertical_align: InlineVerticalAlign,
    strut: Option<InlineStrutMetrics>,
    metrics: Option<InlineVerticalBounds>,
    anchor: LineVerticalAnchor,
    relative_offset: f32,
    global_offset: f32,
}

#[derive(Clone, Copy, Debug)]
enum LineVerticalAnchor {
    Root,
    State(usize),
}

#[derive(Clone, Copy, Debug)]
enum PendingLineMember {
    State(usize),
    Item(usize),
}

#[derive(Clone, Copy, Debug)]
struct PendingLineAlignment {
    member: PendingLineMember,
    bounds: InlineVerticalBounds,
    vertical_align: InlineVerticalAlign,
}

#[derive(Clone, Copy, Debug)]
struct InlineVerticalBounds {
    top: f32,
    bottom: f32,
}

impl InlineVerticalBounds {
    const ZERO: Self = Self {
        top: 0.0,
        bottom: 0.0,
    };

    fn from_strut(strut: InlineStrutMetrics) -> Self {
        Self {
            top: -strut.line_ascent,
            bottom: strut.line_descent,
        }
    }

    fn shifted(self, offset: f32) -> Self {
        Self {
            top: self.top + offset,
            bottom: self.bottom + offset,
        }
    }

    fn height(self) -> f32 {
        (self.bottom - self.top).max(0.0)
    }

    fn include(&mut self, other: Self) {
        self.top = self.top.min(other.top);
        self.bottom = self.bottom.max(other.bottom);
    }
}

fn glyph_line_bounds(
    primary_strut: Option<InlineStrutMetrics>,
    used_font: &parley::layout::FontMetrics,
    used_line_height: f32,
    include_used_font_metrics: bool,
    font_baseline: FontBaseline,
) -> InlineVerticalBounds {
    let used_strut = inline_strut_metrics(
        InlineFontMetrics {
            ascent: used_font.ascent,
            descent: used_font.descent,
            line_height: used_line_height,
            x_height: used_font.x_height.unwrap_or(used_font.ascent * 0.56),
        },
        true,
        font_baseline,
    );
    let used_bounds = InlineVerticalBounds::from_strut(used_strut);
    let mut bounds = primary_strut.map_or(used_bounds, InlineVerticalBounds::from_strut);
    if include_used_font_metrics {
        bounds.include(used_bounds);
    }
    bounds
}

fn build_line_inline_box_states(
    context: &InlineFormattingContext,
    line_range: Range<usize>,
    geometries: &[InlineItemVerticalGeometry],
) -> Vec<LineInlineBoxState> {
    let mut present = std::collections::BTreeSet::new();
    for unit in &context.text_units {
        if ranges_overlap(&unit.output_range, &line_range) {
            for ancestor in &unit.ancestors {
                mark_structural_path(context, *ancestor, &mut present);
            }
        }
    }
    for geometry in geometries {
        mark_structural_path(context, geometry.structural_parent, &mut present);
        if let Some(box_id) = geometry.edge_box {
            mark_structural_path(context, box_id, &mut present);
        }
    }

    context
        .structural_boxes
        .iter()
        .filter(|state| present.contains(&state.box_id.index()))
        .map(|state| LineInlineBoxState {
            box_id: state.box_id,
            parent_box: state.parent,
            parent: None,
            depth: structural_box_depth(context, state.box_id),
            vertical_align: state.vertical_align,
            strut: state.strut,
            metrics: None,
            anchor: LineVerticalAnchor::Root,
            relative_offset: 0.0,
            global_offset: 0.0,
        })
        .collect()
}

fn mark_structural_path(
    context: &InlineFormattingContext,
    mut box_id: LayoutBoxId,
    present: &mut std::collections::BTreeSet<usize>,
) {
    while box_id != context.root_style {
        let Some(state) = context.structural_box(box_id) else {
            break;
        };
        present.insert(box_id.index());
        box_id = state.parent;
    }
}

fn structural_box_depth(context: &InlineFormattingContext, mut box_id: LayoutBoxId) -> usize {
    let mut depth = 0;
    while box_id != context.root_style {
        let Some(state) = context.structural_box(box_id) else {
            break;
        };
        depth += 1;
        box_id = state.parent;
    }
    depth
}

fn alignment_reference(
    context: &InlineFormattingContext,
    states: &[LineInlineBoxState],
    parent: Option<usize>,
) -> Option<InlineStrutMetrics> {
    parent
        .and_then(|index| states.get(index).and_then(|state| state.strut))
        .or_else(|| parent.is_none().then_some(context.parent_strut).flatten())
}

fn include_in_parent(
    bounds: InlineVerticalBounds,
    parent: Option<usize>,
    states: &mut [LineInlineBoxState],
    root_bounds: &mut Option<InlineVerticalBounds>,
) {
    let target = parent
        .and_then(|index| states.get_mut(index).map(|state| &mut state.metrics))
        .unwrap_or(root_bounds);
    match target {
        Some(metrics) => metrics.include(bounds),
        None => *target = Some(bounds),
    }
}

fn nearest_top_or_bottom_target(
    states: &[LineInlineBoxState],
    mut parent: Option<usize>,
) -> Option<usize> {
    while let Some(index) = parent {
        let state = &states[index];
        if matches!(
            state.vertical_align.kind,
            LayoutInlineAlignment::Top | LayoutInlineAlignment::Bottom
        ) {
            return Some(index);
        }
        parent = state.parent;
    }
    None
}

fn resolve_pending_alignments(
    pending: Vec<PendingLineAlignment>,
    target_anchor: LineVerticalAnchor,
    target_metrics: &mut Option<InlineVerticalBounds>,
    states: &mut [LineInlineBoxState],
    geometries: &mut [InlineItemVerticalGeometry],
) {
    if pending.is_empty() {
        return;
    }
    let aligned = target_metrics.unwrap_or(InlineVerticalBounds::ZERO);
    let mut maximum = aligned;
    for child in &pending {
        let height = child.bounds.height();
        if height <= maximum.height() {
            continue;
        }
        maximum = match child.vertical_align.kind {
            LayoutInlineAlignment::Top => InlineVerticalBounds {
                top: aligned.top,
                bottom: aligned.top + height,
            },
            LayoutInlineAlignment::Bottom => InlineVerticalBounds {
                top: aligned.bottom - height,
                bottom: aligned.bottom,
            },
            _ => maximum,
        };
    }
    for child in pending {
        let offset = match child.vertical_align.kind {
            LayoutInlineAlignment::Top => maximum.top - child.bounds.top,
            LayoutInlineAlignment::Bottom => maximum.bottom - child.bounds.bottom,
            _ => 0.0,
        } - child.vertical_align.baseline_shift;
        match child.member {
            PendingLineMember::State(index) => {
                states[index].anchor = target_anchor;
                states[index].relative_offset = offset;
            }
            PendingLineMember::Item(index) => {
                geometries[index].anchor = target_anchor;
                geometries[index].relative_offset = offset;
            }
        }
        let shifted = child.bounds.shifted(offset);
        match target_metrics {
            Some(metrics) => metrics.include(shifted),
            None => *target_metrics = Some(shifted),
        }
    }
}

fn anchor_global_offset(anchor: LineVerticalAnchor, states: &[LineInlineBoxState]) -> f32 {
    match anchor {
        LineVerticalAnchor::Root => 0.0,
        LineVerticalAnchor::State(index) => {
            states.get(index).map_or(0.0, |state| state.global_offset)
        }
    }
}

/// CSS line boxes ending in a preserved newline exist even when they contain
/// no paintable item. Parley's explicit break reason covers both preserved
/// segment breaks and the normalized `<br>` control.
fn css_line_is_phantom(break_reason: BreakReason, has_in_flow_content: bool) -> bool {
    !has_in_flow_content && break_reason != BreakReason::Explicit
}

fn non_edge_vertical_offset(
    vertical_align: InlineVerticalAlign,
    parent: Option<InlineStrutMetrics>,
    item: InlineVerticalBounds,
) -> f32 {
    let baseline_shift = -vertical_align.baseline_shift;
    let (parent_text_top, parent_text_bottom, parent_x_height) = parent
        .map_or((0.0, 0.0, 0.0), |strut| {
            (-strut.text_ascent, strut.text_descent, strut.x_height)
        });
    let alignment_shift = match vertical_align.kind {
        LayoutInlineAlignment::Baseline => 0.0,
        LayoutInlineAlignment::TextTop => parent_text_top - item.top,
        LayoutInlineAlignment::Middle => -parent_x_height * 0.5 - (item.top + item.bottom) * 0.5,
        LayoutInlineAlignment::TextBottom => parent_text_bottom - item.bottom,
        LayoutInlineAlignment::Top | LayoutInlineAlignment::Bottom => 0.0,
    };
    alignment_shift + baseline_shift
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SourceFragmentKey {
    box_index: usize,
    source_byte_start: usize,
    source_byte_end: usize,
    source_utf16_start: usize,
    source_utf16_end: usize,
    is_forced_line_break: bool,
    line_index: usize,
    rtl: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct FragmentAccumulator {
    min_inline: Option<f32>,
    min_block: Option<f32>,
    max_inline: Option<f32>,
    max_block: Option<f32>,
    has_start_edge: bool,
    has_end_edge: bool,
}

impl FragmentAccumulator {
    fn include(&mut self, rect: FlowRelativeRect) {
        self.include_inline_axis(rect.inline_offset, rect.inline_size);
        self.min_block = Some(
            self.min_block
                .map_or(rect.block_offset, |value| value.min(rect.block_offset)),
        );
        self.max_block = Some(
            self.max_block
                .map_or(rect.block_offset + rect.block_size, |value| {
                    value.max(rect.block_offset + rect.block_size)
                }),
        );
    }

    fn include_inline_axis(&mut self, offset: f32, size: f32) {
        self.min_inline = Some(self.min_inline.map_or(offset, |value| value.min(offset)));
        self.max_inline = Some(
            self.max_inline
                .map_or(offset + size, |value| value.max(offset + size)),
        );
    }

    fn include_block_axis(&mut self, offset: f32, size: f32) {
        self.min_block = Some(self.min_block.map_or(offset, |value| value.min(offset)));
        self.max_block = Some(
            self.max_block
                .map_or(offset + size, |value| value.max(offset + size)),
        );
    }

    fn rect(self, fallback_block_rect: FlowRelativeRect) -> Option<FlowRelativeRect> {
        let min_inline = self.min_inline?;
        let min_block = self.min_block.unwrap_or(fallback_block_rect.block_offset);
        let max_block = self
            .max_block
            .unwrap_or(fallback_block_rect.block_offset + fallback_block_rect.block_size);
        Some(FlowRelativeRect::new(
            min_inline,
            min_block,
            (self.max_inline? - min_inline).max(0.0),
            (max_block - min_block).max(0.0),
        ))
    }
}

fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

pub(crate) fn prepare_inline_contexts<N>(
    world: &mut LayoutWorld<N>,
    services: &mut DocumentLayoutServices,
) where
    N: Copy + Debug + Eq + Hash,
{
    services.begin_inline_layout_pass();
    for layout_box in &mut world.boxes {
        layout_box.inline_layout = None;
        layout_box.inline_context_owner = None;
        layout_box.inline_flattened = false;
    }

    let owners = (0..world.boxes.len())
        .map(LayoutBoxId::from_index)
        .filter(|id| world.boxes[id.index()].inline_formatting_context)
        .collect::<Vec<_>>();
    let mut initialized = false;
    for owner in owners {
        // A normal inline descendant is already flattened into the ancestor's
        // Parley tree. Atomic inline boxes still establish their own inner IFC.
        if world.boxes[owner.index()].inline_flattened {
            continue;
        }
        let input = collect_inline_input(world, owner);
        if input.units.is_empty() && input.objects.is_empty() {
            continue;
        }
        let parley = services.parley_mut();
        let context = input.build(world, parley);
        world.boxes[owner.index()].inline_layout = Some(context);
        initialized = true;
    }
    if initialized {
        services.text_layout_passes = services.text_layout_passes.saturating_add(1);
    }
}

struct InlineBuildInput {
    text: String,
    units: Vec<InlineTextUnit>,
    objects: Vec<(usize, InlineObject, InlineBoxKind)>,
    source_map: Vec<InlineSourceMapEntry>,
    root_style: LayoutBoxId,
}

struct ResolvedInlineTextStyle {
    text: TextStyle<'static, 'static, TextBrush>,
    structural_parent: LayoutBoxId,
    sample: Option<char>,
}

fn intern_resolved_inline_style(
    styles: &mut Vec<ResolvedInlineTextStyle>,
    style: TextStyle<'static, 'static, TextBrush>,
    structural_parent: LayoutBoxId,
    sample: Option<char>,
) -> usize {
    let style_slot = styles
        .iter()
        .position(|candidate| {
            candidate.text == style && candidate.structural_parent == structural_parent
        })
        .unwrap_or_else(|| {
            let index = styles.len();
            styles.push(ResolvedInlineTextStyle {
                text: style,
                structural_parent,
                sample: None,
            });
            index
        });
    if styles[style_slot].sample.is_none() {
        styles[style_slot].sample = sample;
    }
    style_slot
}

fn append_resolved_inline_run(
    runs: &mut Vec<(Range<usize>, usize)>,
    range: Range<usize>,
    style_slot: usize,
) {
    match runs.last_mut() {
        Some((previous_range, previous_slot))
            if *previous_slot == style_slot && previous_range.end == range.start =>
        {
            previous_range.end = range.end;
        }
        _ => runs.push((range, style_slot)),
    }
}

impl InlineBuildInput {
    fn build<N>(
        mut self,
        world: &LayoutWorld<N>,
        parley: &mut crate::text::ParleyDocumentServices,
    ) -> InlineFormattingContext
    where
        N: Copy + Debug + Eq + Hash,
    {
        let selection = project_inline_selection(world, &self.source_map);
        let mut root_text_style = world.boxes[self.root_style.index()]
            .style
            .parley_text_style();
        parley.resolve_font_families(&mut root_text_style, None);
        let quantize = true;
        let mut styles = Vec::new();
        let root_style_slot = intern_resolved_inline_style(
            &mut styles,
            root_text_style.clone(),
            self.root_style,
            None,
        );
        let mut resolved_runs = Vec::<(Range<usize>, usize)>::new();
        for unit in &self.units {
            let unit_style = &world.boxes[unit.style_box.index()].style;
            let mut base_style = unit_style.parley_text_style();
            // `vertical-align` belongs to the structural inline box, not to
            // each descendant glyph. Keep glyphs baseline-aligned within their
            // direct box state; closing that state moves the complete subtree.
            base_style.brush.paint = !unit.kind.is_control();
            let structural_parent = unit.ancestors.last().copied().unwrap_or(self.root_style);
            if !parley.requires_character_font_resolution(&base_style) {
                let sample = (!unit.kind.is_control())
                    .then(|| self.text[unit.output_range.clone()].chars().next())
                    .flatten();
                parley.resolve_font_families(&mut base_style, None);
                let style_slot = intern_resolved_inline_style(
                    &mut styles,
                    base_style,
                    structural_parent,
                    sample,
                );
                append_resolved_inline_run(
                    &mut resolved_runs,
                    unit.output_range.clone(),
                    style_slot,
                );
                continue;
            }
            for (relative_start, character) in self.text[unit.output_range.clone()].char_indices() {
                let start = unit.output_range.start + relative_start;
                let end = start + character.len_utf8();
                let mut style = base_style.clone();
                parley.resolve_font_families(&mut style, Some(character));
                let style_slot = intern_resolved_inline_style(
                    &mut styles,
                    style,
                    structural_parent,
                    (!unit.kind.is_control()).then_some(character),
                );
                append_resolved_inline_run(&mut resolved_runs, start..end, style_slot);
            }
        }
        let mut object_transition_style_slots = Vec::with_capacity(self.objects.len());
        for (_, object, _) in &self.objects {
            let style_box = match object.role {
                InlineObjectRole::StartEdge => Some(object.box_id),
                InlineObjectRole::EndEdge => {
                    Some(object.ancestors.last().copied().unwrap_or(self.root_style))
                }
                InlineObjectRole::Atomic
                | InlineObjectRole::Float
                | InlineObjectRole::OutOfFlow => None,
            };
            let Some(style_box) = style_box else {
                object_transition_style_slots.push(None);
                continue;
            };
            let mut style = world.boxes[style_box.index()].style.parley_text_style();
            parley.resolve_font_families(&mut style, None);
            let style_slot = intern_resolved_inline_style(&mut styles, style, style_box, None);
            object_transition_style_slots.push(Some(style_slot));
        }
        let mut builder = parley.layout_context.style_run_builder(
            &mut parley.font_context,
            &self.text,
            1.0,
            quantize,
        );
        // CSS fixes the paragraph base direction from the IFC owner's
        // inherited `direction`; first-strong inference is not equivalent for
        // empty, numeric, neutral, or object-only lines.
        builder.set_base_direction(
            match world.boxes[self.root_style.index()].style.direction() {
                InlineDirection::Ltr => parley::BaseDirection::Ltr,
                InlineDirection::Rtl => parley::BaseDirection::Rtl,
            },
        );
        builder.reserve(styles.len(), resolved_runs.len().max(1));
        let style_indices = styles
            .iter()
            .map(|style| builder.push_style(style.text.clone()))
            .collect::<Vec<_>>();
        builder.set_root_style(style_indices[root_style_slot]);
        if resolved_runs.is_empty() {
            builder.push_style_run(style_indices[root_style_slot], 0..0);
        } else {
            for (range, style_slot) in &resolved_runs {
                builder.push_style_run(style_indices[*style_slot], range.clone());
            }
        }
        for (object_id, ((byte_index, _, kind), transition_style_slot)) in self
            .objects
            .iter()
            .zip(&object_transition_style_slots)
            .enumerate()
        {
            let inline_box = InlineBox {
                id: u64::try_from(object_id).expect("one IFC exceeded the u64 object limit"),
                kind: *kind,
                index: *byte_index,
                width: 0.0,
                height: 0.0,
                baseline: None,
            };
            if let Some(style_slot) = transition_style_slot {
                builder
                    .push_inline_box_with_style_transition(inline_box, style_indices[*style_slot]);
            } else {
                builder.push_inline_box(inline_box);
            }
        }
        let layout = builder.build(&self.text);
        let font_metrics = styles
            .iter()
            .map(|style| parley.inline_font_metrics(&style.text, style.sample))
            .collect();
        let style_parents = styles.iter().map(|style| style.structural_parent).collect();
        let font_baseline = world.boxes[self.root_style.index()].style.font_baseline();
        let parent_strut =
            measure_inline_strut(parley, root_text_style.clone(), quantize, font_baseline);
        let mut structural_boxes = Vec::new();
        for (_, object, _) in &self.objects {
            if object.role != InlineObjectRole::StartEdge
                || structural_boxes
                    .iter()
                    .any(|state: &InlineStructuralBox| state.box_id == object.box_id)
            {
                continue;
            }
            let mut style = world.boxes[object.box_id.index()].style.parley_text_style();
            parley.resolve_font_families(&mut style, None);
            structural_boxes.push(InlineStructuralBox {
                box_id: object.box_id,
                parent: object.ancestors.last().copied().unwrap_or(self.root_style),
                vertical_align: object.vertical_align,
                strut: measure_inline_strut(parley, style, quantize, font_baseline),
                include_used_font_metrics: world.boxes[object.box_id.index()]
                    .style
                    .includes_used_font_metrics(),
            });
        }
        let objects = self
            .objects
            .drain(..)
            .map(|(_, object, _)| object)
            .collect();
        InlineFormattingContext {
            root_style: self.root_style,
            quantize,
            font_baseline,
            unbroken: layout,
            laid_out: None,
            text_units: self.units,
            source_map: self.source_map,
            selection,
            objects,
            font_metrics,
            parent_strut,
            root_includes_used_font_metrics: world.boxes[self.root_style.index()]
                .style
                .includes_used_font_metrics(),
            style_parents,
            structural_boxes,
            line_placements: Vec::new(),
            fragments: InlineFragments::default(),
        }
    }
}

fn measure_inline_strut(
    parley: &mut crate::text::ParleyDocumentServices,
    style: TextStyle<'static, 'static, TextBrush>,
    quantize: bool,
    font_baseline: FontBaseline,
) -> Option<InlineStrutMetrics> {
    let metrics = parley.inline_font_metrics(&style, None)?;
    Some(inline_strut_metrics(metrics, quantize, font_baseline))
}

fn inline_strut_metrics(
    metrics: InlineFontMetrics,
    quantize: bool,
    font_baseline: FontBaseline,
) -> InlineStrutMetrics {
    let (ascent, descent, leading_above, leading_below) = if quantize {
        let ascent = metrics.ascent.round();
        let descent = metrics.descent.round();
        let leading = metrics.line_height - ascent - descent;
        let leading_above = (leading * 0.5).floor();
        let leading_below = leading.round() - leading_above;
        (ascent, descent, leading_above, leading_below)
    } else {
        let half_leading = (metrics.line_height - metrics.ascent - metrics.descent) * 0.5;
        (metrics.ascent, metrics.descent, half_leading, half_leading)
    };
    let (ascent, descent) =
        inline_font_ascent_and_descent(font_baseline, ascent, descent, quantize);
    InlineStrutMetrics {
        line_ascent: ascent + leading_above,
        line_descent: descent + leading_below,
        text_ascent: ascent,
        text_descent: descent,
        x_height: metrics.x_height,
    }
}

/// Express Parley's alphabetic font metrics relative to the IFC baseline.
///
/// CSS vertical text uses the ideographic central baseline, whose ascent and
/// descent split the font height in half. Blink performs this conversion in
/// `FontMetrics::GetFontHeight(FontBaseline)` before constructing line
/// struts. Doing the same at our Parley-to-CSS line-metrics boundary keeps
/// line layout, fragment geometry, and exported baselines on one protocol.
fn inline_font_ascent_and_descent(
    font_baseline: FontBaseline,
    alphabetic_ascent: f32,
    alphabetic_descent: f32,
    quantize: bool,
) -> (f32, f32) {
    match font_baseline {
        FontBaseline::Alphabetic => (alphabetic_ascent, alphabetic_descent),
        FontBaseline::Central => {
            let height = alphabetic_ascent + alphabetic_descent;
            let descent = if quantize {
                (height * 0.5).floor()
            } else {
                height * 0.5
            };
            (height - descent, descent)
        }
    }
}

/// Distance from the line-over edge to a synthesized font baseline.
///
/// This is the IFC-side counterpart of Taffy's synthesized flex/grid
/// baseline. Keeping both consumers on `FontBaseline` prevents atomic inline
/// and container alignment from choosing different fallback baselines.
pub(crate) const fn synthesized_font_ascent(font_baseline: FontBaseline, block_size: f32) -> f32 {
    match font_baseline {
        FontBaseline::Alphabetic => block_size,
        FontBaseline::Central => block_size * 0.5,
    }
}

fn project_inline_selection<N>(
    world: &LayoutWorld<N>,
    source_map: &[InlineSourceMapEntry],
) -> Option<InlineSelection>
where
    N: Copy + Debug + Eq + Hash,
{
    let mut selected_start = None::<usize>;
    let mut selected_end = None::<usize>;
    let mut caret = None::<(usize, PaintColor)>;

    for entry in source_map {
        let Some(selection) = world.boxes[entry.box_id.index()].text_selection else {
            continue;
        };
        if selection.is_caret() {
            if caret.is_none() {
                caret = caret_output_offset(source_map, entry.box_id, selection.start)
                    .map(|offset| (offset, world.boxes[entry.box_id.index()].style.text_color()));
            }
            continue;
        }
        let selected = selection.start.min(selection.end)..selection.start.max(selection.end);
        if !ranges_overlap(&entry.source_utf16_range, &selected) {
            continue;
        }
        selected_start = Some(selected_start.map_or(entry.output_range.start, |start| {
            start.min(entry.output_range.start)
        }));
        selected_end = Some(selected_end.map_or(entry.output_range.end, |end| {
            end.max(entry.output_range.end)
        }));
    }

    match (selected_start, selected_end) {
        (Some(start), Some(end)) if start < end => Some(InlineSelection::Range(start..end)),
        _ => caret.map(|(offset, color)| InlineSelection::Caret { offset, color }),
    }
}

fn caret_output_offset(
    source_map: &[InlineSourceMapEntry],
    box_id: LayoutBoxId,
    utf16_offset: usize,
) -> Option<usize> {
    let entries = source_map
        .iter()
        .filter(|entry| entry.box_id == box_id)
        .collect::<Vec<_>>();
    let first = entries.first()?;
    if utf16_offset <= first.source_utf16_range.start {
        return Some(first.output_range.start);
    }
    for entry in &entries {
        if utf16_offset < entry.source_utf16_range.end {
            return Some(entry.output_range.start);
        }
        if utf16_offset == entry.source_utf16_range.end {
            return Some(entry.output_range.end);
        }
    }
    entries.last().map(|entry| entry.output_range.end)
}

fn collect_inline_input<N>(world: &mut LayoutWorld<N>, owner: LayoutBoxId) -> InlineBuildInput
where
    N: Copy + Debug + Eq + Hash,
{
    let mut normalizer = InlineNormalizer::new(owner);
    let children = world.boxes[owner.index()].children.clone();
    for child in children {
        collect_box(world, owner, child, &mut Vec::new(), &mut normalizer);
    }
    normalizer.finish()
}

fn collect_box<N>(
    world: &mut LayoutWorld<N>,
    owner: LayoutBoxId,
    id: LayoutBoxId,
    ancestors: &mut Vec<LayoutBoxId>,
    normalizer: &mut InlineNormalizer,
) where
    N: Copy + Debug + Eq + Hash,
{
    let kind = world.boxes[id.index()].kind;
    let display = world.boxes[id.index()].style.display();
    if kind == LayoutBoxKind::PseudoMarker && world.boxes[id.index()].outside_list_marker {
        return;
    }
    world.boxes[id.index()].inline_context_owner = Some(owner);

    if kind == LayoutBoxKind::Text {
        world.boxes[id.index()].inline_flattened = true;
        let text = world.boxes[id.index()].text.clone().unwrap_or_default();
        normalizer.push_text(
            id,
            &text,
            world.boxes[id.index()].style.white_space_collapse(),
            world.boxes[id.index()].style.text_transform(),
            ancestors,
        );
        return;
    }
    if kind == LayoutBoxKind::LineBreak {
        world.boxes[id.index()].inline_flattened = true;
        normalizer.hard_break(id, ancestors);
        return;
    }

    if world.boxes[id.index()].style.is_floated() {
        normalizer.push_object(
            id,
            InlineObjectRole::Float,
            InlineBoxKind::CustomOutOfFlow,
            ancestors,
            world.boxes[id.index()].style.vertical_align(),
        );
        return;
    }

    let out_of_flow = world.boxes[id.index()].style.is_out_of_flow();
    if out_of_flow {
        normalizer.push_object(
            id,
            InlineObjectRole::OutOfFlow,
            InlineBoxKind::OutOfFlow,
            ancestors,
            world.boxes[id.index()].style.vertical_align(),
        );
        return;
    }

    let structural_inline = display.is_inline_flow()
        && !matches!(
            kind,
            LayoutBoxKind::Replaced
                | LayoutBoxKind::FormControl
                | LayoutBoxKind::ImageFallback
                | LayoutBoxKind::InlineTableWrapper
        );
    if !structural_inline {
        normalizer.push_object(
            id,
            InlineObjectRole::Atomic,
            InlineBoxKind::InFlow,
            ancestors,
            world.boxes[id.index()].style.vertical_align(),
        );
        return;
    }

    world.boxes[id.index()].inline_flattened = true;
    let vertical_align = world.boxes[id.index()].style.vertical_align();
    normalizer.open_inline(
        id,
        world.boxes[id.index()].style.unicode_bidi(),
        world.boxes[id.index()].style.direction(),
        ancestors,
        vertical_align,
    );
    ancestors.push(id);
    let children = world.boxes[id.index()].children.clone();
    for child in children {
        collect_box(world, owner, child, ancestors, normalizer);
    }
    ancestors.pop();
    normalizer.close_inline(
        id,
        world.boxes[id.index()].style.unicode_bidi(),
        ancestors,
        vertical_align,
    );
}

struct PendingWhitespace {
    output_index: usize,
    unit_index: usize,
    object_index: usize,
    style_box: LayoutBoxId,
    ancestors: Vec<LayoutBoxId>,
    sources: Vec<SourceOrigin>,
    contains_segment_break: bool,
}

struct PendingCarriageReturn {
    style_box: LayoutBoxId,
    mode: InlineWhiteSpaceCollapse,
    ancestors: Vec<LayoutBoxId>,
    origin: SourceOrigin,
}

struct InlineNormalizer {
    root_style: LayoutBoxId,
    text: String,
    units: Vec<InlineTextUnit>,
    objects: Vec<(usize, InlineObject, InlineBoxKind)>,
    pending: Option<PendingWhitespace>,
    pending_carriage_return: Option<PendingCarriageReturn>,
    line_has_content: bool,
    capitalize_word_start: bool,
}

impl InlineNormalizer {
    fn new(root_style: LayoutBoxId) -> Self {
        Self {
            root_style,
            text: String::new(),
            units: Vec::new(),
            objects: Vec::new(),
            pending: None,
            pending_carriage_return: None,
            line_has_content: false,
            capitalize_word_start: true,
        }
    }

    fn push_text(
        &mut self,
        box_id: LayoutBoxId,
        text: &str,
        mode: InlineWhiteSpaceCollapse,
        transform: InlineTextTransform,
        ancestors: &[LayoutBoxId],
    ) {
        let mut utf16_offset = 0;
        let mut characters = text.char_indices().peekable();
        if let Some(pending) = self.pending_carriage_return.take() {
            if let Some(&(byte_offset, '\n')) = characters.peek() {
                characters.next();
                let utf16_end = '\n'.len_utf16();
                self.push_character(
                    pending.style_box,
                    '\n',
                    pending.mode,
                    &pending.ancestors,
                    vec![
                        pending.origin,
                        SourceOrigin {
                            box_id,
                            byte_range: byte_offset..byte_offset + '\n'.len_utf8(),
                            utf16_range: 0..utf16_end,
                        },
                    ],
                );
                utf16_offset = utf16_end;
            } else {
                self.push_character(
                    pending.style_box,
                    '\n',
                    pending.mode,
                    &pending.ancestors,
                    vec![pending.origin],
                );
            }
        }
        while let Some((byte_offset, source_char)) = characters.next() {
            let byte_end = byte_offset + source_char.len_utf8();
            let utf16_end = utf16_offset + source_char.len_utf16();
            let origin = SourceOrigin {
                box_id,
                byte_range: byte_offset..byte_end,
                utf16_range: utf16_offset..utf16_end,
            };
            utf16_offset = utf16_end;
            if source_char == '\r' {
                if let Some(&(next_byte, '\n')) = characters.peek() {
                    characters.next();
                    let lf_utf16_end = utf16_offset + '\n'.len_utf16();
                    self.push_character(
                        box_id,
                        '\n',
                        mode,
                        ancestors,
                        vec![
                            origin,
                            SourceOrigin {
                                box_id,
                                byte_range: next_byte..next_byte + '\n'.len_utf8(),
                                utf16_range: utf16_offset..lf_utf16_end,
                            },
                        ],
                    );
                    utf16_offset = lf_utf16_end;
                    continue;
                }
                if characters.peek().is_none() {
                    self.pending_carriage_return = Some(PendingCarriageReturn {
                        style_box: box_id,
                        mode,
                        ancestors: ancestors.to_vec(),
                        origin,
                    });
                    break;
                }
            }
            let transformed = self.transform_char(source_char, transform);
            for character in transformed {
                self.push_character(box_id, character, mode, ancestors, vec![origin.clone()]);
            }
        }
    }

    fn transform_char(&mut self, character: char, transform: InlineTextTransform) -> Vec<char> {
        let transformed = match transform {
            InlineTextTransform::None => vec![character],
            InlineTextTransform::Uppercase => character.to_uppercase().collect(),
            InlineTextTransform::Lowercase => character.to_lowercase().collect(),
            InlineTextTransform::Capitalize
                if self.capitalize_word_start && character.is_alphabetic() =>
            {
                character.to_uppercase().collect()
            }
            InlineTextTransform::Capitalize => vec![character],
        };
        if character.is_alphanumeric() {
            self.capitalize_word_start = false;
        } else if !is_combining_mark(character) {
            self.capitalize_word_start = true;
        }
        transformed
    }

    fn push_character(
        &mut self,
        style_box: LayoutBoxId,
        character: char,
        mode: InlineWhiteSpaceCollapse,
        ancestors: &[LayoutBoxId],
        sources: Vec<SourceOrigin>,
    ) {
        let is_segment_break = matches!(character, '\n' | '\r' | '\u{000C}');
        let collapsible = character == ' ' || character == '\t' || is_segment_break;
        match mode {
            InlineWhiteSpaceCollapse::Collapse if collapsible => {
                self.queue_whitespace(style_box, ancestors, sources, is_segment_break);
            }
            InlineWhiteSpaceCollapse::PreserveBreaks if is_segment_break => {
                self.pending = None;
                self.append_unit(
                    style_box,
                    '\n',
                    ancestors,
                    sources,
                    InlineTextUnitKind::ForcedLineBreak { element_box: None },
                );
                self.line_has_content = false;
            }
            InlineWhiteSpaceCollapse::PreserveBreaks if collapsible => {
                self.queue_whitespace(style_box, ancestors, sources, false);
            }
            InlineWhiteSpaceCollapse::Preserve | InlineWhiteSpaceCollapse::BreakSpaces => {
                self.flush_pending();
                let character = if matches!(character, '\r' | '\u{000C}') {
                    '\n'
                } else {
                    character
                };
                self.append_unit(
                    style_box,
                    character,
                    ancestors,
                    sources,
                    if character == '\n' {
                        InlineTextUnitKind::ForcedLineBreak { element_box: None }
                    } else {
                        InlineTextUnitKind::Text
                    },
                );
                self.line_has_content = character != '\n';
            }
            InlineWhiteSpaceCollapse::Collapse | InlineWhiteSpaceCollapse::PreserveBreaks => {
                self.flush_pending();
                self.append_unit(
                    style_box,
                    character,
                    ancestors,
                    sources,
                    InlineTextUnitKind::Text,
                );
                self.line_has_content = true;
            }
        }
    }

    fn queue_whitespace(
        &mut self,
        style_box: LayoutBoxId,
        ancestors: &[LayoutBoxId],
        sources: Vec<SourceOrigin>,
        segment_break: bool,
    ) {
        let pending = self.pending.get_or_insert_with(|| PendingWhitespace {
            output_index: self.text.len(),
            unit_index: self.units.len(),
            object_index: self.objects.len(),
            style_box,
            ancestors: ancestors.to_vec(),
            sources: Vec::new(),
            contains_segment_break: false,
        });
        pending.sources.extend(sources);
        pending.contains_segment_break |= segment_break;
    }

    fn flush_pending(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if !self.line_has_content {
            return;
        }

        // Inline boundaries and bidi controls can be collected while a
        // collapsible space is still pending. If the space survives, it
        // precedes all of those later items in DOM order. Insert it at the
        // point where the collapsible sequence began instead of appending it
        // after the deferred boundaries.
        self.text.insert(pending.output_index, ' ');
        for unit in &mut self.units[pending.unit_index..] {
            unit.output_range.start += 1;
            unit.output_range.end += 1;
        }
        for (byte_index, _, _) in &mut self.objects[pending.object_index..] {
            *byte_index += 1;
        }
        self.units.insert(
            pending.unit_index,
            InlineTextUnit {
                output_range: pending.output_index..pending.output_index + 1,
                style_box: pending.style_box,
                ancestors: pending.ancestors,
                sources: pending.sources,
                kind: InlineTextUnitKind::Text,
            },
        );
    }

    fn hard_break(&mut self, box_id: LayoutBoxId, ancestors: &[LayoutBoxId]) {
        self.flush_pending_carriage_return();
        self.pending = None;
        // Blink's forced-break item belongs to LayoutBR for geometry, but its
        // text height comes from the current InlineBoxState. A style authored
        // directly on `<br>` therefore does not replace the enclosing strut.
        let style_box = ancestors.last().copied().unwrap_or(self.root_style);
        self.append_unit(
            style_box,
            '\n',
            ancestors,
            Vec::new(),
            InlineTextUnitKind::ForcedLineBreak {
                element_box: Some(box_id),
            },
        );
        self.line_has_content = false;
        self.capitalize_word_start = true;
    }

    fn open_inline(
        &mut self,
        box_id: LayoutBoxId,
        bidi: InlineUnicodeBidi,
        direction: InlineDirection,
        ancestors: &[LayoutBoxId],
        vertical_align: InlineVerticalAlign,
    ) {
        // CSS Writing Modes injects the opening bidi controls outside the
        // inline box boundary. Keep the opaque item order aligned with
        // Blink's InlineItemsBuilder: enter bidi context, then open the tag.
        for control in bidi_open(bidi, direction) {
            self.append_unit(
                box_id,
                control,
                ancestors,
                Vec::new(),
                InlineTextUnitKind::Control,
            );
        }
        self.push_object(
            box_id,
            InlineObjectRole::StartEdge,
            InlineBoxKind::InlineStart,
            ancestors,
            vertical_align,
        );
    }

    fn close_inline(
        &mut self,
        box_id: LayoutBoxId,
        bidi: InlineUnicodeBidi,
        ancestors: &[LayoutBoxId],
        vertical_align: InlineVerticalAlign,
    ) {
        // Close the inline box before leaving its injected bidi context.
        self.push_object(
            box_id,
            InlineObjectRole::EndEdge,
            InlineBoxKind::InlineEnd,
            ancestors,
            vertical_align,
        );
        for control in bidi_close(bidi) {
            self.append_unit(
                box_id,
                control,
                ancestors,
                Vec::new(),
                InlineTextUnitKind::Control,
            );
        }
    }

    fn push_object(
        &mut self,
        box_id: LayoutBoxId,
        role: InlineObjectRole,
        kind: InlineBoxKind,
        ancestors: &[LayoutBoxId],
        vertical_align: InlineVerticalAlign,
    ) {
        self.flush_pending_carriage_return();
        if matches!(
            role,
            InlineObjectRole::Atomic | InlineObjectRole::Float | InlineObjectRole::OutOfFlow
        ) {
            self.flush_pending();
            self.line_has_content = true;
        }
        self.objects.push((
            self.text.len(),
            InlineObject {
                box_id,
                role,
                ancestors: ancestors.to_vec(),
                vertical_align,
            },
            kind,
        ));
    }

    fn append_unit(
        &mut self,
        style_box: LayoutBoxId,
        character: char,
        ancestors: &[LayoutBoxId],
        sources: Vec<SourceOrigin>,
        kind: InlineTextUnitKind,
    ) {
        let start = self.text.len();
        self.text.push(character);
        self.units.push(InlineTextUnit {
            output_range: start..self.text.len(),
            style_box,
            ancestors: ancestors.to_vec(),
            sources,
            kind,
        });
    }

    fn finish(mut self) -> InlineBuildInput {
        self.flush_pending_carriage_return();
        // Pending collapsed whitespace at the end of an IFC is discarded.
        self.pending = None;
        let source_map = self
            .units
            .iter()
            .flat_map(|unit| {
                unit.sources.iter().map(|source| InlineSourceMapEntry {
                    output_range: unit.output_range.clone(),
                    box_id: source.box_id,
                    source_byte_range: source.byte_range.clone(),
                    source_utf16_range: source.utf16_range.clone(),
                    is_forced_line_break: unit.kind.is_forced_line_break(),
                })
            })
            .collect();
        InlineBuildInput {
            text: self.text,
            units: self.units,
            objects: self.objects,
            source_map,
            root_style: self.root_style,
        }
    }

    fn flush_pending_carriage_return(&mut self) {
        let Some(pending) = self.pending_carriage_return.take() else {
            return;
        };
        self.push_character(
            pending.style_box,
            '\n',
            pending.mode,
            &pending.ancestors,
            vec![pending.origin],
        );
    }
}

fn bidi_open(bidi: InlineUnicodeBidi, direction: InlineDirection) -> Vec<char> {
    let (embed, override_control, isolate) = match direction {
        InlineDirection::Ltr => ('\u{202A}', '\u{202D}', '\u{2066}'),
        InlineDirection::Rtl => ('\u{202B}', '\u{202E}', '\u{2067}'),
    };
    match bidi {
        InlineUnicodeBidi::Normal => Vec::new(),
        InlineUnicodeBidi::Embed => vec![embed],
        InlineUnicodeBidi::Isolate => vec![isolate],
        InlineUnicodeBidi::BidiOverride => vec![override_control],
        InlineUnicodeBidi::IsolateOverride => vec![isolate, override_control],
        InlineUnicodeBidi::Plaintext => vec!['\u{2068}'],
    }
}

fn bidi_close(bidi: InlineUnicodeBidi) -> Vec<char> {
    match bidi {
        InlineUnicodeBidi::Normal => Vec::new(),
        InlineUnicodeBidi::Embed | InlineUnicodeBidi::BidiOverride => vec!['\u{202C}'],
        InlineUnicodeBidi::Isolate | InlineUnicodeBidi::Plaintext => vec!['\u{2069}'],
        InlineUnicodeBidi::IsolateOverride => vec!['\u{202C}', '\u{2069}'],
    }
}

fn is_combining_mark(character: char) -> bool {
    matches!(character as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hanging_whitespace_shrinks_the_used_line_box_at_the_trailing_edge() {
        let full = FlowRelativeRect::new(10.0, 20.0, 30.0, 12.0);
        assert_eq!(
            line_box_rect_without_hanging(full, 6.0, false),
            (FlowRelativeRect::new(10.0, 20.0, 24.0, 12.0), true)
        );
        assert_eq!(
            line_box_rect_without_hanging(full, 6.0, true),
            (FlowRelativeRect::new(16.0, 20.0, 24.0, 12.0), true)
        );
        assert_eq!(
            line_box_rect_without_hanging(full, 0.0, false),
            (full, false)
        );
    }

    #[test]
    fn hanging_children_stay_relative_to_the_used_line_fragment() {
        let full_line = FlowRelativeRect::new(10.0, 20.0, 30.0, 12.0);
        let child = FlowRelativeRect::new(11.0, 22.0, 8.0, 6.0);
        let (used_line, has_hanging) = line_box_rect_without_hanging(full_line, 6.0, true);
        assert!(has_hanging);

        let full_relative = full_line.line_relative_rect(child);
        let used_relative = used_line.line_relative_rect(child);
        assert_eq!(used_relative.inline_offset, -5.0);

        let outer = Size {
            width: 150.0,
            height: 225.0,
        };
        for writing_mode in [
            WritingMode::HorizontalTb,
            WritingMode::VerticalLr,
            WritingMode::VerticalRl,
        ] {
            let coordinates = InlineCoordinateSpace::new(writing_mode);
            assert_eq!(
                coordinates.to_physical_line_rect(used_line, used_relative, outer),
                coordinates.to_physical_line_rect(full_line, full_relative, outer),
                "{writing_mode:?}",
            );
        }
    }

    #[test]
    fn hanging_line_clips_scrollable_overflow_only_on_its_inline_axis() {
        let horizontal = InlineLineFragment {
            line_index: 0,
            used_rect: PaintRect::new(10.0, 20.0, 30.0, 12.0),
            has_hanging: true,
            inline_axis: LayoutPhysicalAxis::Horizontal,
        };
        assert_eq!(
            horizontal.adjust_scrollable_overflow(PaintRect::new(5.0, 15.0, 40.0, 22.0)),
            PaintRect::new(10.0, 15.0, 30.0, 22.0)
        );
        assert_eq!(
            horizontal.adjust_scrollable_overflow(PaintRect::new(5.0, 15.0, 20.0, 22.0)),
            PaintRect::new(10.0, 15.0, 20.0, 22.0),
            "Blink moves a start-side hanging overflow rect without shrinking it",
        );

        let vertical = InlineLineFragment {
            line_index: 0,
            used_rect: PaintRect::new(10.0, 20.0, 12.0, 30.0),
            has_hanging: true,
            inline_axis: LayoutPhysicalAxis::Vertical,
        };
        assert_eq!(
            vertical.adjust_scrollable_overflow(PaintRect::new(5.0, 15.0, 22.0, 40.0)),
            PaintRect::new(5.0, 20.0, 22.0, 30.0)
        );
        assert_eq!(
            vertical.adjust_scrollable_overflow(PaintRect::new(5.0, 15.0, 22.0, 20.0)),
            PaintRect::new(5.0, 20.0, 22.0, 20.0),
        );
    }

    #[test]
    fn line_relative_rects_cross_the_physical_boundary_once() {
        let line = FlowRelativeRect::new(20.0, 10.0, 30.0, 20.0);
        let child = LineRelativeRect::new(2.0, 0.0, 10.0, 5.0);
        let outer = Size {
            width: 150.0,
            height: 225.0,
        };
        assert_eq!(
            InlineCoordinateSpace::new(WritingMode::HorizontalTb)
                .to_physical_line_rect(line, child, outer),
            PaintRect::new(22.0, 10.0, 10.0, 5.0),
        );
        assert_eq!(
            InlineCoordinateSpace::new(WritingMode::VerticalLr)
                .to_physical_line_rect(line, child, outer),
            PaintRect::new(25.0, 22.0, 5.0, 10.0),
        );
        assert_eq!(
            InlineCoordinateSpace::new(WritingMode::VerticalRl)
                .to_physical_line_rect(line, child, outer),
            PaintRect::new(135.0, 22.0, 5.0, 10.0),
        );
    }

    #[test]
    fn vertical_lr_baseline_uses_line_over_instead_of_block_start() {
        let line = InlineLinePlacement {
            line_index: 0,
            rect: FlowRelativeRect::new(0.0, 10.0, 30.0, 20.0),
            baseline: 15.0,
            phantom: false,
            content_offset: 0.0,
            item_offsets: Vec::new(),
            glyph_offsets: Vec::new(),
            box_block_placements: Vec::new(),
        };
        assert_eq!(
            InlineCoordinateSpace::new(WritingMode::VerticalLr).to_physical_line_baseline(
                &line,
                Size {
                    width: 150.0,
                    height: 225.0,
                },
            ),
            Point {
                x: Some(25.0),
                y: None,
            }
        );
    }

    #[test]
    fn vertical_line_baselines_round_trip_from_line_over() {
        let size = Size {
            width: 150.0,
            height: 225.0,
        };
        for writing_mode in [WritingMode::VerticalLr, WritingMode::VerticalRl] {
            let coordinates = InlineCoordinateSpace::new(writing_mode);
            let physical = coordinates.to_physical_line_block_baseline(Some(10.0), size);
            assert_eq!(
                physical,
                Point {
                    x: Some(140.0),
                    y: None
                }
            );
            assert_eq!(
                coordinates.to_line_block_baseline(physical, size),
                Some(10.0),
            );
        }
    }

    #[test]
    fn vertical_inline_fragments_share_one_physical_box_model() {
        let fragment = InlineBoxFragment {
            line_index: 0,
            box_id: LayoutBoxId::from_index(0),
            rect: PaintRect::new(50.0, 20.0, 20.0, 100.0),
            has_start_edge: true,
            has_end_edge: true,
        };
        let geometry = inline_fragment_box_geometry(
            &fragment,
            WritingDirection::new(WritingMode::VerticalRl, Direction::Ltr),
            Rect {
                left: 1.0,
                right: 2.0,
                top: 3.0,
                bottom: 4.0,
            },
            Rect {
                left: 5.0,
                right: 6.0,
                top: 7.0,
                bottom: 8.0,
            },
            Rect {
                left: 9.0,
                right: 10.0,
                top: 11.0,
                bottom: 12.0,
            },
        );

        assert_eq!(geometry.border_rect, PaintRect::new(36.0, 23.0, 50.0, 93.0));
        assert_eq!(
            geometry.padding_rect,
            PaintRect::new(45.0, 34.0, 31.0, 70.0)
        );
        assert_eq!(
            geometry.content_rect,
            PaintRect::new(50.0, 41.0, 20.0, 55.0)
        );
        assert_eq!(
            geometry.margin_rect,
            PaintRect::new(35.0, 20.0, 53.0, 100.0)
        );
        assert_eq!(
            geometry.border_widths,
            PaintEdgeSizes::new(11.0, 10.0, 12.0, 9.0)
        );
        assert_eq!(
            geometry.padding_widths,
            PaintEdgeSizes::new(7.0, 6.0, 8.0, 5.0)
        );
    }

    #[test]
    fn inline_fragment_box_model_preserves_signed_margins() {
        let fragment = InlineBoxFragment {
            line_index: 0,
            box_id: LayoutBoxId::from_index(0),
            rect: PaintRect::new(10.0, 20.0, 40.0, 20.0),
            has_start_edge: true,
            has_end_edge: true,
        };
        let geometry = inline_fragment_box_geometry(
            &fragment,
            WritingDirection::new(WritingMode::HorizontalTb, Direction::Ltr),
            Rect {
                left: -5.0,
                right: -3.0,
                top: -2.0,
                bottom: -4.0,
            },
            Rect::zero(),
            Rect::zero(),
        );

        // The fragment's inline span is the margin span. Negative margins
        // make the border box extend beyond it, while negative block margins
        // shrink the physical margin box toward the border box.
        assert_eq!(geometry.border_rect, PaintRect::new(5.0, 20.0, 48.0, 20.0));
        assert_eq!(geometry.margin_rect, PaintRect::new(10.0, 22.0, 40.0, 14.0));
    }

    #[test]
    fn relative_insets_follow_both_logical_axes() {
        let inset = Rect {
            left: Some(10.0),
            right: Some(20.0),
            top: Some(30.0),
            bottom: Some(40.0),
        };
        assert_eq!(
            flow_relative_inset_offset(
                inset,
                WritingDirection::new(WritingMode::HorizontalTb, Direction::Rtl),
            ),
            Point { x: -20.0, y: 30.0 },
        );
        assert_eq!(
            flow_relative_inset_offset(
                inset,
                WritingDirection::new(WritingMode::VerticalRl, Direction::Ltr),
            ),
            Point { x: -20.0, y: 30.0 },
        );
        assert_eq!(
            flow_relative_inset_offset(
                inset,
                WritingDirection::new(WritingMode::VerticalRl, Direction::Rtl),
            ),
            Point { x: -20.0, y: -40.0 },
        );
    }

    #[test]
    fn normal_line_height_unites_metrics_from_the_shaped_fallback_font() {
        let primary = InlineStrutMetrics {
            line_ascent: 8.0,
            line_descent: 2.0,
            text_ascent: 8.0,
            text_descent: 2.0,
            x_height: 4.0,
        };
        let fallback = parley::layout::FontMetrics {
            ascent: 18.0,
            descent: 6.0,
            leading: 6.0,
            underline_offset: 0.0,
            underline_size: 0.0,
            strikethrough_offset: 0.0,
            strikethrough_size: 0.0,
            cap_height: None,
            x_height: None,
        };

        let explicit = glyph_line_bounds(
            Some(primary),
            &fallback,
            30.0,
            false,
            FontBaseline::Alphabetic,
        );
        assert_eq!(explicit.top, -8.0);
        assert_eq!(explicit.bottom, 2.0);

        let normal = glyph_line_bounds(
            Some(primary),
            &fallback,
            30.0,
            true,
            FontBaseline::Alphabetic,
        );
        assert_eq!(normal.top, -21.0);
        assert_eq!(normal.bottom, 9.0);
    }

    #[test]
    fn central_inline_struts_split_font_and_line_heights_at_the_baseline() {
        let strut = inline_strut_metrics(
            InlineFontMetrics {
                ascent: 8.0,
                descent: 3.0,
                line_height: 21.0,
                x_height: 4.0,
            },
            true,
            FontBaseline::Central,
        );

        // Blink's integer central baseline keeps the odd pixel on the ascent
        // side, then distributes the remaining line-height leading around it.
        assert_eq!(strut.text_ascent, 6.0);
        assert_eq!(strut.text_descent, 5.0);
        assert_eq!(strut.line_ascent, 11.0);
        assert_eq!(strut.line_descent, 10.0);
    }

    #[test]
    fn text_edge_alignment_excludes_line_height_leading() {
        let strut = inline_strut_metrics(
            InlineFontMetrics {
                ascent: 10.0,
                descent: 2.0,
                line_height: 20.0,
                x_height: 5.0,
            },
            false,
            FontBaseline::Alphabetic,
        );
        let baseline = strut.line_ascent;

        assert_eq!(baseline - strut.line_ascent, 0.0);
        assert_eq!(baseline - strut.text_ascent, 4.0);
        assert_eq!(baseline + strut.text_descent, 16.0);
        assert_eq!(baseline + strut.line_descent, 20.0);
        assert_eq!(
            non_edge_vertical_offset(
                InlineVerticalAlign {
                    kind: LayoutInlineAlignment::TextTop,
                    baseline_shift: 0.0,
                },
                Some(strut),
                InlineVerticalBounds {
                    top: -baseline,
                    bottom: 8.0 - baseline,
                },
            ),
            4.0,
        );
    }

    #[test]
    fn explicit_break_prevents_an_otherwise_empty_line_from_being_phantom() {
        assert!(!css_line_is_phantom(BreakReason::Explicit, false));
        assert!(css_line_is_phantom(BreakReason::None, false));
        assert!(!css_line_is_phantom(BreakReason::None, true));
    }

    #[test]
    fn element_line_break_keeps_geometry_owner_separate_from_inline_style_parent() {
        let root = LayoutBoxId::from_index(0);
        let inline = LayoutBoxId::from_index(1);
        let line_break = LayoutBoxId::from_index(2);
        let mut normalizer = InlineNormalizer::new(root);
        normalizer.hard_break(line_break, &[inline]);
        let input = normalizer.finish();

        assert_eq!(input.text, "\n");
        assert_eq!(input.units.len(), 1);
        assert_eq!(input.units[0].style_box, inline);
        assert_eq!(
            input.units[0].kind.element_line_break_box(),
            Some(line_break)
        );
        assert!(input.source_map.is_empty());
    }

    #[test]
    fn parley_forced_break_and_optional_editor_tail_map_to_css_phantom_lines() {
        let text = "\n";
        let mut font_context = parley::FontContext::new();
        let mut layout_context = parley::LayoutContext::<TextBrush>::new();
        let mut builder = layout_context.style_run_builder(&mut font_context, text, 1.0, true);
        let style = builder.push_style(TextStyle::default());
        builder.push_style_run(style, ..);
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        let mut lines = layout.lines();
        let forced_break_line = lines.next().expect("preserved newline must create a line");
        assert_eq!(forced_break_line.break_reason(), BreakReason::Explicit);
        assert!(!css_line_is_phantom(
            forced_break_line.break_reason(),
            false,
        ));
        for editor_tail in lines {
            let break_reason = editor_tail.break_reason();
            assert!(
                css_line_is_phantom(break_reason, false),
                "Parley editor tail must not become an extra CSS line box: {break_reason:?}"
            );
        }
    }

    fn normalize(
        chunks: &[(LayoutBoxId, &str)],
        mode: InlineWhiteSpaceCollapse,
        transform: InlineTextTransform,
    ) -> InlineBuildInput {
        let root = LayoutBoxId::from_index(0);
        let mut normalizer = InlineNormalizer::new(root);
        for (box_id, text) in chunks {
            normalizer.push_text(*box_id, text, mode, transform, &[root]);
        }
        normalizer.finish()
    }

    #[test]
    fn preserve_merges_crlf_across_adjacent_text_nodes_with_both_origins() {
        let first = LayoutBoxId::from_index(1);
        let second = LayoutBoxId::from_index(2);
        let input = normalize(
            &[(first, "A\r"), (second, "\nB")],
            InlineWhiteSpaceCollapse::Preserve,
            InlineTextTransform::None,
        );

        assert_eq!(input.text, "A\nB");
        assert_eq!(
            input.source_map,
            vec![
                InlineSourceMapEntry {
                    output_range: 0..1,
                    box_id: first,
                    source_byte_range: 0..1,
                    source_utf16_range: 0..1,
                    is_forced_line_break: false,
                },
                InlineSourceMapEntry {
                    output_range: 1..2,
                    box_id: first,
                    source_byte_range: 1..2,
                    source_utf16_range: 1..2,
                    is_forced_line_break: true,
                },
                InlineSourceMapEntry {
                    output_range: 1..2,
                    box_id: second,
                    source_byte_range: 0..1,
                    source_utf16_range: 0..1,
                    is_forced_line_break: true,
                },
                InlineSourceMapEntry {
                    output_range: 2..3,
                    box_id: second,
                    source_byte_range: 1..2,
                    source_utf16_range: 1..2,
                    is_forced_line_break: false,
                },
            ]
        );
    }

    #[test]
    fn collapse_turns_a_cjk_segment_break_into_space_across_text_nodes() {
        let first = LayoutBoxId::from_index(1);
        let second = LayoutBoxId::from_index(2);
        let input = normalize(
            &[(first, "\u{4e2d}\n"), (second, "\u{6587}")],
            InlineWhiteSpaceCollapse::Collapse,
            InlineTextTransform::None,
        );

        assert_eq!(input.text, "\u{4e2d} \u{6587}");
        assert_eq!(input.source_map.len(), 3);
        assert_eq!(input.source_map[0].box_id, first);
        assert_eq!(input.source_map[0].source_byte_range, 0..3);
        assert_eq!(input.source_map[0].source_utf16_range, 0..1);
        assert_eq!(input.source_map[1].output_range, 3..4);
        assert_eq!(input.source_map[1].box_id, first);
        assert_eq!(input.source_map[1].source_byte_range, 3..4);
        assert_eq!(input.source_map[1].source_utf16_range, 1..2);
        assert_eq!(input.source_map[2].box_id, second);
        assert_eq!(input.source_map[2].source_byte_range, 0..3);
        assert_eq!(input.source_map[2].source_utf16_range, 0..1);
    }

    #[test]
    fn break_spaces_preserves_the_source_stream_without_synthetic_controls() {
        let text = LayoutBoxId::from_index(1);
        let input = normalize(
            &[(text, "A  B")],
            InlineWhiteSpaceCollapse::BreakSpaces,
            InlineTextTransform::None,
        );

        assert_eq!(input.text, "A  B");
        assert_eq!(
            input
                .units
                .iter()
                .filter(|unit| unit.kind == InlineTextUnitKind::Control)
                .count(),
            0
        );
        assert_eq!(input.source_map.len(), 4);
        assert!(
            input
                .source_map
                .iter()
                .all(|entry| { !input.text[entry.output_range.clone()].is_empty() })
        );
    }

    #[test]
    fn collapsed_spaces_remain_in_dom_order_across_inline_boundaries() {
        let root = LayoutBoxId::from_index(0);
        let first_inline = LayoutBoxId::from_index(1);
        let first_text = LayoutBoxId::from_index(2);
        let outer_space = LayoutBoxId::from_index(3);
        let second_inline = LayoutBoxId::from_index(4);
        let second_text = LayoutBoxId::from_index(5);
        let trailing_text = LayoutBoxId::from_index(6);
        let mut normalizer = InlineNormalizer::new(root);

        normalizer.open_inline(
            first_inline,
            InlineUnicodeBidi::Normal,
            InlineDirection::Ltr,
            &[],
            InlineVerticalAlign::default(),
        );
        normalizer.push_text(
            first_text,
            "A",
            InlineWhiteSpaceCollapse::Collapse,
            InlineTextTransform::None,
            &[first_inline],
        );
        normalizer.close_inline(
            first_inline,
            InlineUnicodeBidi::Normal,
            &[],
            InlineVerticalAlign::default(),
        );
        normalizer.push_text(
            outer_space,
            " ",
            InlineWhiteSpaceCollapse::Collapse,
            InlineTextTransform::None,
            &[],
        );
        normalizer.open_inline(
            second_inline,
            InlineUnicodeBidi::Embed,
            InlineDirection::Ltr,
            &[],
            InlineVerticalAlign::default(),
        );
        normalizer.push_text(
            second_text,
            "B ",
            InlineWhiteSpaceCollapse::Collapse,
            InlineTextTransform::None,
            &[second_inline],
        );
        normalizer.close_inline(
            second_inline,
            InlineUnicodeBidi::Embed,
            &[],
            InlineVerticalAlign::default(),
        );
        normalizer.push_text(
            trailing_text,
            "C",
            InlineWhiteSpaceCollapse::Collapse,
            InlineTextTransform::None,
            &[],
        );

        let input = normalizer.finish();
        assert_eq!(input.text, "A \u{202a}B \u{202c}C");
        assert_eq!(
            input
                .objects
                .iter()
                .map(|(index, object, _)| (*index, object.box_id, object.role))
                .collect::<Vec<_>>(),
            vec![
                (0, first_inline, InlineObjectRole::StartEdge),
                (1, first_inline, InlineObjectRole::EndEdge),
                (5, second_inline, InlineObjectRole::StartEdge),
                (7, second_inline, InlineObjectRole::EndEdge),
            ]
        );
        assert_eq!(input.units[1].output_range, 1..2);
        assert!(input.units[1].ancestors.is_empty());
        assert_eq!(input.units[4].output_range, 6..7);
        assert_eq!(input.units[4].ancestors, vec![second_inline]);
    }

    #[test]
    fn uppercase_expansion_retains_byte_and_utf16_source_ranges() {
        let text = LayoutBoxId::from_index(1);
        let input = normalize(
            &[(text, "\u{df}\u{1f642}")],
            InlineWhiteSpaceCollapse::Preserve,
            InlineTextTransform::Uppercase,
        );

        assert_eq!(input.text, "SS\u{1f642}");
        assert_eq!(input.source_map.len(), 3);
        for entry in &input.source_map[..2] {
            assert_eq!(entry.box_id, text);
            assert_eq!(entry.source_byte_range, 0..2);
            assert_eq!(entry.source_utf16_range, 0..1);
        }
        assert_eq!(input.source_map[0].output_range, 0..1);
        assert_eq!(input.source_map[1].output_range, 1..2);
        assert_eq!(input.source_map[2].output_range, 2..6);
        assert_eq!(input.source_map[2].source_byte_range, 2..6);
        assert_eq!(input.source_map[2].source_utf16_range, 1..3);
    }
}
