use parley::{
    Affinity, PositionedLayoutItem,
    editing::{Cursor, Selection},
};

use super::{PaintProjectionMetrics, cull::rects_intersect};
use crate::{
    LayoutBox, LayoutBoxId, LayoutRect, LayoutTransform2D, PaintBrush, PaintColor, PaintFragment,
    PaintGlyph, PaintGlyphRun, PaintShape, PaintSnapshot, PaintTextDecoration, PaintTextShadow,
    inline::{InlineFormattingContext, InlinePaintBounds, InlineSelection, glyph_cluster_ranges},
};

const SELECTION_COLOR: PaintColor = PaintColor::new(180.0 / 255.0, 213.0 / 255.0, 1.0, 1.0);
// These expansion ratios and the 0.3 px cap follow Blitz's AnyRender text
// painter at d788124a. Moli combines the matched face's synthesis metadata with
// the computed CSS weight threshold and font-synthesis-weight permission.
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
        project_selection(layout_box, transform, local_cull, snapshot, None);
    }

    let text: Option<std::sync::Arc<str>> = (snapshot.retain_text_semantics
        && phase == TextPaintPhase::Foreground)
        .then(|| std::sync::Arc::from(context.text.as_str()));
    let paint = TextRunPaint {
        origin_x,
        origin_y,
        transform,
        phase,
        text,
    };
    for (line_index, line) in text_layout.lines().enumerate() {
        let mut mapped_run = None;
        let mut cluster_ranges = Vec::new().into_iter();
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
            continue;
        }
        let line_placement = context.line_placements.get(line_index);
        for (item_index, item) in line.items().enumerate() {
            let glyph_run = match item {
                PositionedLayoutItem::InlineBox(_) => continue,
                PositionedLayoutItem::GlyphRun(glyph_run) => glyph_run,
            };
            let run = glyph_run.run();
            if paint.text.is_some() && mapped_run != Some(run.index()) {
                cluster_ranges = glyph_cluster_ranges(run).into_iter();
                mapped_run = Some(run.index());
            }
            // Advance even for masked/hidden style runs, so styled portions of
            // the same Parley run keep their original cluster associations.
            let clusters = cluster_ranges
                .by_ref()
                .take(if paint.text.is_some() {
                    glyph_run.glyphs().count()
                } else {
                    0
                })
                .collect();
            if let Some(TextClipMaskScope::InlineBox(target)) = mask_scope
                && !context.style_is_within_box(usize::from(glyph_run.style_index()), target)
            {
                continue;
            }
            if !glyph_run.style().brush.paint {
                continue;
            }
            let vertical_offset = line_placement
                .map(|placement| placement.item_offset(item_index))
                .unwrap_or_default();
            paint.project_run(&glyph_run, vertical_offset, clusters, snapshot);
        }
    }
}

struct TextRunPaint {
    origin_x: f32,
    origin_y: f32,
    transform: LayoutTransform2D,
    phase: TextPaintPhase,
    text: Option<std::sync::Arc<str>>,
}

