//! Canvas shaping uses the same document font collection as layout, but does
//! not build a DOM layout tree or touch the frozen geometry snapshot.

use super::*;
use crate::{
    PaintColor, PaintFontResource, PaintGlyph, PaintGlyphRun, PaintPath, PaintPathElement,
    PaintPoint, PaintRect, PaintSnapshot, PaintTransform2D, PaintViewport,
};

/// Resolved Canvas font shorthand in CSS pixels.
pub struct CanvasFont {
    pub family: String,
    pub size: f32,
    pub weight: f32,
    pub stretch: f32,
    pub style: WebFontStyle,
}

/// Measurement and paint share exactly the same shaped runs and advances.
pub struct ShapedCanvasText {
    pub width: f64,
    pub fonts: Vec<PaintFontResource>,
    pub runs: Vec<PaintGlyphRun>,
}

impl DocumentLayoutServices {
    pub fn shape_canvas_text(&mut self, text: &str, font: &CanvasFont) -> ShapedCanvasText {
        if text.is_empty() || font.size <= 0.0 || !font.size.is_finite() {
            return ShapedCanvasText {
                width: 0.0,
                fonts: Vec::new(),
                runs: Vec::new(),
            };
        }
        let text: String = text
            .chars()
            .map(|ch| match ch {
                '\t' | '\n' | '\r' | '\u{000c}' => ' ',
                ch => ch,
            })
            .collect();
        let mut snapshot = PaintSnapshot::new(PaintViewport::new(0, 0, 1.0), PaintColor::default());
        let services = self.parley_mut();
        let style = TextStyle {
            font_family: FontFamily::Source(Cow::Owned(font.family.clone())),
            font_size: font.size,
            font_weight: FontWeight::new(font.weight),
            font_width: FontWidth::from_percentage(font.stretch),
            font_style: font.style.to_fontique(),
            ..Default::default()
        };
        // Preserve kerning/ligatures within consecutive equal style runs while
        // respecting character-specific CSS unicode-range font selection.
        let mut styles: Vec<(usize, TextStyle<'static, 'static, TextBrush>)> = Vec::new();
        for (offset, character) in text.char_indices() {
            let mut resolved = style.clone();
            services.resolve_font_families(&mut resolved, Some(character));
            if styles
                .last()
                .is_none_or(|(_, previous)| *previous != resolved)
            {
                styles.push((offset, resolved));
            }
        }
        // Canvas measures fractional advances; pixel quantization changes both
        // small-font widths and the result of fractional font-size changes.
        let mut builder = services.layout_context.style_run_builder(
            &mut services.font_context,
            &text,
            1.0,
            false,
        );
        for (index, (start, style)) in styles.iter().enumerate() {
            let end = styles
                .get(index + 1)
                .map_or(text.len(), |(offset, _)| *offset);
            let style_index = builder.push_style(style.clone());
            builder.push_style_run(style_index, *start..end);
        }
        let mut layout = builder.build(&text);
        layout.break_all_lines(None);
        let mut runs = Vec::new();
        for line in layout.lines() {
            let baseline = line.metrics().baseline;
            for item in line.items() {
                let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                let synthesis = run.synthesis();
                // Match the layout text bridge: a 500-weight request falling
                // back to regular is not faux bold in Chromium.
                let embolden = if font.weight >= 600.0 && synthesis.embolden() {
                    PaintPoint::new(
                        (run.font_size() * 0.015_125).min(0.3),
                        (run.font_size() * 0.012_1).min(0.3),
                    )
                } else {
                    PaintPoint::ZERO
                };
                let font = snapshot.intern_font(run.font());
                runs.push(PaintGlyphRun {
                    font,
                    font_size: run.font_size(),
                    normalized_coords: run.normalized_coords().to_vec(),
                    color: PaintColor::BLACK,
                    glyph_skew_radians: synthesis.skew().map(f32::to_radians),
                    glyph_embolden: embolden,
                    glyphs: glyph_run
                        .positioned_glyphs()
                        .map(|glyph| PaintGlyph {
                            id: glyph.id,
                            x: glyph.x,
                            y: glyph.y - baseline,
                        })
                        .collect(),
                    transform: PaintTransform2D::IDENTITY,
                });
            }
        }
        ShapedCanvasText {
            width: f64::from(layout.full_width()),
            fonts: snapshot.fonts,
            runs,
        }
    }
}

