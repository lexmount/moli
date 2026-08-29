//! Capture-aware conservative paint culling.
//!
//! Blink maps a cull rect through its paint property trees and falls back to
//! an infinite rect below effects that can move pixels. Moli has a linear
//! paint-order stream instead of retained paint chunks, so this module builds
//! the equivalent one-shot sidecar: exact event ink bounds, balanced stacking
//! context ranges, and the few boundaries where descendant culling must stop.

use std::{fmt::Debug, hash::Hash};

use super::{
    filters::{expanded_filter_clip, project_filters},
    geometry::BoxAreas,
};
use crate::{
    LayoutBoxId, LayoutPoint, LayoutRect, LayoutTransform2D, PaintFilter, PaintFragment,
    inline::InlinePaintBounds, projection::OutputProjection, stacking::PaintOrderEvent,
};

const CULL_ANTIALIAS_MARGIN: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum PaintCullBounds {
    Empty,
    Bounded(LayoutRect),
    Unbounded,
}

impl PaintCullBounds {
    fn include(&mut self, other: Self) {
        *self = match (*self, other) {
            (Self::Unbounded, _) | (_, Self::Unbounded) => Self::Unbounded,
            (Self::Empty, other) => other,
            (current, Self::Empty) => current,
            (Self::Bounded(current), Self::Bounded(other)) => Self::Bounded(current.union(other)),
        };
    }

