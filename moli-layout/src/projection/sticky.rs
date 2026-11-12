//! CSS relationship/inset preparation and a geometry-only sticky axis solver.
//!
//! The constraints are stack-local to this projection. They retain reference
//! spaces, not global edges: an ancestor may itself move before we resolve them.
//! This extraction preserves the existing inset and containing-block policy;
//! extending that policy does not require changing coordinate composition.

use std::{fmt::Debug, hash::Hash};

use taffy::MaybeResolve;

use crate::{
    LayoutBoxId, LayoutError, LayoutPoint, LayoutRect, LayoutScrollExtent, LayoutSize,
    LayoutTransform2D, LayoutWorld,
};

#[derive(Clone, Copy, Debug, Default)]
struct StickyAxisConstraint {
    start_inset: Option<f32>,
    end_inset: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
struct StickyAxisGeometry {
    normal_start: f32,
    size: f32,
    scrollport_start: f32,
    scrollport_end: f32,
    containing_range: Option<(f32, f32)>,
}

/// Returns this node's additional displacement, never an ancestor's delta.
fn solve_sticky_axis(constraint: StickyAxisConstraint, geometry: StickyAxisGeometry) -> f32 {
    let mut target = geometry.normal_start;
    if let Some(start) = constraint.start_inset {
        target = target.max(geometry.scrollport_start + start);
    }
    if let Some(end) = constraint.end_inset {
        target = target.min(geometry.scrollport_end - end - geometry.size);
    }
    if (constraint.start_inset.is_some() || constraint.end_inset.is_some())
        && let Some((minimum, maximum)) = geometry.containing_range
    {
        target = target.clamp(
            minimum.min(geometry.normal_start),
            maximum.max(geometry.normal_start),
        );
    }
    target - geometry.normal_start
}

pub(super) struct StickyConstraints {
    pub(super) scroll_container: LayoutBoxId,
    scrollport: LayoutRect,
    size: LayoutSize,
    x: StickyAxisConstraint,
    y: StickyAxisConstraint,
    containing: Option<ContainingBlockConstraint>,
}

struct ContainingBlockConstraint {
    border_start: LayoutPoint,
    padding_start: LayoutPoint,
    padding_end: LayoutPoint,
    far_edge: LayoutPoint,
}

pub(super) fn prepare_constraints<N>(
    index: usize,
    world: &LayoutWorld<N>,
    scroll_extents: &[LayoutScrollExtent],
) -> Result<Option<StickyConstraints>, LayoutError>
where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &world.boxes[index];
    if layout_box.style.position() != crate::LayoutPosition::Sticky {
        return Ok(None);
    }
    let mut ancestor = layout_box.parent;
    let mut scroll_container = world.root;
    let mut remaining = world.boxes.len();
    while let Some(candidate) = ancestor {
        let candidate_box =
            world
                .boxes
                .get(candidate.index())
                .ok_or(LayoutError::InvalidBoxReference {
                    index: candidate.index(),
                })?;
        if remaining == 0 {
            return Err(LayoutError::NumericTreeCycle {
                index: candidate.index(),
            });
        }
        remaining -= 1;
        if candidate == world.root || scroll_extents[candidate.index()].is_scroll_container {
            scroll_container = candidate;
            break;
        }
        ancestor = candidate_box.parent;
    }
    let scrollport = scroll_extents[scroll_container.index()].scrollport;
    let layout = layout_box.final_layout;
    let inset = layout_box.style.sticky_inset();
    let resolve = crate::style::resolve_stylo_calc_value;
    let containing = layout_box.layout_parent.map(|parent| {
        let containing = world.boxes[parent.index()].final_layout;
        let mut right = containing.size.width - containing.border.right;
        let mut bottom = containing.size.height - containing.border.bottom;
        if parent != world.root && scroll_extents[parent.index()].is_scroll_container {
            let extent = &scroll_extents[parent.index()];
            right = right.max(extent.scrollport.x + extent.scroll_size.width);
            bottom = bottom.max(extent.scrollport.y + extent.scroll_size.height);
        }
        ContainingBlockConstraint {
            border_start: LayoutPoint::new(containing.border.left, containing.border.top),
            padding_start: LayoutPoint::new(containing.padding.left, containing.padding.top),
            padding_end: LayoutPoint::new(containing.padding.right, containing.padding.bottom),
            far_edge: LayoutPoint::new(right, bottom),
        }
    });
    Ok(Some(StickyConstraints {
        scroll_container,
        scrollport,
        size: LayoutSize::new(layout.size.width, layout.size.height),
        x: StickyAxisConstraint {
            start_inset: inset.left.maybe_resolve(scrollport.width, resolve),
            end_inset: inset.right.maybe_resolve(scrollport.width, resolve),
        },
        y: StickyAxisConstraint {
            start_inset: inset.top.maybe_resolve(scrollport.height, resolve),
            end_inset: inset.bottom.maybe_resolve(scrollport.height, resolve),
        },
        containing,
    }))
}

