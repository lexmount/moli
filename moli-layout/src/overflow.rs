use std::{collections::VecDeque, fmt::Debug, hash::Hash};

use crate::{
    LayoutBoxId, LayoutPoint, LayoutRect, LayoutScrollbarAxis, LayoutSize, LayoutTransform2D,
    LayoutViewport, LayoutWorld,
    style::{LayoutOverflowMode, ResolvedLayoutStyle, ResolvedLayoutTransform},
};

/// The physical edge from which an axis scrolls. Flex packing can reverse it
/// independently of the writing direction (including with wrap-reverse).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollStartEdge {
    Min,
    Max,
}

impl ScrollStartEdge {
    fn from_reversed(reversed: bool) -> Self {
        if reversed { Self::Max } else { Self::Min }
    }

    fn reachable_edges(self, min: f32, max: f32, port_min: f32, port_max: f32) -> (f32, f32) {
        match self {
            Self::Min => (min.max(port_min), max.max(port_min)),
            Self::Max => (min.min(port_max), max.min(port_max)),
        }
    }

    fn flow_margins(self, min: f32, max: f32, size: f32) -> (f32, f32) {
        // Negative margins can retract the scroll-end edge, but cannot remove
        // more than the fragment's size or retract the opposite edge.
        match self {
            Self::Min => (min.max(0.0), max.max(-size)),
            Self::Max => (min.max(-size), max.max(0.0)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ScrollOrigin {
    horizontal: ScrollStartEdge,
    vertical: ScrollStartEdge,
}

impl ScrollOrigin {
    fn for_style(style: &ResolvedLayoutStyle, is_viewport: bool) -> Self {
        let mode = style.writing_mode();
        let mut inline_reversed = mode.is_inline_flow_reversed(style.taffy.direction);
        let mut block_reversed = mode.is_block_flow_reversed();
        if !is_viewport && style.taffy.display == taffy::Display::Flex {
            let reverse = matches!(
                style.taffy.flex_direction,
                taffy::FlexDirection::RowReverse | taffy::FlexDirection::ColumnReverse
            );
            let wrap_reverse = style.taffy.flex_wrap == taffy::FlexWrap::WrapReverse;
            if matches!(
                style.taffy.flex_direction,
                taffy::FlexDirection::Column | taffy::FlexDirection::ColumnReverse
            ) {
                block_reversed ^= reverse;
                inline_reversed ^= wrap_reverse;
            } else {
                inline_reversed ^= reverse;
                block_reversed ^= wrap_reverse;
            }
        }
        let reversed = mode.to_physical(taffy::LogicalSize {
            inline_size: inline_reversed,
            block_size: block_reversed,
        });
        Self {
            horizontal: ScrollStartEdge::from_reversed(reversed.width),
            vertical: ScrollStartEdge::from_reversed(reversed.height),
        }
    }

    fn reachable_rect(self, rect: LayoutRect, port: LayoutRect) -> LayoutRect {
        let (left, right) =
            self.horizontal
                .reachable_edges(rect.x, rect.right(), port.x, port.right());
        let (top, bottom) =
            self.vertical
                .reachable_edges(rect.y, rect.bottom(), port.y, port.bottom());
        LayoutRect::new(left, top, right - left, bottom - top)
    }

    fn add_visual_overflow(
        self,
        overflow: &mut LayoutRect,
        contribution: LayoutRect,
        scrollport: Option<LayoutRect>,
    ) {
        let reachable =
            scrollport.map_or(contribution, |port| self.reachable_rect(contribution, port));
        // Clip before union: a rectangle unreachable in one axis must not
        // extend the other axis through the empty space back to the scrollport.
        if reachable.width > 0.0 && reachable.height > 0.0 {
            *overflow = overflow.union(reachable);
        }
    }

    fn flow_bounds(self, flow: taffy::InFlowLayout, size: taffy::Size<f32>) -> LayoutRect {
        let (left, right) =
            self.horizontal
                .flow_margins(flow.margin.left, flow.margin.right, size.width);
        let (top, bottom) =
            self.vertical
                .flow_margins(flow.margin.top, flow.margin.bottom, size.height);
        LayoutRect::new(
            flow.location.x - left,
            flow.location.y - top,
            size.width + left + right,
            size.height + top + bottom,
        )
    }
}

/// One derivation of content extent and signed scroll range, shared by
/// automatic scrollbar feedback and frozen CSSOM/paint projection.
pub(crate) struct ScrollDimensions {
    pub(crate) size: LayoutSize,
    pub(crate) minimum: LayoutPoint,
    pub(crate) maximum: LayoutPoint,
}

impl ScrollDimensions {
    fn from_rects(port: LayoutRect, overflow: LayoutRect) -> Self {
        let overflow = port.union(overflow);
        Self {
            size: LayoutSize::new(overflow.width, overflow.height),
            minimum: LayoutPoint::new(overflow.x - port.x, overflow.y - port.y),
            maximum: LayoutPoint::new(
                overflow.right() - port.right(),
                overflow.bottom() - port.bottom(),
            ),
        }
    }

    pub(crate) fn overflowing_axes(&self) -> (bool, bool) {
        (
            self.maximum.x - self.minimum.x > f32::EPSILON,
            self.maximum.y - self.minimum.y > f32::EPSILON,
        )
    }
}

/// Geometry needed to resolve scrollable overflow and no other projection
/// concern. Keeping this sidecar smaller than `OutputProjection` lets automatic
/// scrollbar feedback converge without allocating sources, fragments, clips,
/// paint order, diagnostics, or hit-test state on every numeric iteration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OverflowBoxGeometry {
    pub(crate) border_box: LayoutRect,
    pub(crate) padding_box: LayoutRect,
    pub(crate) content_box: LayoutRect,
    pub(crate) margin_box: LayoutRect,
    pub(crate) resolved_transform: ResolvedLayoutTransform,
    pub(crate) local_scrollport: LayoutRect,
    pub(crate) vertical_gutter: f32,
    pub(crate) vertical_leading_gutter: f32,
    pub(crate) horizontal_gutter: f32,
    pub(crate) horizontal_leading_gutter: f32,
    local_overflow: LayoutRect,
    inflow_bounds: Option<LayoutRect>,
    scroll_origin: ScrollOrigin,
}

/// One pass-local, incrementally refreshed scrollable-overflow projection.
///
/// The initial build touches each numeric box and overflow edge once. After a
/// scrollbar changes layout, callers provide the boxes Taffy actually touched;
/// a worklist expands only through their overflow ancestors. Unaffected local
/// geometry and descendant aggregates remain reusable across feedback turns.
pub(crate) struct OverflowProjection {
    geometries: Vec<OverflowBoxGeometry>,
    scrollable_overflow: Vec<LayoutRect>,
    parents: Vec<Option<LayoutBoxId>>,
    children: Vec<Vec<LayoutBoxId>>,
    affected: Vec<bool>,
    remaining_affected_children: Vec<usize>,
}

impl OverflowProjection {
    pub(crate) fn new<N>(world: &LayoutWorld<N>, viewport: LayoutViewport) -> Self
    where
        N: Copy + Debug + Eq + Hash,
    {
        let count = world.boxes.len();
        let geometries = (0..count)
            .map(|index| project_box(world, viewport, LayoutBoxId::from_index(index)))
            .collect::<Vec<_>>();
        let parents = (0..count)
            .map(|index| overflow_parent(world, LayoutBoxId::from_index(index)))
            .collect::<Vec<_>>();
        let mut children = vec![Vec::new(); count];
        for (index, parent) in parents.iter().copied().enumerate() {
            if let Some(parent) = parent {
                children[parent.index()].push(LayoutBoxId::from_index(index));
            }
        }
        let mut projection = Self {
            scrollable_overflow: geometries
                .iter()
                .map(|geometry| geometry.local_overflow)
                .collect(),
            geometries,
            parents,
            children,
            affected: vec![false; count],
            remaining_affected_children: vec![0; count],
        };
        projection.resolve_all(world);
        projection
    }

    pub(crate) fn len(&self) -> usize {
        self.geometries.len()
    }

    pub(crate) fn geometry(&self, id: LayoutBoxId) -> OverflowBoxGeometry {
        self.geometries[id.index()]
    }

    pub(crate) fn scrollable_overflow(&self, id: LayoutBoxId) -> LayoutRect {
        self.scrollable_overflow[id.index()]
    }

    pub(crate) fn scroll_dimensions(&self, id: LayoutBoxId) -> ScrollDimensions {
        let geometry = self.geometry(id);
        let reachable = geometry
            .scroll_origin
            .reachable_rect(self.scrollable_overflow(id), geometry.local_scrollport);
        ScrollDimensions::from_rects(geometry.local_scrollport, reachable)
    }

    pub(crate) fn overflowing_axes<N>(
        &self,
        world: &LayoutWorld<N>,
        id: LayoutBoxId,
    ) -> (bool, bool)
    where
        N: Copy + Debug + Eq + Hash,
    {
        if !establishes_scroll_container(world, id) {
            return (false, false);
        }
        self.scroll_dimensions(id).overflowing_axes()
    }

    /// Reprojects the boxes changed by numeric layout and their overflow
    /// ancestors. The returned identities are exactly the boxes whose current
    /// overflow state may need another automatic-scrollbar admission check.
    pub(crate) fn refresh<N>(
        &mut self,
        world: &LayoutWorld<N>,
        viewport: LayoutViewport,
        touched: &[LayoutBoxId],
    ) -> Vec<LayoutBoxId>
    where
        N: Copy + Debug + Eq + Hash,
    {
        assert_eq!(self.geometries.len(), world.boxes.len());
        let mut affected = Vec::new();
        for touched in touched.iter().copied() {
            let mut current = Some(touched);
            while let Some(id) = current {
                if self.affected[id.index()] {
                    break;
                }
                self.affected[id.index()] = true;
                affected.push(id);
                current = self.parents[id.index()];
            }
        }
        if affected.is_empty() {
            return affected;
        }

        for id in affected.iter().copied() {
            let geometry = project_box(world, viewport, id);
            self.geometries[id.index()] = geometry;
            self.scrollable_overflow[id.index()] = geometry.local_overflow;
            self.remaining_affected_children[id.index()] = self.children[id.index()]
                .iter()
                .filter(|child| self.affected[child.index()])
                .count();
        }

        let mut ready = affected
            .iter()
            .copied()
            .filter(|id| self.remaining_affected_children[id.index()] == 0)
            .collect::<VecDeque<_>>();
        let mut resolved = 0usize;
        while let Some(id) = ready.pop_front() {
            self.resolve_one(world, id);
            resolved = resolved.saturating_add(1);
            if let Some(parent) = self.parents[id.index()]
                && self.affected[parent.index()]
            {
                let remaining = &mut self.remaining_affected_children[parent.index()];
                *remaining = remaining
                    .checked_sub(1)
                    .expect("overflow worklist child count underflowed");
                if *remaining == 0 {
                    ready.push_back(parent);
                }
            }
        }
        assert_eq!(
            resolved,
            affected.len(),
            "overflow dependencies must form an acyclic forest"
        );
        for id in affected.iter().copied() {
            self.affected[id.index()] = false;
            self.remaining_affected_children[id.index()] = 0;
        }
        affected
    }

    fn resolve_all<N>(&mut self, world: &LayoutWorld<N>)
    where
        N: Copy + Debug + Eq + Hash,
    {
        let mut remaining_children = self.children.iter().map(Vec::len).collect::<Vec<_>>();
        let mut ready = remaining_children
            .iter()
            .enumerate()
            .filter_map(|(index, remaining)| {
                (*remaining == 0).then_some(LayoutBoxId::from_index(index))
            })
            .collect::<VecDeque<_>>();
        let mut resolved = 0usize;
        while let Some(id) = ready.pop_front() {
            self.resolve_one(world, id);
            resolved = resolved.saturating_add(1);
            if let Some(parent) = self.parents[id.index()] {
                let remaining = &mut remaining_children[parent.index()];
                *remaining = remaining
                    .checked_sub(1)
                    .expect("overflow child count underflowed");
                if *remaining == 0 {
                    ready.push_back(parent);
                }
            }
        }
        assert_eq!(
            resolved,
            self.geometries.len(),
            "overflow dependencies must form an acyclic forest"
        );
    }

    fn resolve_one<N>(&mut self, world: &LayoutWorld<N>, id: LayoutBoxId)
    where
        N: Copy + Debug + Eq + Hash,
    {
        let geometry = self.geometries[id.index()];
        let mut overflow = geometry.local_overflow;
        let is_scroller = establishes_scroll_container(world, id);
        let mut inflow_bounds = is_scroller.then_some(geometry.inflow_bounds).flatten();
        for child in self.children[id.index()].iter().copied() {
            geometry.scroll_origin.add_visual_overflow(
                &mut overflow,
                self.child_contribution(world, child),
                is_scroller.then_some(geometry.local_scrollport),
            );
            if is_scroller {
                let layout = world.boxes[child.index()].final_layout;
                if let Some(flow) = layout.in_flow {
                    let bounds = geometry.scroll_origin.flow_bounds(flow, layout.size);
                    inflow_bounds =
                        Some(inflow_bounds.map_or(bounds, |current| current.union(bounds)));
                }
            }
        }
        if let Some(bounds) = inflow_bounds {
            let padding = world.boxes[id.index()].final_layout.padding;
            let padded = outset_rect(
                bounds,
                padding.top,
                padding.right,
                padding.bottom,
                padding.left,
            );
            // Even zero-area in-flow fragments establish trailing padding.
            overflow = overflow.union(
                geometry
                    .scroll_origin
                    .reachable_rect(padded, geometry.local_scrollport),
            );
        }
        self.scrollable_overflow[id.index()] = overflow;
    }

    fn child_contribution<N>(&self, world: &LayoutWorld<N>, child: LayoutBoxId) -> LayoutRect
    where
        N: Copy + Debug + Eq + Hash,
    {
        let geometry = self.geometries[child.index()];
        let mut visual_overflow = self.scrollable_overflow[child.index()];
        let modes = overflow_modes(world, child);
        if modes[0] != LayoutOverflowMode::Visible {
            let right = visual_overflow.right().min(geometry.border_box.right());
            visual_overflow.x = visual_overflow.x.max(geometry.border_box.x);
            visual_overflow.width = (right - visual_overflow.x).max(0.0);
        }
        if modes[1] != LayoutOverflowMode::Visible {
            let bottom = visual_overflow.bottom().min(geometry.border_box.bottom());
            visual_overflow.y = visual_overflow.y.max(geometry.border_box.y);
            visual_overflow.height = (bottom - visual_overflow.y).max(0.0);
        }
        let visual_overflow = geometry.border_box.union(visual_overflow);
        let location = world.boxes[child.index()].final_layout.location;
        let layout_translation = LayoutTransform2D::translation(location.x, location.y);
        let local_to_parent = layout_translation.concatenate(geometry.resolved_transform.transform);
        local_to_parent.map_rect(visual_overflow).bounding_rect()
    }
}

fn overflow_parent<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> Option<LayoutBoxId>
where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &world.boxes[id.index()];
    // Only viewport-anchored fixed boxes are outside the document's scrolling
    // contents. A fixed child of a transformed containing block still belongs
    // to that block's overflow and scroll translation.
    if id == world.root
        || (layout_box.style.is_fixed_positioned()
            && layout_box.positioned_containing_block.is_none())
    {
        return None;
    }
    layout_box.layout_parent.or(Some(world.root))
}

fn project_box<N>(
    world: &LayoutWorld<N>,
    viewport: LayoutViewport,
    id: LayoutBoxId,
) -> OverflowBoxGeometry
where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &world.boxes[id.index()];
    let layout = layout_box.final_layout;
    let is_root = id == world.root;
    let border_box = LayoutRect::new(
        0.0,
        0.0,
        layout.size.width.max(0.0),
        layout.size.height.max(0.0),
    );
    let padding_box = inset_rect(
        border_box,
        layout.border.top,
        layout.border.right,
        layout.border.bottom,
        layout.border.left,
    );
    let vertical_gutter = scrollbar_gutter_thickness(world, id, LayoutScrollbarAxis::Vertical);
    let vertical_leading_gutter =
        scrollbar_leading_gutter_thickness(world, id, LayoutScrollbarAxis::Vertical);
    let horizontal_gutter = scrollbar_gutter_thickness(world, id, LayoutScrollbarAxis::Horizontal);
    let horizontal_leading_gutter =
        scrollbar_leading_gutter_thickness(world, id, LayoutScrollbarAxis::Horizontal);
    let mut content_box = inset_rect(
        padding_box,
        layout.padding.top,
        layout.padding.right,
        layout.padding.bottom,
        layout.padding.left,
    );
    if !is_root {
        content_box.width = (content_box.width - vertical_gutter).max(0.0);
        content_box.height = (content_box.height - horizontal_gutter).max(0.0);
        content_box.x += vertical_leading_gutter;
        content_box.y += horizontal_leading_gutter;
    }
    let margin_box = outset_rect(
        border_box,
        layout.margin.top,
        layout.margin.right,
        layout.margin.bottom,
        layout.margin.left,
    );
    let resolved_transform = layout_box
        .style
        .resolved_2d_transform(border_box.width, border_box.height);
    let local_scrollport = scrollport_for_box(
        is_root,
        viewport,
        padding_box,
        vertical_gutter,
        vertical_leading_gutter,
        horizontal_gutter,
        horizontal_leading_gutter,
    );
    let mut local_overflow = local_scrollport;
    let scroll_origin = ScrollOrigin::for_style(&layout_box.style, is_root);
    let clip_to_scrollport = establishes_scroll_container(world, id).then_some(local_scrollport);
    let mut inflow_bounds: Option<LayoutRect> = None;
    if is_root {
        local_overflow = local_overflow.union(padding_box);
    }
    if let Some(context) = layout_box.inline_layout.as_ref() {
        let origin = LayoutPoint::new(
            layout.border.left
                + layout.padding.left
                + if is_root {
                    0.0
                } else {
                    vertical_leading_gutter
                },
            layout.border.top
                + layout.padding.top
                + if is_root {
                    0.0
                } else {
                    horizontal_leading_gutter
                },
        );
        for line in &context.fragments.lines {
            let rect = offset_rect(line.rect, origin);
            if rect.width > 0.0 && rect.height > 0.0 {
                local_overflow = local_overflow.union(rect);
                inflow_bounds = Some(inflow_bounds.map_or(rect, |current| current.union(rect)));
            }
        }
        for fragment in &context.fragments.text {
            if fragment.kind == crate::inline::InlineTextFragmentKind::Content {
                scroll_origin.add_visual_overflow(
                    &mut local_overflow,
                    offset_rect(fragment.rect, origin),
                    clip_to_scrollport,
                );
            }
        }
        for fragment in &context.fragments.boxes {
            scroll_origin.add_visual_overflow(
                &mut local_overflow,
                offset_rect(fragment.box_model.border, origin),
                clip_to_scrollport,
            );
        }
    }
    OverflowBoxGeometry {
        border_box,
        padding_box,
        content_box,
        margin_box,
        resolved_transform,
        local_scrollport,
        vertical_gutter,
        vertical_leading_gutter,
        horizontal_gutter,
        horizontal_leading_gutter,
        local_overflow,
        inflow_bounds,
        scroll_origin,
    }
}

fn scrollbar_gutter_thickness<N>(
    world: &LayoutWorld<N>,
    id: LayoutBoxId,
    axis: LayoutScrollbarAxis,
) -> f32
where
    N: Copy + Debug + Eq + Hash,
{
    if id == world.root {
        world
            .viewport_scroll_policy
            .scrollbar_gutter_thickness(axis)
    } else if world.is_viewport_defining_body(id) {
        0.0
    } else {
        world.boxes[id.index()]
            .style
            .scrollbar_gutter_thickness(axis)
    }
}

fn scrollbar_leading_gutter_thickness<N>(
    world: &LayoutWorld<N>,
    id: LayoutBoxId,
    axis: LayoutScrollbarAxis,
) -> f32
where
    N: Copy + Debug + Eq + Hash,
{
    if id == world.root {
        world
            .viewport_scroll_policy
            .scrollbar_leading_gutter_thickness(axis)
    } else if world.is_viewport_defining_body(id) {
        0.0
    } else {
        world.boxes[id.index()]
            .style
            .scrollbar_leading_gutter_thickness(axis, false)
    }
}

fn establishes_scroll_container<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    if id == world.root {
        world.viewport_scroll_policy.establishes_scroll_container()
    } else if world.is_viewport_defining_body(id) {
        false
    } else {
        world.boxes[id.index()].style.establishes_scroll_container()
    }
}

