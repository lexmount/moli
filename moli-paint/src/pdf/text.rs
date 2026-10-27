use std::collections::HashMap;

use anyrender_vello_cpu::{GlyphInkBounds, GlyphMeasurer};

use krilla::{
    geom::{Path, Point},
    graphic::Graphic,
    paint::{Fill, LineJoin, Stroke},
    surface::Surface,
    text::{Font, GlyphId, KrillaGlyph, Tag},
};
use moli_layout::{PaintGlyph, PaintGlyphRun, PaintSnapshot, PaintTextStroke};
use peniko::kurbo::{self, Affine, BezPath, Diagonal2, Join, PathEl, Rect, Vec2};
use skrifa::raw::{FontRef, TableProvider};
use skrifa::{
    MetadataProvider,
    instance::{NormalizedCoord, Size},
    metrics::GlyphMetrics,
    outline::DrawSettings,
};

use super::{
    PdfError,
    primitives::{fill, transform},
};

#[derive(Default)]
pub(super) struct FontCache {
    fonts: HashMap<(usize, Vec<i16>), Font>,
    pub(super) glyphs: GlyphMeasurer,
    // This cache lives for one PDF export, across runs and pages. Positions and
    // paint order do not affect the expanded contour or its reusable graphics.
    expanded: HashMap<ExpandedGlyphKey, Option<ExpandedGlyph>>,
    #[cfg(test)]
    pub(super) expanded_glyphs: Vec<u32>,
}
impl FontCache {
    fn get(&mut self, snapshot: &PaintSnapshot, run: &PaintGlyphRun) -> Result<Font, PdfError> {
        let key = (run.font.index(), run.normalized_coords.clone());
        if let Some(font) = self.fonts.get(&key) {
            return Ok(font.clone());
        }
        let data = &snapshot.font(run.font).expect("validated font").font;
        let coords = variation_coordinates(data.data.as_ref(), data.index, &run.normalized_coords)?;
        let (bytes, _) = data.data.clone().into_raw_parts();
        let font = Font::new_variable(bytes.into(), data.index, &coords)
            .ok_or_else(|| PdfError::Encoding("font cannot be embedded".into()))?;
        self.fonts.insert(key, font.clone());
        Ok(font)
    }
}

// Keep PDF font semantics separate from the shared prepared-glyph ink query.
// Native culling and raster allocation consume the CPU adapter's bounds for
// the selected representation; unavailable geometry retains the em guard.
struct GlyphGeometry<'a> {
    font: FontRef<'a>,
    metrics: GlyphMetrics<'a>,
    ink: Vec<GlyphInkBounds>,
    font_size: f32,
    skew: f32,
    embolden: Diagonal2,
    ink_outset: f64,
}

impl<'a> GlyphGeometry<'a> {
    fn new(
        snapshot: &'a PaintSnapshot,
        run: &PaintGlyphRun,
        coords: &'a [NormalizedCoord],
        stroke: Option<PaintTextStroke>,
        raster: bool,
        cache: &mut GlyphMeasurer,
    ) -> Result<Self, PdfError> {
        let data = &snapshot.font(run.font).expect("validated font").font;
        let face = skrifa::FontRef::from_index(data.data.as_ref(), data.index)
            .map_err(|error| PdfError::Encoding(error.to_string()))?;
        // Snapshot synthesis and stroke widths are both local CSS pixels.
        let embolden = Diagonal2::new(
            f64::from(run.glyph_embolden.x.max(0.0)),
            f64::from(run.glyph_embolden.y.max(0.0)),
        );
        let mut ink = measure_ink(snapshot, run, stroke, raster, cache);
        if !raster {
            // Krilla picks the largest bitmap strike and applies its bearings
            // differently from the CPU renderer. The adapter's image bounds describe
            // its own prepared representation, so retain the native fallback for
            // these glyphs instead of treating CPU placement as native PDF ink.
            let bitmaps = face.bitmap_strikes();
            if bitmaps.format().is_some() {
                let colors = face.color_glyphs();
                for (ink, glyph) in ink.iter_mut().zip(&run.glyphs) {
                    let id = skrifa::GlyphId::new(glyph.id);
                    if colors.get(id).is_none()
                        && bitmaps.glyph_for_size(Size::unscaled(), id).is_some()
                    {
                        *ink = GlyphInkBounds::Unavailable;
                    }
                }
            }
        }
        Ok(Self {
            font: face.clone(),
            metrics: face.glyph_metrics(skrifa::instance::Size::new(run.font_size), coords),
            ink,
            font_size: run.font_size,
            skew: run.glyph_skew_radians.unwrap_or_default().tan(),
            embolden,
            ink_outset: f64::from(PaintTextStroke::ink_outset(
                stroke.map_or(0.0, |stroke| stroke.width),
                embolden.xx.max(embolden.yy) as f32,
            )),
        })
    }