impl StickyConstraints {
    /// `scrollport_origin` uses transform-free visual document CSS pixels.
    /// `parent_transform` maps parent-local layout pixels into that space;
    /// `normal_location` and `parent_scroll` remain parent-local, already zoomed.
    /// The transform-free projection contains only translations, so the solved
    /// document displacement is also a parent-local delta. No style/tree reads
    /// occur here; ancestor geometry has already been resolved by the caller.
    pub(super) fn resolve_offset(
        &self,
        scrollport_origin: LayoutPoint,
        parent_transform: LayoutTransform2D,
        parent_scroll: LayoutPoint,
        normal_location: LayoutPoint,
    ) -> LayoutPoint {
        let normal = parent_transform.map_point(LayoutPoint::new(
            normal_location.x - parent_scroll.x,
            normal_location.y - parent_scroll.y,
        ));
        let containing_origin =
            parent_transform.map_point(LayoutPoint::new(-parent_scroll.x, -parent_scroll.y));
        let range = self.containing.as_ref().map(|containing| {
            (
                LayoutPoint::new(
                    containing_origin.x + containing.border_start.x + containing.padding_start.x,
                    containing_origin.y + containing.border_start.y + containing.padding_start.y,
                ),
                LayoutPoint::new(
                    containing_origin.x + containing.far_edge.x
                        - containing.padding_end.x
                        - self.size.width,
                    containing_origin.y + containing.far_edge.y
                        - containing.padding_end.y
                        - self.size.height,
                ),
            )
        });
        // Keep the original f32 evaluation order at snapping boundaries.
        let scrollport = crate::overflow::offset_rect(self.scrollport, scrollport_origin);
        LayoutPoint::new(
            solve_sticky_axis(
                self.x,
                StickyAxisGeometry {
                    normal_start: normal.x,
                    size: self.size.width,
                    scrollport_start: scrollport.x,
                    scrollport_end: scrollport.right(),
                    containing_range: range.map(|(min, max)| (min.x, max.x)),
                },
            ),
            solve_sticky_axis(
                self.y,
                StickyAxisGeometry {
                    normal_start: normal.y,
                    size: self.size.height,
                    scrollport_start: scrollport.y,
                    scrollport_end: scrollport.bottom(),
                    containing_range: range.map(|(min, max)| (min.y, max.y)),
                },
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticky_axis_respects_insets_containing_limits_and_unconstrained_axes() {
        // Normal, box size, scrollport, insets, containing limits, local delta.
        for (normal, size, port, insets, containing, expected) in [
            (
                84.0,
                40.0,
                (100.0, 340.0),
                (Some(60.0), None),
                Some((0.0, 560.0)),
                76.0,
            ),
            (300.0, 40.0, (0.0, 240.0), (None, Some(10.0)), None, -110.0),
            (
                0.0,
                30.0,
                (0.0, 100.0),
                (Some(10.0), Some(20.0)),
                None,
                10.0,
            ),
            (
                0.0,
                30.0,
                (100.0, 200.0),
                (None, None),
                Some((10.0, 20.0)),
                0.0,
            ),
            (
                84.0,
                40.0,
                (100.0, 340.0),
                (Some(60.0), None),
                Some((0.0, 120.0)),
                36.0,
            ),
            (
                500.0,
                40.0,
                (100.0, 340.0),
                (Some(60.0), None),
                Some((0.0, 120.0)),
                0.0,
            ),
            // Preserve the parent's current oversized/both-inset policy. A CSS
            // policy correction belongs here, separately from this extraction.
            (
                0.0,
                150.0,
                (0.0, 100.0),
                (Some(10.0), Some(10.0)),
                None,
                -60.0,
            ),
        ] {
            let delta = solve_sticky_axis(
                StickyAxisConstraint {
                    start_inset: insets.0,
                    end_inset: insets.1,
                },
                StickyAxisGeometry {
                    normal_start: normal,
                    size,
                    scrollport_start: port.0,
                    scrollport_end: port.1,
                    containing_range: containing,
                },
            );
            assert_eq!(delta, expected);
        }
    }
}
