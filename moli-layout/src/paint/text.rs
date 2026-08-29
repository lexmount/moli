use parley::{Affinity, Cursor, PositionedLayoutItem, Selection};

use super::{PaintProjectionMetrics, cull::rects_intersect};
use crate::{
    LayoutBox, LayoutBoxId, LayoutRect, LayoutTransform2D, PaintBrush, PaintColor, PaintFragment,
    PaintGlyph, PaintGlyphRun, PaintShape, PaintSnapshot, PaintTextDecoration, PaintTextShadow,
    inline::{InlineFormattingContext, InlineObjectRole, InlinePaintBounds, InlineSelection},
};

const SELECTION_COLOR: PaintColor = PaintColor::new(180.0 / 255.0, 213.0 / 255.0, 1.0, 1.0);
// These expansion ratios and the 0.3 px cap follow Blitz's AnyRender text
// painter at d788124a. Moli records them only when Fontique/Parley asks
// for faux bold and the computed font-synthesis-weight permits it.
const SYNTHETIC_EMBOLDEN_X_EM: f32 = 0.015_125;
const SYNTHETIC_EMBOLDEN_Y_EM: f32 = 0.012_1;
const MAX_SYNTHETIC_EMBOLDEN_PX: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextPaintPhase {
    Foreground,
    ClipMask,
}

/// Selects which pass-owned glyphs participate in a text-clip paint phase.
///
/// A normal CSS box replays all IFCs in its subtree. A flattened inline box
/// shares its ancestor's Parley layout, so its mask selects only glyph runs
/// encountered while that exact structural inline is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextClipMaskScope {
    AllGlyphs,
    InlineBox(LayoutBoxId),
}

/// Projects Parley text, selection, and caret geometry into owned paint commands.
///
/// The selection geometry and 1.5 CSS px caret width are direct ports of
/// `blitz-paint/src/text.rs::draw_text_selection` and
/// `render.rs::draw_text_input_text` at Blitz d788124a. Moli's only
/// adaptation is applying its pass-local vertical-alignment sidecar before the
/// resulting rectangles cross the owned snapshot boundary.
pub(super) fn project_text<N>(
    layout_box: &LayoutBox<N>,
    transform: LayoutTransform2D,
    local_cull: Option<LayoutRect>,
    metrics: &mut PaintProjectionMetrics,
    snapshot: &mut PaintSnapshot,
) {
    project_text_phase(
        layout_box,
        transform,
        local_cull,
        metrics,
        snapshot,
        TextPaintPhase::Foreground,
        None,
    );
}

/// Projects the box's shaped text as an opaque alpha mask.
///
/// Chromium uses a dedicated `PaintPhase::kTextClip` for
/// `background-clip:text`: glyph and decoration colors become opaque black,
/// while shadows, selection, and caret paint are omitted. The resulting ink
/// is consumed by a `DestIn` layer, so only alpha is observable.
pub(super) fn project_text_clip_mask<N>(
    layout_box: &LayoutBox<N>,
    transform: LayoutTransform2D,
    local_cull: Option<LayoutRect>,
    metrics: &mut PaintProjectionMetrics,
    snapshot: &mut PaintSnapshot,
    scope: TextClipMaskScope,
) {
    project_text_phase(
        layout_box,
        transform,
        local_cull,
        metrics,
        snapshot,
        TextPaintPhase::ClipMask,
        Some(scope),
    );
}

