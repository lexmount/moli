//! Composition of already resolved placement. No CSS, ancestor lookup or world
//! access belongs here. Precise query mappings and paint share the same input.

use super::ProjectedCoordinateSpace;
use crate::{
    LayoutCoordinateSpaceId, LayoutOutputBoxId, LayoutPoint, LayoutTransform2D,
    layout_tree::LayoutCoordinateSpace, style::ResolvedLayoutTransform,
};

#[derive(Clone, Copy)]
/// A geometric anchor after CSS containing-block decisions have been resolved.
pub(super) enum ProjectionAnchor<'a> {
    Parent(&'a ProjectedCoordinateSpace),
    Document,
    Viewport,
}

#[derive(Clone, Copy)]
pub(super) struct ResolvedPlacement<'a> {
    pub(super) anchor: ProjectionAnchor<'a>,
    /// Parent-local, already zoomed layout CSS pixels from numeric placement.
    pub(super) layout_location_in_parent: LayoutPoint,
    /// Only this node's adjustment. Its parent space already carries ancestors.
    pub(super) local_sticky_delta_in_parent: LayoutPoint,
    /// Actual clamped scrolling in parent-local layout pixels. Some(ZERO)
    /// still establishes a paint property-space boundary.
    pub(super) parent_scroll: Option<LayoutPoint>,
    pub(super) transform: ResolvedLayoutTransform,
}

impl ResolvedPlacement<'_> {
    pub(super) fn positioned_location(self) -> LayoutPoint {
        LayoutPoint::new(
            self.layout_location_in_parent.x + self.local_sticky_delta_in_parent.x,
            self.layout_location_in_parent.y + self.local_sticky_delta_in_parent.y,
        )
    }
}

pub(super) fn compose_layout_space(
    index: usize,
    placement: ResolvedPlacement<'_>,
    viewport_scroll: LayoutPoint,
) -> LayoutCoordinateSpace {
    let (parent_id, parent_transform, parent_without_css) = match placement.anchor {
        ProjectionAnchor::Parent(parent) => (
            parent.layout.id,
            parent.layout.local_to_document,
            parent.layout.local_to_document_ignoring_css_transforms,
        ),
        ProjectionAnchor::Document => (
            LayoutCoordinateSpaceId::from_index(0),
            LayoutTransform2D::IDENTITY,
            LayoutTransform2D::IDENTITY,
        ),
        ProjectionAnchor::Viewport => {
            let anchor = LayoutTransform2D::translation(viewport_scroll.x, viewport_scroll.y);
            (LayoutCoordinateSpaceId::from_index(0), anchor, anchor)
        }
    };
    let location = placement.positioned_location();
    let scroll = placement.parent_scroll.unwrap_or(LayoutPoint::ZERO);
    let translation = LayoutTransform2D::translation(location.x - scroll.x, location.y - scroll.y);
    let local_to_parent = translation.concatenate(placement.transform.transform);
    let local_to_document = parent_transform.concatenate(local_to_parent);
    let local_to_document_ignoring_css_transforms = parent_without_css.concatenate(translation);
    let document_to_viewport =
        LayoutTransform2D::translation(-viewport_scroll.x, -viewport_scroll.y);
    LayoutCoordinateSpace {
        id: LayoutCoordinateSpaceId::from_index(index + 1),
        owner: Some(LayoutOutputBoxId::from_index(index)),
        parent: Some(parent_id),
        local_to_parent,
        local_to_document,
        local_to_viewport: document_to_viewport.concatenate(local_to_document),
        local_to_document_ignoring_css_transforms,
        local_to_viewport_ignoring_css_transforms: document_to_viewport
            .concatenate(local_to_document_ignoring_css_transforms),
    }
}

#[cfg(test)]
mod tests {
    use super::super::paint_space::derive_paint_space;
    use super::*;
    use crate::LayoutRect;

