use std::collections::HashMap;

use krilla::{
    geom::Point,
    paint::{LineJoin, Stroke},
    surface::Surface,
    text::{Font, GlyphId, KrillaGlyph, Tag},
};
use moli_layout::{PaintGlyph, PaintGlyphRun, PaintSnapshot, PaintTextStroke};
use peniko::kurbo::{Affine, Rect};
use skrifa::raw::{FontRef, TableProvider};
use skrifa::{MetadataProvider, instance::NormalizedCoord, metrics::GlyphMetrics};

use super::{
    PdfError,
    primitives::{fill, transform},
};

#[derive(Default)]
pub(super) struct FontCache(HashMap<(usize, Vec<i16>), Font>);
impl FontCache {
    fn get(&mut self, snapshot: &PaintSnapshot, run: &PaintGlyphRun) -> Result<Font, PdfError> {
        let key = (run.font.index(), run.normalized_coords.clone());
        if let Some(font) = self.0.get(&key) {
            return Ok(font.clone());
        }
        let data = &snapshot.font(run.font).expect("validated font").font;
        let coords = variation_coordinates(data.data.as_ref(), data.index, &run.normalized_coords)?;
        let (bytes, _) = data.data.clone().into_raw_parts();
        let font = Font::new_variable(bytes.into(), data.index, &coords)
            .ok_or_else(|| PdfError::Encoding("font cannot be embedded".into()))?;
        self.0.insert(key, font.clone());
        Ok(font)
    }
}

// As in SkPDFStrike::Make and SkStrokeRec::GetInflationRadius, resolve font
// metrics together with paint. Native culling and filter allocation then share
// the same outline, synthesis, miter extent, and transform interpretation.
struct GlyphGeometry<'a> {
    metrics: GlyphMetrics<'a>,
    raw_glyf_bounds: bool,
    font_size: f32,
    skew: f32,
    embolden: f32,
    ink_outset: f64,
}

impl<'a> GlyphGeometry<'a> {
    fn new(
        snapshot: &'a PaintSnapshot,
        run: &PaintGlyphRun,
        coords: &'a [NormalizedCoord],
        stroke: Option<PaintTextStroke>,
    ) -> Result<Self, PdfError> {
        let data = &snapshot.font(run.font).expect("validated font").font;
        let face = skrifa::FontRef::from_index(data.data.as_ref(), data.index)
            .map_err(|error| PdfError::Encoding(error.to_string()))?;
        // Snapshot synthesis and stroke widths are both local CSS pixels.
        let embolden = run.glyph_embolden.x.max(run.glyph_embolden.y).max(0.0);
        Ok(Self {
            metrics: face.glyph_metrics(skrifa::instance::Size::new(run.font_size), coords),
            raw_glyf_bounds: face.glyf().is_ok() && face.gvar().is_err(),
            font_size: run.font_size,
            skew: run.glyph_skew_radians.unwrap_or_default().tan(),
            embolden,
            ink_outset: f64::from(PaintTextStroke::ink_outset(
                stroke.map_or(0.0, |stroke| stroke.width),
                embolden,
            )),
        })
    }

    fn outline_bounds(&self, id: skrifa::GlyphId) -> Option<Rect> {
        let bounds = self.metrics.bounds(id)?;
        // Static glyf metrics retain raw xMin, while the outline scaler
        // shifts the origin to the hmtx side bearing. Variable/CFF metrics
        // already come from the scaled outline and need no adjustment.
        let left = if self.raw_glyf_bounds {
            self.metrics.left_side_bearing(id).unwrap_or(bounds.x_min)
        } else {
            bounds.x_min
        };
        Some(Rect::new(
            left as f64,
            -bounds.y_max as f64,
            (left + bounds.x_max - bounds.x_min) as f64,
            -bounds.y_min as f64,
        ))
    }