fn overflow_modes<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> [LayoutOverflowMode; 2]
where
    N: Copy + Debug + Eq + Hash,
{
    if id == world.root {
        world.viewport_scroll_policy.effective_overflow
    } else if world.is_viewport_defining_body(id) {
        [LayoutOverflowMode::Visible; 2]
    } else {
        world.boxes[id.index()].style.overflow_modes()
    }
}

pub(crate) fn inset_rect(
    rect: LayoutRect,
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
) -> LayoutRect {
    let top = top.max(0.0);
    let right = right.max(0.0);
    let bottom = bottom.max(0.0);
    let left = left.max(0.0);
    LayoutRect::new(
        rect.x + left,
        rect.y + top,
        (rect.width - left - right).max(0.0),
        (rect.height - top - bottom).max(0.0),
    )
}

pub(crate) fn outset_rect(
    rect: LayoutRect,
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
) -> LayoutRect {
    LayoutRect::new(
        rect.x - left,
        rect.y - top,
        (rect.width + left + right).max(0.0),
        (rect.height + top + bottom).max(0.0),
    )
}

pub(crate) fn offset_rect(rect: LayoutRect, offset: LayoutPoint) -> LayoutRect {
    LayoutRect::new(
        rect.x + offset.x,
        rect.y + offset.y,
        rect.width,
        rect.height,
    )
}