    fn native_artwork(&self, id: skrifa::GlyphId) -> bool {
        self.font.color_glyphs().get(id).is_some()
            || self
                .font
                .bitmap_strikes()
                .glyph_for_size(Size::unscaled(), id)
                .is_some()
    }

    fn expanded_outline(&self, id: skrifa::GlyphId, coords: &'a [NormalizedCoord]) -> Option<Path> {
        use skrifa::outline::pen::PathElement as E;
        // COLR and bitmap glyphs use their native artwork. Moli does not install
        // Krilla's SVG renderer, so an SVG record still falls back to this base
        // outline. Cacheability does not depend on a run's clip or transform.
        if self.native_artwork(id) {
            return None;
        }
        let outline = self.font.outline_glyphs().get(id)?;
        let mut elements = Vec::new();
        outline
            .draw(
                DrawSettings::unhinted(Size::new(self.font_size), coords),
                &mut elements,
            )
            .ok()?;
        let point = |x, y| (f64::from(x), f64::from(y)).into();
        let path = BezPath::from_vec(
            elements
                .into_iter()
                .map(|element| match element {
                    E::MoveTo { x, y } => PathEl::MoveTo(point(x, y)),
                    E::LineTo { x, y } => PathEl::LineTo(point(x, y)),
                    E::QuadTo { cx0, cy0, x, y } => PathEl::QuadTo(point(cx0, cy0), point(x, y)),
                    E::CurveTo {
                        cx0,
                        cy0,
                        cx1,
                        cy1,
                        x,
                        y,
                    } => PathEl::CurveTo(point(cx0, cy0), point(cx1, cy1), point(x, y)),
                    E::Close => PathEl::ClosePath,
                })
                .collect(),
        );
        // Match Glifo's expansion controls in local CSS pixels. Stroking this
        // contour preserves its inner edge as well as its outer ink extent.
        let path = kurbo::expand_path(
            &path,
            self.embolden,
            Join::Miter,
            f64::from(PaintTextStroke::MITER_LIMIT),
            0.1,
        );
        super::primitives::path(&(Affine::FLIP_Y * path))
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

fn measure_ink(
    snapshot: &PaintSnapshot,
    run: &PaintGlyphRun,
    stroke: Option<PaintTextStroke>,
    raster: bool,
    cache: &mut GlyphMeasurer,
) -> Vec<GlyphInkBounds> {
    let font = &snapshot.font(run.font).expect("validated font").font;
    // PDF filter fallbacks render at two samples per CSS pixel. Preparing at
    // that device scale also matches vertical hinting and baseline snapping.
    let scale = if raster { 2.0 } else { 1.0 };
    let matrix = Affine::scale(scale) * crate::geometry::to_backend_transform(run.transform);
    let x = f64::from(run.glyph_embolden.x.max(0.0));
    let y = f64::from(run.glyph_embolden.y.max(0.0));
    let embolden = Vec2::new(x, y);
    let glyph_transform = Some(Affine::skew(
        -f64::from(run.glyph_skew_radians.unwrap_or_default()).tan(),
        0.0,
    ));
    let glyphs = || {
        run.glyphs.iter().map(|glyph| anyrender::Glyph {
            id: glyph.id,
            x: glyph.x,
            y: glyph.y,
        })
    };
    let mut measure = |hint, stroke: Option<&peniko::kurbo::Stroke>| {
        cache.measure(
            font,
            run.font_size,
            hint,
            &run.normalized_coords,
            embolden,
            stroke,
            matrix,
            glyph_transform,
            glyphs(),
        )
    };
    let mut ink = measure(raster && embolden == Vec2::ZERO, None);
    if let Some(stroke) = stroke {
        let stroke = peniko::kurbo::Stroke::new(f64::from(stroke.width))
            .with_join(Join::Miter)
            .with_miter_limit(f64::from(PaintTextStroke::MITER_LIMIT));
        let stroked = measure(false, Some(&stroke));
        for (fill, stroke) in ink.iter_mut().zip(stroked) {
            *fill = match (*fill, stroke) {
                (GlyphInkBounds::Unavailable, _) | (_, GlyphInkBounds::Unavailable) => {
                    GlyphInkBounds::Unavailable
                }
                (GlyphInkBounds::Empty, ink) | (ink, GlyphInkBounds::Empty) => ink,
                (
                    GlyphInkBounds::Bounds { bounds: fill, .. },
                    GlyphInkBounds::Bounds {
                        representation,
                        bounds: stroke,
                    },
                ) => GlyphInkBounds::Bounds {
                    representation,
                    bounds: fill.union(stroke),
                },
            };
        }
    }
    for ink in &mut ink {
        if let GlyphInkBounds::Bounds { bounds, .. } = ink {
            *bounds = Affine::scale(1.0 / scale).transform_rect_bbox(*bounds);
        }
    }

    ink
}

pub(super) fn ink_bounds(
    snapshot: &PaintSnapshot,
    run: &PaintGlyphRun,
    cache: &mut GlyphMeasurer,
) -> Result<Rect, PdfError> {
    if run.glyphs.is_empty() || !run.font_size.is_finite() || run.font_size <= 0.0 {
        return Ok(Rect::ZERO);
    }
    let coords = normalized_coords(run);
    let geometry = GlyphGeometry::new(snapshot, run, &coords, visible_stroke(run), true, cache)?;
    let matrix = crate::geometry::to_backend_transform(run.transform);
    let mut bounds: Option<Rect> = None;
    for (glyph, ink) in run.glyphs.iter().zip(&geometry.ink) {
        let local = match *ink {
            GlyphInkBounds::Empty => continue,
            GlyphInkBounds::Bounds { bounds, .. } => bounds,
            GlyphInkBounds::Unavailable => {
                let guard = f64::from(run.font_size) * 2.0;
                geometry.map_bounds(Rect::new(-guard, -guard, guard, guard), glyph, matrix)
            }
        };
        bounds = Some(bounds.map_or(local, |bounds| bounds.union(local)));
    }
    // Geometric ink excludes device sampling. Keep an antialiasing guard;
    // hinting itself is already included by the prepared-glyph query.
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

#[derive(Clone, PartialEq, Eq, Hash)]
struct ExpandedGlyphKey {
    // Krilla's font identity includes the font data, collection index and
    // variation location, independently of a run's position and paint.
    font: Font,
    glyph: u32,
    font_size_bits: u32,
    embolden_bits: [u32; 2],
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ExpandedPaint {
    Fill(Fill),
    Stroke(Stroke),
}

struct ExpandedGlyph {
    path: Path,
    graphics: HashMap<ExpandedPaint, Graphic>,
}

impl ExpandedGlyph {
    fn prepare_graphic(&mut self, surface: &mut Surface<'_>, paint: ExpandedPaint) {
        self.graphics
            .entry(paint.clone())
            .or_insert_with(|| match paint {
                ExpandedPaint::Fill(fill) => glyph_graphic(surface, &self.path, Some(fill), None),
                ExpandedPaint::Stroke(stroke) => {
                    glyph_graphic(surface, &self.path, None, Some(stroke))
                }
            });
    }
}

fn glyph_graphic(
    surface: &mut Surface<'_>,
    path: &Path,
    fill: Option<Fill>,
    stroke: Option<Stroke>,
) -> Graphic {
    let mut stream = surface.stream_builder();
    let mut glyph = stream.surface();
    glyph.set_fill(fill);
    glyph.set_stroke(stroke);
    glyph.draw_path(path);
    glyph.finish();
    Graphic::new(stream.finish(), false)
}

fn stroke_paint(stroke: PaintTextStroke) -> Stroke {
    let paint = fill(stroke.color);
    Stroke {
        paint: paint.paint,
        opacity: paint.opacity,
        width: stroke.width,
        line_join: LineJoin::Miter,
        miter_limit: PaintTextStroke::MITER_LIMIT,
        ..Default::default()
    }
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
    let geometry = GlyphGeometry::new(
        snapshot,
        run,
        &coords,
        text_stroke,
        false,
        &mut fonts.glyphs,
    )?;
    let metrics = &geometry.metrics;
    let skew = geometry.skew;
    let embolden = geometry.embolden;
    let synthesized = embolden != Diagonal2::new(0.0, 0.0);
    let fallback_embolden = embolden.xx.max(embolden.yy) as f32;
    // Native fill-and-stroke can represent an opaque isotropic expansion.
    // Color artwork has no expandable base contour and must be painted once.
    // SVG fallback depends on this glyph's record, not the font's other glyphs.
    // Keep translucent fills on the outline/artwork pass: Krilla splits a
    // translucent native fill-and-stroke into two operations, including COLR.
    let native_fill = |id: skrifa::GlyphId| {
        !synthesized
            || (run.color.alpha >= 1.0
                && (geometry.native_artwork(id)
                    || (embolden.xx == embolden.yy
                        && !geometry
                            .font
                            .svg()
                            .and_then(|table| table.glyph_data(id))
                            .is_ok_and(|data| data.is_some()))))
    };
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
        for (glyph, ink) in run.glyphs[start..end].iter().zip(&geometry.ink[start..end]) {
            let id = skrifa::GlyphId::new(glyph.id);
            let local = match *ink {
                GlyphInkBounds::Bounds { bounds, .. } => bounds,
                GlyphInkBounds::Empty => {
                    geometry.map_bounds(geometry.advance_bounds(id), glyph, matrix)
                }
                GlyphInkBounds::Unavailable => {
                    let guard = f64::from(font_size) * 2.0;
                    geometry.map_bounds(
                        Rect::new(-guard, -guard, guard, guard).union(geometry.advance_bounds(id)),
                        glyph,
                        matrix,
                    )
                }
            };
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
    // Prepare only glyph IDs that survived this page's cluster/clip checks.
    // Cache the local contour and painted graphics for the entire document;
    // each occurrence only supplies its placement transform.
    let key = |glyph| ExpandedGlyphKey {
        font: font.clone(),
        glyph,
        font_size_bits: font_size.to_bits(),
        embolden_bits: [
            (embolden.xx as f32).to_bits(),
            (embolden.yy as f32).to_bits(),
        ],
    };
    let fill_key = ExpandedPaint::Fill(fill(run.color));
    let stroke_key = visible_stroke.map(|stroke| ExpandedPaint::Stroke(stroke_paint(stroke)));
    if synthesized && !invisible {
        for (outlined, indices) in &batches {
            for &i in indices {
                let glyph = run.glyphs[i];
                let id = skrifa::GlyphId::new(glyph.id);
                let path_fill = has_fill && (*outlined || !native_fill(id));
                if geometry.native_artwork(id) || (!path_fill && visible_stroke.is_none()) {
                    continue;
                }
                let entry = fonts.expanded.entry(key(glyph.id)).or_insert_with(|| {
                    #[cfg(test)]
                    fonts.expanded_glyphs.push(glyph.id);
                    geometry
                        .expanded_outline(skrifa::GlyphId::new(glyph.id), &coords)
                        .map(|path| ExpandedGlyph {
                            path,
                            graphics: HashMap::new(),
                        })
                });
                if let Some(glyph) = entry {
                    if path_fill {
                        glyph.prepare_graphic(surface, fill_key.clone());
                    }
                    if let Some(stroke) = &stroke_key {
                        glyph.prepare_graphic(surface, stroke.clone());
                    }
                }
            }
        }
    }
    let expanded = &fonts.expanded;
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
        if invisible {
            if !outlined {
                surface.draw_invisible_glyphs(
                    Point::from_xy(0.0, 0.0),
                    &glyphs,
                    font.clone(),
                    source_text,
                    font_size,
                );
            }
            surface.pop();
            continue;
        }
        // Choose the synthetic passes from the style, then resolve each
        // glyph's contour or native outline fallback independently below.
        // A neighboring glyph must not change a fallback glyph's paint mode.
        let expanded_stroke = synthesized && visible_stroke.is_some();
        let expanded_fill = synthesized
            && has_fill
            && indices.iter().any(|&i| {
                let id = skrifa::GlyphId::new(run.glyphs[i].id);
                (outlined && !geometry.native_artwork(id)) || !native_fill(id)
            });
        let semantic_copy = !outlined && (expanded_fill || (!has_fill && expanded_stroke));
        if semantic_copy {
            surface.draw_invisible_glyphs(
                Point::from_xy(0.0, 0.0),
                &glyphs,
                font.clone(),
                source_text,
                font_size,
            );
        }
        let draw_paths = |surface: &mut Surface<'_>, filling: bool| {
            for &i in &indices {
                let glyph = run.glyphs[i];
                // Krilla's COLR/bitmap stroke fallback paints the artwork as a
                // fill. If a fill pass already paints it, do not composite the
                // same translucent layers again in the stroke pass.
                if !filling && has_fill && geometry.native_artwork(skrifa::GlyphId::new(glyph.id)) {
                    continue;
                }
                let dy = glyph.y - first.y;
                surface.push_transform(&transform(Affine::translate((
                    f64::from(glyph.x - first.x + skew * dy),
                    f64::from(dy),
                ))));
                let graphic = expanded
                    .get(&key(glyph.id))
                    .and_then(Option::as_ref)
                    .and_then(|glyph| {
                        let paint = if filling {
                            &fill_key
                        } else {
                            stroke_key.as_ref().unwrap()
                        };
                        glyph.graphics.get(paint)
                    });
                if let Some(graphic) = graphic {
                    surface.draw_graphic(graphic.clone());
                } else {
                    // Keep the existing native fallback for image glyphs or
                    // unavailable outlines, without adding another text copy.
                    let paint = fill(run.color);
                    surface.set_fill(filling.then(|| paint.clone()));
                    surface.set_stroke(if filling {
                        synthesized.then(|| Stroke {
                            paint: paint.paint,
                            opacity: paint.opacity,
                            width: 2.0 * fallback_embolden,
                            line_join: LineJoin::Miter,
                            miter_limit: PaintTextStroke::MITER_LIMIT,
                            ..Default::default()
                        })
                    } else {
                        visible_stroke.map(|stroke| Stroke {
                            width: stroke.width + 2.0 * fallback_embolden,
                            ..stroke_paint(stroke)
                        })
                    });
                    let native = KrillaGlyph {
                        glyph_id: GlyphId::new(glyph.id),
                        text_range: run
                            .text
                            .as_ref()
                            .map_or(0..0, |text| text.clusters[i].clone()),
                        x_advance: 0.0,
                        y_advance: 0.0,
                        x_offset: 0.0,
                        y_offset: 0.0,
                        location: None,
                    };
                    surface.draw_glyphs(
                        Point::from_xy(0.0, 0.0),
                        &[native],
                        font.clone(),
                        source_text,
                        font_size,
                        true,
                    );
                }
                surface.pop();
            }
        };
        let draw_stroke = |surface: &mut Surface<'_>| {
            if let Some(stroke) = visible_stroke {
                if expanded_stroke
                    || (has_fill
                        && indices.iter().any(|&i| {
                            geometry.native_artwork(skrifa::GlyphId::new(run.glyphs[i].id))
                        }))
                {
                    draw_paths(surface, false);
                } else {
                    surface.set_fill(None);
                    surface.set_stroke(Some(Stroke {
                        width: stroke.width + 2.0 * fallback_embolden,
                        ..stroke_paint(stroke)
                    }));
                    surface.draw_glyphs(
                        Point::from_xy(0.0, 0.0),
                        &glyphs,
                        font.clone(),
                        source_text,
                        font_size,
                        outlined || has_fill || semantic_copy,
                    );
                }
            }
        };
        let stroke_first = visible_stroke.is_some_and(|stroke| stroke.paint_first);
        if stroke_first || !has_fill {
            draw_stroke(surface);
        }
        if has_fill {
            if expanded_fill {
                draw_paths(surface, true);
            } else {
                let paint = fill(run.color);
                surface.set_stroke(synthesized.then(|| Stroke {
                    paint: paint.paint.clone(),
                    opacity: paint.opacity,
                    width: 2.0 * fallback_embolden,
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
                    outlined || semantic_copy,
                );
            }
            if !stroke_first {
                draw_stroke(surface);
            }
        }
        surface.set_stroke(None);
        surface.pop();
    }
    Ok(())
}
