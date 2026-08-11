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

#[derive(Clone, Debug)]
pub(crate) struct InlineObject {
    pub(crate) box_id: LayoutBoxId,
    pub(crate) role: InlineObjectRole,
    pub(crate) ancestors: Vec<LayoutBoxId>,
    pub(crate) vertical_align: InlineVerticalAlign,
}

#[derive(Clone, Debug)]
pub(crate) struct InlineFormattingContext {
    pub(crate) unbroken: Layout<TextBrush>,
    pub(crate) laid_out: Option<Layout<TextBrush>>,
    pub(crate) text_units: Vec<InlineTextUnit>,
    pub(crate) source_map: Vec<InlineSourceMapEntry>,
    pub(crate) selection: Option<InlineSelection>,
    pub(crate) objects: Vec<InlineObject>,
    /// Primary-font metrics indexed by Parley's style index. Glyph runs may
    /// use fallback fonts, but their CSSOM rectangles must not inherit those
    /// fallback metrics.
    pub(crate) font_metrics: Vec<Option<InlineFontMetrics>>,
    /// The IFC owner's primary-font strut used while reconstructing CSS line
    /// baselines. Fallback glyph fonts must not replace its line height or
    /// x-height.
    pub(crate) parent_strut: Option<InlineStrutMetrics>,
    /// Font strut of each object's nearest structural inline parent. CSS
    /// non-baseline alignment is relative to that parent inline box, not
    /// necessarily to the IFC owner.
    pub(crate) object_alignment_parent_struts: Vec<Option<InlineStrutMetrics>>,
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
    pub(crate) rect: PaintRect,
    pub(crate) baseline: f32,
    /// CSS phantom line boxes retain positions for their inline descendants,
    /// but do not contribute height, baselines, or block margin-collapse
    /// barriers.
    pub(crate) phantom: bool,
    content_offset: f32,
    item_offsets: Vec<f32>,
    glyph_offsets: Vec<InlineGlyphOffset>,
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
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct InlineGlyphOffset {
    run_index: usize,
    style_index: usize,
    offset: f32,
}

impl InlineFormattingContext {
    pub(crate) fn object(&self, id: u64) -> Option<&InlineObject> {
        usize::try_from(id)
            .ok()
            .and_then(|index| self.objects.get(index))
    }

