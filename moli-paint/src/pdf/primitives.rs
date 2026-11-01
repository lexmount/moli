use std::collections::HashMap;

use anyrender::{Paint, Scene, recording::RenderCommand};
use krilla::{
    color::rgb,
    geom::{Path, PathBuilder, Transform},
    image::Image,
    num::NormalizedF32,
    paint::{Fill, FillRule, Stroke},
    surface::Surface,
};
use moli_layout::{
    PaintBlendMode, PaintCaptureSurface, PaintColor, PaintCompositeMode, PaintFragment,
    PaintGlyphRun, PaintSnapshot, PaintTransform2D,
};
use peniko::kurbo::{Affine, PathEl, Rect, Shape};

use super::{PdfError, text::FontCache};

pub(super) fn transform(affine: Affine) -> Transform {
    let [a, b, c, d, e, f] = affine.as_coeffs().map(|v| v as f32);
    Transform::from_row(a, b, c, d, e, f)
}
pub(super) fn path(shape: &impl Shape) -> Option<Path> {
    let mut out = PathBuilder::new();
    for el in shape.path_elements(0.1) {
        match el {
            PathEl::MoveTo(p) => out.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => out.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(p, q) => out.quad_to(p.x as f32, p.y as f32, q.x as f32, q.y as f32),
            PathEl::CurveTo(p, q, r) => out.cubic_to(
                p.x as f32, p.y as f32, q.x as f32, q.y as f32, r.x as f32, r.y as f32,
            ),
            PathEl::ClosePath => out.close(),
        }
    }
    out.finish()
}
pub(super) fn unit(value: f32) -> NormalizedF32 {
    NormalizedF32::new(if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    })
    .unwrap()
}
pub(super) fn fill(color: PaintColor) -> Fill {
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Fill {
        paint: rgb::Color::new(
            channel(color.red),
            channel(color.green),
            channel(color.blue),
        )
        .into(),
        opacity: unit(color.alpha),
        rule: FillRule::NonZero,
    }
}

/// Resolve matching pops once and mark enclosing groups that must be isolated
/// together. A destination mask must include the destination it modifies.
pub(super) fn scope_ends(fragments: &[PaintFragment]) -> Result<Vec<(usize, bool)>, PdfError> {
    let mut scopes = vec![(0, false); fragments.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, fragment) in fragments.iter().enumerate() {
        match fragment {
            PaintFragment::PushLayer {
                filter,
                blend_mode,
                composite,
                ..
            } => {
                if *composite != PaintCompositeMode::SrcOver
                    || *blend_mode == PaintBlendMode::PlusLighter
                {
                    // These operators consume the destination as well as their
                    // source. A source-only fallback would change the result.
                    let parent = stack.iter().rev().find(|&&parent| {
                        matches!(
                            fragments[parent],
                            PaintFragment::PushLayer {
                                composite: PaintCompositeMode::SrcOver,
                                ..
                            }
                        )
                    });
                    let Some(&parent) = parent else {
                        return Err(PdfError::Encoding(
                            "destination-dependent compositing requires an isolated parent layer"
                                .into(),
                        ));
                    };
                    scopes[parent].1 = true;
                }
                scopes[i].1 = filter.is_some()
                    || *blend_mode == PaintBlendMode::PlusLighter
                    || *composite != PaintCompositeMode::SrcOver;
                stack.push(i);
            }
            PaintFragment::PushClip { .. } => stack.push(i),
            PaintFragment::PopLayer => {
                let start = stack.pop().expect("validated paint scopes");
                scopes[start].0 = i;
            }
            _ => {}
        }
    }
    Ok(scopes)
}