    fn advance_bounds(&self, id: skrifa::GlyphId) -> Rect {
        // Keep spaces searchable even when they have no outline.
        Rect::new(
            0.0,
            -self.font_size as f64,
            self.metrics.advance_width(id).unwrap_or(self.font_size) as f64,
            self.font_size as f64 * 0.2,
        )
    }

    fn map_bounds(&self, bounds: Rect, glyph: &PaintGlyph, matrix: Affine) -> Rect {
        let matrix = matrix
            * Affine::translate((glyph.x as f64, glyph.y as f64))
            * Affine::skew(-self.skew as f64, 0.0);
        matrix.transform_rect_bbox(bounds.inflate(self.ink_outset, self.ink_outset))
    }
}

fn normalized_coords(run: &PaintGlyphRun) -> Vec<NormalizedCoord> {
    run.normalized_coords
        .iter()
        .map(|&v| NormalizedCoord::from_bits(v))
        .collect()
}

fn visible_stroke(run: &PaintGlyphRun) -> Option<PaintTextStroke> {
    run.stroke
        .filter(|stroke| stroke.width.is_finite() && stroke.width > 0.0 && stroke.color.alpha > 0.0)
}

pub(super) fn ink_bounds(snapshot: &PaintSnapshot, run: &PaintGlyphRun) -> Result<Rect, PdfError> {
    let coords = normalized_coords(run);
    let geometry = GlyphGeometry::new(snapshot, run, &coords, visible_stroke(run))?;
    let matrix = crate::geometry::to_backend_transform(run.transform);
    let mut bounds: Option<Rect> = None;
    for glyph in &run.glyphs {
        // Bitmap/color fonts can lack outline metrics. Retain the old em
        // guard only for these glyphs, rather than padding every glyph run.
        let guard = f64::from(run.font_size) * 2.0;
        let local = match geometry.outline_bounds(skrifa::GlyphId::new(glyph.id)) {
            Some(bounds) if bounds.width() <= 0.0 || bounds.height() <= 0.0 => continue,
            Some(bounds) => bounds,
            None => Rect::new(-guard, -guard, guard, guard),
        };
        // Hinting can move the outline in local glyph space; account for it
        // before scaling or skewing, unlike the device sampling guard below.
        let local = geometry.map_bounds(local.inflate(1.0, 1.0), glyph, matrix);
        bounds = Some(bounds.map_or(local, |bounds| bounds.union(local)));
    }
    // Raster allocation also needs room for hinting and antialiasing at the
    // device boundary; this does not change native PDF text ownership.
    Ok(bounds.map_or(Rect::ZERO, |bounds| bounds.inflate(1.0, 1.0)))
}

// Parley exports post-avar normalized coordinates; Krilla takes design-space
// axes. Invert the avar v1 piecewise maps before denormalizing with fvar.
pub(super) fn variation_coordinates(
    data: &[u8],
    index: u32,
    coords: &[i16],
) -> Result<Vec<(Tag, f32)>, PdfError> {
    if coords.iter().all(|v| *v == 0) {
        return Ok(Vec::new());
    }
    let font =
        FontRef::from_index(data, index).map_err(|error| PdfError::Encoding(error.to_string()))?;
    let fvar = font
        .fvar()
        .map_err(|error| PdfError::Encoding(error.to_string()))?;
    let axes = fvar
        .axes()
        .map_err(|error| PdfError::Encoding(error.to_string()))?;
    let avar = font.avar().ok();
    if avar.as_ref().is_some_and(|avar| avar.version().major > 1) {
        return Err(PdfError::Encoding(
            "avar v2 font instances are not supported by the PDF adapter".into(),
        ));
    }
    let mut result = Vec::new();
    for (i, axis) in axes.iter().enumerate() {
        let mut value = f32::from(coords.get(i).copied().unwrap_or(0)) / 16384.0;
        if let Some(map) = avar
            .as_ref()
            .and_then(|a| a.axis_segment_maps().get(i)?.ok())
        {
            for pair in map.axis_value_maps().windows(2) {
                let low = pair[0].to_coordinate().to_f32();
                let high = pair[1].to_coordinate().to_f32();
                if value >= low && value <= high && high > low {
                    let from = pair[0].from_coordinate().to_f32();
                    let to = pair[1].from_coordinate().to_f32();
                    value = from + (to - from) * (value - low) / (high - low);
                    break;
                }
            }
        }
        let default = axis.default_value().to_f32();
        let value = default
            + value
                * if value < 0.0 {
                    default - axis.min_value().to_f32()
                } else {
                    axis.max_value().to_f32() - default
                };
        result.push((Tag::new(&axis.axis_tag().to_be_bytes()), value));
    }
    Ok(result)
}

