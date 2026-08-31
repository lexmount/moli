use taffy::{AbsoluteAxis, AlignItems, AlignItemsKeyword, Line, Point, Size};

use crate::style::ResolvedLayoutStyle;

/// Physical edge carried with a static-position anchor. Keeping the edge is
/// essential: a center or end anchor cannot be converted to a used location
/// until the real out-of-flow box has been sized by its containing block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StaticPositionEdge {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PositionedStaticPosition {
    pub(crate) global_anchor: Point<f32>,
    pub(crate) horizontal_edge: StaticPositionEdge,
    pub(crate) vertical_edge: StaticPositionEdge,
}

/// Produces the static-position anchor for a direct Grid child whose actual
/// CSS containing block is a more distant ancestor.
///
/// Grid placement lines are deliberately absent from this input. Per CSS
/// Grid and Blink's `PlaceOutOfFlowItems`, authored grid lines define the
/// containing rectangle only when Grid itself generates the containing block.
/// Otherwise the static-position rectangle is the Grid content box.
pub(crate) fn grid_static_position(
    content_origin: Point<f32>,
    content_size: Size<f32>,
    child_style: &ResolvedLayoutStyle,
    grid_style: &ResolvedLayoutStyle,
) -> PositionedStaticPosition {
    let horizontal_edge =
        grid_static_position_edge(child_style, grid_style, AbsoluteAxis::Horizontal);
    let vertical_edge = grid_static_position_edge(child_style, grid_style, AbsoluteAxis::Vertical);

    PositionedStaticPosition {
        global_anchor: Point {
            x: static_position_anchor(content_origin.x, content_size.width, horizontal_edge),
            y: static_position_anchor(content_origin.y, content_size.height, vertical_edge),
        },
        horizontal_edge,
        vertical_edge,
    }
}

fn grid_static_position_edge(
    child_style: &ResolvedLayoutStyle,
    grid_style: &ResolvedLayoutStyle,
    axis: AbsoluteAxis,
) -> StaticPositionEdge {
    let grid_writing_mode = grid_style.writing_mode();
    let alignment = if grid_writing_mode.inline_axis() == axis {
        child_style
            .taffy
            .justify_self
            .or(grid_style.taffy.justify_items)
    } else {
        child_style
            .taffy
            .align_self
            .or(grid_style.taffy.align_items)
    }
    .unwrap_or(AlignItems::NORMAL);

    let child_flow_reversed = child_style
        .writing_mode()
        .is_axis_flow_reversed(axis, child_style.taffy.direction);
    let grid_flow_reversed =
        grid_writing_mode.is_axis_flow_reversed(axis, grid_style.taffy.direction);
    let keyword = match alignment.keyword {
        AlignItemsKeyword::SelfStart if child_flow_reversed != grid_flow_reversed => {
            AlignItemsKeyword::End
        }
        AlignItemsKeyword::SelfStart => AlignItemsKeyword::Start,
        AlignItemsKeyword::SelfEnd if child_flow_reversed != grid_flow_reversed => {
            AlignItemsKeyword::Start
        }
        AlignItemsKeyword::SelfEnd => AlignItemsKeyword::End,
        keyword => keyword,
    };
    let logical_edge = match keyword {
        AlignItemsKeyword::End | AlignItemsKeyword::FlexEnd => StaticPositionEdge::End,
        AlignItemsKeyword::Center => StaticPositionEdge::Center,
        AlignItemsKeyword::Normal
        | AlignItemsKeyword::Start
        | AlignItemsKeyword::FlexStart
        | AlignItemsKeyword::Baseline
        | AlignItemsKeyword::Stretch => StaticPositionEdge::Start,
        AlignItemsKeyword::SelfStart | AlignItemsKeyword::SelfEnd => {
            unreachable!("self-relative Grid alignment was resolved above")
        }
    };
    if grid_flow_reversed {
        match logical_edge {
            StaticPositionEdge::Start => StaticPositionEdge::End,
            StaticPositionEdge::Center => StaticPositionEdge::Center,
            StaticPositionEdge::End => StaticPositionEdge::Start,
        }
    } else {
        logical_edge
    }
}

fn static_position_anchor(start: f32, size: f32, edge: StaticPositionEdge) -> f32 {
    match edge {
        StaticPositionEdge::Start => start,
        StaticPositionEdge::Center => start + size * 0.5,
        StaticPositionEdge::End => start + size,
    }
}

pub(crate) fn used_static_axis_position(
    anchor: f32,
    edge: StaticPositionEdge,
    border_box_size: f32,
    margin_start: f32,
    margin_end: f32,
) -> f32 {
    match edge {
        StaticPositionEdge::Start => anchor + margin_start,
        StaticPositionEdge::Center => {
            anchor - border_box_size * 0.5 + (margin_start - margin_end) * 0.5
        }
        StaticPositionEdge::End => anchor - border_box_size - margin_end,
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

    #[test]
    fn typed_static_edges_apply_the_sized_margin_box_once() {
        assert_eq!(
            used_static_axis_position(100.0, StaticPositionEdge::Start, 20.0, 3.0, 7.0),
            103.0
        );
        assert_eq!(
            used_static_axis_position(100.0, StaticPositionEdge::Center, 20.0, 3.0, 7.0),
            88.0
        );
        assert_eq!(
            used_static_axis_position(100.0, StaticPositionEdge::End, 20.0, 3.0, 7.0),
            73.0
        );
    }
}
