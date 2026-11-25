use parley::{Layout, PositionedLayoutItem};

use crate::{
    PaintPoint, ResolvedLayoutStyle, stylo_to_parley::TextBrush, text::ParleyDocumentServices,
};

use super::InlineLinePlacement;

#[derive(Debug)]
pub(crate) struct InlineLineClamp {
    pub(super) limit: usize,
    marker: Layout<TextBrush>,
    pub(crate) show_marker: bool,
}

impl InlineLineClamp {
    pub(super) fn new(
        style: &ResolvedLayoutStyle,
        parley: &mut ParleyDocumentServices,
    ) -> Option<Self> {
        let limit = style.legacy_line_clamp()?;
        let text = "…";
        let mut text_style = style.parley_text_style();
        parley.resolve_font_families(&mut text_style, text.chars().next());
        let mut builder =
            parley
                .layout_context
                .style_run_builder(&mut parley.font_context, text, 1.0, true);
        let style = builder.push_style(text_style);
        builder.push_style_run(style, 0..text.len());
        let mut marker = builder.build(text);
        marker.break_all_lines(None);
        Some(Self {
            limit,
            marker,
            show_marker: false,
        })
    }

    pub(super) fn marker_reservation(
        &self,
        layout: &Layout<TextBrush>,
        available_width: f32,
    ) -> Option<(usize, f32)> {
        let width = self.marker.width();
        (width > 0.0
            && width <= available_width
            && layout.lines().skip(self.limit).any(|line| !line.is_empty()))
        .then_some((self.limit - 1, width))
    }

    pub(super) fn marker_layout(
        &self,
        layout: &Layout<TextBrush>,
        placements: &[InlineLinePlacement],
    ) -> Option<(&Layout<TextBrush>, PaintPoint)> {
        if !self.show_marker || self.limit >= layout.len() {
            return None;
        }
        let line = layout.get(self.limit - 1)?;
        let metrics = line.metrics();
        let visible_advance = line
            .items()
            .map(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => run.advance(),
                PositionedLayoutItem::InlineBox(inline) => inline.width,
            })
            .sum::<f32>()
            - metrics.hanging_advance;
        // A single unbreakable item may exceed the reserved width.
        let reserved_width = metrics.inline_max_coord - metrics.inline_min_coord;
        let tolerance = reserved_width.abs().max(1.0) * f32::EPSILON * 8.0;
        if visible_advance > reserved_width + tolerance {
            return None;
        }
        let mut x = metrics.inline_min_coord + metrics.offset;
        if layout.is_rtl() {
            x += metrics.hanging_advance - self.marker.width();
        } else {
            x += visible_advance;
        }
        let y = placements.get(self.limit - 1)?.baseline - self.marker.first_baseline()?;
        Some((&self.marker, PaintPoint::new(x, y)))
    }
}
