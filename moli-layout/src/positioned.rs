use taffy::{
    AbsoluteAxis, AbstractAxis, AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword,
    AlignSelf, AlignmentSafety, Direction, FlexWrap, Line, Point, Rect, Size, WritingMode,
};

use crate::ResolvedLayoutStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogicalStaticEdge {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LogicalStaticAlignment {
    edge: LogicalStaticEdge,
    safety: AlignmentSafety,
}

impl From<LogicalStaticEdge> for LogicalStaticAlignment {
    fn from(edge: LogicalStaticEdge) -> Self {
        Self {
            edge,
            safety: AlignmentSafety::Unsafe,
        }
    }
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
    safety: Size<AlignmentSafety>,
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
            safety: Size {
                width: AlignmentSafety::Unsafe,
                height: AlignmentSafety::Unsafe,
            },
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

    /// Fit-content width available on the side(s) selected by the static
    /// position. A center edge constrains both sides, unlike a start point.
    pub(crate) fn available_width(self, containing_width: f32) -> f32 {
        let bounds = self
            .horizontal_axis()
            .inset_modified_bounds(containing_width);
        bounds.end - bounds.start
    }

    fn horizontal_axis(self) -> StaticPositionAxis {
        StaticPositionAxis {
            offset: self.point.x,
            edge: match self.horizontal_edge {
                HorizontalStaticEdge::Left => PhysicalAxisStaticEdge::Min,
                HorizontalStaticEdge::Center => PhysicalAxisStaticEdge::Center,
                HorizontalStaticEdge::Right => PhysicalAxisStaticEdge::Max,
            },
            safety: self.safety.width,
        }
    }

    pub(crate) fn border_box_origin(
        self,
        box_size: Size<f32>,
        margin: Rect<f32>,
        containing_size: Size<f32>,
        containing_writing_mode: WritingMode,
        containing_direction: Direction,
    ) -> Point<f32> {
        let horizontal = self.horizontal_axis();
        let vertical = StaticPositionAxis {
            offset: self.point.y,
            edge: match self.vertical_edge {
                VerticalStaticEdge::Top => PhysicalAxisStaticEdge::Min,
                VerticalStaticEdge::Center => PhysicalAxisStaticEdge::Center,
                VerticalStaticEdge::Bottom => PhysicalAxisStaticEdge::Max,
            },
            safety: self.safety.height,
        };
        Point {
            x: horizontal.border_box_start(
                containing_size.width,
                box_size.width,
                Line {
                    start: margin.left,
                    end: margin.right,
                },
                containing_writing_mode
                    .is_axis_flow_reversed(AbsoluteAxis::Horizontal, containing_direction),
            ),
            y: vertical.border_box_start(
                containing_size.height,
                box_size.height,
                Line {
                    start: margin.top,
                    end: margin.bottom,
                },
                containing_writing_mode
                    .is_axis_flow_reversed(AbsoluteAxis::Vertical, containing_direction),
            ),
        }
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

/// Grid contributes alignment edges even when it does not establish the
/// positioned child's containing block. These edges are relative to the grid's
/// writing direction, including self-relative alignment in orthogonal flows.
pub(crate) fn grid_static_alignment(
    child: &ResolvedLayoutStyle,
    container: &ResolvedLayoutStyle,
) -> (LogicalStaticAlignment, LogicalStaticAlignment) {
    let resolve = |logical_axis| {
        let alignment = grid_item_alignment(child, container, logical_axis);
        let axis = match logical_axis {
            AbstractAxis::Inline => container.writing_mode().inline_axis(),
            AbstractAxis::Block => container.writing_mode().block_axis(),
        };
        let edge = match alignment.keyword() {
            AlignItemsKeyword::Center => LogicalStaticEdge::Center,
            AlignItemsKeyword::End | AlignItemsKeyword::FlexEnd => LogicalStaticEdge::End,
            AlignItemsKeyword::SelfStart | AlignItemsKeyword::SelfEnd => {
                let starts_match = child
                    .writing_mode()
                    .is_axis_flow_reversed(axis, child.taffy.direction)
                    == container
                        .writing_mode()
                        .is_axis_flow_reversed(axis, container.taffy.direction);
                if (alignment.keyword() == AlignItemsKeyword::SelfStart) == starts_match {
                    LogicalStaticEdge::Start
                } else {
                    LogicalStaticEdge::End
                }
            }
            AlignItemsKeyword::Start
            | AlignItemsKeyword::FlexStart
            | AlignItemsKeyword::Baseline
            | AlignItemsKeyword::Stretch => LogicalStaticEdge::Start,
        };
        LogicalStaticAlignment {
            edge,
            safety: alignment.safety,
        }
    };
    // As in Blink's AlignmentOffsetForOutOfFlow, overflow safety belongs to
    // absolute layout in the actual containing block, not to this static-
    // position contribution from a different formatting parent.
    (resolve(AbstractAxis::Inline), resolve(AbstractAxis::Block))
}

fn grid_item_alignment(
    child: &ResolvedLayoutStyle,
    container: &ResolvedLayoutStyle,
    axis: AbstractAxis,
) -> AlignItems {
    use style::values::specified::align::AlignFlags;

    // CSS alignment is resolved against the formatting parent, which need
    // not be the numeric parent. Keep physical left/right through this
    // conversion; their logical edge depends on the grid's direction.
    let convert = |flags: AlignFlags| {
        let flags = match flags.value() {
            AlignFlags::LEFT | AlignFlags::RIGHT => flags.with_value(
                if (flags.value() == AlignFlags::LEFT)
                    == (container.taffy.direction == Direction::Ltr)
                {
                    AlignFlags::START
                } else {
                    AlignFlags::END
                },
            ),
            _ => flags,
        };
        stylo_taffy::convert::item_alignment(flags)
    };
    let child_alignment = child.computed.as_ref().map_or_else(
        || match axis {
            AbstractAxis::Inline => child.taffy.justify_self,
            AbstractAxis::Block => child.taffy.align_self,
        },
        |computed| {
            convert(match axis {
                AbstractAxis::Inline => computed.clone_justify_self().0,
                AbstractAxis::Block => computed.clone_align_self().0,
            })
        },
    );
    child_alignment
        .or_else(|| {
            container.computed.as_ref().map_or_else(
                || match axis {
                    AbstractAxis::Inline => container.taffy.justify_items,
                    AbstractAxis::Block => container.taffy.align_items,
                },
                |computed| {
                    convert(match axis {
                        AbstractAxis::Inline => *computed.clone_justify_items().computed.0,
                        AbstractAxis::Block => computed.clone_align_items().0,
                    })
                },
            )
        })
        .unwrap_or(AlignItems::STRETCH)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PhysicalAxisStaticEdge {
    Min,
    Center,
    Max,
}

/// One physical axis of the inset-modified containing block. A centered
/// static position grows equally toward both containing-block edges until
/// the nearer edge is reached, as in Blink's ComputeUnclampedIMCBInOneAxis.
struct StaticPositionAxis {
    offset: f32,
    edge: PhysicalAxisStaticEdge,
    safety: AlignmentSafety,
}

impl StaticPositionAxis {
    fn inset_modified_bounds(&self, available: f32) -> Line<f32> {
        match self.edge {
            PhysicalAxisStaticEdge::Min => Line {
                start: self.offset,
                end: available,
            },
            PhysicalAxisStaticEdge::Max => Line {
                start: 0.0,
                end: self.offset,
            },
            PhysicalAxisStaticEdge::Center => {
                let half = self.offset.min(available - self.offset);
                Line {
                    start: self.offset - half,
                    end: self.offset + half,
                }
            }
        }
    }

    fn border_box_start(
        &self,
        available: f32,
        size: f32,
        margin: Line<f32>,
        containing_start_reversed: bool,
    ) -> f32 {
        let bounds = self.inset_modified_bounds(available);
        let margin_box_size = size + margin.start + margin.end;
        if self.safety == AlignmentSafety::Safe && margin_box_size > bounds.end - bounds.start {
            return if containing_start_reversed {
                bounds.end - size - margin.end
            } else {
                bounds.start + margin.start
            };
        }
        match self.edge {
            PhysicalAxisStaticEdge::Min => self.offset + margin.start,
            PhysicalAxisStaticEdge::Center => {
                self.offset - size / 2.0 + (margin.start - margin.end) / 2.0
            }
            PhysicalAxisStaticEdge::Max => self.offset - size - margin.end,
        }
    }
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
    inline_alignment: LogicalStaticAlignment,
    block_alignment: LogicalStaticAlignment,
) -> PhysicalStaticPosition {
    let inline_axis = writing_mode.inline_axis();
    let (inline_offset, inline_physical_edge) = physical_axis_static_position(
        match inline_axis {
            AbsoluteAxis::Horizontal => content_origin.x,
            AbsoluteAxis::Vertical => content_origin.y,
        },
        content_size.get_abs(inline_axis),
        inline_alignment.edge,
        writing_mode.is_inline_flow_reversed(direction),
    );
    let block_axis = writing_mode.block_axis();
    let (block_offset, block_physical_edge) = physical_axis_static_position(
        match block_axis {
            AbsoluteAxis::Horizontal => content_origin.x,
            AbsoluteAxis::Vertical => content_origin.y,
        },
        content_size.get_abs(block_axis),
        block_alignment.edge,
        writing_mode.is_block_flow_reversed(),
    );

    let mut position = match inline_axis {
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
    };
    position.safety = match inline_axis {
        AbsoluteAxis::Horizontal => Size {
            width: inline_alignment.safety,
            height: block_alignment.safety,
        },
        AbsoluteAxis::Vertical => Size {
            width: block_alignment.safety,
            height: inline_alignment.safety,
        },
    };
    position
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
            position.border_box_origin(
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
                Size {
                    width: 200.0,
                    height: 100.0
                },
                WritingMode::HorizontalTb,
                Direction::Ltr,
            ),
            Point { x: 88.0, y: 43.0 }
        );
    }

    #[test]
    fn static_alignment_uses_inset_modified_containing_bounds_for_safety() {
        let center = StaticPositionAxis {
            offset: 130.0,
            edge: PhysicalAxisStaticEdge::Center,
            safety: AlignmentSafety::Safe,
        };
        let margins = Line {
            start: 0.0,
            end: 0.0,
        };
        // The static position may come from a 156px grid content box, but its
        // absolute containing block is 400px wide. A 220px child still fits
        // symmetrically around 130px and must not fall back to the grid start.
        assert_eq!(center.border_box_start(400.0, 220.0, margins, false), 20.0);
        assert_eq!(center.border_box_start(400.0, 280.0, margins, false), 0.0);
        assert_eq!(center.border_box_start(400.0, 280.0, margins, true), -20.0);

        let end_center = StaticPositionAxis {
            offset: 350.0,
            ..center
        };
        assert_eq!(
            end_center.border_box_start(400.0, 140.0, margins, false),
            300.0
        );
        assert_eq!(
            end_center.border_box_start(400.0, 140.0, margins, true),
            260.0
        );

        let unsafe_center = StaticPositionAxis {
            safety: AlignmentSafety::Unsafe,
            ..center
        };
        assert_eq!(
            unsafe_center.border_box_start(400.0, 280.0, margins, false),
            -10.0
        );

        let end = StaticPositionAxis {
            edge: PhysicalAxisStaticEdge::Max,
            ..center
        };
        assert_eq!(end.border_box_start(400.0, 140.0, margins, false), 0.0);
        assert_eq!(end.border_box_start(400.0, 140.0, margins, true), -10.0);
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
            LogicalStaticEdge::Start.into(),
            LogicalStaticEdge::Start.into(),
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