    fn placement(anchor: ProjectionAnchor<'_>, point: LayoutPoint) -> ResolvedPlacement<'_> {
        ResolvedPlacement {
            anchor,
            layout_location_in_parent: point,
            local_sticky_delta_in_parent: LayoutPoint::ZERO,
            parent_scroll: None,
            transform: ResolvedLayoutTransform::IDENTITY,
        }
    }

    fn project(
        index: usize,
        placement: ResolvedPlacement<'_>,
        scroll: LayoutPoint,
    ) -> ProjectedCoordinateSpace {
        ProjectedCoordinateSpace {
            layout: compose_layout_space(index, placement, scroll),
            paint: derive_paint_space(placement, scroll),
        }
    }

    #[test]
    fn parent_and_local_sticky_adjustments_are_applied_once() {
        let mut parent = placement(ProjectionAnchor::Document, LayoutPoint::ZERO);
        parent.local_sticky_delta_in_parent.y = 100.0;
        let parent = project(0, parent, LayoutPoint::ZERO);
        let mut child = placement(ProjectionAnchor::Parent(&parent), LayoutPoint::ZERO);
        child.local_sticky_delta_in_parent.y = 20.0;
        let child = project(1, child, LayoutPoint::ZERO);
        assert_eq!(
            child.layout.local_to_parent.map_point(LayoutPoint::ZERO).y,
            20.0
        );
        assert_eq!(
            child
                .layout
                .local_to_viewport
                .map_point(LayoutPoint::ZERO)
                .y,
            120.0
        );
        assert_eq!(
            child.paint.local_transform().map_point(LayoutPoint::ZERO).y,
            120.0
        );
    }

    #[test]
    fn viewport_anchor_cancels_only_root_scroll() {
        let scroll = LayoutPoint::new(50.0, 100.0);
        let location = LayoutPoint::new(10.0, 20.0);
        for (anchor, expected) in [
            (ProjectionAnchor::Viewport, location),
            (ProjectionAnchor::Document, LayoutPoint::new(-40.0, -80.0)),
        ] {
            let space = project(0, placement(anchor, location), scroll);
            assert_eq!(
                space.layout.local_to_viewport.map_point(LayoutPoint::ZERO),
                expected
            );
            assert_eq!(
                space.paint.local_transform().map_point(LayoutPoint::ZERO),
                expected
            );
        }
    }

    #[test]
    fn paint_rounding_and_zero_scroll_boundaries_do_not_change_precise_queries() {
        let mut input = placement(ProjectionAnchor::Document, LayoutPoint::new(0.75, 0.25));
        input.transform = ResolvedLayoutTransform {
            transform: LayoutTransform2D::scale(2.0, 2.0),
            has_unsupported_3d: false,
            establishes_property_space: true,
        };
        let space = project(0, input, LayoutPoint::ZERO);
        assert_eq!(
            space.layout.local_to_viewport.map_point(LayoutPoint::ZERO),
            LayoutPoint::new(0.75, 0.25)
        );
        assert_eq!(
            space.paint.local_transform().map_point(LayoutPoint::ZERO),
            LayoutPoint::new(1.0, 0.0)
        );
        assert_eq!(
            space
                .layout
                .local_to_viewport_ignoring_css_transforms
                .map_point(LayoutPoint::new(2.0, 3.0)),
            LayoutPoint::new(2.75, 3.25)
        );

        let parent = project(
            0,
            placement(ProjectionAnchor::Document, LayoutPoint::new(0.75, 0.25)),
            LayoutPoint::ZERO,
        );
        let input = placement(ProjectionAnchor::Parent(&parent), LayoutPoint::ZERO);
        let ordinary = project(1, input, LayoutPoint::ZERO);
        let scrolled = project(
            1,
            ResolvedPlacement {
                parent_scroll: Some(LayoutPoint::ZERO),
                ..input
            },
            LayoutPoint::ZERO,
        );
        assert_eq!(ordinary.layout, scrolled.layout);
        assert_eq!(ordinary.paint.paint_offset, LayoutPoint::new(0.75, 0.25));
        assert_eq!(scrolled.paint.paint_offset, LayoutPoint::new(-0.25, 0.25));
        let identity_boundary = project(
            1,
            ResolvedPlacement {
                transform: ResolvedLayoutTransform {
                    establishes_property_space: true,
                    ..ResolvedLayoutTransform::IDENTITY
                },
                ..input
            },
            LayoutPoint::ZERO,
        );
        assert_eq!(ordinary.layout, identity_boundary.layout);
        assert_eq!(identity_boundary.paint, scrolled.paint);
    }

    #[test]
    fn exact_mapping_roundtrips_and_singular_transforms_remain_explicit() {
        let viewport = LayoutPoint::new(31.5, 81.25);
        for transform in [
            LayoutTransform2D::IDENTITY,
            LayoutTransform2D::scale(1.25, 0.75),
            LayoutTransform2D::rotation(0.63),
            LayoutTransform2D::new([1.0, 0.25, -0.125, 2.0, 9.5, -4.5]),
        ] {
            let mut input = placement(ProjectionAnchor::Document, LayoutPoint::new(17.25, 22.5));
            input.parent_scroll = Some(LayoutPoint::new(4.25, -7.5));
            input.transform.transform = transform;
            let space = project(0, input, viewport);
            let point = LayoutPoint::new(-12.25, 6.75);
            let mapped = space.layout.local_to_viewport.map_point(point);
            let roundtrip = space
                .layout
                .local_to_viewport
                .inverse()
                .unwrap()
                .map_point(mapped);
            assert!((roundtrip.x - point.x).abs() < 0.0001);
            assert!((roundtrip.y - point.y).abs() < 0.0001);
            assert_eq!(
                space
                    .layout
                    .local_to_viewport
                    .map_rect(LayoutRect::new(0.0, 0.0, 5.0, 8.0))
                    .points[0],
                space.layout.local_to_viewport.map_point(LayoutPoint::ZERO)
            );
        }
        let mut input = placement(ProjectionAnchor::Document, LayoutPoint::ZERO);
        input.transform.transform = LayoutTransform2D::scale(0.0, 1.0);
        assert!(
            project(0, input, viewport)
                .layout
                .local_to_viewport
                .inverse()
                .is_none()
        );
    }
}