pub(super) struct PdfPainter<'a, 's> {
    pub surface: &'a mut Surface<'s>,
    snapshot: &'a PaintSnapshot,
    pub source: Rect,
    page_height: f64,
    page_index: usize,
    fonts: &'a mut FontCache,
    images: &'a mut HashMap<(usize, bool), Image>,
    clips: Vec<Rect>,
    clip_shapes: Vec<PaintFragment>,
    layers: Vec<usize>,
}
impl<'a, 's> PdfPainter<'a, 's> {
    pub fn new(
        surface: &'a mut Surface<'s>,
        snapshot: &'a PaintSnapshot,
        source: Rect,
        page_height: f64,
        page_index: usize,
        fonts: &'a mut FontCache,
        images: &'a mut HashMap<(usize, bool), Image>,
    ) -> Self {
        Self {
            surface,
            snapshot,
            source,
            page_height,
            page_index,
            fonts,
            images,
            clips: vec![Rect::new(
                0.0,
                0.0,
                source.x1,
                snapshot.surface.css_height as f64,
            )],
            clip_shapes: Vec::new(),
            layers: Vec::new(),
        }
    }
    pub fn fill_canvas(&mut self) {
        self.surface
            .set_fill(Some(fill(self.snapshot.canvas_color)));
        self.surface.set_stroke(None);
        if let Some(path) = path(&self.source.intersect(Rect::new(
            0.0,
            0.0,
            self.snapshot.surface.css_width as f64,
            self.snapshot.surface.css_height as f64,
        ))) {
            self.surface.draw_path(&path);
        }
    }
    pub fn paint(
        &mut self,
        fragments: &[PaintFragment],
        scopes: &[(usize, bool)],
    ) -> Result<(), PdfError> {
        let mut i = 0;
        while i < fragments.len() {
            let fragment = &fragments[i];
            if matches!(fragment, PaintFragment::PushLayer { opacity, filter, .. }
                if *opacity <= 0.0 || matches!(filter, Some(moli_layout::PaintFilter::Opacity(value)) if *value <= 0.0))
            {
                i = scopes[i].0 + 1;
                continue;
            }
            if scopes[i].1 {
                let end = scopes[i].0;
                if self.rasterize(&fragments[i..=end])? {
                    self.effect_text(&fragments[i..=end])?;
                }
                i = end + 1;
                continue;
            }
            match fragment {
                PaintFragment::GlyphRun(run) => self.glyphs(run, false)?,
                PaintFragment::TextShadow(_) | PaintFragment::BoxShadow(_) => {
                    self.rasterize(std::slice::from_ref(fragment))?;
                }
                PaintFragment::PushClip {
                    shape,
                    transform: matrix,
                } => {
                    self.push_clip(shape, *matrix);
                    self.layers.push(1);
                }
                PaintFragment::PushLayer {
                    clip,
                    transform: matrix,
                    opacity,
                    blend_mode,
                    ..
                } => {
                    self.push_clip(clip, *matrix);
                    self.surface.push_isolated();
                    self.surface.push_blend_mode(blend(*blend_mode));
                    self.surface.push_opacity(unit(*opacity));
                    self.layers.push(4);
                }
                PaintFragment::PopLayer => {
                    for _ in 0..self.layers.pop().expect("validated layers") {
                        self.surface.pop();
                    }
                    self.clips.pop();
                    self.clip_shapes.pop();
                }
                PaintFragment::Image(image) if image.repeat_bounds.is_some() => {
                    // PDF's native image command draws one tile. Reuse the bounded
                    // raster fallback for a repeated brush without expanding tiles.
                    self.rasterize(std::slice::from_ref(fragment))?;
                }
                PaintFragment::Image(image) => {
                    let resource = self.snapshot.image(image.image).expect("validated image");
                    let interpolate = image.sampling == moli_layout::PaintImageSampling::Linear;
                    let key = (image.image.index(), interpolate);
                    if let std::collections::hash_map::Entry::Vacant(entry) = self.images.entry(key)
                    {
                        entry.insert(super::image::from_rgba(&resource.image, interpolate)?);
                    }
                    let data = self.images[&key].clone();
                    let matrix = crate::geometry::to_backend_transform(image.transform)
                        * Affine::translate((
                            image.destination.x as f64,
                            image.destination.y as f64,
                        ))
                        * Affine::scale_non_uniform(
                            image.destination.width as f64 / resource.image.width as f64,
                            image.destination.height as f64 / resource.image.height as f64,
                        );
                    self.surface.push_transform(&transform(matrix));
                    self.surface.draw_image(
                        data,
                        super::image_size(resource.image.width, resource.image.height)?,
                    );
                    self.surface.pop();
                }
                _ => {
                    // Share the existing lowering of border styles, decorations,
                    // gradients and SVG paths. Text bypasses AnyRender entirely.
                    let mut scene = Scene::new();
                    crate::scene::paint_fragment(&mut scene, self.snapshot, fragment, 1.0);
                    if supported(&scene) {
                        self.commands(scene)?;
                    } else {
                        self.rasterize(std::slice::from_ref(fragment))?;
                    }
                }
            }
            i += 1;
        }
        Ok(())
    }
    fn push_clip(&mut self, shape: &moli_layout::PaintShape, matrix: PaintTransform2D) {
        self.clip_shapes.push(PaintFragment::PushClip {
            shape: shape.clone(),
            transform: matrix,
        });
        let shape = crate::geometry::to_backend_shape(shape);
        let matrix = crate::geometry::to_backend_transform(matrix);
        let shape = matrix * shape;
        let bounds = self.clips.last().unwrap().intersect(shape.bounding_box());
        self.clips.push(bounds);
        // A degenerate clip must remain empty, never turn into an absent clip.
        let clip = path(&shape).or_else(|| path(&Rect::ZERO)).unwrap();
        self.surface.push_clip_path(&clip, &FillRule::NonZero);
    }
    fn glyphs(&mut self, run: &PaintGlyphRun, invisible: bool) -> Result<(), PdfError> {
        super::text::draw(
            self.surface,
            self.snapshot,
            run,
            self.fonts,
            *self.clips.last().unwrap(),
            self.source,
            &self.clip_shapes,
            self.page_height,
            self.page_index,
            invisible,
        )
    }
    fn effect_text(&mut self, fragments: &[PaintFragment]) -> Result<(), PdfError> {
        // A transparent text overlay is valid for ordinary filters, but not for
        // arbitrary destination masks: those can erase only part of a cluster.
        // Refuse that combination until its coverage can be proven, rather
        // than exporting invisible source text. Entirely transparent groups
        // have already been eliminated by the raster result's alpha channel.
        if fragments.iter().any(|f| {
            matches!(f,
            PaintFragment::PushLayer { composite, .. } if *composite != PaintCompositeMode::SrcOver)
        }) && fragments.iter().any(|f| {
            matches!(f,
                PaintFragment::GlyphRun(run) if run.text.is_some() && run.color.alpha > 0.0)
        }) {
            return Err(PdfError::Encoding(
                "searchable text inside destination masks is not supported".into(),
            ));
        }
        let mut mask_depth = 0usize;
        let mut masks = Vec::new();
        let mut hidden = 0usize;
        let mut result = Ok(());
        for fragment in fragments {
            match fragment {
                PaintFragment::PushLayer {
                    clip,
                    transform,
                    composite,
                    opacity,
                    filter,
                    ..
                } => {
                    let mask = *composite != PaintCompositeMode::SrcOver;
                    let transparent = *opacity <= 0.0
                        || matches!(filter, Some(moli_layout::PaintFilter::Opacity(value)) if *value <= 0.0);
                    masks.push((mask, transparent));
                    mask_depth += usize::from(mask);
                    hidden += usize::from(transparent);
                    self.push_clip(clip, *transform);
                }
                PaintFragment::PushClip { shape, transform } => {
                    masks.push((false, false));
                    self.push_clip(shape, *transform);
                }
                PaintFragment::PopLayer => {
                    let (mask, transparent) = masks.pop().unwrap();
                    mask_depth -= usize::from(mask);
                    hidden -= usize::from(transparent);
                    self.surface.pop();
                    self.clips.pop();
                    self.clip_shapes.pop();
                }
                PaintFragment::GlyphRun(run)
                    if mask_depth == 0 && hidden == 0 && run.text.is_some() && result.is_ok() =>
                {
                    result = self.glyphs(run, true);
                }
                _ => {}
            }
        }
        result
    }
    fn commands(&mut self, scene: Scene) -> Result<(), PdfError> {
        for command in scene.commands {
            match command {
                RenderCommand::Fill(cmd) => {
                    self.surface.push_transform(&transform(cmd.transform));
                    self.surface.set_stroke(None);
                    self.surface.set_fill(Some(
                        brush(&cmd.brush, cmd.brush_transform, cmd.fill).unwrap(),
                    ));
                    if let Some(shape) = path(&cmd.shape) {
                        self.surface.draw_path(&shape);
                    }
                    self.surface.pop();
                }
                RenderCommand::Stroke(cmd) => {
                    let fill =
                        brush(&cmd.brush, cmd.brush_transform, peniko::Fill::NonZero).unwrap();
                    self.surface.push_transform(&transform(cmd.transform));
                    self.surface.set_fill(None);
                    self.surface.set_stroke(Some(Stroke {
                        paint: fill.paint,
                        opacity: fill.opacity,
                        width: cmd.style.width as f32,
                        miter_limit: cmd.style.miter_limit as f32,
                        line_cap: match cmd.style.start_cap {
                            peniko::kurbo::Cap::Butt => krilla::paint::LineCap::Butt,
                            peniko::kurbo::Cap::Round => krilla::paint::LineCap::Round,
                            peniko::kurbo::Cap::Square => krilla::paint::LineCap::Square,
                        },
                        line_join: match cmd.style.join {
                            peniko::kurbo::Join::Bevel => krilla::paint::LineJoin::Bevel,
                            peniko::kurbo::Join::Round => krilla::paint::LineJoin::Round,
                            peniko::kurbo::Join::Miter => krilla::paint::LineJoin::Miter,
                        },
                        dash: (!cmd.style.dash_pattern.is_empty()).then(|| {
                            krilla::paint::StrokeDash {
                                array: cmd.style.dash_pattern.iter().map(|v| *v as f32).collect(),
                                offset: cmd.style.dash_offset as f32,
                            }
                        }),
                    }));
                    if let Some(shape) = path(&cmd.shape) {
                        self.surface.draw_path(&shape);
                    }
                    self.surface.pop();
                }
                RenderCommand::PushClipLayer(cmd) => {
                    self.surface.push_clip_path(
                        &path(&(cmd.transform * cmd.clip)).unwrap(),
                        &FillRule::NonZero,
                    );
                }
                RenderCommand::PopLayer => self.surface.pop(),
                _ => unreachable!("supported recording commands"),
            }
        }
        Ok(())
    }
    fn rasterize(&mut self, fragments: &[PaintFragment]) -> Result<bool, PdfError> {
        let Some(first) = fragments.first() else {
            return Ok(false);
        };
        let extent = if fragments.len() > 1 {
            effect_bounds(self.snapshot, fragments, &mut self.fonts.glyphs)?.unwrap_or(self.source)
        } else {
            fragment_bounds(self.snapshot, first, &mut self.fonts.glyphs)?.unwrap_or(self.source)
        };
        // Keep filter input beyond the page edge. Cropping it at the output
        // clip would remove pixels that blur back into the printable area.
        let padding = filter_padding(fragments);
        let bounds = extent.intersect(
            self.clips
                .last()
                .unwrap()
                .intersect(self.source)
                .inflate(padding, padding),
        );
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return Ok(false);
        }
        let bounds = bounds.expand();
        let mut snapshot = PaintSnapshot::new(self.snapshot.viewport, PaintColor::TRANSPARENT);
        snapshot.surface =
            PaintCaptureSurface::new(bounds.width() as f32, bounds.height() as f32, 2.0);
        snapshot.fonts = self.snapshot.fonts.clone();
        snapshot.images = self.snapshot.images.clone();
        snapshot.svg_images = self.snapshot.svg_images.clone();
        // PDF export omits backdrop-filter everywhere (and reports it), so a
        // rasterized effect group must not render it either.
        snapshot.fragments = fragments
            .iter()
            .filter(|fragment| !matches!(fragment, PaintFragment::BackdropFilter { .. }))
            .cloned()
            .collect();
        let outer_compositing = match snapshot.fragments.first_mut() {
            Some(PaintFragment::PushLayer {
                opacity,
                blend_mode,
                composite: PaintCompositeMode::SrcOver,
                ..
            }) => {
                let saved = (*opacity, *blend_mode);
                *opacity = 1.0;
                *blend_mode = PaintBlendMode::Normal;
                Some(saved)
            }
            _ => None,
        };
        let translation = Affine::translate((-bounds.x0, -bounds.y0));
        for fragment in &mut snapshot.fragments {
            translate_fragment(fragment, translation);
        }
        let image = crate::raster_snapshot(&snapshot)?;
        if image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 0)
        {
            return Ok(false);
        }
        let size = super::image_size(image.width, image.height)?;
        if let Some((opacity, mode)) = outer_compositing {
            self.surface.push_blend_mode(blend(mode));
            self.surface.push_opacity(unit(opacity));
        }
        self.surface.push_transform(&transform(
            Affine::translate((bounds.x0, bounds.y0)) * Affine::scale(0.5),
        ));
        self.surface.draw_image(
            Image::from_rgba8(image.rgba, image.width, image.height),
            size,
        );
        self.surface.pop();
        if outer_compositing.is_some() {
            self.surface.pop();
            self.surface.pop();
        }
        Ok(true)
    }
}
fn fragment_bounds(
    snapshot: &PaintSnapshot,
    fragment: &PaintFragment,
    glyphs: &mut anyrender_vello_cpu::GlyphMeasurer,
) -> Result<Option<Rect>, PdfError> {
    let (bounds, matrix) = match fragment {
        PaintFragment::Fill {
            shape, transform, ..
        }
        | PaintFragment::PushClip { shape, transform } => (super::rect(shape.bounds()), *transform),
        PaintFragment::PushLayer {
            clip, transform, ..
        } => (super::rect(clip.bounds()), *transform),
        PaintFragment::SvgImage(image) => (super::rect(image.destination), image.transform),
        PaintFragment::Image(image) => (
            super::rect(image.repeat_bounds.unwrap_or(image.destination)),
            image.transform,
        ),
        PaintFragment::Border {
            rect, transform, ..
        } => (super::rect(*rect), *transform),
        PaintFragment::Stroke(stroke) => (
            super::rect(stroke.path.bounds).inflate(
                (stroke.width * stroke.miter_limit.max(1.0)) as f64,
                (stroke.width * stroke.miter_limit.max(1.0)) as f64,
            ),
            stroke.transform,
        ),
        PaintFragment::TextDecoration(decoration) => (
            Rect::new(
                decoration.x as f64,
                decoration.y as f64,
                (decoration.x + decoration.width) as f64,
                decoration.y as f64,
            )
            .inflate(
                decoration.thickness as f64 * 4.0,
                decoration.thickness as f64 * 4.0,
            ),
            decoration.transform,
        ),
        PaintFragment::GlyphRun(run) => {
            return super::text::ink_bounds(snapshot, run, glyphs).map(Some);
        }
        PaintFragment::BoxShadow(shadow) => (
            super::rect(shadow.rect).inflate(
                (shadow.blur_radius * 3.0 + shadow.spread_radius.abs() + shadow.offset.x.abs())
                    as f64,
                (shadow.blur_radius * 3.0 + shadow.spread_radius.abs() + shadow.offset.y.abs())
                    as f64,
            ),
            shadow.transform,
        ),
        PaintFragment::TextShadow(shadow) => {
            let mut bounds = Rect::ZERO;
            for (i, g) in shadow.run.glyphs.iter().enumerate() {
                let point = Rect::new(g.x as f64, g.y as f64, g.x as f64, g.y as f64);
                bounds = if i == 0 { point } else { bounds.union(point) };
            }
            let padding = (shadow.run.font_size * 2.0
                + shadow.blur_radius * 3.0
                + shadow.offset.x.abs().max(shadow.offset.y.abs()))
                as f64;
            (bounds.inflate(padding, padding), shadow.run.transform)
        }
        _ => return Ok(None),
    };
    Ok(Some(
        crate::geometry::to_backend_transform(matrix).transform_rect_bbox(bounds),
    ))
}
fn translate_fragment(fragment: &mut PaintFragment, translation: Affine) {
    let matrix = match fragment {
        PaintFragment::PushLayer { transform, .. }
        | PaintFragment::PushClip { transform, .. }
        | PaintFragment::Fill { transform, .. }
        | PaintFragment::Border { transform, .. } => transform,
        PaintFragment::Stroke(stroke) => &mut stroke.transform,
        PaintFragment::BoxShadow(shadow) => &mut shadow.transform,
        PaintFragment::TextDecoration(decoration) => &mut decoration.transform,
        PaintFragment::TextShadow(shadow) => &mut shadow.run.transform,
        PaintFragment::GlyphRun(run) => &mut run.transform,
        PaintFragment::Image(image) => &mut image.transform,
        PaintFragment::SvgImage(image) => &mut image.transform,
        PaintFragment::BackdropFilter { transform, .. } => transform,
        PaintFragment::PopLayer | PaintFragment::BackdropRoot => return,
    };
    *matrix = PaintTransform2D::new(
        (translation * crate::geometry::to_backend_transform(*matrix)).as_coeffs(),
    );
}

