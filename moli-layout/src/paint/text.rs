use std::{collections::HashSet, sync::Arc};

use super::{PaintProjectionMetrics, cull::rects_intersect};
use crate::{
    LayoutBox, LayoutBoxId, LayoutRect, LayoutTransform2D, PaintBrush, PaintColor, PaintFragment,
    PaintGlyph, PaintGlyphRun, PaintShape, PaintSnapshot, PaintTextDecoration, PaintTextShadow,
    inline::{
        InlinePaintBounds,
        display::{InlineDisplayContext, InlineDisplayItem, InlineItemContent, TextOrigin},
    },
};

const SELECTION_COLOR: PaintColor = PaintColor::new(180.0 / 255.0, 213.0 / 255.0, 1.0, 1.0);
// These expansion ratios and the 0.3 px cap follow Blitz's AnyRender text
// painter at d788124a. Matched-face synthesis metadata is combined with the
// computed CSS weight and font-synthesis-weight permission below.
const SYNTHETIC_EMBOLDEN_X_EM: f32 = 0.015_125;
const SYNTHETIC_EMBOLDEN_Y_EM: f32 = 0.012_1;
const MAX_SYNTHETIC_EMBOLDEN_PX: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextPaintPhase {
    Foreground,
    ClipMask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextClipMaskScope {
    AllGlyphs,
    InlineBox(LayoutBoxId),
}

pub(super) fn project_text<N>(
    layout_box: &LayoutBox<N>,
    display: &InlineDisplayContext<'_>,
    transform: LayoutTransform2D,
    local_cull: Option<LayoutRect>,
    metrics: &mut PaintProjectionMetrics,
    snapshot: &mut PaintSnapshot,
) {
    let text = snapshot
        .retain_text_semantics
        .then(|| Arc::from(layout_box.inline_layout.as_ref().unwrap().text.as_str()));
    for line in &display.lines {
        paint_selection(&line.selection, transform, local_cull, snapshot);
    }
    project_caret(display, transform, local_cull, snapshot);
    project_lines(
        layout_box,
        display,
        transform,
        local_cull,
        metrics,
        snapshot,
        TextPaintPhase::Foreground,
        TextClipMaskScope::AllGlyphs,
        text,
    );
}

pub(super) fn project_text_clip_mask<N>(
    layout_box: &LayoutBox<N>,
    display: &InlineDisplayContext<'_>,
    transform: LayoutTransform2D,
    local_cull: Option<LayoutRect>,
    metrics: &mut PaintProjectionMetrics,
    snapshot: &mut PaintSnapshot,
    scope: TextClipMaskScope,
) {
    project_lines(
        layout_box,
        display,
        transform,
        local_cull,
        metrics,
        snapshot,
        TextPaintPhase::ClipMask,
        scope,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn project_lines<N>(
    layout_box: &LayoutBox<N>,
    display: &InlineDisplayContext<'_>,
    transform: LayoutTransform2D,
    local_cull: Option<LayoutRect>,
    metrics: &mut PaintProjectionMetrics,
    snapshot: &mut PaintSnapshot,
    phase: TextPaintPhase,
    scope: TextClipMaskScope,
    text: Option<Arc<str>>,
) {
    for (line_index, line) in display.lines.iter().enumerate() {
        metrics.text_line_count = metrics.text_line_count.saturating_add(1);
        if line_is_culled(display, line_index, local_cull) {
            metrics.culled_text_line_count = metrics.culled_text_line_count.saturating_add(1);
            continue;
        }
        for item in &display.items[line.items.clone()] {
            let InlineItemContent::Text(view) = &item.content else {
                continue;
            };
            if let TextClipMaskScope::InlineBox(target) = scope
                && (matches!(view.origin, TextOrigin::Generated { .. })
                    || !layout_box
                        .inline_layout
                        .as_ref()
                        .unwrap()
                        .style_is_within_box(usize::from(view.glyphs.style_index()), target))
            {
                continue;
            }
            paint_text_item(item, transform, phase, text.clone(), snapshot);
        }
    }
}

fn paint_text_item(
    item: &InlineDisplayItem<'_>,
    transform: LayoutTransform2D,
    phase: TextPaintPhase,
    source_text: Option<Arc<str>>,
    snapshot: &mut PaintSnapshot,
) {
    let InlineItemContent::Text(view) = &item.content else {
        return;
    };
    let text = if phase == TextPaintPhase::Foreground && snapshot.retain_text_semantics {
        match &view.origin {
            TextOrigin::Source => source_text,
            TextOrigin::Generated { text, .. } => Some((*text).clone()),
        }
    } else {
        None
    };
    let clusters = if text.is_some() {
        view.glyphs
            .glyphs_with_text_ranges()
            .map(|(_, range)| range)
            .collect()
    } else {
        Vec::new()
    };
    TextRunPaint {
        origin_x: view.offset.x,
        origin_y: view.offset.y,
        transform,
        phase,
        text,
    }
    .project_run(&view.glyphs, clusters, snapshot);
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
        clusters: Vec<std::ops::Range<usize>>,
        snapshot: &mut PaintSnapshot,
    ) {
        let glyphs = glyph_run
            .positioned_glyphs()
            .map(|glyph| PaintGlyph {
                id: glyph.id,
                x: self.origin_x + glyph.x,
                y: self.origin_y + glyph.y,
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
        let baseline = self.origin_y + glyph_run.baseline();
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

/// Fragment events reuse finalized views directly. Only semantic text and line
/// accounting are cached; no shape or source mapping is reconstructed here.
pub(super) struct InlineTextPaint<'a> {
    display: &'a InlineDisplayContext<'a>,
    text: Option<Arc<str>>,
    lines: HashSet<usize>,
}

impl<'a> InlineTextPaint<'a> {
    pub(super) fn new(
        display: &'a InlineDisplayContext<'a>,
        source_text: &str,
        retain_text: bool,
    ) -> Self {
        Self {
            display,
            text: retain_text.then(|| Arc::from(source_text)),
            lines: HashSet::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn project_run(
        &mut self,
        item: usize,
        transform: LayoutTransform2D,
        local_cull: Option<LayoutRect>,
        metrics: &mut PaintProjectionMetrics,
        snapshot: &mut PaintSnapshot,
    ) {
        let item = &self.display.items[item];
        let culled = line_is_culled(self.display, item.line, local_cull);
        if self.lines.insert(item.line) {
            metrics.text_line_count = metrics.text_line_count.saturating_add(1);
            if culled {
                metrics.culled_text_line_count = metrics.culled_text_line_count.saturating_add(1);
            }
        }
        if culled {
            return;
        }
        paint_selection(&item.selection, transform, local_cull, snapshot);
        paint_text_item(
            item,
            transform,
            TextPaintPhase::Foreground,
            self.text.clone(),
            snapshot,
        );
    }
}

fn line_is_culled(
    display: &InlineDisplayContext<'_>,
    line: usize,
    cull: Option<LayoutRect>,
) -> bool {
    cull.is_some_and(|cull| match display.lines[line].paint_bounds {
        InlinePaintBounds::Empty => true,
        InlinePaintBounds::Unbounded => false,
        InlinePaintBounds::Bounded(bounds) => !rects_intersect(bounds, cull),
    })
}

fn paint_selection(
    rects: &[LayoutRect],
    transform: LayoutTransform2D,
    local_cull: Option<LayoutRect>,
    snapshot: &mut PaintSnapshot,
) {
    for rect in rects {
        let rect = *rect;
        if local_cull.is_none_or(|cull| rects_intersect(rect, cull)) {
            snapshot.push_fragment(PaintFragment::Fill {
                fill_rule: crate::PaintFillRule::NonZero,
                shape: PaintShape::Rect(rect),
                brush: PaintBrush::Solid(SELECTION_COLOR),
                transform,
            });
        }
    }
}

pub(super) fn project_caret(
    display: &InlineDisplayContext<'_>,
    transform: LayoutTransform2D,
    local_cull: Option<LayoutRect>,
    snapshot: &mut PaintSnapshot,
) {
    if let Some((rect, color)) = display.caret
        && local_cull.is_none_or(|cull| rects_intersect(rect, cull))
    {
        snapshot.push_fragment(PaintFragment::Fill {
            fill_rule: crate::PaintFillRule::NonZero,
            shape: PaintShape::Rect(rect),
            brush: PaintBrush::Solid(color),
            transform,
        });
    }
}

pub(super) fn caret_rect(display: &InlineDisplayContext<'_>) -> Option<LayoutRect> {
    display.caret.map(|(rect, _)| rect)
}
