use std::{fmt::Debug, hash::Hash};

use crate::{
    LayoutBoxId, LayoutPoint, LayoutRect, LayoutScrollbarAxis, LayoutScrollbarGeometry, PaintBrush,
    PaintColor, PaintCornerRadii, PaintCornerRadius, PaintFragment, PaintPath, PaintPathElement,
    PaintShape, PaintSnapshot,
    projection::{OutputProjection, PaintSpace},
};

const DEFAULT_TRACK: PaintColor = PaintColor::new(252.0 / 255.0, 252.0 / 255.0, 252.0 / 255.0, 1.0);
const DEFAULT_THUMB: PaintColor = PaintColor::new(139.0 / 255.0, 139.0 / 255.0, 139.0 / 255.0, 1.0);
const FLUENT_BUTTON_LENGTH: f32 = 18.0;
const FLUENT_ARROW_SIDE: f32 = 9.0;

pub(super) fn project_scrollbars<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let index = id.index();
    if !projection.world.boxes[index].style.is_visible() {
        return;
    }
    let extent = &projection.scroll_extents[index];
    let colors = extent.scrollbar_colors;
    let track = colors.map_or(DEFAULT_TRACK, |colors| colors.track);
    let thumb = colors.map_or(DEFAULT_THUMB, |colors| colors.thumb);
    let paint_space = if id == projection.world.root {
        PaintSpace::ROOT.with_outer_transform(snapshot.viewport_to_surface)
    } else {
        let geometry = &projection.boxes[index];
        projection.coordinate_spaces[geometry.coordinate_space.index()]
            .paint_space(snapshot.viewport_to_surface)
    };

    for scrollbar in [extent.horizontal_scrollbar, extent.vertical_scrollbar]
        .into_iter()
        .flatten()
    {
        paint_rect(snapshot, paint_space, scrollbar.frame, track);
        paint_thumb(snapshot, paint_space, scrollbar, thumb);
        paint_button_arrow(snapshot, paint_space, scrollbar, false, thumb);
        paint_button_arrow(snapshot, paint_space, scrollbar, true, thumb);
    }
    if let Some(corner) = extent.scrollbar_corner {
        paint_rect(snapshot, paint_space, corner, track);
    }
}

fn paint_thumb(
    snapshot: &mut PaintSnapshot,
    paint_space: PaintSpace,
    scrollbar: LayoutScrollbarGeometry,
    color: PaintColor,
) {
    let rect = scrollbar.painted_thumb;
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let rect = paint_space.pre_transform_rect(rect);
    snapshot.push_fragment(PaintFragment::Fill {
        shape: PaintShape::RoundedRect {
            rect,
            // NativeThemeFluent uses an intentionally oversized radius and
            // lets Skia normalize it to the largest possible pill.
            radii: PaintCornerRadii::all(PaintCornerRadius::new(999.0, 999.0)),
        },
        brush: PaintBrush::Solid(color),
        transform: paint_space.property_transform(),
    });
}

fn paint_button_arrow(
    snapshot: &mut PaintSnapshot,
    paint_space: PaintSpace,
    scrollbar: LayoutScrollbarGeometry,
    forward: bool,
    color: PaintColor,
) {
    let button = if forward {
        scrollbar.forward_button
    } else {
        scrollbar.back_button
    };
    let Some(path) = fluent_arrow_path(
        paint_space.pre_transform_rect(button),
        scrollbar.axis,
        forward,
    ) else {
        return;
    };
    snapshot.push_fragment(PaintFragment::Fill {
        shape: PaintShape::Path(path),
        brush: PaintBrush::Solid(color),
        transform: paint_space.property_transform(),
    });
}