pub(super) fn draw(
    surface: &mut Surface<'_>,
    snapshot: &PaintSnapshot,
    run: &PaintGlyphRun,
    fonts: &mut FontCache,
    clip: Rect,
    page_clip: Rect,
    clip_shapes: &[moli_layout::PaintFragment],
    page_height: f64,
    page_index: usize,
    invisible: bool,
) -> Result<(), PdfError> {
    let text_stroke = visible_stroke(run);
    let visible_stroke = text_stroke.filter(|_| !invisible);
    let has_fill = run.color.alpha > 0.0;
    if run.glyphs.is_empty()
        || !run.font_size.is_finite()
        || run.font_size <= 0.0
        || (!has_fill && text_stroke.is_none())
        || clip.width() <= 0.0
        || clip.height() <= 0.0
    {
        return Ok(());
    }
    if let Some(text) = &run.text
        && (text.clusters.len() != run.glyphs.len()
            || text
                .clusters
                .iter()
                .any(|range| text.text.get(range.clone()).is_none()))
    {
        return Err(PdfError::Encoding(
            "invalid glyph-to-text cluster mapping".into(),
        ));
    }
    let matrix = crate::geometry::to_backend_transform(run.transform);
    let font = fonts.get(snapshot, run)?;
    let font_size = run.font_size;
    let source_text = run.text.as_ref().map_or("", |text| text.text.as_ref());
    let coords = normalized_coords(run);
    let geometry = GlyphGeometry::new(snapshot, run, &coords, text_stroke)?;
    let metrics = &geometry.metrics;
    let skew = geometry.skew;
    let embolden = geometry.embolden;
    let mut batches: Vec<(bool, Vec<usize>)> = Vec::new();
    let mut start = 0;
    while start < run.glyphs.len() {
        let range = run
            .text
            .as_ref()
            .map_or(0..0, |text| text.clusters[start].clone());
        let mut end = start + 1;
        if let Some(text) = &run.text {
            while end < run.glyphs.len() && text.clusters[end] == range {
                end += 1;
            }
        }
        let last_page = (f64::from(snapshot.surface.css_height) / page_height)
            .ceil()
            .max(1.0) as usize
            - 1;
        let mut bounds: Option<Rect> = None;
        for glyph in &run.glyphs[start..end] {
            let id = skrifa::GlyphId::new(glyph.id);
            let local = geometry
                .outline_bounds(id)
                .filter(|bounds| bounds.width() > 0.0 && bounds.height() > 0.0)
                .unwrap_or_else(|| geometry.advance_bounds(id));
            let local = geometry.map_bounds(local, glyph, matrix);
            bounds = Some(bounds.map_or(local, |bounds| bounds.union(local)));
        }
        let bounds = bounds.unwrap().intersect(clip);
        let on_page = bounds.intersect(page_clip);
        if on_page.width() > 0.0
            && on_page.height() > 0.0
            && let Some(visible) = super::primitives::visible_cluster_bounds(
                snapshot,
                run,
                start..end,
                bounds,
                clip_shapes,
                source_text
                    .get(range)
                    .is_some_and(|text| !text.is_empty() && text.chars().all(char::is_whitespace)),
            )?
        {
            // Use visible document-space ink before the page clip. If a clip
            // erases one half of boundary text, its remaining ink owns the text.
            let owner =
                ((visible.center().y / page_height).floor().max(0.0) as usize).min(last_page);
            let on_page = visible.intersect(page_clip);
            if on_page.width() <= 0.0
                || on_page.height() <= 0.0
                || (invisible && owner != page_index)
            {
                start = end;
                continue;
            }
            let outlined = run.text.is_none() || owner != page_index;
            if batches
                .last()
                .is_none_or(|(previous, _)| *previous != outlined)
            {
                batches.push((outlined, Vec::new()));
            }
            batches.last_mut().unwrap().1.extend(start..end);
        }
        start = end;
    }
    for (outlined, indices) in batches {
        let first = run.glyphs[indices[0]];
        surface.push_transform(&transform(
            matrix
                * Affine::translate((first.x as f64, first.y as f64))
                * Affine::skew(-(skew as f64), 0.0),
        ));
        let glyphs: Vec<_> = indices
            .iter()
            .enumerate()
            .map(|(position, &i)| {
                let glyph = run.glyphs[i];
                let next = indices
                    .get(position + 1)
                    .map(|&i| run.glyphs[i].x)
                    .unwrap_or_else(|| {
                        glyph.x
                            + metrics
                                .advance_width(skrifa::GlyphId::new(glyph.id))
                                .unwrap_or(font_size)
                    });
                KrillaGlyph {
                    glyph_id: GlyphId::new(glyph.id),
                    text_range: run
                        .text
                        .as_ref()
                        .map_or(0..0, |text| text.clusters[i].clone()),
                    x_advance: (next - glyph.x) / font_size,
                    y_advance: 0.0,
                    x_offset: skew * (glyph.y - first.y) / font_size,
                    y_offset: -(glyph.y - first.y) / font_size,
                    location: None,
                }
            })
            .collect();
        // `-webkit-text-stroke`: a separate stroke-only pass, before or after
        // the fill per `paint-order`. Only the first drawn pass carries the
        // searchable text; the other is emitted as outlines.
        let draw_stroke = |surface: &mut Surface<'_>, outlined: bool| {
            if let Some(stroke) = visible_stroke {
                let paint = fill(stroke.color);
                surface.set_fill(None);
                surface.set_stroke(Some(Stroke {
                    paint: paint.paint,
                    opacity: paint.opacity,
                    width: stroke.width + 2.0 * embolden,
                    line_join: LineJoin::Miter,
                    miter_limit: PaintTextStroke::MITER_LIMIT,
                    ..Default::default()
                }));
                surface.draw_glyphs(
                    Point::from_xy(0.0, 0.0),
                    &glyphs,
                    font.clone(),
                    source_text,
                    font_size,
                    outlined,
                );
            }
        };
        let stroke_first = visible_stroke.is_some_and(|stroke| stroke.paint_first);
        if stroke_first || !has_fill {
            draw_stroke(surface, outlined);
        }
        // Filtered hollow text still needs an invisible fill pass to retain
        // its searchable text after the visible stroke was rasterized.
        if has_fill || invisible {
            let mut paint = fill(run.color);
            if invisible {
                paint.opacity = krilla::num::NormalizedF32::ZERO;
            }
            surface.set_stroke((embolden > 0.0).then(|| Stroke {
                paint: paint.paint.clone(),
                opacity: paint.opacity,
                width: 2.0 * embolden,
                line_join: LineJoin::Miter,
                miter_limit: PaintTextStroke::MITER_LIMIT,
                ..Default::default()
            }));
            surface.set_fill(Some(paint));
            surface.draw_glyphs(
                Point::from_xy(0.0, 0.0),
                &glyphs,
                font.clone(),
                source_text,
                font_size,
                outlined || stroke_first,
            );
            if !stroke_first {
                draw_stroke(surface, true);
            }
        }
        surface.set_stroke(None);
        surface.pop();
    }
    Ok(())
}
