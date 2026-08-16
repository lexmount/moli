use parley::{Affinity, Cursor, PositionedLayoutItem, Selection};

use crate::{
    LayoutBox, LayoutBoxId, LayoutRect, LayoutTransform2D, PaintBrush, PaintColor, PaintFragment,
    PaintGlyph, PaintGlyphRun, PaintShape, PaintSnapshot, PaintTextDecoration, PaintTextShadow,
    inline::{
        FlowRelativeRect, InlineCoordinateSpace, InlineFormattingContext, InlineObjectRole,
        InlineSelection, LineRelativeOffset, LineRelativeRect, flow_relative_line_rect,
    },
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
    snapshot: &mut PaintSnapshot,
) {
    project_text_phase(
        layout_box,
        transform,
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
    snapshot: &mut PaintSnapshot,
    scope: TextClipMaskScope,
) {
    project_text_phase(
        layout_box,
        transform,
        snapshot,
        TextPaintPhase::ClipMask,
        Some(scope),
    );
}

fn project_text_phase<N>(
    layout_box: &LayoutBox<N>,
    transform: LayoutTransform2D,
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
    let content_box_size = taffy::Size {
        width: (layout.size.width
            - layout.border.left
            - layout.border.right
            - layout.padding.left
            - layout.padding.right)
            .max(0.0),
        height: (layout.size.height
            - layout.border.top
            - layout.border.bottom
            - layout.padding.top
            - layout.padding.bottom)
            .max(0.0),
    };
    let inline_coordinates = InlineCoordinateSpace::new(layout_box.style.writing_mode());

    if phase == TextPaintPhase::Foreground {
        project_selection(
            context,
            text_layout,
            origin_x,
            origin_y,
            inline_coordinates,
            content_box_size,
            transform,
            snapshot,
        );
    }

    let mut active_inline_boxes = Vec::new();
    for (line_index, line) in text_layout.lines().enumerate() {
        let line_placement = context.line_placements.get(line_index);
        let line_rect = flow_relative_line_rect(&line, line_placement);
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
                .map(|glyph| {
                    let point = inline_coordinates.to_physical_line_point(
                        line_rect,
                        LineRelativeOffset::new(
                            glyph.x - line_rect.inline_offset,
                            glyph.y + vertical_offset - line_rect.block_offset,
                        ),
                        taffy::Size::ZERO,
                        content_box_size,
                    );
                    PaintGlyph {
                        id: glyph.id,
                        x: origin_x + point.x,
                        y: origin_y + point.y,
                    }
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
            let logical_baseline = glyph_run.baseline() + vertical_offset;
            let inline_offset = glyph_run.offset();
            let inline_advance = glyph_run.advance().max(0.0);
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
            let decoration_fragment = |block_offset: f32| {
                if inline_advance <= 0.0 || decoration_color.alpha <= 0.0 {
                    return None;
                }
                let physical_point = |inline_offset| {
                    let point = inline_coordinates.to_physical_line_point(
                        line_rect,
                        LineRelativeOffset::new(
                            inline_offset - line_rect.inline_offset,
                            block_offset - line_rect.block_offset,
                        ),
                        taffy::Size::ZERO,
                        content_box_size,
                    );
                    crate::PaintPoint::new(origin_x + point.x, origin_y + point.y)
                };
                Some(PaintFragment::TextDecoration(PaintTextDecoration {
                    start: physical_point(inline_offset),
                    end: physical_point(inline_offset + inline_advance),
                    thickness,
                    color: decoration_color,
                    style: decoration.style,
                    transform,
                }))
            };
            if decoration.underline {
                let block_offset = decoration.underline_offset.map_or_else(
                    || logical_baseline - metrics.underline_offset + thickness * 0.5,
                    |offset| logical_baseline + offset + thickness * 0.5,
                );
                if let Some(fragment) = decoration_fragment(block_offset) {
                    snapshot.push_fragment(fragment);
                }
            }
            if decoration.overline
                && let Some(fragment) =
                    decoration_fragment(logical_baseline - metrics.ascent + thickness * 0.5)
            {
                snapshot.push_fragment(fragment);
            }

            snapshot.push_fragment(PaintFragment::GlyphRun(owned_run));

            if decoration.line_through
                && let Some(fragment) = decoration_fragment(
                    logical_baseline - metrics.strikethrough_offset + thickness * 0.5,
                )
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
    inline_coordinates: InlineCoordinateSpace,
    content_box_size: taffy::Size<f32>,
    transform: LayoutTransform2D,
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
                    inline_coordinates,
                    content_box_size,
                    rect.x0 as f32,
                    rect.y0 as f32,
                    (rect.x1 - rect.x0).max(0.0) as f32,
                    (rect.y1 - rect.y0).max(0.0) as f32,
                );
                if rect.width > 0.0 && rect.height > 0.0 {
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
                inline_coordinates,
                content_box_size,
                rect.x0 as f32,
                rect.y0 as f32,
                (rect.x1 - rect.x0).max(1.5) as f32,
                (rect.y1 - rect.y0).max(0.0) as f32,
            );
            if rect.height > 0.0 && color.alpha > 0.0 {
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
    inline_coordinates: InlineCoordinateSpace,
    content_box_size: taffy::Size<f32>,
    x: f32,
    fallback_y: f32,
    width: f32,
    fallback_height: f32,
) -> LayoutRect {
    let logical_content_size = inline_coordinates.to_logical_size(content_box_size);
    let line = context
        .line_placements
        .get(line_index)
        .map(|placement| placement.rect)
        .unwrap_or_else(|| {
            FlowRelativeRect::new(
                0.0,
                fallback_y,
                logical_content_size.inline_size,
                fallback_height,
            )
        });
    let physical = inline_coordinates.to_physical_line_rect(
        line,
        LineRelativeRect::new(x - line.inline_offset, 0.0, width, line.block_size),
        content_box_size,
    );
    LayoutRect::new(
        origin_x + physical.x,
        origin_y + physical.y,
        physical.width,
        physical.height,
    )
}