fn scrollport_for_box(
    is_root: bool,
    viewport: LayoutViewport,
    padding_box: LayoutRect,
    vertical_gutter: f32,
    vertical_leading_gutter: f32,
    horizontal_gutter: f32,
    horizontal_leading_gutter: f32,
) -> LayoutRect {
    if is_root {
        return LayoutRect::new(
            0.0,
            0.0,
            (viewport.css_width as f32 - vertical_gutter).max(0.0),
            (viewport.css_height as f32 - horizontal_gutter).max(0.0),
        );
    }
    let mut scrollport = padding_box;
    scrollport.width = (scrollport.width - vertical_gutter).max(0.0);
    scrollport.height = (scrollport.height - horizontal_gutter).max(0.0);
    scrollport.x += vertical_leading_gutter;
    scrollport.y += horizontal_leading_gutter;
    scrollport
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_scroll_ranges_derive_from_the_port_and_overflow_rectangles() {
        let dimensions = ScrollDimensions::from_rects(
            LayoutRect::new(12.0, 8.0, 100.0, 200.0),
            LayoutRect::new(-8.0, 8.0, 120.0, 250.0),
        );
        assert_eq!(dimensions.size, LayoutSize::new(120.0, 250.0));
        assert_eq!(dimensions.minimum, LayoutPoint::new(-20.0, 0.0));
        assert_eq!(dimensions.maximum, LayoutPoint::new(0.0, 50.0));
        assert_eq!(dimensions.overflowing_axes(), (true, true));
    }

    #[test]
    fn unreachable_area_in_one_axis_does_not_create_scroll_in_the_other() {
        let port = LayoutRect::new(0.0, 0.0, 100.0, 100.0);
        let origin = ScrollOrigin {
            horizontal: ScrollStartEdge::Min,
            vertical: ScrollStartEdge::Max,
        };
        let unreachable = origin.reachable_rect(LayoutRect::new(200.0, 200.0, 100.0, 100.0), port);
        assert_eq!(unreachable, LayoutRect::new(200.0, 100.0, 100.0, 0.0));
        let mut overflow = port;
        origin.add_visual_overflow(
            &mut overflow,
            LayoutRect::new(200.0, 200.0, 100.0, 100.0),
            Some(port),
        );
        assert_eq!(overflow, port);
        // Non-scrolling ancestors must retain it for a later scroll container
        // to evaluate in that container's own coordinate system.
        origin.add_visual_overflow(
            &mut overflow,
            LayoutRect::new(200.0, 200.0, 100.0, 100.0),
            None,
        );
        assert_eq!(overflow, LayoutRect::new(0.0, 0.0, 300.0, 300.0));
    }

    #[test]
    fn negative_flow_margins_retract_only_the_scroll_end_and_preserve_empty_bounds() {
        let origin = ScrollOrigin {
            horizontal: ScrollStartEdge::Max,
            vertical: ScrollStartEdge::Min,
        };
        let bounds = origin.flow_bounds(
            taffy::InFlowLayout {
                location: taffy::Point { x: 100.0, y: 100.0 },
                margin: taffy::Rect {
                    left: -30.0,
                    right: -6.0,
                    top: -4.0,
                    bottom: -100.0,
                },
            },
            taffy::Size {
                width: 20.0,
                height: 10.0,
            },
        );
        assert_eq!(bounds, LayoutRect::new(120.0, 100.0, 0.0, 0.0));
    }
}
