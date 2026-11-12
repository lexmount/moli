//! Paint-only snapping and subpixel propagation. These values never feed CSSOM.

use super::coordinates::{ProjectionAnchor, ResolvedPlacement};
use crate::{LayoutPoint, LayoutRect, LayoutTransform2D};

/// Blink-like linear paint space for one projected coordinate space.
///
/// Chromium describes this space as the combination of a transform and a
/// paint offset. Keeping those two values together lets paint snap geometry
/// before applying CSS and scroll property transforms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaintSpace {
    pub(crate) paint_offset: LayoutPoint,
    property_transform: LayoutTransform2D,
}

impl PaintSpace {
    pub(crate) const ROOT: Self = Self {
        paint_offset: LayoutPoint::ZERO,
        property_transform: LayoutTransform2D::IDENTITY,
    };

    pub(crate) fn with_outer_transform(self, outer: LayoutTransform2D) -> Self {
        Self {
            paint_offset: self.paint_offset,
            property_transform: outer.concatenate(self.property_transform),
        }
    }

    pub(crate) fn local_transform(self) -> LayoutTransform2D {
        self.property_transform
            .concatenate(LayoutTransform2D::translation(
                self.paint_offset.x,
                self.paint_offset.y,
            ))
    }

    pub(crate) fn pre_transform_rect(self, rect: LayoutRect) -> LayoutRect {
        LayoutRect::new(
            rect.x + self.paint_offset.x,
            rect.y + self.paint_offset.y,
            rect.width,
            rect.height,
        )
    }

    pub(crate) const fn property_transform(self) -> LayoutTransform2D {
        self.property_transform
    }
}

#[derive(Clone, Copy)]
struct SubpixelPropagation {
    x: bool,
    y: bool,
}

impl SubpixelPropagation {
    const BOTH: Self = Self { x: true, y: true };
    const NONE: Self = Self { x: false, y: false };

    fn through_transform(transform: LayoutTransform2D) -> Self {
        let [scale_x, skew_y, skew_x, scale_y, _, _] = transform.coefficients;
        if skew_x != 0.0 || skew_y != 0.0 {
            return Self::NONE;
        }
        Self {
            x: scale_x == 1.0,
            y: scale_y == 1.0,
        }
    }
}

fn establish_property_space(
    state: PaintSpace,
    property_transform: LayoutTransform2D,
    propagation: SubpixelPropagation,
) -> PaintSpace {
    let rounded_offset = LayoutPoint::new(
        crate::snapshot::round_layout_pixel(f64::from(state.paint_offset.x)) as f32,
        crate::snapshot::round_layout_pixel(f64::from(state.paint_offset.y)) as f32,
    );
    let residual = LayoutPoint::new(
        state.paint_offset.x - rounded_offset.x,
        state.paint_offset.y - rounded_offset.y,
    );
    PaintSpace {
        property_transform: state
            .property_transform
            .concatenate(LayoutTransform2D::translation(
                rounded_offset.x,
                rounded_offset.y,
            ))
            .concatenate(property_transform),
        paint_offset: LayoutPoint::new(
            if propagation.x { residual.x } else { 0.0 },
            if propagation.y { residual.y } else { 0.0 },
        ),
    }
}

pub(super) fn derive_paint_space(
    placement: ResolvedPlacement<'_>,
    viewport_scroll: LayoutPoint,
) -> PaintSpace {
    let mut state = match placement.anchor {
        ProjectionAnchor::Parent(parent) => parent.paint,
        ProjectionAnchor::Document => PaintSpace {
            property_transform: LayoutTransform2D::translation(
                -viewport_scroll.x,
                -viewport_scroll.y,
            ),
            paint_offset: LayoutPoint::ZERO,
        },
        ProjectionAnchor::Viewport => PaintSpace::ROOT,
    };
    let parent_scroll = placement.parent_scroll;
    let location = placement.positioned_location();
    let resolved_transform = placement.transform;

    // A scroll container establishes a property space even at scroll offset
    // zero. Commit the rounded translation while retaining its fractional
    // residue for descendant snapping, then apply scrolling outside it.
    if let Some(parent_scroll) = parent_scroll {
        state = establish_property_space(
            state,
            LayoutTransform2D::translation(-parent_scroll.x, -parent_scroll.y),
            SubpixelPropagation::BOTH,
        );
    }
    state.paint_offset = LayoutPoint::new(
        state.paint_offset.x + location.x,
        state.paint_offset.y + location.y,
    );

    if resolved_transform.establishes_property_space {
        let propagation = if resolved_transform.has_unsupported_3d {
            SubpixelPropagation::NONE
        } else {
            SubpixelPropagation::through_transform(resolved_transform.transform)
        };
        state = establish_property_space(state, resolved_transform.transform, propagation);
    }
    state
}

#[cfg(test)]
mod paint_space_tests {
    use super::*;
    use crate::style::ResolvedLayoutTransform;

