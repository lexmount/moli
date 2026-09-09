use taffy::compute::{StaticPositionAxis, StaticPositionEdge as PhysicalAxisStaticEdge};
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
        self.translated(Point {
            x: -origin.x,
            y: -origin.y,
        })
    }

    pub(crate) fn translated(self, offset: Point<f32>) -> Self {
        Self {
            point: Point {
                x: self.point.x + offset.x,
                y: self.point.y + offset.y,
            },
            ..self
        }
    }

    /// Fit-content space on both physical axes selected by the static edges.
    /// The child chooses its inline axis from its own writing mode; a center
    /// edge constrains both sides, unlike a start point.
    pub(crate) fn available_size(self, containing_size: Size<f32>) -> Size<f32> {
        let extent = |axis: StaticPositionAxis, size| {
            let bounds = axis.inset_modified_bounds(size);
            bounds.end - bounds.start
        };
        Size {
            width: extent(self.horizontal_axis(), containing_size.width),
            height: extent(self.vertical_axis(), containing_size.height),
        }
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

    fn vertical_axis(self) -> StaticPositionAxis {
        StaticPositionAxis {
            offset: self.point.y,
            edge: match self.vertical_edge {
                VerticalStaticEdge::Top => PhysicalAxisStaticEdge::Min,
                VerticalStaticEdge::Center => PhysicalAxisStaticEdge::Center,
                VerticalStaticEdge::Bottom => PhysicalAxisStaticEdge::Max,
            },
            safety: self.safety.height,
        }
    }

    /// Resolve the border-box origin from the static margin-box edge and used
    /// margins. Auto margins on a static-position axis have already become zero
    /// during absolute sizing; this step must not distribute them a second time.
    pub(crate) fn border_box_origin(
        self,
        box_size: Size<f32>,
        margin: Rect<f32>,
        containing_size: Size<f32>,
        containing_writing_mode: WritingMode,
        containing_direction: Direction,
    ) -> Point<f32> {
        let horizontal = self.horizontal_axis();
        let vertical = self.vertical_axis();
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
}

impl FlexCrossAxisStaticContext {
    pub(crate) fn resolve(self) -> LogicalStaticAlignment {
        let alignment = self
            .align_self
            .or(self.align_items)
            .unwrap_or(AlignItems::STRETCH);
        let keyword = alignment.keyword();
        let wrap_reverse = self.flex_wrap == FlexWrap::WrapReverse;
        let edge = match keyword {
            AlignItemsKeyword::Center => LogicalStaticEdge::Center,
            AlignItemsKeyword::End | AlignItemsKeyword::LastBaseline => LogicalStaticEdge::End,
            AlignItemsKeyword::SelfStart | AlignItemsKeyword::SelfEnd => {
                let starts_match = self
                    .child_writing_mode
                    .is_axis_flow_reversed(self.physical_axis, self.child_direction)
                    == self
                        .container_writing_mode
                        .is_axis_flow_reversed(self.physical_axis, self.container_direction);
                if (keyword == AlignItemsKeyword::SelfStart) == starts_match {
                    LogicalStaticEdge::Start
                } else {
                    LogicalStaticEdge::End
                }
            }
            AlignItemsKeyword::FlexEnd if !wrap_reverse => LogicalStaticEdge::End,
            AlignItemsKeyword::FlexStart | AlignItemsKeyword::Stretch if wrap_reverse => {
                LogicalStaticEdge::End
            }
            AlignItemsKeyword::Start
            | AlignItemsKeyword::FlexStart
            | AlignItemsKeyword::FlexEnd
            | AlignItemsKeyword::Baseline
            | AlignItemsKeyword::Stretch => LogicalStaticEdge::Start,
        };
        // Like Blink's OOF pipeline, the flex parent supplies only the static
        // edge. The child's own self-alignment supplies overflow safety, which
        // is applied later in the actual absolute containing block. Inheriting
        // an alignment keyword must not also inherit the parent's safety.
        LogicalStaticAlignment {
            edge,
            safety: self
                .align_self
                .map_or(AlignmentSafety::Unsafe, |value| value.safety),
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
            AlignItemsKeyword::End
            | AlignItemsKeyword::FlexEnd
            | AlignItemsKeyword::LastBaseline => LogicalStaticEdge::End,
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
        crate::style::taffy_item_alignment(flags)
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

#[cfg(test)]
mod tests {
    use super::*;
    use taffy::{WritingDirection, compute::resolve_absolute_margins};

    const AUTO: Rect<Option<f32>> = Rect {
        left: None,
        right: None,
        top: None,
        bottom: None,
    };
    const ZERO_INSETS: Rect<Option<f32>> = Rect {
        left: Some(0.0),
        right: Some(0.0),
        top: Some(0.0),
        bottom: Some(0.0),
    };
    const HORIZONTAL_LTR: WritingDirection = WritingDirection {
        mode: WritingMode::HorizontalTb,
        direction: Direction::Ltr,
    };

    #[test]
    fn static_position_constrains_both_physical_available_axes() {
        let position = PhysicalStaticPosition::new(
            Point { x: 40.0, y: 70.0 },
            HorizontalStaticEdge::Center,
            VerticalStaticEdge::Bottom,
        );
        assert_eq!(
            position.available_size(Size {
                width: 200.0,
                height: 100.0
            }),
            Size {
                width: 80.0,
                height: 70.0
            },
        );
    }

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
            }
            .resolve()
            .edge,
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
            }
            .resolve()
            .edge,
            LogicalStaticEdge::Center
        );
    }

    #[test]
    fn positive_space_is_shared_even_when_the_box_is_wider_than_that_space() {
        assert_eq!(
            resolve_absolute_margins(
                AUTO,
                ZERO_INSETS,
                Size {
                    width: 1440.0,
                    height: 0.0
                },
                Size {
                    width: 975.0,
                    height: 0.0
                },
                HORIZONTAL_LTR
            ),
            Rect {
                left: 232.5,
                right: 232.5,
                ..Rect::ZERO
            }
        );
    }

    #[test]
    fn inline_negative_space_preserves_the_dominant_start_edge() {
        assert_eq!(
            resolve_absolute_margins(
                AUTO,
                ZERO_INSETS,
                Size {
                    width: 100.0,
                    height: 0.0
                },
                Size {
                    width: 150.0,
                    height: 0.0
                },
                HORIZONTAL_LTR
            ),
            Rect {
                left: 0.0,
                right: -50.0,
                ..Rect::ZERO
            }
        );
        assert_eq!(
            resolve_absolute_margins(
                AUTO,
                ZERO_INSETS,
                Size {
                    width: 100.0,
                    height: 0.0
                },
                Size {
                    width: 150.0,
                    height: 0.0
                },
                WritingDirection {
                    direction: Direction::Rtl,
                    ..HORIZONTAL_LTR
                }
            ),
            Rect {
                left: -50.0,
                right: 0.0,
                ..Rect::ZERO
            }
        );
    }

    #[test]
    fn block_negative_space_is_shared() {
        assert_eq!(
            resolve_absolute_margins(
                AUTO,
                ZERO_INSETS,
                Size {
                    width: 0.0,
                    height: 100.0
                },
                Size {
                    width: 0.0,
                    height: 120.0
                },
                HORIZONTAL_LTR
            ),
            Rect {
                top: -10.0,
                bottom: -10.0,
                ..Rect::ZERO
            }
        );
    }

    #[test]
    fn an_auto_inset_forces_auto_margins_to_zero() {
        assert_eq!(
            resolve_absolute_margins(
                AUTO,
                Rect {
                    left: Some(0.0),
                    ..AUTO
                },
                Size {
                    width: 100.0,
                    height: 100.0
                },
                Size {
                    width: 20.0,
                    height: 20.0
                },
                HORIZONTAL_LTR,
            ),
            Rect::ZERO
        );
    }
}