/// For curved/rotated clips a bounding-box intersection is not sufficient to
/// decide whether text exists in the visible result. Probe only the candidate
/// cluster's clipped ink, with the same CPU clip implementation as screenshots.
/// The PDF glyph itself is still emitted natively. Axis-aligned rectangular
/// clips are already resolved exactly by the caller's bounds intersection.
pub(super) fn visible_cluster_bounds(
    snapshot: &PaintSnapshot,
    run: &PaintGlyphRun,
    range: std::ops::Range<usize>,
    bounds: Rect,
    clips: &[PaintFragment],
    whitespace: bool,
) -> Result<Option<Rect>, PdfError> {
    if clips.iter().all(|clip| {
        matches!(clip,
        PaintFragment::PushClip { shape: moli_layout::PaintShape::Rect(_), transform }
            if transform.coefficients[1] == 0.0 && transform.coefficients[2] == 0.0)
    }) {
        return Ok(Some(bounds));
    }
    let bounds = bounds.expand();
    let mut probe = PaintSnapshot::new(snapshot.viewport, PaintColor::TRANSPARENT);
    probe.surface = PaintCaptureSurface::new(bounds.width() as f32, bounds.height() as f32, 2.0);
    probe.fonts = snapshot.fonts.clone();
    probe.fragments = clips.to_vec();
    let mut run = run.clone();
    run.glyphs = run.glyphs[range].to_vec();
    run.text = None;
    if whitespace {
        probe.fragments.push(PaintFragment::Fill {
            fill_rule: moli_layout::PaintFillRule::NonZero,
            shape: moli_layout::PaintShape::Rect(moli_layout::PaintRect::new(
                bounds.x0 as f32,
                bounds.y0 as f32,
                bounds.width() as f32,
                bounds.height() as f32,
            )),
            brush: moli_layout::PaintBrush::Solid(PaintColor::BLACK),
            transform: PaintTransform2D::IDENTITY,
        });
    } else {
        probe.fragments.push(PaintFragment::GlyphRun(run));
    }
    probe
        .fragments
        .extend(clips.iter().map(|_| PaintFragment::PopLayer));
    let translation = Affine::translate((-bounds.x0, -bounds.y0));
    for fragment in &mut probe.fragments {
        translate_fragment(fragment, translation);
    }
    let pixels = crate::raster_snapshot(&probe)?;
    let mut covered: Option<Rect> = None;
    for (i, pixel) in pixels.rgba.as_chunks::<4>().0.iter().enumerate() {
        if pixel[3] != 0 {
            let x = bounds.x0 + (i % pixels.width as usize) as f64 * 0.5;
            let y = bounds.y0 + (i / pixels.width as usize) as f64 * 0.5;
            let pixel = Rect::new(x, y, x + 0.5, y + 0.5);
            covered = Some(covered.map_or(pixel, |rect| rect.union(pixel)));
        }
    }
    Ok(covered)
}
fn supported(scene: &Scene) -> bool {
    scene.commands.iter().all(|command| match command {
        RenderCommand::Fill(cmd) => brush(&cmd.brush, cmd.brush_transform, cmd.fill).is_some(),
        RenderCommand::Stroke(cmd) => {
            cmd.style.start_cap == cmd.style.end_cap
                && brush(&cmd.brush, cmd.brush_transform, peniko::Fill::NonZero).is_some()
        }
        RenderCommand::PushClipLayer(_) | RenderCommand::PopLayer => true,
        _ => false,
    })
}
fn brush(paint: &Paint, matrix: Option<Affine>, rule: peniko::Fill) -> Option<Fill> {
    let fill_rule = match rule {
        peniko::Fill::NonZero => FillRule::NonZero,
        peniko::Fill::EvenOdd => FillRule::EvenOdd,
    };
    match paint {
        Paint::Solid(color) => {
            let [r, g, b, a] = color.components;
            let mut result = fill(PaintColor::new(r, g, b, a));
            result.rule = fill_rule;
            Some(result)
        }
        Paint::Gradient(gradient) => {
            use krilla::paint::{LinearGradient, RadialGradient, SpreadMethod, Stop};
            // Preserve non-sRGB interpolation and repeating radial gradients
            // through the CPU fallback; PDF's native shading differs there.
            if gradient.interpolation_cs != peniko::color::ColorSpaceTag::Srgb {
                return None;
            }
            let stops = gradient
                .stops
                .iter()
                .map(|stop| {
                    let color = stop.color.to_alpha_color::<peniko::color::Srgb>();
                    let [r, g, b, a] = color.components;
                    Stop {
                        offset: unit(stop.offset),
                        color: rgb::Color::new(
                            (r * 255.0).round() as u8,
                            (g * 255.0).round() as u8,
                            (b * 255.0).round() as u8,
                        )
                        .into(),
                        opacity: unit(a),
                    }
                })
                .collect();
            let spread_method = match gradient.extend {
                peniko::Extend::Pad => SpreadMethod::Pad,
                peniko::Extend::Repeat => SpreadMethod::Repeat,
                peniko::Extend::Reflect => SpreadMethod::Reflect,
            };
            let matrix = transform(matrix.unwrap_or(Affine::IDENTITY));
            let paint = match gradient.kind {
                peniko::GradientKind::Linear(line) => LinearGradient {
                    x1: line.start.x as f32,
                    y1: line.start.y as f32,
                    x2: line.end.x as f32,
                    y2: line.end.y as f32,
                    transform: matrix,
                    spread_method,
                    stops,
                    anti_alias: true,
                }
                .into(),
                peniko::GradientKind::Radial(radial) if gradient.extend == peniko::Extend::Pad => {
                    RadialGradient {
                        fx: radial.start_center.x as f32,
                        fy: radial.start_center.y as f32,
                        fr: radial.start_radius,
                        cx: radial.end_center.x as f32,
                        cy: radial.end_center.y as f32,
                        cr: radial.end_radius,
                        transform: matrix,
                        spread_method,
                        stops,
                        anti_alias: true,
                    }
                    .into()
                }
                _ => return None,
            };
            Some(Fill {
                paint,
                opacity: NormalizedF32::ONE,
                rule: fill_rule,
            })
        }
        _ => None,
    }
}
fn blend(mode: PaintBlendMode) -> krilla::blend::BlendMode {
    use krilla::blend::BlendMode as B;
    match mode {
        PaintBlendMode::Normal | PaintBlendMode::PlusLighter => B::Normal,
        PaintBlendMode::Multiply => B::Multiply,
        PaintBlendMode::Screen => B::Screen,
        PaintBlendMode::Overlay => B::Overlay,
        PaintBlendMode::Darken => B::Darken,
        PaintBlendMode::Lighten => B::Lighten,
        PaintBlendMode::ColorDodge => B::ColorDodge,
        PaintBlendMode::ColorBurn => B::ColorBurn,
        PaintBlendMode::HardLight => B::HardLight,
        PaintBlendMode::SoftLight => B::SoftLight,
        PaintBlendMode::Difference => B::Difference,
        PaintBlendMode::Exclusion => B::Exclusion,
        PaintBlendMode::Hue => B::Hue,
        PaintBlendMode::Saturation => B::Saturation,
        PaintBlendMode::Color => B::Color,
        PaintBlendMode::Luminosity => B::Luminosity,
    }
}

