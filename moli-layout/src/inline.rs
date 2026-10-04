// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The one-Parley-tree-per-IFC shape follows DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8, especially
// `layout/construct.rs::build_inline_layout_into`. Moli deliberately
// keeps the item stream, source mapping, and Parley layout pass-local.
// Relative positioning of atomic inline boxes additionally follows Blitz
// commit 4a9be930accc971675d5730e4fde3cfa13c3b57e.

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Debug,
    hash::Hash,
    ops::Range,
};

use parley::{
    BreakReason, InlineBox, InlineBoxBidi, InlineBoxKind, Layout, PositionedLayoutItem, TextStyle,
};
use taffy::{MaybeResolve as _, Point, Size};

use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutWorld, PaintColor, PaintRect,
    style::{
        InlineDirection, InlineTextTransform, InlineUnicodeBidi, InlineVerticalAlign,
        InlineWhiteSpaceCollapse, LayoutInlineAlignment,
    },
    stylo_to_parley::TextBrush,
    text::{DocumentLayoutServices, InlineFontMetrics},
};

/// Resolve the relative inset applied after Parley has positioned an atomic
/// inline box. Taffy cannot do this itself because atomic IFC children are
/// represented as Parley inline objects and their final locations are written
/// back after line layout.
pub(crate) fn relative_atomic_inset_offset(
    style: &taffy::Style<style::Atom>,
    containing_block_size: Size<f32>,
    container_direction: InlineDirection,
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
    Point {
        x: if container_direction == InlineDirection::Rtl {
            inset
                .right
                .map(|value| -value)
                .or(inset.left)
                .unwrap_or(0.0)
        } else {
            inset
                .left
                .or(inset.right.map(|value| -value))
                .unwrap_or(0.0)
        },
        y: inset
            .top
            .or(inset.bottom.map(|value| -value))
            .unwrap_or(0.0),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InlineSourceMapEntry {
    pub(crate) output_range: Range<usize>,
    pub(crate) box_id: LayoutBoxId,
    pub(crate) source_byte_range: Range<usize>,
    pub(crate) source_utf16_range: Range<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct InlineTextUnit {
    pub(crate) output_range: Range<usize>,
    pub(crate) style_box: LayoutBoxId,
    pub(crate) ancestors: Vec<LayoutBoxId>,
    pub(crate) sources: Vec<SourceOrigin>,
    pub(crate) control: bool,
    /// A normalized CSS space that can still collapse at a line boundary.
    pub(crate) collapsible_whitespace: bool,
    pub(crate) break_spaces_opportunity: bool,
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

impl InlineObjectRole {
    fn parley_bidi(self) -> InlineBoxBidi {
        match self {
            Self::Atomic => InlineBoxBidi::Neutral,
            // Floating and static-position placeholders resolve a neutral
            // U+FFFC in bidi analysis only. An enclosing inline edge's level
            // may otherwise carry them across the following directional run.
            Self::Float | Self::OutOfFlow => InlineBoxBidi::Neutral,
            // CSS tag boundaries do not add bidi characters. Leading edges
            // follow the next participant, closing edges the preceding one.
            Self::StartEdge => InlineBoxBidi::InheritNext,
            Self::EndEdge => InlineBoxBidi::InheritPrevious,
        }
    }
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

/// Used inline-edge dimensions for one numeric layout probe. Margins can
/// create a line, but only border/padding retain an empty inline's line height
/// in quirks mode.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InlineEdgeContribution {
    pub(crate) creates_line: bool,
    pub(crate) has_border_or_padding: bool,
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

#[derive(Debug)]
pub(crate) struct InlineFormattingContext {
    pub(crate) root_style: LayoutBoxId,
    /// Reusable Parley layout for intrinsic and final-width probes. Line
    /// breaking replaces only Parley's line output while retaining the shaped
    /// runs, clusters, glyphs, and their allocations, so probes must not clone
    /// the complete shaped paragraph.
    pub(crate) measurement_layout: Option<Layout<TextBrush>>,
    /// The accepted `PerformLayout` result consumed by paint and CSSOM. This is
    /// kept separate from the reusable measurement layout so a later intrinsic
    /// probe cannot overwrite the last accepted line layout.
    pub(crate) laid_out: Option<Layout<TextBrush>>,
    /// Pass-local memo for intrinsic widths of a pure-text paragraph. Numeric
    /// positioning buffers intentionally remain probe-local: retaining their
    /// capacity on every IFC increases the peak footprint of the fresh layout
    /// world more than it saves allocator traffic.
    pub(crate) content_widths: InlineContentWidthsMemo,
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
    /// Quirks and limited-quirks omit the block's strut and empty, undecorated
    /// inline struts from line height. Keep font metrics for alignment/paint.
    pub(crate) uses_quirks_line_height: bool,
    pub(crate) root_includes_used_font_metrics: bool,
    /// Direct structural parent of each shaped style. Including this identity
    /// in style deduplication prevents glyph runs from crossing a box-state
    /// boundary even when their paint/font properties are otherwise equal.
    pub(crate) style_parents: Vec<LayoutBoxId>,
    pub(crate) structural_boxes: Vec<InlineStructuralBox>,
    pub(crate) line_placements: Vec<InlineLinePlacement>,
    pub(crate) fragments: InlineFragments,
}

#[derive(Debug, Default)]
pub(crate) struct InlineContentWidthsMemo {
    entry: Option<InlineContentWidthsCache>,
    #[cfg(test)]
    hits: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InlineContentWidthsCacheKey {
    indent_bits: u32,
    each_line: bool,
    hanging: bool,
}

impl InlineContentWidthsCacheKey {
    fn new(indent: f32, options: parley::IndentOptions) -> Self {
        Self {
            indent_bits: indent.to_bits(),
            each_line: options.each_line,
            hanging: options.hanging,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct InlineContentWidthsCache {
    key: InlineContentWidthsCacheKey,
    widths: parley::ContentWidths,
}

impl InlineContentWidthsMemo {
    /// Reuses intrinsic widths only when the IFC contains shaped text and no
    /// inline object whose width can change between Taffy probes.
    ///
    /// Parley deliberately recalculates content widths on every call. For a
    /// pure-text IFC, however, the scanned items and cluster advances are
    /// immutable for this fresh layout pass. The key includes text-indent so
    /// this adapter remains correct if Parley starts incorporating indentation
    /// into intrinsic widths in a future release.
    pub(crate) fn content_widths_for_probe(
        &mut self,
        layout: &Layout<TextBrush>,
        indent: f32,
        options: parley::IndentOptions,
    ) -> parley::ContentWidths {
        if layout.inline_boxes().len() != 0 {
            return layout.calculate_content_widths();
        }

        let key = InlineContentWidthsCacheKey::new(indent, options);
        if let Some(cached) = self.entry.filter(|cached| cached.key == key) {
            #[cfg(test)]
            {
                self.hits += 1;
            }
            return cached.widths;
        }

        let widths = layout.calculate_content_widths();
        self.entry = Some(InlineContentWidthsCache { key, widths });
        widths
    }
}

/// Restores the reusable Parley paragraph to its shaped, unbroken state before
/// another inline measurement probe.
///
/// `text-align: justify` mutates whitespace cluster advances. Parley undoes
/// those adjustments when a new line breaker is created, so this reset must
/// happen before intrinsic content widths are read for the next probe. Dropping
/// the breaker immediately also clears the previous line output while retaining
/// its vector capacity and all shaped runs, clusters, and glyphs.
pub(crate) fn reset_inline_layout_for_probe(layout: &mut Layout<TextBrush>) {
    drop(layout.break_lines());
}

/// Place CSS decorations around visual fragments after bidi reordering.
///
/// Edge widths still participate in logical line breaking. Their visual slots
/// follow CSS direction and descendant fragments, independently of the text's
/// resolved bidi levels. Process descendants first so nested decorations are
/// included when placing their parent's physical edges.
pub(crate) fn position_inline_edges<N: Copy + Debug + Eq + Hash>(
    world: &LayoutWorld<N>,
    context: &InlineFormattingContext,
    layout: &mut Layout<TextBrush>,
) {
    if !context.objects.iter().any(|object| {
        matches!(
            object.role,
            InlineObjectRole::StartEdge | InlineObjectRole::EndEdge
        )
    }) {
        return;
    }
    for line_index in 0..layout.len() {
        let line = layout.get(line_index).expect("known line");
        let mut ancestors = vec![Vec::new(); line.len()];
        for run in line.runs() {
            ancestors[run.index()] =
                overlapping_output_ranges(&context.text_units, &run.text_range())
                    .iter()
                    .filter(|unit| !unit.control)
                    .flat_map(|unit| unit.ancestors.iter().copied())
                    .collect();
        }
        let mut edges = BTreeMap::<usize, Vec<(usize, bool)>>::new();
        for (slot, inline_box) in line.inline_box_indices() {
            let Some(object) = context.object(inline_box.id) else {
                continue;
            };
            ancestors[slot] = object.ancestors.clone();
            if matches!(
                object.role,
                InlineObjectRole::StartEdge | InlineObjectRole::EndEdge
            ) {
                let physical_left = (object.role == InlineObjectRole::StartEdge)
                    == (world.boxes[object.box_id.index()].style.direction()
                        == InlineDirection::Ltr);
                edges
                    .entry(object.box_id.index())
                    .or_default()
                    .push((slot, physical_left));
            }
        }
        let mut edges = edges.into_iter().collect::<Vec<_>>();
        edges.sort_by_key(|(_, slots)| std::cmp::Reverse(ancestors[slots[0].0].len()));
        let mut order = (0..line.len()).collect::<Vec<_>>();
        for (box_index, mut slots) in edges {
            let original_position = order
                .iter()
                .position(|slot| slots.iter().any(|(edge, _)| edge == slot))
                .expect("edge on this line");
            order.retain(|slot| !slots.iter().any(|(edge, _)| edge == slot));
            let box_id = LayoutBoxId::from_index(box_index);
            let first = order
                .iter()
                .position(|slot| ancestors[*slot].contains(&box_id));
            let last = order
                .iter()
                .rposition(|slot| ancestors[*slot].contains(&box_id));
            slots.sort_by_key(|(_, left)| !left);
            if let (Some(first), Some(last)) = (first, last) {
                if let Some((slot, _)) = slots.iter().find(|(_, left)| !left) {
                    order.insert(last + 1, *slot);
                }
                if let Some((slot, _)) = slots.iter().find(|(_, left)| *left) {
                    order.insert(first, *slot);
                }
            } else {
                let position = original_position.min(order.len());
                order.splice(position..position, slots.iter().map(|(slot, _)| *slot));
            }
        }
        if order.iter().enumerate().any(|(index, &slot)| index != slot) {
            layout.reorder_line_items(line_index, &order);
        }
    }
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
    pub(crate) rect: PaintRect,
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

/// Numeric line-box result shared by intrinsic measurement and final inline
/// placement. Intrinsic probes need these four values, but do not need the
/// per-item, per-glyph, and per-box vectors stored by [`InlineLinePlacement`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct InlineLineMetrics {
    pub(crate) line_expansion: f32,
    pub(crate) first_baseline: Option<f32>,
    pub(crate) last_baseline: Option<f32>,
    pub(crate) has_non_phantom_line: bool,
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
        self.rect.y += offset;
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

    pub(crate) fn style_is_within_box(&self, index: usize, target: LayoutBoxId) -> bool {
        let mut current = Some(self.style_parent(index));
        while let Some(box_id) = current {
            if box_id == target {
                return true;
            }
            current = self.structural_box(box_id).map(|state| state.parent);
        }
        false
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineLineFragment {
    pub(crate) line_index: usize,
    pub(crate) rect: PaintRect,
    /// Conservative glyph/decoration/shadow ink used only by capture culling.
    /// CSSOM line geometry continues to use `rect`.
    pub(crate) paint_bounds: InlinePaintBounds,
    pub(crate) baseline: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum InlinePaintBounds {
    #[default]
    Empty,
    Bounded(PaintRect),
    Unbounded,
}

impl InlinePaintBounds {
    fn include(&mut self, rect: PaintRect) {
        *self = match *self {
            Self::Empty => Self::Bounded(rect),
            Self::Bounded(current) => Self::Bounded(current.union(rect)),
            Self::Unbounded => Self::Unbounded,
        };
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InlineSourceFragment {
    pub(crate) line_index: usize,
    pub(crate) box_id: LayoutBoxId,
    pub(crate) source_byte_range: Range<usize>,
    pub(crate) source_utf16_range: Range<usize>,
    pub(crate) rtl: bool,
    pub(crate) rect: PaintRect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineBoxFragment {
    pub(crate) line_index: usize,
    pub(crate) box_id: LayoutBoxId,
    pub(crate) rect: PaintRect,
    pub(crate) has_start_edge: bool,
    pub(crate) has_end_edge: bool,
}

pub(crate) fn build_inline_fragments(
    context: &InlineFormattingContext,
    layout: &Layout<TextBrush>,
    line_placements: &[InlineLinePlacement],
) -> InlineFragments {
    // Binary overlap lookup relies on both endpoints being monotonic. Validate
    // each immutable normalization product once, rather than rescanning the
    // complete maps for every visual glyph cluster in debug and test builds.
    debug_assert!(output_ranges_are_monotonic(&context.text_units));
    debug_assert!(output_ranges_are_monotonic(&context.source_map));
    let mut fragments = InlineFragments::default();
    let mut box_fragments = HashMap::<(usize, usize), FragmentAccumulator>::new();
    let mut source_fragments = HashMap::<SourceFragmentKey, FragmentAccumulator>::new();
    let style_paint_outsets = layout
        .styles()
        .iter()
        .map(text_style_paint_outsets)
        .collect::<Vec<_>>();

    for (line_index, line) in layout.lines().enumerate() {
        let metrics = line.metrics();
        let placement = line_placements
            .get(line_index)
            .filter(|placement| placement.line_index == line_index);
        let line_rect = placement.map_or_else(
            || {
                PaintRect::new(
                    metrics.inline_min_coord + metrics.offset,
                    metrics.block_min_coord,
                    metrics.advance,
                    (metrics.block_max_coord - metrics.block_min_coord).max(0.0),
                )
            },
            |placement| placement.rect,
        );
        fragments.lines.push(InlineLineFragment {
            line_index,
            rect: line_rect,
            paint_bounds: context
                .selection
                .as_ref()
                .map_or(InlinePaintBounds::Empty, |_| {
                    InlinePaintBounds::Bounded(line_rect)
                }),
            baseline: placement.map_or(metrics.baseline, |placement| placement.baseline),
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
                    .map(|metrics| inline_strut_metrics(metrics, true));
                let ascent = font_metrics.map_or(run_metrics.ascent, |metrics| metrics.text_ascent);
                let descent =
                    font_metrics.map_or(run_metrics.descent, |metrics| metrics.text_descent);
                let rect = PaintRect::new(
                    metrics.inline_min_coord + cluster.visual_offset().unwrap_or(metrics.offset),
                    metrics.baseline - ascent + vertical_offset,
                    cluster.advance().max(0.0),
                    (ascent + descent).max(0.0),
                );
                if let Some(style) = layout.styles().get(style_index)
                    && style.brush.paint
                {
                    // Parley exposes typographic cluster boxes rather than
                    // exact outline bounds. A two-em guard is deliberately
                    // conservative for italic/color-glyph overhang, while the
                    // style sidecar adds arbitrary CSS shadow/decoration
                    // displacement. This work happens once, alongside final
                    // fragment materialization, not during every paint.
                    let glyph_guard = run.font_size().max(0.0) * 2.0 + 1.0;
                    let line = &mut fragments.lines[line_index];
                    match style_paint_outsets.get(style_index).copied().flatten() {
                        Some(outsets) => {
                            line.paint_bounds.include(outsets.outset(rect, glyph_guard))
                        }
                        None => line.paint_bounds = InlinePaintBounds::Unbounded,
                    }
                }
                for unit in overlapping_output_ranges(&context.text_units, &range) {
                    for ancestor in &unit.ancestors {
                        box_fragments
                            .entry((ancestor.index(), line_index))
                            .or_default()
                            .include_inline_axis(rect.x, rect.width);
                    }
                }
                for source in overlapping_output_ranges(&context.source_map, &range) {
                    source_fragments
                        .entry(SourceFragmentKey {
                            box_index: source.box_id.index(),
                            source_byte_start: source.source_byte_range.start,
                            source_byte_end: source.source_byte_range.end,
                            source_utf16_start: source.source_utf16_range.start,
                            source_utf16_end: source.source_utf16_range.end,
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
                PaintRect::new(
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
                    accumulator.include_inline_axis(rect.x, rect.width);
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

    let mut box_fragments = box_fragments.into_iter().collect::<Vec<_>>();
    box_fragments.sort_unstable_by_key(|(key, _)| *key);
    fragments.boxes = box_fragments
        .into_iter()
        .filter_map(|((box_index, line_index), accumulator)| {
            let line_rect = fragments.lines.get(line_index)?.rect;
            Some(InlineBoxFragment {
                line_index,
                box_id: LayoutBoxId::from_index(box_index),
                rect: accumulator.rect(line_rect)?,
                has_start_edge: accumulator.has_start_edge,
                has_end_edge: accumulator.has_end_edge,
            })
        })
        .collect();
    let mut source_fragments = source_fragments.into_iter().collect::<Vec<_>>();
    source_fragments.sort_unstable_by_key(|(key, _)| *key);
    fragments.text = source_fragments
        .into_iter()
        .filter_map(|(key, accumulator)| {
            let line_rect = fragments.lines.get(key.line_index)?.rect;
            Some(InlineSourceFragment {
                line_index: key.line_index,
                box_id: LayoutBoxId::from_index(key.box_index),
                source_byte_range: key.source_byte_start..key.source_byte_end,
                source_utf16_range: key.source_utf16_start..key.source_utf16_end,
                rtl: key.rtl,
                rect: accumulator.rect(line_rect)?,
            })
        })
        .collect();
    fragments
}

#[derive(Clone, Copy, Debug, Default)]
struct TextPaintOutsets {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

impl TextPaintOutsets {
    fn outset(self, rect: PaintRect, guard: f32) -> PaintRect {
        let top = self.top.max(0.0) + guard;
        let right = self.right.max(0.0) + guard;
        let bottom = self.bottom.max(0.0) + guard;
        let left = self.left.max(0.0) + guard;
        PaintRect::new(
            rect.x - left,
            rect.y - top,
            (rect.width + left + right).max(0.0),
            (rect.height + top + bottom).max(0.0),
        )
    }
}

fn text_style_paint_outsets(style: &parley::layout::Style<TextBrush>) -> Option<TextPaintOutsets> {
    let mut outsets = TextPaintOutsets::default();
    for shadow in style
        .brush
        .shadows
        .iter()
        .filter(|shadow| shadow.color.alpha > 0.0)
    {
        if !shadow.offset.x.is_finite()
            || !shadow.offset.y.is_finite()
            || !shadow.blur_radius.is_finite()
        {
            return None;
        }
        let blur = shadow.blur_radius.max(0.0) * 4.0 + 1.0;
        outsets.left = outsets.left.max(blur - shadow.offset.x);
        outsets.right = outsets.right.max(blur + shadow.offset.x);
        outsets.top = outsets.top.max(blur - shadow.offset.y);
        outsets.bottom = outsets.bottom.max(blur + shadow.offset.y);
    }

    let decoration = style.brush.decoration;
    if decoration.underline || decoration.overline || decoration.line_through {
        // Normal decoration ink remains inside the guarded typographic box.
        // Authored underline offsets can move it arbitrarily far away.
        let displaced = decoration.underline_offset.unwrap_or_default().abs()
            + decoration.thickness.unwrap_or(1.0).max(0.0) * 3.0
            + 1.0;
        outsets.top = outsets.top.max(displaced);
        outsets.bottom = outsets.bottom.max(displaced);
    }
    Some(outsets)
}

/// Breaks a shared IFC text stream while preserving CSS `break-spaces`
/// trailing-space semantics that Parley 0.10 does not model directly.
pub(crate) fn break_inline_lines(
    context: &InlineFormattingContext,
    layout: &mut Layout<TextBrush>,
    max_advance: Option<f32>,
) {
    layout.break_all_lines(max_advance);
    let Some(width) = max_advance.filter(|width| width.is_finite() && *width > 0.0) else {
        return;
    };
    if !context
        .text_units
        .iter()
        .any(|unit| unit.break_spaces_opportunity)
    {
        return;
    }

    // Parley hangs an overflowing U+0020 on the preceding line. That is
    // correct for normal whitespace but not for `break-spaces`, where every
    // preserved space occupies line width. Identify only the lines where the
    // initial break actually overflowed through trailing whitespace.
    let tolerance = width.abs().max(1.0) * f32::EPSILON * 8.0;
    let adjust_lines = layout
        .lines()
        .map(|line| {
            let metrics = line.metrics();
            let line_range = line.text_range();
            metrics.hanging_advance > 0.0
                && metrics.advance > width + tolerance
                && overlapping_output_ranges(&context.text_units, &line_range)
                    .iter()
                    .any(|unit| unit.break_spaces_opportunity)
        })
        .collect::<Vec<_>>();
    if !adjust_lines.iter().any(|adjust| *adjust) {
        return;
    }

    // Moving the affected line width one representable step inward makes the
    // last fitting preserved space use Parley's normal overflowing-space
    // commit. Restore the real CSS width on every committed line so alignment
    // and fragments still observe the containing block, not the breaker shim.
    let adjusted_width = (width - tolerance).max(0.0);
    let mut breaker = layout.break_lines();
    breaker.state_mut().set_layout_max_advance(width);
    let mut line_index = 0;
    let mut use_normal_breaking = false;
    while !breaker.is_done() {
        let line_width = if adjust_lines.get(line_index).copied().unwrap_or(false) {
            adjusted_width
        } else {
            width
        };
        breaker.state_mut().set_line_max_advance(line_width);
        match breaker.break_next() {
            Some(parley::YieldData::LineBreak(_)) => {
                breaker.set_prior_line_width(width);
                line_index += 1;
            }
            Some(
                parley::YieldData::MaxHeightExceeded(_) | parley::YieldData::InlineBoxBreak(_),
            ) => {
                // Neither condition is produced by Moli's rectangular
                // IFC input. Fall back to the already supported normal
                // breaker instead of looping or publishing a partial layout.
                use_normal_breaking = true;
                break;
            }
            None => break,
        }
    }
    breaker.finish();
    if use_normal_breaking {
        layout.break_all_lines(Some(width));
    }
}

/// Reads Parley's line items together with their CSS content contribution.
/// Parley retains line-edge spaces for positioning and selection. Collapse
/// their contribution here before resolving vertical metrics, while keeping
/// every item in the same order for painting and fragment geometry.
fn line_items_with_content<'a>(
    context: &'a InlineFormattingContext,
    layout: &'a Layout<TextBrush>,
    line: &parley::Line<'a, TextBrush>,
) -> impl Iterator<Item = (PositionedLayoutItem<'a, TextBrush>, bool)> + 'a {
    let has_content = |item: &PositionedLayoutItem<'_, TextBrush>| match item {
        PositionedLayoutItem::GlyphRun(glyph_run) => {
            glyph_run.style().brush.paint && glyph_run.glyphs().next().is_some()
        }
        PositionedLayoutItem::InlineBox(positioned) => context
            .object(positioned.id)
            .is_some_and(|object| object.role == InlineObjectRole::Atomic),
    };
    let only_collapsible_spaces = |item: &PositionedLayoutItem<'_, TextBrush>| match item {
        PositionedLayoutItem::GlyphRun(glyph_run) => {
            glyph_run_is_collapsible_whitespace(context, glyph_run)
        }
        PositionedLayoutItem::InlineBox(_) => false,
    };
    // CSS whitespace contribution follows logical input order, before bidi
    // reordering. Parley's visual item order can put an interior space outside
    // the two atomic objects surrounding it in the logical stream.
    let logical_range = |item: &PositionedLayoutItem<'_, TextBrush>| match item {
        PositionedLayoutItem::GlyphRun(glyph_run) => glyph_run
            .run()
            .clusters()
            .filter(|cluster| std::ptr::eq(cluster.style(), glyph_run.style()))
            .map(|cluster| cluster.text_range())
            .reduce(|left, right| left.start.min(right.start)..left.end.max(right.end)),
        PositionedLayoutItem::InlineBox(positioned) => usize::try_from(positioned.id)
            .ok()
            .and_then(|index| layout.inline_boxes().nth(index))
            .map(|object| object.index..object.index),
    };
    let mut content_range: Option<Range<usize>> = None;
    if context.uses_quirks_line_height {
        for item in line.items() {
            if has_content(&item)
                && !only_collapsible_spaces(&item)
                && let Some(range) = logical_range(&item)
            {
                content_range = Some(content_range.map_or(range.clone(), |previous| {
                    previous.start.min(range.start)..previous.end.max(range.end)
                }));
            }
        }
    }
    line.items().map(move |item| {
        let interior = || {
            content_range.as_ref().is_some_and(|content| {
                logical_range(&item)
                    .is_some_and(|range| range.start >= content.start && range.end <= content.end)
            })
        };
        let contributes = has_content(&item)
            && (!context.uses_quirks_line_height || !only_collapsible_spaces(&item) || interior());
        (item, contributes)
    })
}

fn glyph_run_is_collapsible_whitespace(
    context: &InlineFormattingContext,
    glyph_run: &parley::GlyphRun<'_, TextBrush>,
) -> bool {
    let mut found = false;
    let only_spaces = glyph_run
        .run()
        .clusters()
        .filter(|cluster| std::ptr::eq(cluster.style(), glyph_run.style()))
        .all(|cluster| {
            found = true;
            let units = overlapping_output_ranges(&context.text_units, &cluster.text_range());
            !units.is_empty() && units.iter().all(|unit| unit.collapsible_whitespace)
        });
    found && only_spaces
}

/// Builds the pass-local vertical placement sidecar that Parley 0.10 does not
/// provide for CSS `vertical-align`. The sidecar leaves Parley's shaped data
/// immutable and applies the same offsets to glyph projection, atomic boxes,
/// out-of-flow static positions, and fragment geometry.
pub(crate) fn measure_inline_lines(
    context: &InlineFormattingContext,
    layout: &Layout<TextBrush>,
    atomic_baseline_ascents: &[Option<f32>],
    structural_edge_contributions: &[InlineEdgeContribution],
    float_line_clearances: &[f32],
) -> InlineLineMetrics {
    resolve_inline_lines(
        context,
        layout,
        atomic_baseline_ascents,
        structural_edge_contributions,
        float_line_clearances,
        None,
    )
}

pub(crate) fn build_inline_line_placements(
    context: &InlineFormattingContext,
    layout: &Layout<TextBrush>,
    atomic_baseline_ascents: &[Option<f32>],
    structural_edge_contributions: &[InlineEdgeContribution],
    float_line_clearances: &[f32],
) -> (Vec<InlineLinePlacement>, InlineLineMetrics) {
    let mut placements = Vec::with_capacity(layout.lines().len());
    let metrics = resolve_inline_lines(
        context,
        layout,
        atomic_baseline_ascents,
        structural_edge_contributions,
        float_line_clearances,
        Some(&mut placements),
    );
    (placements, metrics)
}

fn resolve_inline_lines(
    context: &InlineFormattingContext,
    layout: &Layout<TextBrush>,
    atomic_baseline_ascents: &[Option<f32>],
    structural_edge_contributions: &[InlineEdgeContribution],
    float_line_clearances: &[f32],
    mut placements: Option<&mut Vec<InlineLinePlacement>>,
) -> InlineLineMetrics {
    let mut result = InlineLineMetrics::default();
    let mut preceding_adjustment = 0.0;
    let mut unadjusted_line_top = 0.0;

    for (line_index, line) in layout.lines().enumerate() {
        let metrics = line.metrics();
        let raw_top = unadjusted_line_top;
        let raw_bottom = raw_top + metrics.line_height.max(0.0);
        let mut geometries = line_items_with_content(context, layout, &line)
            .map(|(item, contributes_to_line)| match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    let run = glyph_run.run();
                    let run_metrics = run.font_metrics();
                    let paint = glyph_run.style().brush.paint;
                    let style_index = glyph_run
                        .glyphs()
                        .next()
                        .map(|_| usize::from(glyph_run.style_index()));
                    let structural_parent =
                        style_index.map_or(context.root_style, |index| context.style_parent(index));
                    let primary_strut = style_index
                        .and_then(|index| context.font_metrics.get(index).copied().flatten())
                        .map(|metrics| inline_strut_metrics(metrics, true));
                    let bounds = glyph_line_bounds(
                        primary_strut,
                        run_metrics,
                        run.line_height(),
                        context.box_includes_used_font_metrics(structural_parent),
                    );
                    InlineItemVerticalGeometry {
                        bounds,
                        initial_top: glyph_run.baseline() + bounds.top,
                        structural_parent,
                        object_index: None,
                        vertical_align: InlineVerticalAlign::default(),
                        // Parley may expose an empty root-style run next to
                        // float/out-of-flow placeholders. It carries the font
                        // style but no glyph geometry and is not in-flow line
                        // content by itself.
                        contributes_to_line,
                        creates_line: contributes_to_line,
                        glyph_key: if paint {
                            style_index.map(|index| (run.index(), index))
                        } else {
                            None
                        },
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
                        .or_else(|| is_atomic.then_some(positioned.height))
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
                        object_index,
                        vertical_align: if is_atomic {
                            object
                                .map(|object| object.vertical_align)
                                .unwrap_or_default()
                        } else {
                            InlineVerticalAlign::default()
                        },
                        contributes_to_line,
                        creates_line: object.is_some_and(|object| match object.role {
                            InlineObjectRole::Atomic => true,
                            InlineObjectRole::StartEdge | InlineObjectRole::EndEdge => object_index
                                .and_then(|index| structural_edge_contributions.get(index))
                                .is_some_and(|edge| edge.creates_line),
                            InlineObjectRole::Float | InlineObjectRole::OutOfFlow => false,
                        }),
                        glyph_key: None,
                        anchor: LineVerticalAnchor::Root,
                        relative_offset: 0.0,
                    }
                }
            })
            .collect::<Vec<_>>();
        // Parley keeps forced breaks as clusters without positioned glyphs.
        // In quirks mode their metrics are a fallback for their own empty box,
        // resolved after that box's content rather than for the whole line.
        let line_break =
            if context.uses_quirks_line_height && line.break_reason() == BreakReason::Explicit {
                line_break_metrics(context, layout, &line)
            } else {
                None
            };
        let mut states = build_line_inline_box_states(
            context,
            layout,
            line.text_range(),
            &geometries,
            line_break.map(|(box_id, _)| box_id),
        );
        let mut state_indices = BTreeMap::new();
        for (index, state) in states.iter().enumerate() {
            state_indices.insert(state.box_id.index(), index);
        }
        let phantom = css_line_is_phantom(
            line.break_reason(),
            geometries.iter().any(|geometry| geometry.creates_line),
        );
        for state in &mut states {
            state.parent = state_indices.get(&state.parent_box.index()).copied();
            state.alignment.anchor = state
                .parent
                .map_or(LineVerticalAnchor::Root, LineVerticalAnchor::State);
        }
        for geometry in &mut geometries {
            let box_id = geometry
                .edge(context)
                .map_or(geometry.structural_parent, |object| object.box_id);
            geometry.anchor = state_indices
                .get(&box_id.index())
                .copied()
                .map_or(LineVerticalAnchor::Root, LineVerticalAnchor::State);
        }
        let line_break = line_break.map(|(box_id, bounds)| {
            let anchor = state_indices
                .get(&box_id.index())
                .copied()
                .map_or(LineVerticalAnchor::Root, LineVerticalAnchor::State);
            (anchor, bounds)
        });

        let fallback_root_bounds = InlineVerticalBounds {
            top: metrics.block_min_coord - metrics.baseline,
            bottom: metrics.block_max_coord - metrics.baseline,
        };
        let mut root_bounds = (!phantom && !context.uses_quirks_line_height).then(|| {
            context
                .parent_strut
                .map_or(fallback_root_bounds, InlineVerticalBounds::from_strut)
        });
        for state in &mut states {
            state.line_contribution =
                (!phantom && !context.uses_quirks_line_height && state.has_strut_content)
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
            // Text supplies its own strut through glyph_line_bounds below.
            // An empty inline only contributes if this fragment has used
            // inline-axis border/padding; margin alone is insufficient.
            if geometry
                .object_index
                .and_then(|index| structural_edge_contributions.get(index))
                .is_some_and(|edge| edge.has_border_or_padding)
                && let LineVerticalAnchor::State(index) = geometry.anchor
                && let Some(strut) = states[index].strut
            {
                include_in_parent(
                    InlineVerticalBounds::from_strut(strut),
                    Some(index),
                    &mut states,
                    &mut root_bounds,
                );
            }
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
            let mut target_metrics = states[state_index].line_contribution.take();
            resolve_pending_alignments(
                target_pending,
                LineVerticalAnchor::State(state_index),
                &mut target_metrics,
                &mut states,
                &mut geometries,
            );
            states[state_index].line_contribution = target_metrics.or_else(|| {
                line_break
                    .filter(|(anchor, _)| *anchor == LineVerticalAnchor::State(state_index))
                    .map(|(_, bounds)| bounds)
            });

            let parent = states[state_index].parent;
            let vertical_align = states[state_index].vertical_align;
            // Empty boxes still have a position. Text-edge alignments activate
            // a zero-sized participant, as Blink's pending alignment does;
            // length/baseline/middle shifts do not create a height operand.
            if matches!(
                vertical_align.kind,
                LayoutInlineAlignment::TextTop | LayoutInlineAlignment::TextBottom
            ) {
                states[state_index]
                    .line_contribution
                    .get_or_insert(InlineVerticalBounds::ZERO);
            }
            let state_bounds = states[state_index]
                .line_contribution
                .unwrap_or(InlineVerticalBounds::ZERO);
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
            states[state_index].alignment.relative_offset = offset;
            if let Some(contribution) = states[state_index].line_contribution {
                include_in_parent(
                    contribution.shifted(offset),
                    parent,
                    &mut states,
                    &mut root_bounds,
                );
            }
        }

        resolve_pending_alignments(
            std::mem::take(&mut pending[root_pending_index]),
            LineVerticalAnchor::Root,
            &mut root_bounds,
            &mut states,
            &mut geometries,
        );
        root_bounds = root_bounds.or_else(|| {
            line_break
                .filter(|(anchor, _)| *anchor == LineVerticalAnchor::Root)
                .map(|(_, bounds)| bounds)
        });

        let bounds = if phantom {
            InlineVerticalBounds::ZERO
        } else if context.uses_quirks_line_height {
            root_bounds.unwrap_or(InlineVerticalBounds::ZERO)
        } else {
            root_bounds.unwrap_or(fallback_root_bounds)
        };
        let line_height = bounds.height();
        if !phantom {
            // CSS baseline adjustment can remove phantom lines or change line
            // heights. Keep float avoidance as a minimum top, so neither that
            // adjustment nor Parley's height sum discards the clearance.
            let clearance = float_line_clearances
                .get(line_index)
                .copied()
                .unwrap_or(0.0);
            preceding_adjustment += (clearance - raw_top - preceding_adjustment).max(0.0);
        }
        let root_baseline = raw_top + preceding_adjustment - bounds.top;

        if !phantom {
            result.has_non_phantom_line = true;
            result.first_baseline.get_or_insert(root_baseline);
            result.last_baseline = Some(root_baseline);
        }

        // Intrinsic and flex/grid probes need only the resolved line height
        // and baselines. The following state walk and vectors exist solely to
        // place final glyphs, atomic objects, and structural fragments.
        if let Some(placements) = placements.as_mut() {
            // Place alignment anchors before the states that reference them.
            for state_index in state_order.iter().rev().copied() {
                states[state_index].alignment.global_offset =
                    states[state_index].alignment.relative_offset
                        + anchor_global_offset(states[state_index].alignment.anchor, &states);
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
                    let baseline = root_baseline + state.alignment.global_offset;
                    // An empty closing fragment has no font box only when
                    // its whole quirks line has no resolved vertical metrics.
                    // Content with zero height still retains the font box.
                    let (top, height) = if context.uses_quirks_line_height
                        && !state.has_strut_content
                        && root_bounds.is_none()
                    {
                        (baseline, 0.0)
                    } else {
                        (
                            baseline - strut.text_ascent,
                            (strut.text_ascent + strut.text_descent).max(0.0),
                        )
                    };
                    Some(InlineBoxBlockPlacement {
                        box_id: state.box_id,
                        top,
                        height,
                    })
                })
                .collect();
            placements.push(InlineLinePlacement {
                line_index,
                rect: PaintRect::new(
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
        }
        // Parley already excludes an empty terminal line from layout.height().
        // Do not subtract its inherited metrics again after an oversized atom
        // has forced an emergency break at the end of the paragraph.
        let measured_height = if line_index + 1 == layout.len() && line.is_empty() {
            0.0
        } else {
            raw_bottom - raw_top
        };
        preceding_adjustment += line_height - measured_height;
        unadjusted_line_top += metrics.line_height.max(0.0);
    }

    result.line_expansion = preceding_adjustment;
    result
}

#[derive(Clone, Copy, Debug)]
struct InlineItemVerticalGeometry {
    /// Line-height bounds relative to this item's own alignment baseline.
    bounds: InlineVerticalBounds,
    /// Parley's original block-start coordinate for converting the resolved
    /// baseline back into an item delta.
    initial_top: f32,
    structural_parent: LayoutBoxId,
    /// Source object identity; its role and used edge contributions stay in
    /// their input records rather than being copied into vertical geometry.
    object_index: Option<usize>,
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

impl InlineItemVerticalGeometry {
    /// Structural edges align to their own box rather than to their parent.
    fn edge<'a>(&self, context: &'a InlineFormattingContext) -> Option<&'a InlineObject> {
        let object = context.objects.get(self.object_index?)?;
        matches!(
            object.role,
            InlineObjectRole::StartEdge | InlineObjectRole::EndEdge
        )
        .then_some(object)
    }
}

fn line_break_metrics(
    context: &InlineFormattingContext,
    layout: &Layout<TextBrush>,
    line: &parley::Line<'_, TextBrush>,
) -> Option<(LayoutBoxId, InlineVerticalBounds)> {
    line.runs().find_map(|run| {
        let cluster = run
            .clusters()
            .find(|cluster| cluster.is_hard_line_break())?;
        let style_index = layout
            .styles()
            .iter()
            .position(|style| std::ptr::eq(style, cluster.style()))?;
        let structural_parent = context.style_parent(style_index);
        let primary_strut =
            context.font_metrics[style_index].map(|metrics| inline_strut_metrics(metrics, true));
        let bounds = glyph_line_bounds(
            primary_strut,
            run.font_metrics(),
            run.line_height(),
            context.box_includes_used_font_metrics(structural_parent),
        );
        Some((structural_parent, bounds))
    })
}

#[derive(Clone, Copy, Debug)]
struct LineInlineBoxState {
    box_id: LayoutBoxId,
    parent_box: LayoutBoxId,
    parent: Option<usize>,
    depth: usize,
    vertical_align: InlineVerticalAlign,
    strut: Option<InlineStrutMetrics>,
    /// Content and start edges, plus closing edges in standards mode,
    /// retain this box's strut and the struts of its structural ancestors.
    has_strut_content: bool,
    /// Bounds participating in the line-height union, independent of font
    /// geometry and the alignment of this fragment. None differs from ZERO.
    line_contribution: Option<InlineVerticalBounds>,
    alignment: InlineBoxAlignment,
}

#[derive(Clone, Copy, Debug)]
struct InlineBoxAlignment {
    anchor: LineVerticalAnchor,
    relative_offset: f32,
    global_offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    used_font: &parley::FontMetrics,
    used_line_height: f32,
    include_used_font_metrics: bool,
) -> InlineVerticalBounds {
    let used_strut = inline_strut_metrics(
        InlineFontMetrics {
            ascent: used_font.ascent,
            descent: used_font.descent,
            line_height: used_line_height,
            x_height: used_font.x_height.unwrap_or(used_font.ascent * 0.56),
        },
        true,
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
    layout: &Layout<TextBrush>,
    line_range: Range<usize>,
    geometries: &[InlineItemVerticalGeometry],
    line_break_box: Option<LayoutBoxId>,
) -> Vec<LineInlineBoxState> {
    let mut present = std::collections::BTreeSet::new();
    let mut strut_boxes = std::collections::BTreeSet::new();
    for unit in overlapping_output_ranges(&context.text_units, &line_range) {
        for ancestor in &unit.ancestors {
            mark_structural_path(context, *ancestor, &mut present);
            if !unit.control {
                mark_structural_path(context, *ancestor, &mut strut_boxes);
            }
        }
    }
    for geometry in geometries {
        mark_structural_path(context, geometry.structural_parent, &mut present);
        if geometry.contributes_to_line {
            mark_structural_path(context, geometry.structural_parent, &mut strut_boxes);
        }
        if let Some(object) = geometry.edge(context) {
            mark_structural_path(context, object.box_id, &mut present);
            // A zero-size closing edge still retains its standards strut.
            // Parley can carry a close after a collapsed trailing space onto
            // the next line; that close belongs to the preceding fragment.
            let follows_wrapped_space = || {
                geometry
                    .object_index
                    .and_then(|index| layout.inline_boxes().nth(index))
                    .filter(|edge| edge.index <= line_range.start)
                    .is_some_and(|edge| {
                        context
                            .text_units
                            .partition_point(|unit| unit.output_range.end <= edge.index)
                            .checked_sub(1)
                            .and_then(|index| context.text_units.get(index))
                            .is_some_and(|unit| {
                                unit.output_range.end == edge.index
                                    && unit.collapsible_whitespace
                                    && unit.ancestors.contains(&object.box_id)
                            })
                    })
            };
            if object.role == InlineObjectRole::StartEdge
                || (!context.uses_quirks_line_height
                    && (geometry.creates_line || !follows_wrapped_space()))
            {
                mark_structural_path(context, object.box_id, &mut strut_boxes);
            }
        }
    }
    if let Some(box_id) = line_break_box {
        mark_structural_path(context, box_id, &mut present);
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
            has_strut_content: strut_boxes.contains(&state.box_id.index()),
            line_contribution: None,
            alignment: InlineBoxAlignment {
                anchor: LineVerticalAnchor::Root,
                relative_offset: 0.0,
                global_offset: 0.0,
            },
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
        if !present.insert(box_id.index()) {
            break;
        }
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
        .and_then(|index| {
            states
                .get_mut(index)
                .map(|state| &mut state.line_contribution)
        })
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
                states[index].alignment.anchor = target_anchor;
                states[index].alignment.relative_offset = offset;
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
        LineVerticalAnchor::State(index) => states
            .get(index)
            .map_or(0.0, |state| state.alignment.global_offset),
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

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct SourceFragmentKey {
    box_index: usize,
    source_byte_start: usize,
    source_byte_end: usize,
    source_utf16_start: usize,
    source_utf16_end: usize,
    line_index: usize,
    rtl: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct FragmentAccumulator {
    min_x: Option<f32>,
    min_y: Option<f32>,
    max_x: Option<f32>,
    max_y: Option<f32>,
    has_start_edge: bool,
    has_end_edge: bool,
}

impl FragmentAccumulator {
    fn include(&mut self, rect: PaintRect) {
        self.include_inline_axis(rect.x, rect.width);
        self.min_y = Some(self.min_y.map_or(rect.y, |value| value.min(rect.y)));
        self.max_y = Some(self.max_y.map_or(rect.y + rect.height, |value| {
            value.max(rect.y + rect.height)
        }));
    }

    fn include_inline_axis(&mut self, x: f32, width: f32) {
        self.min_x = Some(self.min_x.map_or(x, |value| value.min(x)));
        self.max_x = Some(self.max_x.map_or(x + width, |value| value.max(x + width)));
    }

    fn include_block_axis(&mut self, y: f32, height: f32) {
        self.min_y = Some(self.min_y.map_or(y, |value| value.min(y)));
        self.max_y = Some(self.max_y.map_or(y + height, |value| value.max(y + height)));
    }

    fn rect(self, fallback_block_rect: PaintRect) -> Option<PaintRect> {
        let min_x = self.min_x?;
        let min_y = self.min_y.unwrap_or(fallback_block_rect.y);
        let max_y = self
            .max_y
            .unwrap_or(fallback_block_rect.y + fallback_block_rect.height);
        Some(PaintRect::new(
            min_x,
            min_y,
            (self.max_x? - min_x).max(0.0),
            (max_y - min_y).max(0.0),
        ))
    }
}

fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

trait HasInlineOutputRange {
    fn output_range(&self) -> &Range<usize>;
}

impl HasInlineOutputRange for InlineTextUnit {
    fn output_range(&self) -> &Range<usize> {
        &self.output_range
    }
}

impl HasInlineOutputRange for InlineSourceMapEntry {
    fn output_range(&self) -> &Range<usize> {
        &self.output_range
    }
}

/// Returns the contiguous slice intersecting `target` from an output-ordered
/// inline map. Normalization produces non-overlapping ranges, except that a
/// single output range can have multiple source origins (for example CRLF
/// merged across text nodes). Visual glyph clusters are not ordered under
/// bidi, so two binary searches are used instead of a stateful cursor.
fn overlapping_output_ranges<'a, T>(items: &'a [T], target: &Range<usize>) -> &'a [T]
where
    T: HasInlineOutputRange,
{
    if target.is_empty() {
        return &items[..0];
    }
    let start = items.partition_point(|item| item.output_range().end <= target.start);
    let end = start + items[start..].partition_point(|item| item.output_range().start < target.end);
    &items[start..end]
}

fn output_ranges_are_monotonic<T>(items: &[T]) -> bool
where
    T: HasInlineOutputRange,
{
    items.windows(2).all(|pair| {
        pair[0].output_range().start <= pair[1].output_range().start
            && pair[0].output_range().end <= pair[1].output_range().end
    })
}

pub(crate) fn prepare_inline_contexts<N>(
    world: &mut LayoutWorld<N>,
    services: &mut DocumentLayoutServices,
) where
    N: Copy + Debug + Eq + Hash,
{
    for layout_box in &mut world.boxes {
        layout_box.inline_layout = None;
        layout_box.inline_context_owner = None;
        layout_box.inline_flattened = false;
        layout_box.inline_static_position = None;
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
    items: Vec<InlineLogicalItem>,
    source_map: Vec<InlineSourceMapEntry>,
    root_style: LayoutBoxId,
}

fn intern_resolved_inline_style(
    styles: &mut Vec<TextStyle<'static, 'static, TextBrush>>,
    style_parents: &mut Vec<LayoutBoxId>,
    style_samples: &mut Vec<Option<char>>,
    style: TextStyle<'static, 'static, TextBrush>,
    structural_parent: LayoutBoxId,
    sample: Option<char>,
) -> usize {
    let style_slot = styles
        .iter()
        .enumerate()
        .position(|(index, candidate)| {
            *candidate == style && style_parents[index] == structural_parent
        })
        .unwrap_or_else(|| {
            let index = styles.len();
            styles.push(style);
            style_parents.push(structural_parent);
            style_samples.push(None);
            index
        });
    if style_samples[style_slot].is_none() {
        style_samples[style_slot] = sample;
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
    /// Normalize DOM order first, then adapt forced-break fragment ownership
    /// to Parley's byte anchors. Synthetic bidi controls are real intervening
    /// items; consecutive close tags and empty text are the only items that
    /// Blink absorbs into the preceding forced-break fragment.
    fn projected_object_anchors(&self) -> Vec<usize> {
        let mut anchors = self
            .objects
            .iter()
            .map(|(index, _, _)| *index)
            .collect::<Vec<_>>();
        let mut forced_break = None;
        for item in &self.items {
            match *item {
                InlineLogicalItem::Text(index) => {
                    let unit = &self.units[index];
                    forced_break = (!unit.control && &self.text[unit.output_range.clone()] == "\n")
                        .then_some(unit.output_range.start);
                }
                InlineLogicalItem::Object(index) => {
                    if self.objects[index].1.role == InlineObjectRole::EndEdge {
                        if let Some(anchor) = forced_break {
                            anchors[index] = anchor;
                        }
                    } else {
                        forced_break = None;
                    }
                }
            }
        }
        anchors
    }

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
        let mut style_parents = Vec::new();
        let mut style_samples = Vec::new();
        let mut resolved_runs = Vec::<(Range<usize>, usize)>::new();
        for unit in &self.units {
            let mut base_style = world.boxes[unit.style_box.index()]
                .style
                .parley_text_style();
            // `vertical-align` belongs to the structural inline box, not to
            // each descendant glyph. Keep glyphs baseline-aligned within their
            // direct box state; closing that state moves the complete subtree.
            base_style.brush.paint = !unit.control;
            let structural_parent = unit.ancestors.last().copied().unwrap_or(self.root_style);
            if !parley.requires_character_font_resolution(&base_style) {
                let sample = (!unit.control)
                    .then(|| self.text[unit.output_range.clone()].chars().next())
                    .flatten();
                parley.resolve_font_families(&mut base_style, None);
                let style_slot = intern_resolved_inline_style(
                    &mut styles,
                    &mut style_parents,
                    &mut style_samples,
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
                    &mut style_parents,
                    &mut style_samples,
                    style,
                    structural_parent,
                    (!unit.control).then_some(character),
                );
                append_resolved_inline_run(&mut resolved_runs, start..end, style_slot);
            }
        }
        let mut builder = parley.layout_context.style_run_builder(
            &mut parley.font_context,
            &self.text,
            1.0,
            quantize,
        );
        let style_indices = styles
            .iter()
            .map(|style| builder.push_style(style.clone()))
            .collect::<Vec<_>>();
        if resolved_runs.is_empty() {
            let style_index = builder.push_style(root_text_style.clone());
            builder.push_style_run(style_index, 0..0);
        } else {
            for (range, style_slot) in &resolved_runs {
                builder.push_style_run(style_indices[*style_slot], range.clone());
            }
        }
        let object_anchors = self.projected_object_anchors();
        for (object_id, (_, object, kind)) in self.objects.iter().enumerate() {
            builder.push_inline_box_with_bidi(
                InlineBox {
                    id: u64::try_from(object_id).expect("one IFC exceeded the u64 object limit"),
                    kind: *kind,
                    index: object_anchors[object_id],
                    width: 0.0,
                    height: 0.0,
                    baseline: None,
                    vertical_align: parley::VerticalAlign::default(),
                },
                object.role.parley_bidi(),
            );
        }
        let layout = builder.build(&self.text);
        let font_metrics = styles
            .iter()
            .zip(style_samples)
            .map(|(style, sample)| parley.inline_font_metrics(style, sample))
            .collect();
        let parent_strut = measure_inline_strut(parley, root_text_style.clone(), quantize);
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
                strut: measure_inline_strut(parley, style, quantize),
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
            measurement_layout: Some(layout),
            laid_out: None,
            content_widths: InlineContentWidthsMemo::default(),
            text_units: self.units,
            source_map: self.source_map,
            selection,
            objects,
            font_metrics,
            parent_strut,
            uses_quirks_line_height: world.quirks_mode != style::context::QuirksMode::NoQuirks,
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
) -> Option<InlineStrutMetrics> {
    let metrics = parley.inline_font_metrics(&style, None)?;
    Some(inline_strut_metrics(metrics, quantize))
}

fn inline_strut_metrics(metrics: InlineFontMetrics, quantize: bool) -> InlineStrutMetrics {
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
    InlineStrutMetrics {
        line_ascent: ascent + leading_above,
        line_descent: descent + leading_below,
        text_ascent: ascent,
        text_descent: descent,
        x_height: metrics.x_height,
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
    normalizer.close_inline(id, ancestors, vertical_align);
}

struct PendingWhitespace {
    output_index: usize,
    unit_index: usize,
    object_index: usize,
    item_index: usize,
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

/// Keeps text/control/object order even when several objects share a byte
/// offset. Indices refer to their normalized input records, not Parley IDs.
#[derive(Clone, Copy, Debug)]
enum InlineLogicalItem {
    Text(usize),
    Object(usize),
}

#[derive(Clone, Copy, Debug)]
struct InlineBidiContext {
    owner: LayoutBoxId,
    enter: char,
    exit: char,
}

struct InlineNormalizer {
    root_style: LayoutBoxId,
    text: String,
    units: Vec<InlineTextUnit>,
    objects: Vec<(usize, InlineObject, InlineBoxKind)>,
    items: Vec<InlineLogicalItem>,
    bidi_contexts: Vec<InlineBidiContext>,
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
            items: Vec::new(),
            bidi_contexts: Vec::new(),
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
                self.append_forced_break(style_box, ancestors, sources);
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
                if character == '\n' {
                    self.append_forced_break(style_box, ancestors, sources);
                } else {
                    self.append_unit(style_box, character, ancestors, sources, false);
                }
                if mode == InlineWhiteSpaceCollapse::BreakSpaces && character == ' ' {
                    // Parley 0.10 has no CSS `break-spaces` mode. U+200B adds
                    // the required opportunity after every preserved space;
                    // its control brush keeps it out of paint and source
                    // fragments while the actual space remains measurable.
                    let unit_index = self.units.len();
                    self.append_unit(style_box, '\u{200B}', ancestors, Vec::new(), true);
                    self.units[unit_index].break_spaces_opportunity = true;
                }
                self.line_has_content = character != '\n';
            }
            InlineWhiteSpaceCollapse::Collapse | InlineWhiteSpaceCollapse::PreserveBreaks => {
                self.flush_pending();
                self.append_unit(style_box, character, ancestors, sources, false);
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
            item_index: self.items.len(),
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
        for item in &mut self.items[pending.item_index..] {
            if let InlineLogicalItem::Text(index) = item {
                *index += 1;
            }
        }
        self.items.insert(
            pending.item_index,
            InlineLogicalItem::Text(pending.unit_index),
        );
        self.units.insert(
            pending.unit_index,
            InlineTextUnit {
                output_range: pending.output_index..pending.output_index + 1,
                style_box: pending.style_box,
                ancestors: pending.ancestors,
                sources: pending.sources,
                control: false,
                collapsible_whitespace: true,
                break_spaces_opportunity: false,
            },
        );
    }

    fn hard_break(&mut self, box_id: LayoutBoxId, ancestors: &[LayoutBoxId]) {
        self.flush_pending_carriage_return();
        self.pending = None;
        self.append_forced_break(box_id, ancestors, Vec::new());
    }

    fn append_forced_break(
        &mut self,
        box_id: LayoutBoxId,
        ancestors: &[LayoutBoxId],
        sources: Vec<SourceOrigin>,
    ) {
        // CSS bidi contexts end before a paragraph break and resume after it.
        // Associate synthetic controls with the break, without DOM sources.
        for index in (0..self.bidi_contexts.len()).rev() {
            let exit = self.bidi_contexts[index].exit;
            self.append_unit(box_id, exit, ancestors, Vec::new(), true);
        }
        self.append_unit(box_id, '\n', ancestors, sources, false);
        for index in 0..self.bidi_contexts.len() {
            let enter = self.bidi_contexts[index].enter;
            self.append_unit(box_id, enter, ancestors, Vec::new(), true);
        }
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
        self.flush_pending_carriage_return();
        // CSS Writing Modes injects the opening bidi controls outside the
        // inline box boundary. Keep the opaque item order aligned with
        // Blink's InlineItemsBuilder: enter bidi context, then open the tag.
        for (enter, exit) in bidi_controls(bidi, direction) {
            self.append_unit(box_id, enter, ancestors, Vec::new(), true);
            self.bidi_contexts.push(InlineBidiContext {
                owner: box_id,
                enter,
                exit,
            });
        }
        self.push_object(
            box_id,
            InlineObjectRole::StartEdge,
            InlineBoxKind::InFlow,
            ancestors,
            vertical_align,
        );
    }

    fn close_inline(
        &mut self,
        box_id: LayoutBoxId,
        ancestors: &[LayoutBoxId],
        vertical_align: InlineVerticalAlign,
    ) {
        // Close the inline box before leaving its injected bidi context.
        self.push_object(
            box_id,
            InlineObjectRole::EndEdge,
            InlineBoxKind::InFlow,
            ancestors,
            vertical_align,
        );
        while self
            .bidi_contexts
            .last()
            .is_some_and(|context| context.owner == box_id)
        {
            let context = self
                .bidi_contexts
                .pop()
                .expect("checked active bidi context");
            self.append_unit(box_id, context.exit, ancestors, Vec::new(), true);
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
        // Absolutely positioned descendants do not interrupt CSS whitespace
        // collapsing or make an otherwise empty line non-empty.
        // In particular, a hidden loading hint before an inline button must
        // not preserve a leading space and create an extra line beside a float.
        if matches!(role, InlineObjectRole::Atomic | InlineObjectRole::Float) {
            self.flush_pending();
            self.line_has_content = true;
        }
        self.items
            .push(InlineLogicalItem::Object(self.objects.len()));
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
        control: bool,
    ) {
        let start = self.text.len();
        self.text.push(character);
        self.items.push(InlineLogicalItem::Text(self.units.len()));
        self.units.push(InlineTextUnit {
            output_range: start..self.text.len(),
            style_box,
            ancestors: ancestors.to_vec(),
            sources,
            control,
            collapsible_whitespace: false,
            break_spaces_opportunity: false,
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
                })
            })
            .collect();
        InlineBuildInput {
            text: self.text,
            units: self.units,
            objects: self.objects,
            items: self.items,
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

fn bidi_controls(bidi: InlineUnicodeBidi, direction: InlineDirection) -> Vec<(char, char)> {
    let (embed, override_control, isolate) = match direction {
        InlineDirection::Ltr => ('\u{202A}', '\u{202D}', '\u{2066}'),
        InlineDirection::Rtl => ('\u{202B}', '\u{202E}', '\u{2067}'),
    };
    match bidi {
        InlineUnicodeBidi::Normal => Vec::new(),
        InlineUnicodeBidi::Embed => vec![(embed, '\u{202C}')],
        InlineUnicodeBidi::Isolate => vec![(isolate, '\u{2069}')],
        InlineUnicodeBidi::BidiOverride => vec![(override_control, '\u{202C}')],
        InlineUnicodeBidi::IsolateOverride => {
            vec![(isolate, '\u{2069}'), (override_control, '\u{202C}')]
        }
        InlineUnicodeBidi::Plaintext => vec![('\u{2068}', '\u{2069}')],
    }
}

fn is_combining_mark(character: char) -> bool {
    matches!(character as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_output_range_lookup_handles_duplicates_gaps_and_bidi_order() {
        let source = |output_range: Range<usize>| InlineSourceMapEntry {
            output_range,
            box_id: LayoutBoxId::from_index(1),
            source_byte_range: 0..1,
            source_utf16_range: 0..1,
        };
        let entries = vec![
            source(0..1),
            source(1..2),
            source(1..2),
            source(2..5),
            source(7..9),
        ];
        assert!(output_ranges_are_monotonic(&entries));
        assert!(!output_ranges_are_monotonic(&[source(1..3), source(0..4),]));
        assert!(!output_ranges_are_monotonic(&[source(0..4), source(1..3),]));
        let ranges = |query: Range<usize>| {
            overlapping_output_ranges(&entries, &query)
                .iter()
                .map(|entry| entry.output_range.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(ranges(1..3), vec![1..2, 1..2, 2..5]);
        // Visual clusters may query logical text in either direction.
        assert_eq!(ranges(7..8), vec![7..9]);
        assert_eq!(ranges(0..1), vec![0..1]);
        assert!(ranges(5..7).is_empty());
        assert!(ranges(2..2).is_empty());
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
        let fallback = parley::FontMetrics {
            ascent: 18.0,
            descent: 6.0,
            leading: 6.0,
            ..parley::FontMetrics::fallback(0.0)
        };

        let explicit = glyph_line_bounds(Some(primary), &fallback, 30.0, false);
        assert_eq!(explicit.top, -8.0);
        assert_eq!(explicit.bottom, 2.0);

        let normal = glyph_line_bounds(Some(primary), &fallback, 30.0, true);
        assert_eq!(normal.top, -21.0);
        assert_eq!(normal.bottom, 9.0);
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

    #[test]
    fn closing_inline_edges_remain_on_the_forced_break_line() {
        for (bidi, reopen_inline, inner_line, outer_line) in [
            (InlineUnicodeBidi::Normal, false, 0, 0),
            (InlineUnicodeBidi::Embed, false, 1, 1),
            (InlineUnicodeBidi::Normal, true, 0, 1),
        ] {
            let root = LayoutBoxId::from_index(0);
            let outer = LayoutBoxId::from_index(1);
            let inner = LayoutBoxId::from_index(2);
            let atom = LayoutBoxId::from_index(3);
            let reopened = LayoutBoxId::from_index(4);
            let align = InlineVerticalAlign::default();
            let mut normalizer = InlineNormalizer::new(root);
            normalizer.open_inline(
                outer,
                InlineUnicodeBidi::Normal,
                InlineDirection::Ltr,
                &[],
                align,
            );
            normalizer.open_inline(inner, bidi, InlineDirection::Ltr, &[outer], align);
            normalizer.push_object(
                atom,
                InlineObjectRole::Atomic,
                InlineBoxKind::InFlow,
                &[outer, inner],
                align,
            );
            normalizer.hard_break(inner, &[outer, inner]);
            normalizer.close_inline(inner, &[outer], align);
            if reopen_inline {
                normalizer.open_inline(
                    reopened,
                    InlineUnicodeBidi::Normal,
                    InlineDirection::Ltr,
                    &[outer],
                    align,
                );
                normalizer.close_inline(reopened, &[outer], align);
            }
            normalizer.close_inline(outer, &[], align);
            normalizer.hard_break(root, &[]);
            let input = normalizer.finish();

            let mut font_context = parley::FontContext::new();
            let mut layout_context = parley::LayoutContext::<TextBrush>::new();
            let mut builder =
                layout_context.style_run_builder(&mut font_context, &input.text, 1.0, true);
            let style = builder.push_style(TextStyle::default());
            builder.push_style_run(style, ..);
            let anchors = input.projected_object_anchors();
            for (id, (_, object, kind)) in input.objects.iter().enumerate() {
                builder.push_inline_box_with_bidi(
                    InlineBox {
                        id: id as u64,
                        kind: *kind,
                        index: anchors[id],
                        width: 1.0,
                        height: 20.0,
                        baseline: None,
                        vertical_align: parley::VerticalAlign::default(),
                    },
                    object.role.parley_bidi(),
                );
            }
            let mut layout = builder.build(&input.text);
            layout.break_all_lines(None);

            for (box_id, expected_line) in [(inner, inner_line), (outer, outer_line)] {
                let object_id = input
                    .objects
                    .iter()
                    .position(|(_, object, _)| {
                        object.box_id == box_id && object.role == InlineObjectRole::EndEdge
                    })
                    .unwrap() as u64;
                let actual_line = layout.lines().position(|line| {
                    line.items().any(|item| {
                        matches!(item, PositionedLayoutItem::InlineBox(item) if item.id == object_id)
                    })
                });
                assert_eq!(
                    actual_line,
                    Some(expected_line),
                    "{bidi:?}, reopen={reopen_inline}, box={box_id:?}"
                );
            }
        }
    }

    #[test]
    fn preserved_breaks_balance_nested_bidi_contexts_without_synthetic_dom_sources() {
        for mode in [
            InlineWhiteSpaceCollapse::Preserve,
            InlineWhiteSpaceCollapse::PreserveBreaks,
            InlineWhiteSpaceCollapse::BreakSpaces,
        ] {
            let root = LayoutBoxId::from_index(0);
            let outer = LayoutBoxId::from_index(1);
            let inner = LayoutBoxId::from_index(2);
            let text = LayoutBoxId::from_index(3);
            let align = InlineVerticalAlign::default();
            let mut normalizer = InlineNormalizer::new(root);
            normalizer.open_inline(
                outer,
                InlineUnicodeBidi::IsolateOverride,
                InlineDirection::Rtl,
                &[],
                align,
            );
            normalizer.open_inline(
                inner,
                InlineUnicodeBidi::Embed,
                InlineDirection::Ltr,
                &[outer],
                align,
            );
            normalizer.push_text(
                text,
                "a\r\nb",
                mode,
                InlineTextTransform::None,
                &[outer, inner],
            );
            normalizer.close_inline(inner, &[outer], align);
            normalizer.close_inline(outer, &[], align);
            assert!(normalizer.bidi_contexts.is_empty());
            let input = normalizer.finish();
            assert_eq!(
                input.text,
                "\u{2067}\u{202e}\u{202a}a\u{202c}\u{202c}\u{2069}\n\u{2067}\u{202e}\u{202a}b\u{202c}\u{202c}\u{2069}"
            );
            let newline = input.text.find('\n').unwrap();
            assert_eq!(input.source_map.len(), 4);
            assert_eq!(input.source_map[1].output_range, newline..newline + 1);
            assert_eq!(input.source_map[2].output_range, newline..newline + 1);
            for (index, source) in input.source_map.iter().enumerate() {
                assert_eq!(source.box_id, text);
                assert_eq!(source.source_byte_range, index..index + 1);
                assert_eq!(source.source_utf16_range, index..index + 1);
            }
            assert!(
                input
                    .units
                    .iter()
                    .filter(|unit| unit.control)
                    .all(|unit| unit.sources.is_empty())
            );
            assert!(output_ranges_are_monotonic(&input.source_map));
        }
    }

    #[test]
    fn pending_carriage_return_breaks_before_entering_the_next_bidi_context() {
        let root = LayoutBoxId::from_index(0);
        let outer = LayoutBoxId::from_index(1);
        let inner = LayoutBoxId::from_index(2);
        let text = LayoutBoxId::from_index(3);
        let align = InlineVerticalAlign::default();
        let mut normalizer = InlineNormalizer::new(root);
        normalizer.open_inline(
            outer,
            InlineUnicodeBidi::Embed,
            InlineDirection::Ltr,
            &[],
            align,
        );
        normalizer.push_text(
            text,
            "\r",
            InlineWhiteSpaceCollapse::Preserve,
            InlineTextTransform::None,
            &[outer],
        );
        normalizer.open_inline(
            inner,
            InlineUnicodeBidi::Isolate,
            InlineDirection::Rtl,
            &[outer],
            align,
        );
        normalizer.close_inline(inner, &[outer], align);
        normalizer.close_inline(outer, &[], align);
        let input = normalizer.finish();
        assert_eq!(
            input.text,
            "\u{202a}\u{202c}\n\u{202a}\u{2067}\u{2069}\u{202c}"
        );
        assert_eq!(input.source_map.len(), 1);
        assert_eq!(input.source_map[0].source_byte_range, 0..1);
        assert_eq!(input.source_map[0].source_utf16_range, 0..1);
        let anchors = input.projected_object_anchors();
        assert_eq!(anchors[1], "\u{202a}\u{202c}\n\u{202a}\u{2067}".len());
    }

    #[test]
    fn parley_shaped_layout_can_be_rebroken_across_probe_widths() {
        let text = "alpha beta gamma delta epsilon";
        let mut font_context = parley::FontContext::new();
        let mut layout_context = parley::LayoutContext::<TextBrush>::new();
        let mut builder = layout_context.style_run_builder(&mut font_context, text, 1.0, true);
        let style = builder.push_style(TextStyle::default());
        builder.push_style_run(style, ..);
        let shaped = builder.build(text);
        let mut reused = shaped.clone();

        let signature = |layout: &Layout<TextBrush>| {
            (
                layout.width(),
                layout.full_width(),
                layout.height(),
                layout
                    .lines()
                    .map(|line| (line.text_range(), line.break_reason(), *line.metrics()))
                    .collect::<Vec<_>>(),
            )
        };

        // Exercise the same order used by Taffy: intrinsic constraints may
        // alternate with definite widths, and a scrollbar correction may ask
        // the accepted paragraph to break at another width later in the pass.
        for width in [45.0, 160.0, 65.0, 160.0] {
            reused.break_all_lines(Some(width));
            let mut fresh = shaped.clone();
            fresh.break_all_lines(Some(width));
            assert_eq!(signature(&reused), signature(&fresh));
        }
    }

    #[test]
    fn parley_probe_reset_removes_justification_before_intrinsic_widths() {
        let text = "alpha beta gamma delta epsilon zeta eta theta";
        let mut font_context = parley::FontContext::new();
        let mut layout_context = parley::LayoutContext::<TextBrush>::new();
        let mut builder = layout_context.style_run_builder(&mut font_context, text, 1.0, true);
        let style = builder.push_style(TextStyle::default());
        builder.push_style_run(style, ..);
        let shaped = builder.build(text);
        let expected = shaped.calculate_content_widths();
        let mut reused = shaped.clone();

        reused.break_all_lines(Some(120.0));
        reused.align(
            parley::Alignment::Justify,
            parley::AlignmentOptions {
                align_when_overflowing: false,
                last_line_alignment: None,
            },
        );
        let justified = reused.calculate_content_widths();
        assert_eq!(
            justified.max, expected.max,
            "Parley keeps line justification separate from intrinsic shaped advances"
        );

        reset_inline_layout_for_probe(&mut reused);
        let restored = reused.calculate_content_widths();
        assert_eq!(restored.min, expected.min);
        assert_eq!(restored.max, expected.max);
        assert!(
            reused.is_empty(),
            "a fresh probe must not retain the previous line output"
        );
    }

    #[test]
    fn pure_text_content_width_cache_is_keyed_and_object_probes_bypass_it() {
        let text = "alpha beta gamma delta";
        let mut font_context = parley::FontContext::new();
        let mut layout_context = parley::LayoutContext::<TextBrush>::new();
        let mut builder = layout_context.style_run_builder(&mut font_context, text, 1.0, true);
        let style = builder.push_style(TextStyle::default());
        builder.push_style_run(style, ..);
        let layout = builder.build(text);
        let options = parley::IndentOptions::default();
        let mut memo = InlineContentWidthsMemo::default();

        let first = memo.content_widths_for_probe(&layout, 0.0, options);
        let second = memo.content_widths_for_probe(&layout, 0.0, options);
        assert_eq!(first.min, second.min);
        assert_eq!(first.max, second.max);
        assert_eq!(memo.hits, 1);

        let changed_options = parley::IndentOptions {
            each_line: true,
            hanging: false,
        };
        memo.content_widths_for_probe(&layout, 12.0, changed_options);
        assert_eq!(memo.hits, 1);
        assert_eq!(
            memo.entry.expect("cache must be populated").key,
            InlineContentWidthsCacheKey::new(12.0, changed_options),
        );

        let cached = memo.entry;
        let mut object_builder =
            layout_context.style_run_builder(&mut font_context, text, 1.0, true);
        let object_style = object_builder.push_style(TextStyle::default());
        object_builder.push_style_run(object_style, ..);
        object_builder.push_inline_box(InlineBox {
            id: 0,
            kind: InlineBoxKind::InFlow,
            index: 5,
            width: 20.0,
            height: 10.0,
            baseline: None,
            vertical_align: parley::VerticalAlign::default(),
        });
        let mut object_layout = object_builder.build(text);
        let object_first = memo.content_widths_for_probe(&object_layout, 12.0, changed_options);
        object_layout.inline_boxes_mut().next().unwrap().width = 60.0;
        let object_second = memo.content_widths_for_probe(&object_layout, 12.0, changed_options);
        assert!(object_second.max > object_first.max);
        assert_eq!(memo.hits, 1);
        assert_eq!(
            memo.entry.map(|entry| entry.key),
            cached.map(|entry| entry.key),
            "a dynamic inline-object probe must neither reuse nor replace the pure-text cache",
        );
    }

    #[test]
    fn intrinsic_line_summary_matches_materialized_line_placements() {
        let root = LayoutBoxId::from_index(0);
        let text = "first line\nsecond line";
        let mut font_context = parley::FontContext::new();
        let mut layout_context = parley::LayoutContext::<TextBrush>::new();
        let mut builder = layout_context.style_run_builder(&mut font_context, text, 1.0, true);
        let style = builder.push_style(TextStyle::default());
        builder.push_style_run(style, ..);
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(80.0));

        let context = InlineFormattingContext {
            root_style: root,
            measurement_layout: Some(layout.clone()),
            laid_out: None,
            content_widths: InlineContentWidthsMemo::default(),
            text_units: Vec::new(),
            source_map: Vec::new(),
            selection: None,
            objects: Vec::new(),
            font_metrics: vec![None],
            parent_strut: None,
            uses_quirks_line_height: false,
            root_includes_used_font_metrics: false,
            style_parents: vec![root],
            structural_boxes: Vec::new(),
            line_placements: Vec::new(),
            fragments: InlineFragments::default(),
        };
        let summary = measure_inline_lines(&context, &layout, &[], &[], &[]);
        let (placements, materialized_summary) =
            build_inline_line_placements(&context, &layout, &[], &[], &[]);

        assert_eq!(summary, materialized_summary);
        assert_eq!(placements.len(), layout.lines().len());
        assert_eq!(
            placements
                .iter()
                .find(|line| !line.phantom)
                .map(|line| line.baseline),
            summary.first_baseline
        );
        assert_eq!(
            placements
                .iter()
                .rev()
                .find(|line| !line.phantom)
                .map(|line| line.baseline),
            summary.last_baseline
        );
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
                },
                InlineSourceMapEntry {
                    output_range: 1..2,
                    box_id: first,
                    source_byte_range: 1..2,
                    source_utf16_range: 1..2,
                },
                InlineSourceMapEntry {
                    output_range: 1..2,
                    box_id: second,
                    source_byte_range: 0..1,
                    source_utf16_range: 0..1,
                },
                InlineSourceMapEntry {
                    output_range: 2..3,
                    box_id: second,
                    source_byte_range: 1..2,
                    source_utf16_range: 1..2,
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
    fn break_spaces_inserts_non_source_break_controls_after_preserved_spaces() {
        let text = LayoutBoxId::from_index(1);
        let input = normalize(
            &[(text, "A  B")],
            InlineWhiteSpaceCollapse::BreakSpaces,
            InlineTextTransform::None,
        );

        assert_eq!(input.text, "A \u{200B} \u{200B}B");
        assert_eq!(input.units.iter().filter(|unit| unit.control).count(), 2);
        assert_eq!(
            input
                .units
                .iter()
                .filter(|unit| unit.break_spaces_opportunity)
                .count(),
            2
        );
        assert_eq!(input.source_map.len(), 4);
        assert!(
            input
                .source_map
                .iter()
                .all(|entry| { &input.text[entry.output_range.clone()] != "\u{200B}" })
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
        normalizer.close_inline(first_inline, &[], InlineVerticalAlign::default());
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
        normalizer.close_inline(second_inline, &[], InlineVerticalAlign::default());
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