impl ShapedCanvasText {
    /// Outlines from the same glyph IDs/positions used by fillText. Kept apart
    /// from shaping so measureText and filled text do not pay outline costs.
    pub fn outline_path(&self) -> PaintPath {
        use kurbo::Shape;
        use skrifa::{FontRef, GlyphId, MetadataProvider, instance::Size, outline::DrawSettings};
        let mut pen = CanvasOutlinePen::default();
        for run in &self.runs {
            let data = &self.fonts[run.font.index()].font;
            let Ok(font) = FontRef::from_index(data.data.as_ref(), data.index) else {
                continue;
            };
            let outlines = font.outline_glyphs();
            let coords: Vec<_> = run
                .normalized_coords
                .iter()
                .map(|coord| skrifa::instance::NormalizedCoord::from_bits(*coord))
                .collect();
            pen.skew = run.glyph_skew_radians.map_or(0.0, f32::tan);
            for glyph in &run.glyphs {
                let Some(outline) = outlines.get(GlyphId::new(glyph.id)) else {
                    continue;
                };
                pen.origin = (glyph.x, glyph.y);
                let _ = outline.draw(
                    DrawSettings::unhinted(Size::new(run.font_size), coords.as_slice()),
                    &mut pen,
                );
            }
        }
        let bounds = pen.path.bounding_box();
        PaintPath {
            elements: pen
                .path
                .elements()
                .iter()
                .map(|element| match *element {
                    kurbo::PathEl::MoveTo(p) => PaintPathElement::MoveTo(paint_point(p)),
                    kurbo::PathEl::LineTo(p) => PaintPathElement::LineTo(paint_point(p)),
                    kurbo::PathEl::QuadTo(a, b) => {
                        PaintPathElement::QuadTo(paint_point(a), paint_point(b))
                    }
                    kurbo::PathEl::CurveTo(a, b, c) => {
                        PaintPathElement::CubicTo(paint_point(a), paint_point(b), paint_point(c))
                    }
                    kurbo::PathEl::ClosePath => PaintPathElement::Close,
                })
                .collect(),
            bounds: PaintRect::new(
                bounds.x0 as f32,
                bounds.y0 as f32,
                bounds.width() as f32,
                bounds.height() as f32,
            ),
        }
    }
}

fn paint_point(point: kurbo::Point) -> PaintPoint {
    PaintPoint::new(point.x as f32, point.y as f32)
}

#[derive(Default)]
struct CanvasOutlinePen {
    path: kurbo::BezPath,
    origin: (f32, f32),
    skew: f32,
}

impl CanvasOutlinePen {
    fn point(&self, x: f32, y: f32) -> kurbo::Point {
        // Font outlines are y-up, Canvas is y-down at the alphabetic baseline.
        kurbo::Point::new(
            f64::from(self.origin.0 + x + self.skew * y),
            f64::from(self.origin.1 - y),
        )
    }
}

impl skrifa::outline::OutlinePen for CanvasOutlinePen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(self.point(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(self.point(x, y));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.path.quad_to(self.point(cx, cy), self.point(x, y));
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        self.path
            .curve_to(self.point(ax, ay), self.point(bx, by), self.point(x, y));
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(size: f32) -> CanvasFont {
        CanvasFont {
            family: "CanvasFixture".into(),
            size,
            weight: 400.0,
            stretch: 100.0,
            style: WebFontStyle::Normal,
        }
    }

    #[test]
    fn canvas_text_shapes_registered_font_at_fractional_sizes_without_layout_pass() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        services
            .register_web_font(WebFontRegistration::new(
                "canvas-fixture",
                WebFontFace::new("CanvasFixture"),
                include_bytes!("../../tests/fixtures/moli-ahem.ttf").to_vec(),
            ))
            .unwrap();
        let text = services.shape_canvas_text("A A", &font(10.5));
        assert!((text.width - 18.9).abs() < 0.001, "{}", text.width);
        assert_eq!(
            text.runs.iter().map(|run| run.glyphs.len()).sum::<usize>(),
            3
        );
        assert!(!text.fonts.is_empty());
        assert_eq!(services.text_layout_passes, 0);
        let whitespace = services.shape_canvas_text("A\nA\t", &font(10.5));
        assert!((whitespace.width - 25.2).abs() < 0.001);
        assert_eq!(services.shape_canvas_text("", &font(10.5)).width, 0.0);
        assert_eq!(services.shape_canvas_text("A", &font(0.0)).width, 0.0);
        let outline = text.outline_path();
        assert!(!outline.elements.is_empty());
        assert!(outline.bounds.width > 0.0);
    }

    #[test]
    fn canvas_measurement_uses_per_glyph_advances_and_non_latin_outlines() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        services
            .register_web_font(WebFontRegistration::new(
                "canvas-hebrew",
                WebFontFace::new("CanvasFixture"),
                include_bytes!("../../tests/fixtures/moli-hebrew-emoji.ttf").to_vec(),
            ))
            .unwrap();
        let aleph = services.shape_canvas_text("א", &font(16.0));
        let bet = services.shape_canvas_text("ב", &font(16.0));
        assert!(aleph.width > bet.width, "{} vs {}", aleph.width, bet.width);
        assert!(!aleph.outline_path().elements.is_empty());
        let bigger = services.shape_canvas_text("א", &font(16.5));
        assert!((bigger.width / aleph.width - 16.5 / 16.0).abs() < 0.001);
    }
}
