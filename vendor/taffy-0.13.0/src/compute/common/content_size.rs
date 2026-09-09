//! Generic CSS content size code that is shared between all CSS algorithms.
use crate::geometry::{Point, Rect, Size};
use crate::style::Overflow;
use crate::util::sys::{f32_max, f32_min};
use crate::Direction;

/// Convert a child's border-box location from the container's border-box
/// coordinate space into the coordinate space used by `Layout::content_size`.
///
/// Scrollbar gutters sit between the border and the scrollable padding box. A
/// leading gutter therefore shifts child layout without contributing to the
/// scrollable overflow extent. Horizontal coordinates are mirrored for RTL so
/// that the logical start edge remains the content-size origin.
#[inline(always)]
pub(crate) fn content_size_contribution_location(
    location: Point<f32>,
    size: Size<f32>,
    container_width: f32,
    border: Rect<f32>,
    scrollbar_insets: Rect<f32>,
    direction: Direction,
) -> Point<f32> {
    let x = if direction.is_rtl() {
        container_width - (location.x + size.width) - border.right - scrollbar_insets.right
    } else {
        location.x - border.left - scrollbar_insets.left
    };
    Point { x, y: location.y - border.top - scrollbar_insets.top }
}

#[inline(always)]
/// Determine how much width/height a given node contributes to it's parent's content size
pub(crate) fn compute_content_size_contribution(
    location: Point<f32>,
    size: Size<f32>,
    content_size: Size<f32>,
    overflow: Point<Overflow>,
) -> Size<f32> {
    let size_content_size_contribution = Size {
        width: match overflow.x {
            Overflow::Visible => f32_max(size.width, content_size.width),
            _ => size.width,
        },
        height: match overflow.y {
            Overflow::Visible => f32_max(size.height, content_size.height),
            _ => size.height,
        },
    };
    if size_content_size_contribution.width > 0.0 && size_content_size_contribution.height > 0.0 {
        let max_x = f32_max(location.x + size_content_size_contribution.width, 0.0);
        let min_x = f32_min(location.x, 0.0);
        let max_y = f32_max(location.y + size_content_size_contribution.height, 0.0);
        let min_y = f32_min(location.y, 0.0);

        Size { width: max_x - min_x, height: max_y - min_y }
    } else {
        Size::ZERO
    }
}