impl TextRunPaint {
    fn project_run(
        &self,
        glyph_run: &parley::GlyphRun<'_, crate::stylo_to_parley::TextBrush>,
        vertical_offset: f32,
        clusters: Vec<std::ops::Range<usize>>,
        snapshot: &mut PaintSnapshot,
    ) {
        let glyphs = glyph_run
            .positioned_glyphs()
            .map(|glyph| PaintGlyph {
                id: glyph.id,
                x: self.origin_x + glyph.x,
                y: self.origin_y + glyph.y + vertical_offset,
            })
            .collect::<Vec<_>>();
        if glyphs.is_empty() {
            return;
        }
        let run = glyph_run.run();
        let font = snapshot.intern_font(&run.font().font);
        let synthesis = run.synthesis();
        let glyph_embolden =
            if glyph_run.style().brush.synthetic_bold && synthesis.embolden_with_threshold(0.0) {
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
            normalized_coords: run
                .normalized_coords()
                .iter()
                .map(|coord| coord.to_bits())
                .collect(),
            color: if self.phase == TextPaintPhase::ClipMask {
                PaintColor::BLACK
            } else {
                glyph_run.style().brush.color
            },
            glyph_skew_radians: synthesis.skew().map(f32::to_radians),
            glyph_embolden,
            glyphs,
            text: self.text.as_ref().map(|text| crate::PaintTextSemantics {
                text: text.clone(),
                clusters,
            }),
            transform: self.transform,
            // Text clipping uses both fill and stroke as opaque ink,
            // even when the authored stroke color is transparent.
            stroke: glyph_run.style().brush.stroke.map(|mut stroke| {
                if self.phase == TextPaintPhase::ClipMask {
                    stroke.color = PaintColor::BLACK;
                }
                stroke
            }),
        };
        let brush = &glyph_run.style().brush;
        if self.phase == TextPaintPhase::Foreground {
            for shadow in brush.shadows.iter().rev() {
                snapshot.push_fragment(PaintFragment::TextShadow(PaintTextShadow {
                    run: owned_run.clone(),
                    color: shadow.color,
                    offset: shadow.offset,
                    blur_radius: shadow.blur_radius,
                }));
            }
        }

        let metrics = run.font_metrics();
        let baseline = self.origin_y + glyph_run.baseline() + vertical_offset;
        let x = self.origin_x + glyph_run.offset();
        let width = glyph_run.advance().max(0.0);
        let decoration = brush.decoration;
        let decoration_color = if self.phase == TextPaintPhase::ClipMask {
            PaintColor::BLACK
        } else {
            decoration.color
        };
        let thickness = decoration
            .thickness
            .unwrap_or(metrics.underline_size.max(1.0))
            .max(0.5);
        let decoration_fragment = |y: f32| {
            (width > 0.0 && decoration_color.alpha > 0.0).then_some(PaintFragment::TextDecoration(
                PaintTextDecoration {
                    x,
                    y,
                    width,
                    thickness,
                    color: decoration_color,
                    style: decoration.style,
                    transform: self.transform,
                },
            ))
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
            && let Some(fragment) = decoration_fragment(baseline - metrics.ascent + thickness * 0.5)
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

/// Borrow the shaped runs once per line; fragment-level events must not replay
/// shaping iteration or copy the complete semantic text for every run.
pub(super) struct InlineTextPaint<'a> {
    context: &'a InlineFormattingContext,
    text: Option<std::sync::Arc<str>>,
    lines: std::collections::HashMap<usize, InlineTextLine<'a>>,
}

struct InlineTextLine<'a> {
    items: Vec<PositionedLayoutItem<'a, crate::stylo_to_parley::TextBrush>>,
    clusters: Vec<Vec<std::ops::Range<usize>>>,
}

impl<'a> InlineTextPaint<'a> {
    pub(super) fn new(context: &'a InlineFormattingContext, retain_text: bool) -> Self {
        Self {
            context,
            text: retain_text.then(|| std::sync::Arc::from(context.text.as_str())),
            lines: std::collections::HashMap::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn project_run<N>(
        &mut self,
        layout_box: &LayoutBox<N>,
        line_index: usize,
        item_index: usize,
        transform: LayoutTransform2D,
        local_cull: Option<LayoutRect>,
        metrics: &mut PaintProjectionMetrics,
        snapshot: &mut PaintSnapshot,
    ) {
        let context = self.context;
        let text_layout = context.laid_out.as_ref().unwrap();
        let origin_x = layout_box.final_layout.border.left + layout_box.final_layout.padding.left;
        let origin_y = layout_box.final_layout.border.top + layout_box.final_layout.padding.top;
        let culled =
            local_cull.is_some_and(|cull| {
                context.fragments.lines.get(line_index).is_some_and(|line| {
                    match line.paint_bounds {
                        InlinePaintBounds::Empty => true,
                        InlinePaintBounds::Unbounded => false,
                        InlinePaintBounds::Bounded(bounds) => !rects_intersect(
                            LayoutRect::new(
                                origin_x + bounds.x,
                                origin_y + bounds.y,
                                bounds.width,
                                bounds.height,
                            ),
                            cull,
                        ),
                    }
                })
            });
        let line = self.lines.entry(line_index).or_insert_with(|| {
            metrics.text_line_count = metrics.text_line_count.saturating_add(1);
            if culled {
                metrics.culled_text_line_count = metrics.culled_text_line_count.saturating_add(1);
            }
            let items = text_layout
                .get(line_index)
                .unwrap()
                .items()
                .collect::<Vec<_>>();
            let mut mapped_run = None;
            let mut ranges = Vec::new().into_iter();
            let clusters = items
                .iter()
                .map(|item| {
                    let PositionedLayoutItem::GlyphRun(glyphs) = item else {
                        return Vec::new();
                    };
                    if self.text.is_none()
                        && !matches!(context.selection, Some(InlineSelection::Range(_)))
                    {
                        return Vec::new();
                    }
                    if mapped_run != Some(glyphs.run().index()) {
                        ranges = glyph_cluster_ranges(glyphs.run()).into_iter();
                        mapped_run = Some(glyphs.run().index());
                    }
                    ranges.by_ref().take(glyphs.glyphs().count()).collect()
                })
                .collect();
            InlineTextLine { items, clusters }
        });
        if culled {
            return;
        }
        let PositionedLayoutItem::GlyphRun(glyphs) = &line.items[item_index] else {
            return;
        };
        if !glyphs.style().brush.paint {
            return;
        }
        let clusters = std::mem::take(&mut line.clusters[item_index]);
        if matches!(context.selection, Some(InlineSelection::Range(_)))
            && let Some(range) = clusters
                .iter()
                .cloned()
                .reduce(|range, cluster| range.start.min(cluster.start)..range.end.max(cluster.end))
        {
            project_selection(layout_box, transform, local_cull, snapshot, Some(&range));
        }
        let paint = TextRunPaint {
            origin_x,
            origin_y,
            transform,
            phase: TextPaintPhase::Foreground,
            text: self.text.clone(),
        };
        let vertical_offset = context
            .line_placements
            .get(line_index)
            .map_or(0.0, |line| line.item_offset(item_index));
        paint.project_run(
            glyphs,
            vertical_offset,
            if paint.text.is_some() {
                clusters
            } else {
                Vec::new()
            },
            snapshot,
        );
    }
}

pub(super) fn project_selection<N>(
    layout_box: &LayoutBox<N>,
    transform: LayoutTransform2D,
    local_cull: Option<LayoutRect>,
    snapshot: &mut PaintSnapshot,
    text_range: Option<&std::ops::Range<usize>>,
) {
    let Some(selection) = layout_box
        .inline_layout
        .as_ref()
        .and_then(|context| context.selection.as_ref())
    else {
        return;
    };
    match selection {
        InlineSelection::Range(_) => {
            range_selection_rects(layout_box, text_range, |rect| {
                if local_cull.is_none_or(|cull| rects_intersect(rect, cull)) {
                    snapshot.push_fragment(PaintFragment::Fill {
                        fill_rule: crate::PaintFillRule::NonZero,
                        shape: PaintShape::Rect(rect),
                        brush: PaintBrush::Solid(SELECTION_COLOR),
                        transform,
                    });
                }
            });
        }
        InlineSelection::Caret { color, .. } => {
            let Some(rect) = caret_rect(layout_box) else {
                return;
            };
            if local_cull.is_none_or(|cull| rects_intersect(rect, cull)) {
                snapshot.push_fragment(PaintFragment::Fill {
                    fill_rule: crate::PaintFillRule::NonZero,
                    shape: PaintShape::Rect(rect),
                    brush: PaintBrush::Solid(*color),
                    transform,
                });
            }
        }
    }
}

/// One range geometry path for normal text, glyphless fragments, and culling.
pub(super) fn range_selection_rects<N>(
    layout_box: &LayoutBox<N>,
    text_range: Option<&std::ops::Range<usize>>,
    mut emit: impl FnMut(LayoutRect),
) {
    if !layout_box.style.is_visible() {
        return;
    }
    let Some(context) = layout_box.inline_layout.as_ref() else {
        return;
    };
    let Some(text_layout) = context.laid_out.as_ref() else {
        return;
    };
    let Some(InlineSelection::Range(selected)) = context.selection.as_ref() else {
        return;
    };
    let range = text_range.map_or_else(
        || selected.clone(),
        |text| selected.start.max(text.start)..selected.end.min(text.end),
    );
    if range.is_empty() {
        return;
    }
    let anchor = Cursor::from_byte_index(text_layout, range.start, Affinity::Downstream);
    let focus = Cursor::from_byte_index(text_layout, range.end, Affinity::Downstream);
    Selection::new(anchor, focus).geometry_with(text_layout, |rect, line_index| {
        let rect = selection_rect(
            context,
            line_index,
            layout_box.final_layout.border.left + layout_box.final_layout.padding.left,
            layout_box.final_layout.border.top + layout_box.final_layout.padding.top,
            rect.x0 as f32,
            rect.y0 as f32,
            (rect.x1 - rect.x0).max(0.0) as f32,
            (rect.y1 - rect.y0).max(0.0) as f32,
        );
        if rect.width > 0.0 && rect.height > 0.0 {
            emit(rect);
        }
    });
}

/// Use the same caret ink for event culling and painting, independently of
/// glyph runs and the containing box's border bounds.
pub(super) fn caret_rect<N>(layout_box: &LayoutBox<N>) -> Option<LayoutRect> {
    if !layout_box.style.is_visible() {
        return None;
    }
    let context = layout_box.inline_layout.as_ref()?;
    let text_layout = context.laid_out.as_ref()?;
    let InlineSelection::Caret { offset, color } = context.selection.as_ref()? else {
        return None;
    };
    if color.alpha <= 0.0 {
        return None;
    }
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
        layout_box.final_layout.border.left + layout_box.final_layout.padding.left,
        layout_box.final_layout.border.top + layout_box.final_layout.padding.top,
        rect.x0 as f32,
        rect.y0 as f32,
        (rect.x1 - rect.x0).max(1.5) as f32,
        (rect.y1 - rect.y0).max(0.0) as f32,
    );
    (rect.height > 0.0).then_some(rect)
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
