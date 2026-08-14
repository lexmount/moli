use std::{fmt::Debug, hash::Hash};

use crate::{
    LayoutBoxId, LayoutPoint, LayoutRect, LayoutTransform2D, projection::OutputProjection,
};

/// Transient paint coordinates for operations that must snap before CSS
/// property transforms are applied.
///
/// The retained layout output continues to use one exact local-to-viewport
/// transform. This split representation exists only while producing a paint
/// snapshot: ordinary layout offsets remain in `paint_offset`, while scroll
/// and CSS transforms are committed to `property_transform`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PaintSpace {
    paint_offset: LayoutPoint,
    property_transform: LayoutTransform2D,
}

impl PaintSpace {
    fn new(property_transform: LayoutTransform2D) -> Self {
        Self {
            paint_offset: LayoutPoint::ZERO,
            property_transform,
        }
    }

    fn translate_layout(&mut self, offset: LayoutPoint) {
        self.paint_offset = LayoutPoint::new(
            self.paint_offset.x + offset.x,
            self.paint_offset.y + offset.y,
        );
    }

    fn establish_property_transform(&mut self, transform: LayoutTransform2D) {
        self.property_transform = self
            .property_transform
            .concatenate(LayoutTransform2D::translation(
                self.paint_offset.x,
                self.paint_offset.y,
            ))
            .concatenate(transform);
        self.paint_offset = LayoutPoint::ZERO;
    }

    pub(super) fn offset_rect(self, rect: LayoutRect) -> LayoutRect {
        LayoutRect::new(
            rect.x + self.paint_offset.x,
            rect.y + self.paint_offset.y,
            rect.width,
            rect.height,
        )
    }

    pub(super) const fn property_transform(self) -> LayoutTransform2D {
        self.property_transform
    }

    #[cfg(test)]
    fn exact_transform(self) -> LayoutTransform2D {
        self.property_transform
            .concatenate(LayoutTransform2D::translation(
                self.paint_offset.x,
                self.paint_offset.y,
            ))
    }
}

/// Per-box paint spaces derived once for a single snapshot and then discarded.
pub(super) struct PaintSpaceMap {
    spaces: Vec<PaintSpace>,
}

impl PaintSpaceMap {
    pub(super) fn build<N>(
        projection: &OutputProjection<'_, N>,
        viewport_to_surface: LayoutTransform2D,
    ) -> Self
    where
        N: Copy + Debug + Eq + Hash,
    {
        let box_count = projection.world.boxes.len();
        let mut spaces = vec![None; box_count];
        let mut visiting = vec![false; box_count];
        let viewport_scroll =
            projection.scroll_extents[projection.world.root.index()].applied_offset;

        for index in 0..box_count {
            resolve_box_space(
                index,
                projection,
                viewport_to_surface,
                viewport_scroll,
                &mut spaces,
                &mut visiting,
            );
        }

        Self {
            spaces: spaces
                .into_iter()
                .map(|space| space.expect("validated layout boxes have a paint space"))
                .collect(),
        }
    }

    pub(super) fn get(&self, id: LayoutBoxId) -> PaintSpace {
        self.spaces[id.index()]
    }
}

fn resolve_box_space<N>(
    index: usize,
    projection: &OutputProjection<'_, N>,
    viewport_to_surface: LayoutTransform2D,
    viewport_scroll: LayoutPoint,
    spaces: &mut [Option<PaintSpace>],
    visiting: &mut [bool],
) -> PaintSpace
where
    N: Copy + Debug + Eq + Hash,
{
    if let Some(space) = spaces[index] {
        return space;
    }
    assert!(
        !visiting[index],
        "layout-parent cycles are rejected before paint projection"
    );
    visiting[index] = true;

    let layout_box = &projection.world.boxes[index];
    let parent = layout_box.layout_parent;
    let mut space = if let Some(parent) = parent {
        let parent_space = resolve_box_space(
            parent.index(),
            projection,
            viewport_to_surface,
            viewport_scroll,
            spaces,
            visiting,
        );
        let mut space = parent_space;
        let parent_scroll = &projection.scroll_extents[parent.index()];
        if parent != projection.world.root && parent_scroll.is_scroll_container {
            let offset = parent_scroll.applied_offset;
            space
                .establish_property_transform(LayoutTransform2D::translation(-offset.x, -offset.y));
        }
        space
    } else {
        let is_viewport_anchored = layout_box.style.is_fixed_positioned();
        let transform = if is_viewport_anchored {
            viewport_to_surface
        } else {
            viewport_to_surface.concatenate(LayoutTransform2D::translation(
                -viewport_scroll.x,
                -viewport_scroll.y,
            ))
        };
        PaintSpace::new(transform)
    };

    let location = layout_box.final_layout.location;
    space.translate_layout(LayoutPoint::new(location.x, location.y));
    if layout_box.style.establishes_paint_property_space() {
        space.establish_property_transform(projection.resolved_transforms[index]);
    }

    visiting[index] = false;
    spaces[index] = Some(space);
    space
}

#[cfg(test)]
mod tests {
    use super::PaintSpace;
    use crate::{LayoutPoint, LayoutRect, LayoutTransform2D};

    #[test]
    fn ordinary_layout_offsets_remain_in_pre_transform_paint_space() {
        let mut space = PaintSpace::new(LayoutTransform2D::IDENTITY);
        space.translate_layout(LayoutPoint::new(0.0, 12.5));

        assert_eq!(
            space.offset_rect(LayoutRect::new(0.0, 0.0, 100.0, 12.5)),
            LayoutRect::new(0.0, 12.5, 100.0, 12.5)
        );
        assert_eq!(space.property_transform(), LayoutTransform2D::IDENTITY);
    }

    #[test]
    fn committing_a_property_transform_preserves_exact_geometry() {
        let mut space = PaintSpace::new(LayoutTransform2D::translation(3.0, 4.0));
        space.translate_layout(LayoutPoint::new(7.25, 11.5));
        let before = space
            .exact_transform()
            .concatenate(LayoutTransform2D::scale(2.0, 3.0));

        space.establish_property_transform(LayoutTransform2D::scale(2.0, 3.0));

        assert_eq!(space.paint_offset, LayoutPoint::ZERO);
        assert_eq!(space.exact_transform(), before);
    }
}
