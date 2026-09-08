//! Native form-control appearance currently owned by Blitz.
//!
//! Checkbox and radio geometry/color behavior is a DOM-neutral port of
//! `blitz-paint/src/render/form_controls.rs` at Blitz d788124a. Other controls
//! deliberately retain their CSS background/border/text until Blitz supplies
//! an implementation or a Chromium compatibility contract is added here.

use std::{fmt::Debug, hash::Hash};

use super::geometry::canonical_shape;
use crate::{
    LayoutBox, LayoutElementCategory, LayoutFormControlKind, LayoutInputControlKind, LayoutRect,
    LayoutTransform2D, PaintBrush, PaintColor, PaintCornerRadii, PaintFragment, PaintLineCap,
    PaintLineJoin, PaintPath, PaintPathElement, PaintSnapshot, PaintStroke,
};

pub(super) fn project_form_control_appearance<N>(
    layout_box: &LayoutBox<N>,
    rect: LayoutRect,
    transform: LayoutTransform2D,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let Some(semantics) = layout_box.element_semantics() else {
        return;
    };
    let LayoutElementCategory::FormControl(kind) = semantics.category else {
        return;
    };
    if !semantics.is_replaced() || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let data = semantics
        .metadata
        .form_control
        .as_ref()
        .cloned()
        .unwrap_or_default();
    let LayoutFormControlKind::Input(
        control @ (LayoutInputControlKind::Checkbox | LayoutInputControlKind::Radio),
    ) = kind
    else {
        return;
    };
    let accent = if data.disabled {
        PaintColor::new(209.0 / 255.0, 209.0 / 255.0, 209.0 / 255.0, 1.0)
    } else {
        layout_box.style.text_color()
    };
    let minimum = rect.width.min(rect.height);
    let scale = (minimum - 4.0).max(0.0) / 16.0;
    let frame_radius = scale * 2.0;
    let frame_radii =
        PaintCornerRadii::all(crate::PaintCornerRadius::new(frame_radius, frame_radius));

    let mut fill = |shape, color| {
        snapshot.push_fragment(PaintFragment::Fill {
            shape,
            brush: PaintBrush::Solid(color),
            transform,
        });
    };
    match control {
        LayoutInputControlKind::Checkbox => {
            let shape = canonical_shape(rect, frame_radii);
            if data.checked {
                fill(shape, accent);
                let path = PaintPath {
                    elements: [(2.0, 9.0), (6.0, 13.0), (14.0, 2.0)]
                        .into_iter()
                        .enumerate()
                        .map(|(index, (x, y))| {
                            let point = crate::LayoutPoint::new(
                                rect.x + (x + 2.0) * scale,
                                rect.y + (y + 1.0) * scale,
                            );
                            if index == 0 {
                                PaintPathElement::MoveTo(point)
                            } else {
                                PaintPathElement::LineTo(point)
                            }
                        })
                        .collect(),
                    bounds: rect,
                };
                snapshot.push_fragment(PaintFragment::Stroke(PaintStroke {
                    path,
                    brush: PaintBrush::Solid(PaintColor::WHITE),
                    width: 2.0 * scale,
                    join: PaintLineJoin::Round,
                    start_cap: PaintLineCap::Round,
                    end_cap: PaintLineCap::Round,
                    miter_limit: 10.0,
                    dash_pattern: Vec::new(),
                    dash_offset: 0.0,
                    transform,
                }));
            } else {
                fill(shape, PaintColor::WHITE);
                snapshot.push_fragment(PaintFragment::Stroke(PaintStroke {
                    path: rounded_rect_path(rect, frame_radius),
                    brush: PaintBrush::Solid(accent),
                    width: 1.0,
                    join: PaintLineJoin::Miter,
                    start_cap: PaintLineCap::Butt,
                    end_cap: PaintLineCap::Butt,
                    miter_limit: 4.0,
                    dash_pattern: Vec::new(),
                    dash_offset: 0.0,
                    transform,
                }));
            }
        }
        LayoutInputControlKind::Radio => {
            let center =
                crate::LayoutPoint::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
            let circle = |radius: f32| {
                let diameter = radius * 2.0;
                canonical_shape(
                    LayoutRect::new(center.x - radius, center.y - radius, diameter, diameter),
                    PaintCornerRadii::all(crate::PaintCornerRadius::new(radius, radius)),
                )
            };
            if data.checked {
                fill(circle(8.0 * scale), accent);
                fill(circle(6.0 * scale), PaintColor::WHITE);
                fill(circle(4.0 * scale), accent);
            } else {
                fill(circle(8.0 * scale), PaintColor::new(0.5, 0.5, 0.5, 1.0));
                fill(circle(6.0 * scale), PaintColor::WHITE);
            }
        }
        _ => unreachable!("the match above limits native appearance to checkbox/radio"),
    }
}

fn rounded_rect_path(rect: LayoutRect, radius: f32) -> PaintPath {
    const KAPPA: f32 = 0.552_284_8;
    let radius = radius
        .max(0.0)
        .min(rect.width.max(0.0) / 2.0)
        .min(rect.height.max(0.0) / 2.0);
    let x0 = rect.x;
    let y0 = rect.y;
    let x1 = rect.right();
    let y1 = rect.bottom();
    PaintPath {
        elements: vec![
            PaintPathElement::MoveTo(crate::LayoutPoint::new(x0 + radius, y0)),
            PaintPathElement::LineTo(crate::LayoutPoint::new(x1 - radius, y0)),
            PaintPathElement::CubicTo(
                crate::LayoutPoint::new(x1 - radius + radius * KAPPA, y0),
                crate::LayoutPoint::new(x1, y0 + radius - radius * KAPPA),
                crate::LayoutPoint::new(x1, y0 + radius),
            ),
            PaintPathElement::LineTo(crate::LayoutPoint::new(x1, y1 - radius)),
            PaintPathElement::CubicTo(
                crate::LayoutPoint::new(x1, y1 - radius + radius * KAPPA),
                crate::LayoutPoint::new(x1 - radius + radius * KAPPA, y1),
                crate::LayoutPoint::new(x1 - radius, y1),
            ),
            PaintPathElement::LineTo(crate::LayoutPoint::new(x0 + radius, y1)),
            PaintPathElement::CubicTo(
                crate::LayoutPoint::new(x0 + radius - radius * KAPPA, y1),
                crate::LayoutPoint::new(x0, y1 - radius + radius * KAPPA),
                crate::LayoutPoint::new(x0, y1 - radius),
            ),
            PaintPathElement::LineTo(crate::LayoutPoint::new(x0, y0 + radius)),
            PaintPathElement::CubicTo(
                crate::LayoutPoint::new(x0, y0 + radius - radius * KAPPA),
                crate::LayoutPoint::new(x0 + radius - radius * KAPPA, y0),
                crate::LayoutPoint::new(x0 + radius, y0),
            ),
            PaintPathElement::Close,
        ],
        bounds: rect,
    }
}