    fn misses(self, region: PaintCullRegion) -> bool {
        match (self, region) {
            (Self::Empty, _) => true,
            (Self::Unbounded, _) | (_, PaintCullRegion::Infinite) => false,
            (Self::Bounded(bounds), PaintCullRegion::Bounded(cull)) => {
                !rects_intersect(bounds, cull)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PaintCullRegion {
    Bounded(LayoutRect),
    Infinite,
}

impl PaintCullRegion {
    pub(super) fn for_capture(rect: LayoutRect) -> Self {
        Self::Bounded(outset_rect(
            rect,
            CULL_ANTIALIAS_MARGIN,
            CULL_ANTIALIAS_MARGIN,
            CULL_ANTIALIAS_MARGIN,
            CULL_ANTIALIAS_MARGIN,
        ))
    }

    pub(super) fn local_rect(self, local_to_viewport: LayoutTransform2D) -> Option<LayoutRect> {
        let Self::Bounded(rect) = self else {
            return None;
        };
        let inverse = local_to_viewport.inverse()?;
        let mapped = inverse.map_rect(rect).bounding_rect();
        rect_is_finite(mapped).then_some(mapped)
    }
}

#[derive(Clone, Copy, Debug)]
struct PaintCullPlanEntry {
    bounds: PaintCullBounds,
    matching_pop: Option<usize>,
    disables_descendant_culling: bool,
}

impl Default for PaintCullPlanEntry {
    fn default() -> Self {
        Self {
            bounds: PaintCullBounds::Unbounded,
            matching_pop: None,
            disables_descendant_culling: false,
        }
    }
}

#[derive(Debug)]
struct ContextAccumulator {
    push_index: usize,
    id: LayoutBoxId,
    bounds: PaintCullBounds,
}

pub(super) struct PaintCullPlan {
    entries: Vec<PaintCullPlanEntry>,
}

impl PaintCullPlan {
    pub(super) fn build<N>(projection: &OutputProjection<'_, N>) -> Self
    where
        N: Copy + Debug + Eq + Hash,
    {
        let mut entries = vec![PaintCullPlanEntry::default(); projection.paint_events.len()];
        let mut contexts = Vec::<ContextAccumulator>::new();

        for (index, event) in projection.paint_events.iter().copied().enumerate() {
            match event {
                PaintOrderEvent::PushStackingContext(id) => {
                    contexts.push(ContextAccumulator {
                        push_index: index,
                        id,
                        bounds: PaintCullBounds::Empty,
                    });
                }
                PaintOrderEvent::PopStackingContext(id) => {
                    let Some(context) = contexts.pop() else {
                        continue;
                    };
                    debug_assert_eq!(context.id, id);
                    let (bounds, disables_descendant_culling) =
                        context_output_bounds(projection, id, context.bounds);
                    entries[context.push_index] = PaintCullPlanEntry {
                        bounds,
                        matching_pop: Some(index),
                        disables_descendant_culling,
                    };
                    entries[index] = PaintCullPlanEntry {
                        bounds,
                        matching_pop: None,
                        disables_descendant_culling: false,
                    };
                    if let Some(parent) = contexts.last_mut() {
                        parent.bounds.include(bounds);
                    }
                }
                _ => {
                    let bounds = event_ink_bounds(projection, event);
                    entries[index] = PaintCullPlanEntry {
                        bounds,
                        matching_pop: None,
                        disables_descendant_culling: false,
                    };
                    if let Some(context) = contexts.last_mut() {
                        context.bounds.include(bounds);
                    }
                }
            }
        }

        debug_assert!(contexts.is_empty(), "paint-order contexts must be balanced");
        Self { entries }
    }

    pub(super) fn event_misses(&self, index: usize, region: PaintCullRegion) -> bool {
        self.entries[index].bounds.misses(region)
    }

    pub(super) fn matching_pop(&self, index: usize) -> Option<usize> {
        self.entries[index].matching_pop
    }

    pub(super) fn disables_descendant_culling(&self, index: usize) -> bool {
        self.entries[index].disables_descendant_culling
    }
}

fn event_ink_bounds<N>(
    projection: &OutputProjection<'_, N>,
    event: PaintOrderEvent,
) -> PaintCullBounds
where
    N: Copy + Debug + Eq + Hash,
{
    match event {
        PaintOrderEvent::BoxOutsetShadow(id) => outset_shadow_bounds(projection, id),
        PaintOrderEvent::BoxBackground(id) => box_background_bounds(projection, id),
        PaintOrderEvent::TableCollapsedBorders(id) => table_border_bounds(projection, id),
        PaintOrderEvent::BoxContents(id) => box_contents_bounds(projection, id),
        PaintOrderEvent::BoxOutline(id) => box_outline_bounds(projection, id),
        PaintOrderEvent::PushStackingContext(_) | PaintOrderEvent::PopStackingContext(_) => {
            PaintCullBounds::Empty
        }
    }
}

fn box_background_bounds<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
) -> PaintCullBounds
where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if layout_box.inline_flattened || !layout_box.style.is_visible() {
        return PaintCullBounds::Empty;
    }
    let bounds = map_box_rect(projection, id, projection.boxes[id.index()].border_box);
    if matches!(bounds, PaintCullBounds::Empty) && layout_box.is_replaced() {
        // Replaced-content projection can publish a fallback diagnostic even
        // when its used box has no ink. Do not make diagnostics depend on the
        // capture cull rect.
        PaintCullBounds::Unbounded
    } else {
        bounds
    }
}

fn box_contents_bounds<N>(projection: &OutputProjection<'_, N>, id: LayoutBoxId) -> PaintCullBounds
where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if !layout_box.style.is_visible() {
        return PaintCullBounds::Empty;
    }
    let mut local = projection.boxes[id.index()].border_box;
    if let Some(context) = layout_box.inline_layout.as_ref() {
        let layout = layout_box.final_layout;
        let origin = LayoutPoint::new(
            layout.border.left + layout.padding.left,
            layout.border.top + layout.padding.top,
        );
        for line in &context.fragments.lines {
            match line.paint_bounds {
                InlinePaintBounds::Empty => {}
                InlinePaintBounds::Bounded(bounds) => {
                    local = local.union(offset_rect(bounds, origin));
                }
                InlinePaintBounds::Unbounded => return PaintCullBounds::Unbounded,
            }
        }
    }
    let bounds = map_box_rect(projection, id, local);
    if matches!(bounds, PaintCullBounds::Empty) && layout_box.is_replaced() {
        PaintCullBounds::Unbounded
    } else {
        bounds
    }
}

fn table_border_bounds<N>(projection: &OutputProjection<'_, N>, id: LayoutBoxId) -> PaintCullBounds
where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if !layout_box.style.is_visible() {
        return PaintCullBounds::Empty;
    }
    let Some(borders) = layout_box.collapsed_table_borders.as_ref() else {
        return PaintCullBounds::Empty;
    };
    let mut bounds = PaintCullBounds::Empty;
    for segment in borders.segments() {
        bounds.include(map_box_rect(projection, id, segment.rect));
    }
    bounds
}

fn outset_shadow_bounds<N>(projection: &OutputProjection<'_, N>, id: LayoutBoxId) -> PaintCullBounds
where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if layout_box.inline_flattened || !layout_box.style.is_visible() {
        return PaintCullBounds::Empty;
    }
    let geometry = &projection.boxes[id.index()];
    let paint_space = projection.coordinate_spaces[geometry.coordinate_space.index()].paint;
    let areas = BoxAreas::for_box(projection, id);
    let mut bounds = PaintCullBounds::Empty;
    for shadow in layout_box
        .style
        .box_shadows(
            paint_space.pre_transform_rect(areas.border_rect),
            areas.border_radii,
            paint_space.property_transform(),
        )
        .into_iter()
        .filter(|shadow| !shadow.inset && shadow.color.alpha > 0.0)
    {
        if !shadow.blur_radius.is_finite()
            || !shadow.spread_radius.is_finite()
            || !shadow.offset.x.is_finite()
            || !shadow.offset.y.is_finite()
        {
            return PaintCullBounds::Unbounded;
        }
        // Match the software rasterizer's deliberately conservative shadow
        // clip. Keeping the same four-sigma blur guard makes the culler unable
        // to discard any pixels the backend could still touch.
        let spread = shadow.spread_radius.max(0.0);
        let blur = shadow.blur_radius.max(0.0);
        let extent = spread + blur * 4.0 + shadow.offset.x.hypot(shadow.offset.y) + 1.0;
        let shifted = LayoutRect::new(
            shadow.rect.x + shadow.offset.x,
            shadow.rect.y + shadow.offset.y,
            shadow.rect.width,
            shadow.rect.height,
        );
        bounds.include(map_rect(
            shadow.transform,
            outset_rect(shifted, extent, extent, extent, extent),
        ));
    }
    bounds
}

fn box_outline_bounds<N>(projection: &OutputProjection<'_, N>, id: LayoutBoxId) -> PaintCullBounds
where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if layout_box.inline_flattened || !layout_box.style.is_visible() {
        return PaintCullBounds::Empty;
    }
    let geometry = &projection.boxes[id.index()];
    let paint_space = projection.coordinate_spaces[geometry.coordinate_space.index()].paint;
    let mut bounds = PaintCullBounds::Empty;
    let radii = layout_box
        .style
        .border_radii(geometry.border_box.width, geometry.border_box.height);
    if let Some(PaintFragment::Border {
        rect, transform, ..
    }) = layout_box.style.outline_fragment(
        paint_space.pre_transform_rect(geometry.border_box),
        radii,
        paint_space.property_transform(),
    ) {
        bounds.include(map_rect(transform, rect));
    }

    let extent = &projection.scroll_extents[id.index()];
    let scrollbar_transform = if id == projection.world.root {
        LayoutTransform2D::IDENTITY
    } else {
        paint_space.local_transform()
    };
    for rect in [
        extent.horizontal_scrollbar.map(|scrollbar| scrollbar.frame),
        extent.vertical_scrollbar.map(|scrollbar| scrollbar.frame),
        extent.scrollbar_corner,
    ]
    .into_iter()
    .flatten()
    {
        bounds.include(map_rect(scrollbar_transform, rect));
    }
    bounds
}

fn context_output_bounds<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    source_bounds: PaintCullBounds,
) -> (PaintCullBounds, bool)
where
    N: Copy + Debug + Eq + Hash,
{
    let filters = project_filters(&projection.world.boxes[id.index()].style);
    let moves_pixels = filters.effects.iter().any(filter_moves_pixels);
    let unsupported_transform = projection.resolved_transform_has_unsupported_3d(id);
    if unsupported_transform {
        return (PaintCullBounds::Unbounded, true);
    }
    let PaintCullBounds::Bounded(viewport_bounds) = source_bounds else {
        return (source_bounds, moves_pixels);
    };
    if !moves_pixels {
        return (source_bounds, false);
    }

    let geometry = &projection.boxes[id.index()];
    let transform = projection.coordinate_spaces[geometry.coordinate_space.index()]
        .paint
        .local_transform();
    let Some(inverse) = transform.inverse() else {
        return (PaintCullBounds::Unbounded, true);
    };
    let local_bounds = inverse.map_rect(viewport_bounds).bounding_rect();
    if !rect_is_finite(local_bounds) {
        return (PaintCullBounds::Unbounded, true);
    }
    (
        map_rect(
            transform,
            expanded_filter_clip(local_bounds, &filters.effects),
        ),
        true,
    )
}

fn filter_moves_pixels(filter: &PaintFilter) -> bool {
    matches!(
        filter,
        PaintFilter::Blur(_) | PaintFilter::DropShadow { .. }
    )
}

fn map_box_rect<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    rect: LayoutRect,
) -> PaintCullBounds
where
    N: Copy + Debug + Eq + Hash,
{
    let geometry = &projection.boxes[id.index()];
    let transform = projection.coordinate_spaces[geometry.coordinate_space.index()]
        .paint
        .local_transform();
    map_rect(transform, rect)
}

fn map_rect(transform: LayoutTransform2D, rect: LayoutRect) -> PaintCullBounds {
    if !transform.is_finite() || !rect_is_finite(rect) {
        return PaintCullBounds::Unbounded;
    }
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return PaintCullBounds::Empty;
    }
    let mapped = transform.map_rect(rect).bounding_rect();
    if !rect_is_finite(mapped) {
        PaintCullBounds::Unbounded
    } else if mapped.width <= 0.0 || mapped.height <= 0.0 {
        PaintCullBounds::Empty
    } else {
        PaintCullBounds::Bounded(mapped)
    }
}