    // Existing snapping cases supply paint-only parents. Give those parents an
    // identity query space so the production entry still consumes a placement.
    fn resolve_paint_space(
        parent: Option<PaintSpace>,
        viewport_scroll: LayoutPoint,
        is_viewport_anchored: bool,
        parent_scroll: Option<LayoutPoint>,
        location: LayoutPoint,
        transform: ResolvedLayoutTransform,
    ) -> PaintSpace {
        let parent = parent.map(|paint| super::super::ProjectedCoordinateSpace {
            layout: super::super::compose_layout_space(
                0,
                ResolvedPlacement {
                    anchor: ProjectionAnchor::Document,
                    layout_location_in_parent: LayoutPoint::ZERO,
                    local_sticky_delta_in_parent: LayoutPoint::ZERO,
                    parent_scroll: None,
                    transform: ResolvedLayoutTransform::IDENTITY,
                },
                LayoutPoint::ZERO,
            ),
            paint,
        });
        let anchor = match &parent {
            Some(parent) => ProjectionAnchor::Parent(parent),
            None if is_viewport_anchored => ProjectionAnchor::Viewport,
            None => ProjectionAnchor::Document,
        };
        derive_paint_space(
            ResolvedPlacement {
                anchor,
                layout_location_in_parent: location,
                local_sticky_delta_in_parent: LayoutPoint::ZERO,
                parent_scroll,
                transform,
            },
            viewport_scroll,
        )
    }

    fn property_transform(transform: LayoutTransform2D) -> ResolvedLayoutTransform {
        ResolvedLayoutTransform {
            transform,
            has_unsupported_3d: false,
            establishes_property_space: true,
        }
    }

    #[test]
    fn ordinary_offsets_accumulate_without_an_extra_property_transform() {
        let root = resolve_paint_space(
            None,
            LayoutPoint::ZERO,
            false,
            None,
            LayoutPoint::ZERO,
            ResolvedLayoutTransform::IDENTITY,
        );
        let ordinary = resolve_paint_space(
            Some(root),
            LayoutPoint::ZERO,
            false,
            None,
            LayoutPoint::new(0.0, 12.5),
            ResolvedLayoutTransform::IDENTITY,
        );
        assert_eq!(ordinary.paint_offset, LayoutPoint::new(0.0, 12.5));
        assert_eq!(ordinary.property_transform, LayoutTransform2D::IDENTITY);
        assert_eq!(
            ordinary.local_transform(),
            LayoutTransform2D::translation(0.0, 12.5)
        );
    }

    #[test]
    fn translation_property_space_keeps_the_fractional_residual() {
        let transformed = resolve_paint_space(
            Some(PaintSpace {
                paint_offset: LayoutPoint::new(0.75, 0.25),
                property_transform: LayoutTransform2D::IDENTITY,
            }),
            LayoutPoint::ZERO,
            false,
            None,
            LayoutPoint::ZERO,
            property_transform(LayoutTransform2D::translation(5.0, 7.0)),
        );
        assert_eq!(transformed.paint_offset, LayoutPoint::new(-0.25, 0.25));
        assert_eq!(
            transformed.property_transform,
            LayoutTransform2D::translation(6.0, 7.0)
        );
        assert_eq!(
            transformed.local_transform(),
            LayoutTransform2D::translation(5.75, 7.25)
        );
    }

    #[test]
    fn scale_discards_only_the_residual_on_scaled_axes() {
        let transformed = resolve_paint_space(
            Some(PaintSpace {
                paint_offset: LayoutPoint::new(0.75, 0.75),
                property_transform: LayoutTransform2D::IDENTITY,
            }),
            LayoutPoint::ZERO,
            false,
            None,
            LayoutPoint::ZERO,
            property_transform(LayoutTransform2D::scale(2.0, 1.0)),
        );
        assert_eq!(transformed.paint_offset, LayoutPoint::new(0.0, -0.25));
        assert_eq!(
            transformed.property_transform,
            LayoutTransform2D::translation(1.0, 1.0)
                .concatenate(LayoutTransform2D::scale(2.0, 1.0))
        );
    }

    #[test]
    fn scroll_property_space_exists_at_zero_offset_and_preserves_residual() {
        let scrolled_child = resolve_paint_space(
            Some(PaintSpace {
                paint_offset: LayoutPoint::new(0.75, 0.0),
                property_transform: LayoutTransform2D::IDENTITY,
            }),
            LayoutPoint::ZERO,
            false,
            Some(LayoutPoint::ZERO),
            LayoutPoint::new(0.125, 0.0),
            ResolvedLayoutTransform::IDENTITY,
        );
        assert_eq!(
            scrolled_child.property_transform,
            LayoutTransform2D::translation(1.0, 0.0)
        );
        assert_eq!(scrolled_child.paint_offset, LayoutPoint::new(-0.125, 0.0));
        assert_eq!(
            scrolled_child.local_transform(),
            LayoutTransform2D::translation(0.875, 0.0)
        );
    }
}