/// Ports NativeThemeFluent::GetArrowRect and NativeThemeBase::PathForArrow.
fn fluent_arrow_path(
    button: LayoutRect,
    axis: LayoutScrollbarAxis,
    forward: bool,
) -> Option<PaintPath> {
    if button.width <= 0.0 || button.height <= 0.0 {
        return None;
    }
    let min_side = button.width.min(button.height).round();
    let max_side = button.width.max(button.height).round();
    let scale = max_side / FLUENT_BUTTON_LENGTH;
    let mut side = (FLUENT_ARROW_SIDE * scale).ceil().min(min_side);
    let difference = min_side - side;
    if difference.rem_euclid(2.0) >= 1.0 {
        side += 1.0;
    }
    if side <= 0.0 {
        return None;
    }

    let mut arrow = LayoutRect::new(
        (button.x + (button.width - side) / 2.0).floor(),
        (button.y + (button.height - side) / 2.0).floor(),
        side,
        side,
    );
    let direction = if forward { 1.0 } else { -1.0 };
    let offset = (direction * scale).round();
    match axis {
        LayoutScrollbarAxis::Horizontal => arrow.x += offset,
        LayoutScrollbarAxis::Vertical => arrow.y += offset,
    }

    let half = side / 2.0;
    let (mut first, mut second, mut tip) = match axis {
        LayoutScrollbarAxis::Vertical => {
            let arrow_height = (side.round() / 2.0).floor() + 1.0;
            let base_y = arrow.bottom() - (arrow_height / 2.0).floor() + 1.0;
            (
                LayoutPoint::new(arrow.x, base_y),
                LayoutPoint::new(arrow.right(), base_y),
                LayoutPoint::new(arrow.x + half, base_y - arrow_height),
            )
        }
        LayoutScrollbarAxis::Horizontal => {
            let arrow_width = (side.round() / 2.0).floor() + 1.0;
            let base_x = arrow.x + (arrow_width / 2.0).floor();
            (
                LayoutPoint::new(base_x, arrow.y),
                LayoutPoint::new(base_x, arrow.bottom()),
                LayoutPoint::new(base_x + arrow_width, arrow.y + half),
            )
        }
    };
    if matches!(axis, LayoutScrollbarAxis::Vertical) && forward {
        let center = arrow.y + half;
        for point in [&mut first, &mut second, &mut tip] {
            point.y = center * 2.0 - point.y;
        }
    } else if matches!(axis, LayoutScrollbarAxis::Horizontal) && !forward {
        let center = arrow.x + half;
        for point in [&mut first, &mut second, &mut tip] {
            point.x = center * 2.0 - point.x;
        }
    }
    Some(PaintPath {
        elements: vec![
            PaintPathElement::MoveTo(first),
            PaintPathElement::LineTo(second),
            PaintPathElement::LineTo(tip),
            PaintPathElement::Close,
        ],
        bounds: arrow,
    })
}

fn paint_rect(
    snapshot: &mut PaintSnapshot,
    paint_space: PaintSpace,
    rect: LayoutRect,
    color: PaintColor,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || color.alpha <= 0.0 {
        return;
    }
    snapshot.push_fragment(PaintFragment::Fill {
        shape: PaintShape::Rect(paint_space.pre_transform_rect(rect)),
        brush: PaintBrush::Solid(color),
        transform: paint_space.property_transform(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_arrow_paths_match_chromiums_fluent_geometry() {
        let vertical_button = LayoutRect::new(0.0, 0.0, 15.0, 18.0);
        assert_arrow(
            fluent_arrow_path(vertical_button, LayoutScrollbarAxis::Vertical, false).unwrap(),
            LayoutRect::new(3.0, 3.0, 9.0, 9.0),
            [
                LayoutPoint::new(3.0, 11.0),
                LayoutPoint::new(12.0, 11.0),
                LayoutPoint::new(7.5, 6.0),
            ],
        );
        assert_arrow(
            fluent_arrow_path(vertical_button, LayoutScrollbarAxis::Vertical, true).unwrap(),
            LayoutRect::new(3.0, 5.0, 9.0, 9.0),
            [
                LayoutPoint::new(3.0, 6.0),
                LayoutPoint::new(12.0, 6.0),
                LayoutPoint::new(7.5, 11.0),
            ],
        );

        let horizontal_button = LayoutRect::new(0.0, 0.0, 18.0, 15.0);
        assert_arrow(
            fluent_arrow_path(horizontal_button, LayoutScrollbarAxis::Horizontal, false).unwrap(),
            LayoutRect::new(3.0, 3.0, 9.0, 9.0),
            [
                LayoutPoint::new(10.0, 3.0),
                LayoutPoint::new(10.0, 12.0),
                LayoutPoint::new(5.0, 7.5),
            ],
        );
        assert_arrow(
            fluent_arrow_path(horizontal_button, LayoutScrollbarAxis::Horizontal, true).unwrap(),
            LayoutRect::new(5.0, 3.0, 9.0, 9.0),
            [
                LayoutPoint::new(7.0, 3.0),
                LayoutPoint::new(7.0, 12.0),
                LayoutPoint::new(12.0, 7.5),
            ],
        );
    }

    fn assert_arrow(path: PaintPath, bounds: LayoutRect, points: [LayoutPoint; 3]) {
        assert_eq!(path.bounds, bounds);
        assert_eq!(
            path.elements,
            vec![
                PaintPathElement::MoveTo(points[0]),
                PaintPathElement::LineTo(points[1]),
                PaintPathElement::LineTo(points[2]),
                PaintPathElement::Close,
            ]
        );
    }
}
