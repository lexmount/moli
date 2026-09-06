//! Adapter between the browser-independent Canvas path geometry
//! ([`moli_canvas::path::CanvasPath`]) and the page-paint types
//! (`moli-layout`).
//!
//! The path geometry and its current-path transform semantics live in
//! `moli-canvas`, backed by kurbo-native types. This module only converts an
//! already-built native path into the `moli-layout` snapshot shapes that the
//! page rasterizer (vello) can fill and stroke. It holds no drawing state.

use kurbo::{Affine, Point, Rect};
use moli_canvas::path::CanvasPathData;
use moli_layout::{LayoutPoint, PaintPath, PaintPathElement, PaintRect, PaintTransform2D};

/// Converts a native canvas path into a `moli-layout` `PaintPath`. Elements are
/// stored by `moli-canvas` with the same f32 truncation this adapter consumes.
pub(super) fn native_paint_path(data: &CanvasPathData) -> PaintPath {
    PaintPath {
        elements: data.elements.iter().map(path_element_to_layout).collect(),
        bounds: paint_rect(data.bounds),
    }
}

/// Converts the current canvas transform (user space -> canvas space) into a
/// `moli-layout` paint transform.
pub(super) fn native_transform(affine: Affine) -> PaintTransform2D {
    PaintTransform2D::new(affine.as_coeffs())
}

/// Converts a kurbo rectangle (in canvas pixel space) into a `moli-layout`
/// paint rectangle. `moli-canvas` stores f32-truncated bounds, so the `f32`
/// casts recover the original values exactly.
fn paint_rect(rect: Rect) -> PaintRect {
    PaintRect::new(
        rect.x0 as f32,
        rect.y0 as f32,
        rect.width() as f32,
        rect.height() as f32,
    )
}

fn path_element_to_layout(element: &kurbo::PathEl) -> PaintPathElement {
    match *element {
        kurbo::PathEl::MoveTo(p) => PaintPathElement::MoveTo(layout_point(p)),
        kurbo::PathEl::LineTo(p) => PaintPathElement::LineTo(layout_point(p)),
        kurbo::PathEl::QuadTo(a, p) => PaintPathElement::QuadTo(layout_point(a), layout_point(p)),
        kurbo::PathEl::CurveTo(a, b, p) => {
            PaintPathElement::CubicTo(layout_point(a), layout_point(b), layout_point(p))
        }
        kurbo::PathEl::ClosePath => PaintPathElement::Close,
    }
}

fn layout_point(p: Point) -> LayoutPoint {
    LayoutPoint::new(p.x as f32, p.y as f32)
}
