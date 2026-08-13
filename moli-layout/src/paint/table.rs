//! Projection of table-owner collapsed borders into ordinary snapshot border
//! primitives. Conflict resolution and joint geometry stay in `table`; the
//! raster backend remains unaware of CSS table semantics.

use std::{fmt::Debug, hash::Hash};

use crate::{
    LayoutBoxId, PaintBorderColors, PaintBorderStyle, PaintBorderStyles, PaintColor,
    PaintCornerRadii, PaintEdgeSizes, PaintFragment, PaintSnapshot, projection::OutputProjection,
};

pub(super) fn project_collapsed_table_borders<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if !layout_box.style.is_visible() {
        return;
    }
    let Some(borders) = layout_box.collapsed_table_borders.as_ref() else {
        return;
    };
    let geometry = &projection.boxes[id.index()];
    let paint_space = projection.coordinate_spaces[geometry.coordinate_space.index()]
        .paint_space(snapshot.viewport_to_surface);

    for segment in borders.segments() {
        let mut widths = PaintEdgeSizes::default();
        let mut colors = PaintBorderColors::all(PaintColor::TRANSPARENT);
        let mut styles = PaintBorderStyles::all(PaintBorderStyle::None);
        if segment.horizontal {
            widths.top = segment.rect.height;
            colors.top = segment.color;
            styles.top = segment.style;
        } else {
            widths.left = segment.rect.width;
            colors.left = segment.color;
            styles.left = segment.style;
        }
        snapshot.push_fragment(PaintFragment::Border {
            rect: paint_space.pre_transform_rect(segment.rect),
            widths,
            colors,
            styles,
            radii: PaintCornerRadii::ZERO,
            transform: paint_space.property_transform(),
        });
    }
}
