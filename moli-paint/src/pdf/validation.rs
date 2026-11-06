use moli_layout::*;

use super::PdfError;

// Validate owned commands before passing coordinates to Krilla (or a local
// raster fallback). Resource identities, operation counts and scope depths are
// checked by the shared paint validator.
pub(super) fn validate(snapshot: &PaintSnapshot) -> Result<(), PdfError> {
    if snapshot.links.len() > 100_000 {
        return Err(PdfError::ResourceLimit);
    }
    for link in &snapshot.links {
        if link.before_fragment > snapshot.fragments.len()
            || !rect(link.rect)
            || link.rect.width < 0.0
            || link.rect.height < 0.0
            || !link
                .transform
                .coefficients
                .into_iter()
                .all(|v| (v as f32).is_finite())
            || link.target.destination.is_some_and(|p| !point(p))
            || link.target.url.is_empty()
            || link.target.url.len() > 65_536
        {
            return Err(PdfError::Encoding("invalid hyperlink metadata".into()));
        }
    }
    for fragment in &snapshot.fragments {
        let (valid, matrix) = match fragment {
            PaintFragment::PushLayer {
                clip,
                transform,
                opacity,
                filter,
                ..
            } => (
                shape(clip) && opacity.is_finite() && filter.is_none_or(valid_filter),
                *transform,
            ),
            PaintFragment::PushClip {
                shape: value,
                transform,
            } => (shape(value), *transform),
            PaintFragment::Fill {
                shape: value,
                brush: paint,
                transform,
                ..
            } => (shape(value) && brush(paint), *transform),
            PaintFragment::Stroke(stroke) => (
                path(&stroke.path)
                    && finite([stroke.width, stroke.miter_limit, stroke.dash_offset])
                    && stroke.dash_pattern.iter().all(|v| v.is_finite()),
                stroke.transform,
            ),
            PaintFragment::Border {
                rect: value,
                widths,
                radii,
                transform,
                ..
            } => (
                rect(*value)
                    && finite([widths.top, widths.right, widths.bottom, widths.left])
                    && corners(*radii),
                *transform,
            ),
            PaintFragment::BoxShadow(shadow) => (
                rect(shadow.rect)
                    && corners(shadow.radii)
                    && point(shadow.offset)
                    && finite([shadow.blur_radius, shadow.spread_radius]),
                shadow.transform,
            ),
            PaintFragment::TextDecoration(value) => (
                finite([value.x, value.y, value.width, value.thickness]),
                value.transform,
            ),
            PaintFragment::GlyphRun(run) => (glyphs(run), run.transform),
            PaintFragment::TextShadow(shadow) => (
                glyphs(&shadow.run) && point(shadow.offset) && shadow.blur_radius.is_finite(),
                shadow.run.transform,
            ),
            PaintFragment::Image(image) => (
                rect(image.destination) && image.repeat_bounds.is_none_or(rect),
                image.transform,
            ),
            PaintFragment::SvgImage(image) => (rect(image.destination), image.transform),
            PaintFragment::PopLayer
            | PaintFragment::BackdropRoot
            | PaintFragment::BackdropFilter { .. } => continue,
        };
        if !valid
            || !matrix
                .coefficients
                .into_iter()
                .all(|v| (v as f32).is_finite())
        {
            return Err(PdfError::Encoding(
                "invalid paint geometry or text mapping".into(),
            ));
        }
    }
    Ok(())
}
fn finite<const N: usize>(values: [f32; N]) -> bool {
    values.into_iter().all(f32::is_finite)
}
fn point(p: PaintPoint) -> bool {
    finite([p.x, p.y])
}
fn rect(r: PaintRect) -> bool {
    finite([r.x, r.y, r.width, r.height, r.x + r.width, r.y + r.height])
}
fn corners(c: PaintCornerRadii) -> bool {
    [c.top_left, c.top_right, c.bottom_left, c.bottom_right]
        .into_iter()
        .all(|r| finite([r.x, r.y]))
}
fn shape(s: &PaintShape) -> bool {
    match s {
        PaintShape::Rect(r) => rect(*r),
        PaintShape::RoundedRect { rect: r, radii } => rect(*r) && corners(*radii),
        PaintShape::Path(p) => path(p),
    }
}
fn path(p: &PaintPath) -> bool {
    rect(p.bounds)
        && p.elements.iter().all(|element| match *element {
            PaintPathElement::MoveTo(a) | PaintPathElement::LineTo(a) => point(a),
            PaintPathElement::QuadTo(a, b) => point(a) && point(b),
            PaintPathElement::CubicTo(a, b, c) => point(a) && point(b) && point(c),
            PaintPathElement::Close => true,
        })
}
fn glyphs(run: &PaintGlyphRun) -> bool {
    run.font_size.is_finite()
        && point(run.glyph_embolden)
        && run
            .glyph_skew_radians
            .is_none_or(|v| v.is_finite() && v.tan().is_finite())
        && run.glyphs.iter().all(|g| finite([g.x, g.y]))
        && run
            .stroke
            .is_none_or(|s| s.width.is_finite() && s.width >= 0.0)
        && run.text.as_ref().is_none_or(|t| {
            t.clusters.len() == run.glyphs.len()
                && t.clusters.iter().all(|r| t.text.get(r.clone()).is_some())
        })
}
fn stops(values: &[PaintGradientStop]) -> bool {
    values.iter().all(|s| s.offset.is_finite())
}
fn brush(b: &PaintBrush) -> bool {
    match b {
        PaintBrush::Solid(_) => true,
        PaintBrush::LinearGradient(g) => point(g.start) && point(g.end) && stops(&g.stops),
        PaintBrush::RadialGradient(g) => {
            point(g.start_center)
                && point(g.end_center)
                && finite([g.start_radius, g.end_radius])
                && stops(&g.stops)
                && g.transform.is_finite()
        }
        PaintBrush::ConicGradient(g) => {
            point(g.center)
                && finite([g.start_angle_radians, g.end_angle_radians])
                && stops(&g.stops)
                && g.transform.is_finite()
        }
    }
}
fn valid_filter(f: PaintFilter) -> bool {
    match f {
        PaintFilter::Blur(v)
        | PaintFilter::Brightness(v)
        | PaintFilter::Contrast(v)
        | PaintFilter::Grayscale(v)
        | PaintFilter::HueRotate(v)
        | PaintFilter::Invert(v)
        | PaintFilter::Opacity(v)
        | PaintFilter::Saturate(v)
        | PaintFilter::Sepia(v) => v.is_finite(),
        PaintFilter::DropShadow {
            offset,
            blur_radius,
            ..
        } => point(offset) && blur_radius.is_finite(),
    }
}