    fn object_alignment_parent_strut(&self, id: u64) -> Option<InlineStrutMetrics> {
        usize::try_from(id)
            .ok()
            .and_then(|index| self.object_alignment_parent_struts.get(index))
            .copied()
            .flatten()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InlineFragments {
    pub(crate) lines: Vec<InlineLineFragment>,
    pub(crate) text: Vec<InlineSourceFragment>,
    pub(crate) boxes: Vec<InlineBoxFragment>,
}

impl InlineFragments {
    pub(crate) fn translate_block_axis(&mut self, offset: f32) {
        for line in &mut self.lines {
            line.rect.y += offset;
            line.baseline += offset;
        }
        for text in &mut self.text {
            text.rect.y += offset;
        }
        for inline_box in &mut self.boxes {
            inline_box.rect.y += offset;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineLineFragment {
    pub(crate) line_index: usize,
    pub(crate) rect: PaintRect,
    pub(crate) baseline: f32,
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
    let mut fragments = InlineFragments::default();
    let mut box_fragments = BTreeMap::<(usize, usize), FragmentAccumulator>::new();
    let mut source_fragments = BTreeMap::<SourceFragmentKey, FragmentAccumulator>::new();

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
            baseline: placement.map_or(metrics.baseline, |placement| placement.baseline),
        });

        for run in line.runs() {
            let run_metrics = run.metrics();
            for cluster in run.visual_clusters() {
                let range = cluster.text_range();
                let style_index = cluster
                    .glyphs()
                    .next()
                    .map(|glyph| glyph.style_index())
                    .unwrap_or_default();
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
                for unit in context
                    .text_units
                    .iter()
                    .filter(|unit| ranges_overlap(&unit.output_range, &range))
                {
                    for ancestor in &unit.ancestors {
                        box_fragments
                            .entry((ancestor.index(), line_index))
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
                    accumulator.include(rect);
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
            metrics.trailing_whitespace > 0.0
                && metrics.advance > width + tolerance
                && context.text_units.iter().any(|unit| {
                    unit.break_spaces_opportunity && ranges_overlap(&unit.output_range, &line_range)
                })
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

/// Builds the pass-local vertical placement sidecar that Parley 0.10 does not
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
        // Parley's block min/max coordinates are selection bounds and may
        // extend outside the CSS line box when leading is negative. CSS
        // top/bottom alignment and block sizing use the accumulated line
        // height instead.
        let raw_top = unadjusted_line_top;
        let raw_bottom = raw_top + metrics.line_height.max(0.0);
        let mut geometries = line
            .items()
            .map(|item| match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    let run = glyph_run.run();
                    let run_metrics = run.metrics();
                    let paint = glyph_run.style().brush.paint;
                    let style_index = glyph_run.glyphs().next().map(|glyph| glyph.style_index());
                    let primary_strut = style_index
                        .and_then(|index| context.font_metrics.get(index).copied().flatten())
                        .map(|metrics| inline_strut_metrics(metrics, true));
                    let (top, bottom, baseline_ascent) = primary_strut.map_or_else(
                        || {
                            let half_leading = (run_metrics.line_height
                                - run_metrics.ascent
                                - run_metrics.descent)
                                * 0.5;
                            (
                                glyph_run.baseline() - run_metrics.ascent - half_leading,
                                glyph_run.baseline() + run_metrics.descent + half_leading,
                                run_metrics.ascent + half_leading,
                            )
                        },
                        |strut| {
                            (
                                glyph_run.baseline() - strut.line_ascent,
                                glyph_run.baseline() + strut.line_descent,
                                strut.line_ascent,
                            )
                        },
                    );
                    InlineItemVerticalGeometry {
                        top,
                        bottom,
                        baseline_ascent: Some(baseline_ascent),
                        uses_internal_atomic_baseline: false,
                        intrinsic_baseline_adjustment: 0.0,
                        vertical_align: glyph_run.style().brush.vertical_align,
                        // Parley may expose an empty root-style run next to
                        // float/out-of-flow placeholders. It carries the font
                        // style but no glyph geometry and is not in-flow line
                        // content by itself.
                        contributes_to_line: paint && style_index.is_some(),
                        creates_line: paint && style_index.is_some(),
                        alignment_parent_strut: None,
                        glyph_key: if paint {
                            style_index.map(|index| (run.index(), index))
                        } else {
                            None
                        },
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
                    let baseline_ascent =
                        internal_baseline_ascent.or_else(|| is_atomic.then_some(positioned.height));
                    InlineItemVerticalGeometry {
                        top: positioned.y,
                        bottom: positioned.y + positioned.height,
                        baseline_ascent,
                        uses_internal_atomic_baseline: internal_baseline_ascent.is_some(),
                        intrinsic_baseline_adjustment: internal_baseline_ascent
                            .map(|ascent| positioned.height - ascent)
                            .unwrap_or_default(),
                        vertical_align: object
                            .map(|object| object.vertical_align)
                            .unwrap_or_default(),
                        contributes_to_line: object
                            .is_some_and(|object| object.role == InlineObjectRole::Atomic),
                        creates_line: object.is_some_and(|object| match object.role {
                            InlineObjectRole::Atomic => true,
                            InlineObjectRole::StartEdge | InlineObjectRole::EndEdge => object_index
                                .and_then(|index| structural_edge_contributions.get(index))
                                .copied()
                                .unwrap_or(false),
                            InlineObjectRole::Float | InlineObjectRole::OutOfFlow => false,
                        }),
                        alignment_parent_strut: context
                            .object_alignment_parent_strut(positioned.id),
                        glyph_key: None,
                    }
                }
            })
            .collect::<Vec<_>>();
        let phantom = css_line_is_phantom(
            line.break_reason(),
            geometries.iter().any(|geometry| geometry.creates_line),
        );
        // Every non-phantom CSS line starts with the IFC owner's zero-width font strut.
        // Reconstruct it explicitly whenever this sidecar owns in-flow
        // vertical geometry. Float and out-of-flow placeholders do not create
        // a normal-flow line contribution and retain Parley's raw position.
        let reconstructs_parent_strut = !phantom && context.parent_strut.is_some();
        let root_parent_x_height = context
            .parent_strut
            .filter(|_| reconstructs_parent_strut)
            .map(|strut| strut.x_height)
            .or_else(|| {
                line.items().find_map(|item| match item {
                    PositionedLayoutItem::GlyphRun(glyph_run)
                        if glyph_run.style().brush.paint
                            && glyph_run.style().brush.vertical_align.kind
                                == LayoutInlineAlignment::Baseline
                            && glyph_run.style().brush.vertical_align.baseline_shift == 0.0 =>
                    {
                        glyph_run.run().metrics().x_height
                    }
                    PositionedLayoutItem::InlineBox(_) => None,
                    PositionedLayoutItem::GlyphRun(_) => None,
                })
            })
            .unwrap_or_else(|| (metrics.ascent * 0.5).max(0.0));
        // Parley baseline-aligns every inline box before Moli applies
        // CSS vertical-align. A tall atomic box can therefore inflate ascent
        // and create synthetic negative leading, shifting the text baseline
        // below the accumulated CSS line height. Undo only that atomic-box
        // artifact; genuine negative font leading remains on the glyph run.
        let has_atomic = line.items().any(|item| match item {
            PositionedLayoutItem::InlineBox(positioned) => context
                .object(positioned.id)
                .is_some_and(|object| object.role == InlineObjectRole::Atomic),
            PositionedLayoutItem::GlyphRun(_) => false,
        });
        let parley_baseline_adjustment = if has_atomic {
            metrics.leading.min(0.0) * 0.5
        } else {
            0.0
        };
        for geometry in &mut geometries {
            geometry.top += parley_baseline_adjustment;
            geometry.bottom += parley_baseline_adjustment;
        }
        let parley_parent_baseline = metrics.baseline + parley_baseline_adjustment;
        let consumes_internal_atomic_baseline = geometries.iter().any(|geometry| {
            geometry.uses_internal_atomic_baseline
                && geometry.vertical_align.kind == LayoutInlineAlignment::Baseline
                && geometry.vertical_align.baseline_shift == 0.0
        });
        let reconstructs_parent_baseline =
            consumes_internal_atomic_baseline || reconstructs_parent_strut;
        let baseline_ascent = reconstructs_parent_baseline
            .then(|| {
                geometries
                    .iter()
                    .filter(|geometry| {
                        geometry.contributes_to_line
                            && geometry.vertical_align.kind == LayoutInlineAlignment::Baseline
                            && geometry.vertical_align.baseline_shift == 0.0
                    })
                    .filter_map(|geometry| geometry.baseline_ascent)
                    .fold(
                        context
                            .parent_strut
                            .filter(|_| reconstructs_parent_strut)
                            .map(|strut| strut.line_ascent),
                        |maximum, ascent| {
                            Some(maximum.map_or(ascent, |maximum| maximum.max(ascent)))
                        },
                    )
            })
            .flatten();
        let parent_baseline_adjustment = baseline_ascent
            .map(|ascent| raw_top + ascent - parley_parent_baseline)
            .unwrap_or_default();
        let mut base_offsets = Vec::with_capacity(geometries.len());
        for geometry in &mut geometries {
            let intrinsic_baseline_adjustment =
                if geometry.vertical_align.kind == LayoutInlineAlignment::Baseline {
                    geometry.intrinsic_baseline_adjustment
                } else {
                    0.0
                };
            let offset = parent_baseline_adjustment + intrinsic_baseline_adjustment;
            geometry.top += offset;
            geometry.bottom += offset;
            base_offsets.push(parley_baseline_adjustment + offset);
        }
        let parent_baseline = parley_parent_baseline + parent_baseline_adjustment;
        let (parent_text_top, parent_text_bottom) = context
            .parent_strut
            .filter(|_| reconstructs_parent_strut)
            .map(|strut| {
                (
                    parent_baseline - strut.text_ascent,
                    parent_baseline + strut.text_descent,
                )
            })
            .or_else(|| {
                geometries
                    .iter()
                    .find(|geometry| {
                        geometry.glyph_key.is_some()
                            && geometry.vertical_align.kind == LayoutInlineAlignment::Baseline
                            && geometry.vertical_align.baseline_shift == 0.0
                    })
                    .map(|geometry| (geometry.top, geometry.bottom))
            })
            .unwrap_or((raw_top, raw_bottom));

        // First position baseline-, text-edge-, and middle-aligned content.
        // Top/bottom depend on the resulting aligned subtree and are resolved
        // in the second pass, matching the ordering used by browser IFCs.
        let (mut min_y, mut max_y) = if phantom {
            (raw_top, raw_top)
        } else {
            context
                .parent_strut
                .filter(|_| reconstructs_parent_strut)
                .map_or((raw_top, raw_bottom), |strut| {
                    (
                        parent_baseline - strut.line_ascent,
                        parent_baseline + strut.line_descent,
                    )
                })
        };
        let mut item_offsets = base_offsets;
        for (item_index, geometry) in geometries.iter().enumerate() {
            if matches!(
                geometry.vertical_align.kind,
                LayoutInlineAlignment::Top | LayoutInlineAlignment::Bottom
            ) {
                continue;
            }
            let (alignment_text_top, alignment_text_bottom, alignment_x_height) = geometry
                .alignment_parent_strut
                .map(|strut| {
                    (
                        parent_baseline - strut.text_ascent,
                        parent_baseline + strut.text_descent,
                        strut.x_height,
                    )
                })
                .unwrap_or((parent_text_top, parent_text_bottom, root_parent_x_height));
            let offset = non_edge_vertical_offset(
                geometry.vertical_align,
                parent_baseline,
                alignment_text_top,
                alignment_text_bottom,
                alignment_x_height,
                geometry.top,
                geometry.bottom,
            );
            item_offsets[item_index] += offset;
            if geometry.contributes_to_line {
                min_y = min_y.min(geometry.top + offset);
                max_y = max_y.max(geometry.bottom + offset);
            }
        }
        for (item_index, geometry) in geometries.iter().enumerate() {
            let offset = match geometry.vertical_align.kind {
                LayoutInlineAlignment::Top => {
                    min_y - geometry.top - geometry.vertical_align.baseline_shift
                }
                LayoutInlineAlignment::Bottom => {
                    max_y - geometry.bottom - geometry.vertical_align.baseline_shift
                }
                _ => continue,
            };
            item_offsets[item_index] += offset;
            if geometry.contributes_to_line {
                min_y = min_y.min(geometry.top + offset);
                max_y = max_y.max(geometry.bottom + offset);
            }
        }
        let line_height = (max_y - min_y).max(0.0);
        let line_content_offset = preceding_adjustment + raw_top - min_y;
        for offset in &mut item_offsets {
            *offset += line_content_offset;
        }
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
        placements.push(InlineLinePlacement {
            line_index,
            rect: PaintRect::new(
                metrics.inline_min_coord + metrics.offset,
                raw_top + preceding_adjustment,
                metrics.advance,
                line_height,
            ),
            baseline: parent_baseline + line_content_offset,
            phantom,
            content_offset: parley_baseline_adjustment
                + parent_baseline_adjustment
                + line_content_offset,
            item_offsets,
            glyph_offsets,
        });
        preceding_adjustment += line_height - (raw_bottom - raw_top);
        unadjusted_line_top += metrics.line_height.max(0.0);
    }

    (placements, preceding_adjustment)
}

#[derive(Clone, Copy, Debug)]
struct InlineItemVerticalGeometry {
    top: f32,
    bottom: f32,
    baseline_ascent: Option<f32>,
    uses_internal_atomic_baseline: bool,
    intrinsic_baseline_adjustment: f32,
    vertical_align: InlineVerticalAlign,
    /// Whether this item supplies block-axis geometry to the line.
    contributes_to_line: bool,
    /// Whether this item prevents the line from being a CSS phantom line box.
    /// Structural inline edges with non-zero inline-axis decorations create a
    /// line without themselves affecting its block-axis height.
    creates_line: bool,
    alignment_parent_strut: Option<InlineStrutMetrics>,
    glyph_key: Option<(usize, usize)>,
}

/// CSS line boxes ending in a preserved newline exist even when they contain
/// no paintable item. Parley's explicit break reason covers both preserved
/// segment breaks and the normalized `<br>` control.
fn css_line_is_phantom(break_reason: BreakReason, has_in_flow_content: bool) -> bool {
    !has_in_flow_content && break_reason != BreakReason::Explicit
}

fn non_edge_vertical_offset(
    vertical_align: InlineVerticalAlign,
    parent_baseline: f32,
    parent_text_top: f32,
    parent_text_bottom: f32,
    parent_x_height: f32,
    item_top: f32,
    item_bottom: f32,
) -> f32 {
    let baseline_shift = -vertical_align.baseline_shift;
    let alignment_shift = match vertical_align.kind {
        LayoutInlineAlignment::Baseline => 0.0,
        LayoutInlineAlignment::TextTop => parent_text_top - item_top,
        LayoutInlineAlignment::Middle => {
            parent_baseline - parent_x_height * 0.5 - (item_top + item_bottom) * 0.5
        }
        LayoutInlineAlignment::TextBottom => parent_text_bottom - item_bottom,
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
    source_map: Vec<InlineSourceMapEntry>,
    root_style: LayoutBoxId,
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
        parley.resolve_system_font_families(&mut root_text_style);
        let quantize = true;
        let mut styles = Vec::new();
        let mut style_slots = Vec::with_capacity(self.units.len());
        for unit in &self.units {
            let mut style = world.boxes[unit.style_box.index()]
                .style
                .parley_text_style();
            // `vertical-align` is not inherited, but moving an inline box
            // moves all of its descendants. Compose the structural inline
            // ancestry without forcing a shaping boundary for equivalent
            // paths.
            style.brush.vertical_align =
                compose_vertical_align(world, unit.ancestors.iter().copied());
            style.brush.paint = !unit.control;
            parley.resolve_system_font_families(&mut style);
            let style_slot = styles
                .iter()
                .position(|candidate| *candidate == style)
                .unwrap_or_else(|| {
                    let index = styles.len();
                    styles.push(style);
                    index
                });
            style_slots.push(style_slot);
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
        if self.units.is_empty() {
            let style_index = builder.push_style(root_text_style.clone());
            builder.push_style_run(style_index, 0..0);
        } else {
            let mut run_start = 0;
            let mut current_slot = style_slots[0];
            for (index, unit) in self.units.iter().enumerate().skip(1) {
                if style_slots[index] == current_slot {
                    continue;
                }
                builder.push_style_run(
                    style_indices[current_slot],
                    run_start..unit.output_range.start,
                );
                run_start = unit.output_range.start;
                current_slot = style_slots[index];
            }
            builder.push_style_run(style_indices[current_slot], run_start..self.text.len());
        }
        for (object_id, (byte_index, _, kind)) in self.objects.iter().enumerate() {
            builder.push_inline_box(InlineBox {
                id: u64::try_from(object_id).expect("one IFC exceeded the u64 object limit"),
                kind: *kind,
                index: *byte_index,
                width: 0.0,
                height: 0.0,
            });
        }
        let layout = builder.build(&self.text);
        let mut style_samples = vec![None; styles.len()];
        for (unit, style_slot) in self.units.iter().zip(&style_slots) {
            if style_samples[*style_slot].is_none() && !unit.control {
                style_samples[*style_slot] = self.text[unit.output_range.clone()].chars().next();
            }
        }
        let font_metrics = styles
            .iter()
            .zip(style_samples)
            .map(|(style, sample)| parley.inline_font_metrics(style, sample))
            .collect();
        let parent_strut = measure_inline_strut(parley, root_text_style.clone(), quantize);
        let object_alignment_parent_struts = self
            .objects
            .iter()
            .map(|(_, object, _)| {
                if object.vertical_align.kind == LayoutInlineAlignment::Baseline {
                    return None;
                }
                let parent = object.ancestors.last().copied().unwrap_or(self.root_style);
                if parent == self.root_style {
                    return parent_strut;
                }
                let mut style = world.boxes[parent.index()].style.parley_text_style();
                parley.resolve_system_font_families(&mut style);
                measure_inline_strut(parley, style, quantize)
            })
            .collect();
        let objects = self
            .objects
            .drain(..)
            .map(|(_, object, _)| object)
            .collect();
        InlineFormattingContext {
            unbroken: layout,
            laid_out: None,
            text_units: self.units,
            source_map: self.source_map,
            selection,
            objects,
            font_metrics,
            parent_strut,
            object_alignment_parent_struts,
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
        let vertical_align = compose_vertical_align(world, ancestors.iter().copied());
        normalizer.push_object(
            id,
            InlineObjectRole::Float,
            InlineBoxKind::CustomOutOfFlow,
            ancestors,
            vertical_align,
        );
        return;
    }

    let out_of_flow = world.boxes[id.index()].style.is_out_of_flow();
    if out_of_flow {
        let vertical_align = compose_vertical_align(world, ancestors.iter().copied());
        normalizer.push_object(
            id,
            InlineObjectRole::OutOfFlow,
            InlineBoxKind::OutOfFlow,
            ancestors,
            vertical_align,
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
        let vertical_align =
            compose_vertical_align(world, ancestors.iter().copied().chain(std::iter::once(id)));
        normalizer.push_object(
            id,
            InlineObjectRole::Atomic,
            InlineBoxKind::InFlow,
            ancestors,
            vertical_align,
        );
        return;
    }

    world.boxes[id.index()].inline_flattened = true;
    let vertical_align =
        compose_vertical_align(world, ancestors.iter().copied().chain(std::iter::once(id)));
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

fn compose_vertical_align<N>(
    world: &LayoutWorld<N>,
    path: impl IntoIterator<Item = LayoutBoxId>,
) -> InlineVerticalAlign
where
    N: Copy + Debug + Eq + Hash,
{
    path.into_iter()
        .fold(InlineVerticalAlign::default(), |alignment, box_id| {
            alignment.compose(world.boxes[box_id.index()].style.vertical_align())
        })
}

struct PendingWhitespace {
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
                self.append_unit(style_box, '\n', ancestors, sources, false);
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
                self.append_unit(style_box, character, ancestors, sources, false);
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
        self.append_unit(
            pending.style_box,
            ' ',
            &pending.ancestors,
            pending.sources,
            false,
        );
    }

    fn hard_break(&mut self, box_id: LayoutBoxId, ancestors: &[LayoutBoxId]) {
        self.flush_pending_carriage_return();
        self.pending = None;
        self.append_unit(box_id, '\n', ancestors, Vec::new(), false);
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
        self.push_object(
            box_id,
            InlineObjectRole::StartEdge,
            InlineBoxKind::InFlow,
            ancestors,
            vertical_align,
        );
        for control in bidi_open(bidi, direction) {
            self.append_unit(box_id, control, ancestors, Vec::new(), true);
        }
    }

    fn close_inline(
        &mut self,
        box_id: LayoutBoxId,
        bidi: InlineUnicodeBidi,
        ancestors: &[LayoutBoxId],
        vertical_align: InlineVerticalAlign,
    ) {
        for control in bidi_close(bidi) {
            self.append_unit(box_id, control, ancestors, Vec::new(), true);
        }
        self.push_object(
            box_id,
            InlineObjectRole::EndEdge,
            InlineBoxKind::InFlow,
            ancestors,
            vertical_align,
        );
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
        control: bool,
    ) {
        let start = self.text.len();
        self.text.push(character);
        self.units.push(InlineTextUnit {
            output_range: start..self.text.len(),
            style_box,
            ancestors: ancestors.to_vec(),
            sources,
            control,
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
                baseline,
                baseline - strut.text_ascent,
                baseline + strut.text_descent,
                strut.x_height,
                0.0,
                8.0,
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
