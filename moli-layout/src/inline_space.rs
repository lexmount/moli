//! The line-relative coordinate boundary between shaping and physical fragments.
//!
//! Parley positions in line-left/line-over coordinates; bidi has already
//! resolved the inline direction. Block progression and line orientation are
//! separate: vertical-lr progresses right, but its line-over edge is also right.
//! This is Blink's ToLineWritingMode/line-fragment distinction, not a transform
//! on the CSS box or a second application of the paragraph's direction.

use taffy::{Direction, LogicalOffset, LogicalSize, Rect, Size, WritingDirection, WritingMode};

use crate::{LayoutPoint, LayoutRect, LayoutTextDirection, LayoutTransform2D};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LineRelativeRect {
    pub(crate) inline_offset: f32,
    pub(crate) block_offset: f32,
    pub(crate) inline_size: f32,
    pub(crate) block_size: f32,
}

impl LineRelativeRect {
    pub(crate) fn new(
        inline_offset: f32,
        block_offset: f32,
        inline_size: f32,
        block_size: f32,
    ) -> Self {
        Self {
            inline_offset,
            block_offset,
            inline_size,
            block_size,
        }
    }

    /// Rectangular arithmetic inside the line algorithm, before projection.
    pub(crate) fn as_rect(self) -> LayoutRect {
        LayoutRect::new(
            self.inline_offset,
            self.block_offset,
            self.inline_size,
            self.block_size,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct InlineWritingMode(pub(crate) WritingMode);

impl InlineWritingMode {
    pub(crate) fn text_direction(self, rtl: bool) -> LayoutTextDirection {
        match (
            self.0.is_horizontal(),
            rtl ^ (self.0 == WritingMode::SidewaysLr),
        ) {
            (true, false) => LayoutTextDirection::LeftToRight,
            (true, true) => LayoutTextDirection::RightToLeft,
            (false, false) => LayoutTextDirection::TopToBottom,
            (false, true) => LayoutTextDirection::BottomToTop,
        }
    }

    /// Physical edges expressed as line-left/right/over/under. These are not
    /// flow-relative start/end: the inline algorithm itself resolves bidi.
    pub(crate) fn line_edges<T: Copy>(self, edges: Rect<T>) -> Rect<T> {
        let line_mode = if self.0 == WritingMode::VerticalLr {
            WritingMode::VerticalRl
        } else {
            self.0
        };
        let edges = WritingDirection::new(line_mode, Direction::Ltr).to_logical_box_strut(edges);
        Rect {
            left: edges.inline_start,
            right: edges.inline_end,
            top: edges.block_start,
            bottom: edges.block_end,
        }
    }

    pub(crate) fn space(self, content_size: Size<f32>) -> InlineCoordinateSpace {
        InlineCoordinateSpace {
            mode: self,
            content_size,
            content_origin: LayoutPoint::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct InlineCoordinateSpace {
    pub(crate) mode: InlineWritingMode,
    pub(crate) content_size: Size<f32>,
    pub(crate) content_origin: LayoutPoint,
}

impl InlineCoordinateSpace {
    pub(crate) fn with_origin(mut self, origin: LayoutPoint) -> Self {
        self.content_origin = origin;
        self
    }

    pub(crate) fn logical_size(self) -> LogicalSize<f32> {
        self.mode.0.to_logical(self.content_size)
    }

    /// Map one line's coordinates to the IFC content box. The same map owns
    /// atomic placement, text/inline fragments, glyph paint and selections.
    pub(crate) fn line_transform(self, line: LineRelativeRect) -> LayoutTransform2D {
        match self.mode.0 {
            WritingMode::HorizontalTb => LayoutTransform2D::IDENTITY,
            WritingMode::VerticalRl | WritingMode::SidewaysRl => LayoutTransform2D::new([
                0.0,
                1.0,
                -1.0,
                0.0,
                f64::from(self.content_size.width),
                0.0,
            ]),
            WritingMode::VerticalLr => LayoutTransform2D::new([
                0.0,
                1.0,
                -1.0,
                0.0,
                f64::from(2.0 * line.block_offset + line.block_size),
                0.0,
            ]),
            WritingMode::SidewaysLr => LayoutTransform2D::new([
                0.0,
                -1.0,
                1.0,
                0.0,
                0.0,
                f64::from(self.content_size.height),
            ]),
        }
    }

    pub(crate) fn line_rect(self, line: LineRelativeRect, rect: LayoutRect) -> LayoutRect {
        self.line_transform(line).map_rect(rect).bounding_rect()
    }

    /// BFC floats use block progression coordinates, not line-over coordinates.
    pub(crate) fn flow_origin(self, offset: LogicalOffset<f32>, size: Size<f32>) -> LayoutPoint {
        let point = WritingDirection::new(self.mode.0, Direction::Ltr)
            .converter(self.content_size)
            .to_physical_point(offset, size);
        LayoutPoint::new(point.x, point.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_lr_flips_line_orientation_without_flipping_block_progression() {
        let space = InlineWritingMode(WritingMode::VerticalLr).space(Size {
            width: 100.0,
            height: 200.0,
        });
        let first = LineRelativeRect::new(0.0, 10.0, 120.0, 30.0);
        let second = LineRelativeRect::new(0.0, 40.0, 120.0, 20.0);
        assert_eq!(
            space.line_rect(first, first.as_rect()),
            LayoutRect::new(10.0, 0.0, 30.0, 120.0)
        );
        assert_eq!(
            space.line_rect(second, second.as_rect()),
            LayoutRect::new(40.0, 0.0, 20.0, 120.0)
        );
        assert_eq!(
            space
                .line_transform(first)
                .map_point(LayoutPoint::new(12.0, 15.0)),
            LayoutPoint::new(35.0, 12.0)
        );
    }

    #[test]
    fn line_projection_preserves_empty_bounds_and_does_not_apply_bidi_again() {
        let line = LineRelativeRect::new(0.0, 0.0, 100.0, 20.0);
        let rect = LayoutRect::new(-150.0, 5.0, 250.0, 0.0);
        for (mode, expected) in [
            (
                WritingMode::HorizontalTb,
                LayoutRect::new(-150.0, 5.0, 250.0, 0.0),
            ),
            (
                WritingMode::VerticalRl,
                LayoutRect::new(75.0, -150.0, 0.0, 250.0),
            ),
            (
                WritingMode::VerticalLr,
                LayoutRect::new(15.0, -150.0, 0.0, 250.0),
            ),
            (
                WritingMode::SidewaysRl,
                LayoutRect::new(75.0, -150.0, 0.0, 250.0),
            ),
            (
                WritingMode::SidewaysLr,
                LayoutRect::new(5.0, 100.0, 0.0, 250.0),
            ),
        ] {
            let space = InlineWritingMode(mode).space(Size {
                width: 80.0,
                height: 200.0,
            });
            assert_eq!(space.line_rect(line, rect), expected, "{mode:?}");
            assert_eq!(
                space
                    .line_transform(line)
                    .inverse()
                    .unwrap()
                    .map_rect(expected)
                    .bounding_rect(),
                rect
            );
        }
    }
}