fn project_text_phase<N>(
    layout_box: &LayoutBox<N>,
    transform: LayoutTransform2D,
    local_cull: Option<LayoutRect>,
    metrics: &mut PaintProjectionMetrics,
    snapshot: &mut PaintSnapshot,
    phase: TextPaintPhase,
    mask_scope: Option<TextClipMaskScope>,
) {
    let Some(context) = layout_box.inline_layout.as_ref() else {
        return;
    };
    let Some(text_layout) = context.laid_out.as_ref() else {
        return;
    };
    let layout = layout_box.final_layout;
    let origin_x = layout.border.left + layout.padding.left;
    let origin_y = layout.border.top + layout.padding.top;

    if phase == TextPaintPhase::Foreground {
        project_selection(
            context,
            text_layout,
            origin_x,
            origin_y,
            transform,
            local_cull,
            snapshot,
        );
    }

    let mut active_inline_boxes = Vec::new();
    for (line_index, line) in text_layout.lines().enumerate() {
        metrics.text_line_count = metrics.text_line_count.saturating_add(1);
        let line_is_culled =
            local_cull.is_some_and(|cull| {
                context.fragments.lines.get(line_index).is_some_and(|line| {
                    match line.paint_bounds {
                        InlinePaintBounds::Empty => true,
                        InlinePaintBounds::Unbounded => false,
                        InlinePaintBounds::Bounded(bounds) => {
                            let bounds = LayoutRect::new(
                                origin_x + bounds.x,
                                origin_y + bounds.y,
                                bounds.width,
                                bounds.height,
                            );
                            !rects_intersect(bounds, cull)
                        }
                    }
                })
            });
        if line_is_culled {
            metrics.culled_text_line_count = metrics.culled_text_line_count.saturating_add(1);
            // A background-clip:text replay tracks structural inline start/end
            // markers across lines. Even when a line's glyph ink is culled,
            // retain that tiny state transition so a later visible line uses
            // the correct InlineBox mask scope.
            if mask_scope.is_some() {
                for item in line.items() {
                    if let PositionedLayoutItem::InlineBox(positioned) = item {
                        update_active_inline_boxes(
                            context,
                            positioned.id,
                            &mut active_inline_boxes,
                        );
                    }
                }
            }
            continue;
        }
        let line_placement = context.line_placements.get(line_index);
        for (item_index, item) in line.items().enumerate() {
            let glyph_run = match item {
                PositionedLayoutItem::InlineBox(positioned) => {
                    update_active_inline_boxes(context, positioned.id, &mut active_inline_boxes);
                    continue;
                }
                PositionedLayoutItem::GlyphRun(glyph_run) => glyph_run,
            };
            if let Some(TextClipMaskScope::InlineBox(target)) = mask_scope
                && !active_inline_boxes.contains(&target)
            {
                continue;
            }
            if !glyph_run.style().brush.paint {
                continue;
            }
            let run = glyph_run.run();
            let vertical_offset = line_placement
                .map(|placement| placement.item_offset(item_index))
                .unwrap_or_default();
            let glyphs = glyph_run
                .positioned_glyphs()
                .map(|glyph| PaintGlyph {
                    id: glyph.id,
                    x: origin_x + glyph.x,
                    y: origin_y + glyph.y + vertical_offset,
                })
                .collect::<Vec<_>>();
            if glyphs.is_empty() {
                continue;
            }
            let font = snapshot.intern_font(run.font());
            let synthesis = run.synthesis();
            let glyph_embolden = if glyph_run.style().brush.synthetic_bold && synthesis.embolden() {
                let font_size = run.font_size().max(0.0);
                crate::PaintPoint::new(
                    (SYNTHETIC_EMBOLDEN_X_EM * font_size).min(MAX_SYNTHETIC_EMBOLDEN_PX),
                    (SYNTHETIC_EMBOLDEN_Y_EM * font_size).min(MAX_SYNTHETIC_EMBOLDEN_PX),
                )
            } else {
                crate::PaintPoint::ZERO
            };
            let owned_run = PaintGlyphRun {
                font,
                font_size: run.font_size(),
                normalized_coords: run.normalized_coords().to_vec(),
                color: if phase == TextPaintPhase::ClipMask {
                    PaintColor::BLACK
                } else {
                    glyph_run.style().brush.color
                },
                glyph_skew_radians: synthesis.skew().map(f32::to_radians),
                glyph_embolden,
                glyphs,
                transform,
            };
            let brush = &glyph_run.style().brush;
            if phase == TextPaintPhase::Foreground {
                for shadow in brush.shadows.iter().rev() {
                    snapshot.push_fragment(PaintFragment::TextShadow(PaintTextShadow {
                        run: owned_run.clone(),
                        color: shadow.color,
                        offset: shadow.offset,
                        blur_radius: shadow.blur_radius,
                    }));
                }
            }

            let metrics = run.metrics();
            let baseline = origin_y + glyph_run.baseline() + vertical_offset;
            let x = origin_x + glyph_run.offset();
            let width = glyph_run.advance().max(0.0);
            let decoration = brush.decoration;
            let decoration_color = if phase == TextPaintPhase::ClipMask {
                PaintColor::BLACK
            } else {
                decoration.color
            };
            let thickness = decoration
                .thickness
                .unwrap_or(metrics.underline_size.max(1.0))
                .max(0.5);
            let decoration_fragment = |y: f32| {
                (width > 0.0 && decoration_color.alpha > 0.0).then_some(
                    PaintFragment::TextDecoration(PaintTextDecoration {
                        x,
                        y,
                        width,
                        thickness,
                        color: decoration_color,
                        style: decoration.style,
                        transform,
                    }),
                )
            };
            if decoration.underline {
                let y = decoration.underline_offset.map_or_else(
                    || baseline - metrics.underline_offset + thickness * 0.5,
                    |offset| baseline + offset + thickness * 0.5,
                );
                if let Some(fragment) = decoration_fragment(y) {
                    snapshot.push_fragment(fragment);
                }
            }
            if decoration.overline
                && let Some(fragment) =
                    decoration_fragment(baseline - metrics.ascent + thickness * 0.5)
            {
                snapshot.push_fragment(fragment);
            }

            snapshot.push_fragment(PaintFragment::GlyphRun(owned_run));

            if decoration.line_through
                && let Some(fragment) =
                    decoration_fragment(baseline - metrics.strikethrough_offset + thickness * 0.5)
            {
                snapshot.push_fragment(fragment);
            }
        }
    }
}

