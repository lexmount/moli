use std::collections::HashMap;

use krilla::{
    geom::Point,
    paint::Stroke,
    surface::Surface,
    text::{Font, GlyphId, KrillaGlyph, Tag},
};
use moli_layout::{PaintGlyphRun, PaintSnapshot};
use peniko::kurbo::{Affine, Rect};
use skrifa::raw::{FontRef, TableProvider};

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
    let text_stroke = run.stroke.filter(|stroke| {
        !invisible && stroke.width.is_finite() && stroke.width > 0.0 && stroke.color.alpha > 0.0
    });
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
    let skew = run.glyph_skew_radians.unwrap_or_default().tan();
    let source_text = run.text.as_ref().map_or("", |text| text.text.as_ref());
    let data = &snapshot.font(run.font).expect("validated font").font;
    use skrifa::MetadataProvider;
    let face = skrifa::FontRef::from_index(data.data.as_ref(), data.index)
        .map_err(|error| PdfError::Encoding(error.to_string()))?;
    let coords: Vec<_> = run
        .normalized_coords
        .iter()
        .map(|v| skrifa::instance::NormalizedCoord::from_bits(*v))
        .collect();
    let metrics = face.glyph_metrics(skrifa::instance::Size::new(font_size), coords.as_slice());
    // The owned snapshot stores synthetic weight expansion in CSS pixels,
    // which are also the units used by PDF stroke widths.
    let embolden = run.glyph_embolden.x.max(run.glyph_embolden.y).max(0.0);
    let ink_outset = f64::from(embolden + text_stroke.map_or(0.0, |stroke| stroke.width * 0.5));
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
            let glyph_bounds = metrics
                .bounds(id)
                .filter(|b| b.x_max > b.x_min && b.y_max > b.y_min);
            // Spaces and bitmap/color fonts may have no outline box. Retain
            // their advance box, so spaces remain actual text in the PDF.
            let local = glyph_bounds.map_or_else(
                || {
                    Rect::new(
                        0.0,
                        -font_size as f64,
                        metrics.advance_width(id).unwrap_or(font_size) as f64,
                        font_size as f64 * 0.2,
                    )
                },
                |b| {
                    Rect::new(
                        b.x_min as f64,
                        -b.y_max as f64,
                        b.x_max as f64,
                        -b.y_min as f64,
                    )
                },
            );
            let glyph_matrix = matrix
                * Affine::translate((glyph.x as f64, glyph.y as f64))
                * Affine::skew(-skew as f64, 0.0);
            // Stroke width is local to the glyph's transform. Expand before
            // mapping so scaled/skewed ink reaches clip and page boundaries.
            let local = glyph_matrix.transform_rect_bbox(local.inflate(ink_outset, ink_outset));
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
            if let Some(stroke) = text_stroke {
                let paint = fill(stroke.color);
                surface.set_fill(None);
                surface.set_stroke(Some(Stroke {
                    paint: paint.paint,
                    opacity: paint.opacity,
                    width: stroke.width + 2.0 * embolden,
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
        let stroke_first = text_stroke.is_some_and(|stroke| stroke.paint_first);
        if stroke_first || !has_fill {
            draw_stroke(surface, outlined);
        }
        if has_fill {
            let mut paint = fill(run.color);
            if invisible {
                paint.opacity = krilla::num::NormalizedF32::ZERO;
            }
            surface.set_stroke((embolden > 0.0).then(|| Stroke {
                paint: paint.paint.clone(),
                opacity: paint.opacity,
                width: 2.0 * embolden,
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