pub(super) fn rects_intersect(left: LayoutRect, right: LayoutRect) -> bool {
    rect_is_finite(left)
        && rect_is_finite(right)
        && left.width >= 0.0
        && left.height >= 0.0
        && right.width >= 0.0
        && right.height >= 0.0
        && left.x <= right.right()
        && left.right() >= right.x
        && left.y <= right.bottom()
        && left.bottom() >= right.y
}

fn rect_is_finite(rect: LayoutRect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .into_iter()
        .all(f32::is_finite)
}

fn offset_rect(rect: LayoutRect, offset: LayoutPoint) -> LayoutRect {
    LayoutRect::new(
        rect.x + offset.x,
        rect.y + offset.y,
        rect.width,
        rect.height,
    )
}

fn outset_rect(rect: LayoutRect, top: f32, right: f32, bottom: f32, left: f32) -> LayoutRect {
    LayoutRect::new(
        rect.x - left,
        rect.y - top,
        (rect.width + left + right).max(0.0),
        (rect.height + top + bottom).max(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touching_and_partially_overlapping_rects_are_kept() {
        let capture = LayoutRect::new(0.0, 0.0, 100.0, 100.0);
        assert!(rects_intersect(
            capture,
            LayoutRect::new(100.0, 20.0, 10.0, 10.0)
        ));
        assert!(rects_intersect(
            capture,
            LayoutRect::new(99.0, 99.0, 10.0, 10.0)
        ));
        assert!(!rects_intersect(
            capture,
            LayoutRect::new(101.0, 20.0, 10.0, 10.0)
        ));
    }

    #[test]
    fn singular_or_non_finite_mappings_disable_culling() {
        assert_eq!(
            PaintCullRegion::Bounded(LayoutRect::new(0.0, 0.0, 10.0, 10.0))
                .local_rect(LayoutTransform2D::scale(0.0, 0.0)),
            None
        );
        assert_eq!(
            map_rect(
                LayoutTransform2D::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]),
                LayoutRect::new(0.0, 0.0, 10.0, 10.0),
            ),
            PaintCullBounds::Unbounded
        );
    }

    #[test]
    fn capture_cull_rect_keeps_an_antialias_guard_band() {
        assert_eq!(
            PaintCullRegion::for_capture(LayoutRect::new(10.0, 20.0, 30.0, 40.0)),
            PaintCullRegion::Bounded(LayoutRect::new(9.0, 19.0, 32.0, 42.0))
        );
    }
}