fn update_active_inline_boxes(
    context: &InlineFormattingContext,
    object_id: u64,
    active: &mut Vec<LayoutBoxId>,
) {
    let Some(object) = context.object(object_id) else {
        return;
    };
    active.clear();
    active.extend(object.ancestors.iter().copied());
    if object.role == InlineObjectRole::StartEdge {
        active.push(object.box_id);
    }
}

fn project_selection(
    context: &InlineFormattingContext,
    text_layout: &parley::Layout<crate::stylo_to_parley::TextBrush>,
    origin_x: f32,
    origin_y: f32,
    transform: LayoutTransform2D,
    local_cull: Option<LayoutRect>,
    snapshot: &mut PaintSnapshot,
) {
    let Some(selection) = context.selection.as_ref() else {
        return;
    };
    match selection {
        InlineSelection::Range(range) => {
            let anchor = Cursor::from_byte_index(text_layout, range.start, Affinity::Downstream);
            let focus = Cursor::from_byte_index(text_layout, range.end, Affinity::Downstream);
            Selection::new(anchor, focus).geometry_with(text_layout, |rect, line_index| {
                let rect = selection_rect(
                    context,
                    line_index,
                    origin_x,
                    origin_y,
                    rect.x0 as f32,
                    rect.y0 as f32,
                    (rect.x1 - rect.x0).max(0.0) as f32,
                    (rect.y1 - rect.y0).max(0.0) as f32,
                );
                if rect.width > 0.0
                    && rect.height > 0.0
                    && local_cull.is_none_or(|cull| rects_intersect(rect, cull))
                {
                    snapshot.push_fragment(PaintFragment::Fill {
                        shape: PaintShape::Rect(rect),
                        brush: PaintBrush::Solid(SELECTION_COLOR),
                        transform,
                    });
                }
            });
        }
        InlineSelection::Caret { offset, color } => {
            let cursor = Cursor::from_byte_index(text_layout, *offset, Affinity::Downstream);
            let rect = cursor.geometry(text_layout, 1.5);
            let line_index = text_layout
                .lines()
                .enumerate()
                .find(|(_, line)| line.text_range().contains(offset))
                .map(|(index, _)| index)
                .unwrap_or_else(|| text_layout.lines().count().saturating_sub(1));
            let rect = selection_rect(
                context,
                line_index,
                origin_x,
                origin_y,
                rect.x0 as f32,
                rect.y0 as f32,
                (rect.x1 - rect.x0).max(1.5) as f32,
                (rect.y1 - rect.y0).max(0.0) as f32,
            );
            if rect.height > 0.0
                && color.alpha > 0.0
                && local_cull.is_none_or(|cull| rects_intersect(rect, cull))
            {
                snapshot.push_fragment(PaintFragment::Fill {
                    shape: PaintShape::Rect(rect),
                    brush: PaintBrush::Solid(*color),
                    transform,
                });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn selection_rect(
    context: &InlineFormattingContext,
    line_index: usize,
    origin_x: f32,
    origin_y: f32,
    x: f32,
    fallback_y: f32,
    width: f32,
    fallback_height: f32,
) -> LayoutRect {
    let (y, height) = context
        .line_placements
        .get(line_index)
        .map(|placement| (placement.rect.y, placement.rect.height))
        .unwrap_or((fallback_y, fallback_height));
    LayoutRect::new(origin_x + x, origin_y + y, width, height)
}