impl Drop for PdfPainter<'_, '_> {
    fn drop(&mut self) {
        // Krilla requires balanced scopes even when encoding fails partway through a page.
        for count in self.layers.drain(..).rev() {
            for _ in 0..count {
                self.surface.pop();
            }
        }
    }
}

fn filter_padding(fragments: &[PaintFragment]) -> f64 {
    fragments
        .iter()
        .filter_map(|fragment| {
            let PaintFragment::PushLayer {
                filter: Some(filter),
                transform,
                ..
            } = fragment
            else {
                return None;
            };
            let radius = match filter {
                moli_layout::PaintFilter::Blur(radius) => radius * 3.0,
                moli_layout::PaintFilter::DropShadow {
                    offset,
                    blur_radius,
                    ..
                } => blur_radius * 3.0 + offset.x.abs().max(offset.y.abs()),
                _ => 0.0,
            };
            let [a, b, c, d, _, _] = transform.coefficients;
            Some(radius as f64 * (a.abs() + c.abs()).max(b.abs() + d.abs()))
        })
        .sum()
}
fn effect_bounds(
    snapshot: &PaintSnapshot,
    fragments: &[PaintFragment],
    glyphs: &mut anyrender_vello_cpu::GlyphMeasurer,
) -> Result<Option<Rect>, PdfError> {
    let mut bounds: Option<Rect> = None;
    for fragment in fragments {
        if matches!(
            fragment,
            PaintFragment::PushLayer { .. }
                | PaintFragment::PushClip { .. }
                | PaintFragment::PopLayer
        ) {
            continue;
        }
        let Some(local) = fragment_bounds(snapshot, fragment, glyphs)? else {
            return Ok(None);
        };
        if local.width() > 0.0 && local.height() > 0.0 {
            bounds = Some(bounds.map_or(local, |bounds| bounds.union(local)));
        }
    }
    let padding = filter_padding(fragments);
    Ok(Some(bounds.map_or(Rect::ZERO, |bounds| {
        bounds.inflate(padding, padding)
    })))
}
