use taffy::{
    AbsoluteAxis, AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignSelf,
    AlignmentSafety, Direction, FlexWrap, Line, Point, Rect, Size, WritingMode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogicalStaticEdge {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HorizontalStaticEdge {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VerticalStaticEdge {
    Top,
    Center,
    Bottom,
}

/// Physical static-position contract consumed by absolute positioning.
///
/// A point alone is insufficient: a centered point denotes the center of the
/// margin box, while an end point denotes its far edge. This is the same
/// distinction represented by Blink's `PhysicalStaticPosition`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PhysicalStaticPosition {
    point: Point<f32>,
    horizontal_edge: HorizontalStaticEdge,
    vertical_edge: VerticalStaticEdge,
}

impl PhysicalStaticPosition {
    pub(crate) const fn new(
        point: Point<f32>,
        horizontal_edge: HorizontalStaticEdge,
        vertical_edge: VerticalStaticEdge,
    ) -> Self {
        Self {
            point,
            horizontal_edge,
            vertical_edge,
        }
    }

    pub(crate) fn relative_to(self, origin: Point<f32>) -> Self {
        Self {
            point: Point {
                x: self.point.x - origin.x,
                y: self.point.y - origin.y,
            },
            ..self
        }
    }

    pub(crate) fn margin_box_origin(self, box_size: Size<f32>, margin: Rect<f32>) -> Point<f32> {
        let x = match self.horizontal_edge {
            HorizontalStaticEdge::Left => self.point.x + margin.left,
            HorizontalStaticEdge::Center => {
                self.point.x - box_size.width / 2.0 + (margin.left - margin.right) / 2.0
            }
            HorizontalStaticEdge::Right => self.point.x - box_size.width - margin.right,
        };
        let y = match self.vertical_edge {
            VerticalStaticEdge::Top => self.point.y + margin.top,
            VerticalStaticEdge::Center => {
                self.point.y - box_size.height / 2.0 + (margin.top - margin.bottom) / 2.0
            }
            VerticalStaticEdge::Bottom => self.point.y - box_size.height - margin.bottom,
        };
        Point { x, y }
    }
}

pub(crate) fn flex_main_axis_static_edge(
    justify_content: Option<AlignContent>,
    is_reverse: bool,
) -> LogicalStaticEdge {
    match justify_content
        .unwrap_or(AlignContent::FLEX_START)
        .keyword()
    {
        AlignContentKeyword::FlexEnd => {
            if is_reverse {
                LogicalStaticEdge::Start
            } else {
                LogicalStaticEdge::End
            }
        }
        AlignContentKeyword::Center
        | AlignContentKeyword::SpaceAround
        | AlignContentKeyword::SpaceEvenly => LogicalStaticEdge::Center,
        AlignContentKeyword::Start => LogicalStaticEdge::Start,
        AlignContentKeyword::End => LogicalStaticEdge::End,
        AlignContentKeyword::FlexStart
        | AlignContentKeyword::Stretch
        | AlignContentKeyword::SpaceBetween => {
            if is_reverse {
                LogicalStaticEdge::End
            } else {
                LogicalStaticEdge::Start
            }
        }
    }
}

pub(crate) struct FlexCrossAxisStaticContext {
    pub(crate) align_self: Option<AlignSelf>,
    pub(crate) align_items: Option<AlignItems>,
    pub(crate) flex_wrap: FlexWrap,
    pub(crate) child_writing_mode: WritingMode,
    pub(crate) child_direction: Direction,
    pub(crate) container_writing_mode: WritingMode,
    pub(crate) container_direction: Direction,
    pub(crate) physical_axis: AbsoluteAxis,
    pub(crate) overflows: bool,
}

impl FlexCrossAxisStaticContext {
    pub(crate) fn resolve(self) -> LogicalStaticEdge {
        let alignment = self
            .align_self
            .or(self.align_items)
            .unwrap_or(AlignItems::STRETCH);
        let mut keyword = if alignment.safety == AlignmentSafety::Safe && self.overflows {
            AlignItemsKeyword::Start
        } else {
            alignment.keyword()
        };

        keyword =
            match keyword {
                AlignItemsKeyword::Start => AlignItemsKeyword::FlexStart,
                AlignItemsKeyword::End => AlignItemsKeyword::FlexEnd,
                AlignItemsKeyword::SelfStart | AlignItemsKeyword::SelfEnd => {
                    let child_start_reversed = self
                        .child_writing_mode
                        .is_axis_flow_reversed(self.physical_axis, self.child_direction);
                    let container_start_reversed = self
                        .container_writing_mode
                        .is_axis_flow_reversed(self.physical_axis, self.container_direction);
                    let starts_match = child_start_reversed == container_start_reversed;
                    match (keyword, starts_match) {
                        (AlignItemsKeyword::SelfStart, true)
                        | (AlignItemsKeyword::SelfEnd, false) => AlignItemsKeyword::FlexStart,
                        (AlignItemsKeyword::SelfStart, false)
                        | (AlignItemsKeyword::SelfEnd, true) => AlignItemsKeyword::FlexEnd,
                        _ => unreachable!("self-relative alignment was matched above"),
                    }
                }
                keyword => keyword,
            };

        if self.flex_wrap == FlexWrap::WrapReverse {
            keyword = match keyword {
                AlignItemsKeyword::FlexStart => AlignItemsKeyword::FlexEnd,
                AlignItemsKeyword::FlexEnd => AlignItemsKeyword::FlexStart,
                keyword => keyword,
            };
        }

        match keyword {
            AlignItemsKeyword::Center => LogicalStaticEdge::Center,
            AlignItemsKeyword::FlexEnd => LogicalStaticEdge::End,
            AlignItemsKeyword::Stretch if self.flex_wrap == FlexWrap::WrapReverse => {
                LogicalStaticEdge::End
            }
            AlignItemsKeyword::Start
            | AlignItemsKeyword::End
            | AlignItemsKeyword::FlexStart
            | AlignItemsKeyword::SelfStart
            | AlignItemsKeyword::SelfEnd
            | AlignItemsKeyword::Baseline
            | AlignItemsKeyword::Stretch => LogicalStaticEdge::Start,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PhysicalAxisStaticEdge {
    Min,
    Center,
    Max,
}

fn physical_axis_static_position(
    min: f32,
    size: f32,
    edge: LogicalStaticEdge,
    start_is_reversed: bool,
) -> (f32, PhysicalAxisStaticEdge) {
    match (edge, start_is_reversed) {
        (LogicalStaticEdge::Start, false) | (LogicalStaticEdge::End, true) => {
            (min, PhysicalAxisStaticEdge::Min)
        }
        (LogicalStaticEdge::Center, _) => (min + size / 2.0, PhysicalAxisStaticEdge::Center),
        (LogicalStaticEdge::End, false) | (LogicalStaticEdge::Start, true) => {
            (min + size, PhysicalAxisStaticEdge::Max)
        }
    }
}

pub(crate) fn physical_static_position_from_logical(
    content_origin: Point<f32>,
    content_size: Size<f32>,
    writing_mode: WritingMode,
    direction: Direction,
    inline_edge: LogicalStaticEdge,
    block_edge: LogicalStaticEdge,
) -> PhysicalStaticPosition {
    let inline_axis = writing_mode.inline_axis();
    let (inline_offset, inline_physical_edge) = physical_axis_static_position(
        match inline_axis {
            AbsoluteAxis::Horizontal => content_origin.x,
            AbsoluteAxis::Vertical => content_origin.y,
        },
        content_size.get_abs(inline_axis),
        inline_edge,
        writing_mode.is_inline_flow_reversed(direction),
    );
    let block_axis = writing_mode.block_axis();
    let (block_offset, block_physical_edge) = physical_axis_static_position(
        match block_axis {
            AbsoluteAxis::Horizontal => content_origin.x,
            AbsoluteAxis::Vertical => content_origin.y,
        },
        content_size.get_abs(block_axis),
        block_edge,
        writing_mode.is_block_flow_reversed(),
    );

    match inline_axis {
        AbsoluteAxis::Horizontal => PhysicalStaticPosition::new(
            Point {
                x: inline_offset,
                y: block_offset,
            },
            match inline_physical_edge {
                PhysicalAxisStaticEdge::Min => HorizontalStaticEdge::Left,
                PhysicalAxisStaticEdge::Center => HorizontalStaticEdge::Center,
                PhysicalAxisStaticEdge::Max => HorizontalStaticEdge::Right,
            },
            match block_physical_edge {
                PhysicalAxisStaticEdge::Min => VerticalStaticEdge::Top,
                PhysicalAxisStaticEdge::Center => VerticalStaticEdge::Center,
                PhysicalAxisStaticEdge::Max => VerticalStaticEdge::Bottom,
            },
        ),
        AbsoluteAxis::Vertical => PhysicalStaticPosition::new(
            Point {
                x: block_offset,
                y: inline_offset,
            },
            match block_physical_edge {
                PhysicalAxisStaticEdge::Min => HorizontalStaticEdge::Left,
                PhysicalAxisStaticEdge::Center => HorizontalStaticEdge::Center,
                PhysicalAxisStaticEdge::Max => HorizontalStaticEdge::Right,
            },
            match inline_physical_edge {
                PhysicalAxisStaticEdge::Min => VerticalStaticEdge::Top,
                PhysicalAxisStaticEdge::Center => VerticalStaticEdge::Center,
                PhysicalAxisStaticEdge::Max => VerticalStaticEdge::Bottom,
            },
        ),
    }
}

/// Resolve auto margins in one physical axis of an absolutely positioned box.
///
/// CSS Positioned Layout only distributes auto margins when both insets in
/// the axis are definite. Inline-axis negative space preserves the dominant
/// start edge; block-axis negative space is shared between both margins.
pub(crate) fn resolve_absolute_axis_margins(
    margin: Line<Option<f32>>,
    inset: Line<Option<f32>>,
    area_size: f32,
    box_size: f32,
    share_negative_space: bool,
    start_is_dominant: bool,
) -> Line<f32> {
    if inset.start.is_none() || inset.end.is_none() {
        return Line {
            start: margin.start.unwrap_or(0.0),
            end: margin.end.unwrap_or(0.0),
        };
    }

    let free_space = area_size
        - inset.start.unwrap()
        - inset.end.unwrap()
        - box_size
        - margin.start.unwrap_or(0.0)
        - margin.end.unwrap_or(0.0);
    match (margin.start, margin.end) {
        (Some(start), Some(end)) => Line { start, end },
        (None, Some(end)) => Line {
            start: free_space,
            end,
        },
        (Some(start), None) => Line {
            start,
            end: free_space,
        },
        (None, None) if free_space > 0.0 || share_negative_space => {
            let start = free_space / 2.0;
            Line {
                start,
                end: free_space - start,
            }
        }
        (None, None) if start_is_dominant => Line {
            start: 0.0,
            end: free_space,
        },
        (None, None) => Line {
            start: free_space,
            end: 0.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTO: Line<Option<f32>> = Line {
        start: None,
        end: None,
    };
    const ZERO_INSETS: Line<Option<f32>> = Line {
        start: Some(0.0),
        end: Some(0.0),
    };

    #[test]
    fn centered_static_position_centers_the_margin_box() {
        let position = PhysicalStaticPosition::new(
            Point { x: 100.0, y: 50.0 },
            HorizontalStaticEdge::Center,
            VerticalStaticEdge::Center,
        );
        assert_eq!(
            position.margin_box_origin(
                Size {
                    width: 20.0,
                    height: 10.0,
                },
                Rect {
                    left: 4.0,
                    right: 8.0,
                    top: 2.0,
                    bottom: 6.0,
                },
            ),
            Point { x: 88.0, y: 43.0 }
        );
    }

    #[test]
    fn logical_static_position_respects_vertical_flow_and_rtl() {
        let position = physical_static_position_from_logical(
            Point { x: 20.0, y: 10.0 },
            Size {
                width: 160.0,
                height: 80.0,
            },
            WritingMode::VerticalRl,
            Direction::Rtl,
            LogicalStaticEdge::Start,
            LogicalStaticEdge::Start,
        );
        assert_eq!(
            position,
            PhysicalStaticPosition::new(
                Point { x: 180.0, y: 90.0 },
                HorizontalStaticEdge::Right,
                VerticalStaticEdge::Bottom,
            )
        );
    }

    #[test]
    fn flex_static_edges_distinguish_flow_and_flex_relative_values() {
        assert_eq!(
            flex_main_axis_static_edge(Some(AlignContent::START), true),
            LogicalStaticEdge::Start
        );
        assert_eq!(
            flex_main_axis_static_edge(Some(AlignContent::FLEX_START), true),
            LogicalStaticEdge::End
        );
        assert_eq!(
            flex_main_axis_static_edge(Some(AlignContent::FLEX_END), true),
            LogicalStaticEdge::Start
        );
        assert_eq!(
            FlexCrossAxisStaticContext {
                align_self: None,
                align_items: None,
                flex_wrap: FlexWrap::WrapReverse,
                child_writing_mode: WritingMode::HorizontalTb,
                child_direction: Direction::Ltr,
                container_writing_mode: WritingMode::HorizontalTb,
                container_direction: Direction::Ltr,
                physical_axis: AbsoluteAxis::Vertical,
                overflows: false,
            }
            .resolve(),
            LogicalStaticEdge::End
        );
        assert_eq!(
            FlexCrossAxisStaticContext {
                align_self: Some(AlignItems::SAFE_CENTER),
                align_items: None,
                flex_wrap: FlexWrap::NoWrap,
                child_writing_mode: WritingMode::HorizontalTb,
                child_direction: Direction::Ltr,
                container_writing_mode: WritingMode::HorizontalTb,
                container_direction: Direction::Ltr,
                physical_axis: AbsoluteAxis::Vertical,
                overflows: true,
            }
            .resolve(),
            LogicalStaticEdge::Start
        );
    }

    #[test]
    fn positive_space_is_shared_even_when_the_box_is_wider_than_that_space() {
        assert_eq!(
            resolve_absolute_axis_margins(AUTO, ZERO_INSETS, 1440.0, 975.0, false, true),
            Line {
                start: 232.5,
                end: 232.5,
            }
        );
    }

    #[test]
    fn inline_negative_space_preserves_the_dominant_start_edge() {
        assert_eq!(
            resolve_absolute_axis_margins(AUTO, ZERO_INSETS, 100.0, 150.0, false, true),
            Line {
                start: 0.0,
                end: -50.0,
            }
        );
        assert_eq!(
            resolve_absolute_axis_margins(AUTO, ZERO_INSETS, 100.0, 150.0, false, false),
            Line {
                start: -50.0,
                end: 0.0,
            }
        );
    }

    #[test]
    fn block_negative_space_is_shared() {
        assert_eq!(
            resolve_absolute_axis_margins(AUTO, ZERO_INSETS, 100.0, 120.0, true, true),
            Line {
                start: -10.0,
                end: -10.0,
            }
        );
    }

    #[test]
    fn an_auto_inset_forces_auto_margins_to_zero() {
        assert_eq!(
            resolve_absolute_axis_margins(
                AUTO,
                Line {
                    start: Some(0.0),
                    end: None,
                },
                100.0,
                20.0,
                false,
                true,
            ),
            Line {
                start: 0.0,
                end: 0.0,
            }
        );
    }
}
