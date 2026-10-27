use std::{collections::VecDeque, fmt::Debug, hash::Hash};

use crate::{
    LayoutBoxId, LayoutDisplay, LayoutPoint, LayoutRect, LayoutScrollbarAxis, LayoutSize,
    LayoutTransform2D, LayoutViewport, LayoutWorld,
    inline::InlineLineFragment,
    style::{LayoutOverflowMode, ResolvedLayoutTransform},
};

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

    pub(crate) fn scroll_size<N>(&self, world: &LayoutWorld<N>, id: LayoutBoxId) -> LayoutSize
    where
        N: Copy + Debug + Eq + Hash,
    {
        let scrollport = self.geometry(id).local_scrollport;
        let overflow = self.scrollable_overflow(id);
        let reversed = world.boxes[id.index()].style.reversed_scroll_axes();
        let width = if reversed.x {
            scrollport.right() - overflow.x
        } else {
            overflow.right() - scrollport.x
        };
        let height = if reversed.y {
            scrollport.bottom() - overflow.y
        } else {
            overflow.bottom() - scrollport.y
        };
        LayoutSize::new(scrollport.width.max(width), scrollport.height.max(height))
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
        let geometry = self.geometry(id);
        let size = self.scroll_size(world, id);
        let horizontal_range = size.width - geometry.local_scrollport.width;
        let vertical_range = size.height - geometry.local_scrollport.height;
        (
            horizontal_range > f32::EPSILON,
            vertical_range > f32::EPSILON,
        )
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
        let mut overflow = self.geometries[id.index()].local_overflow;
        for child in self.children[id.index()].iter().copied() {
            overflow = overflow.union(self.child_contribution(world, id, child));
        }
        self.scrollable_overflow[id.index()] = overflow;
    }

    fn child_contribution<N>(
        &self,
        world: &LayoutWorld<N>,
        parent: LayoutBoxId,
        child: LayoutBoxId,
    ) -> LayoutRect
    where
        N: Copy + Debug + Eq + Hash,
    {
        let geometry = self.geometries[child.index()];
        let mut visual_overflow = self.scrollable_overflow[child.index()];
        if clips_overflow(world, child) {
            let modes = world.boxes[child.index()].style.overflow_modes();
            // Clip/visible does not establish a scroll container. Clip just
            // that physical axis and keep overflow on the other axis. Native
            // controls and scrolling mechanisms still clip both axes.
            let independent_clip = !establishes_scroll_container(world, child)
                && modes.contains(&LayoutOverflowMode::Clip);
            if !independent_clip || modes[0] == LayoutOverflowMode::Clip {
                visual_overflow.x = geometry.border_box.x;
                visual_overflow.width = geometry.border_box.width;
            }
            if !independent_clip || modes[1] == LayoutOverflowMode::Clip {
                visual_overflow.y = geometry.border_box.y;
                visual_overflow.height = geometry.border_box.height;
            }
        }
        let location = world.boxes[child.index()].final_layout.location;
        let layout_translation = LayoutTransform2D::translation(location.x, location.y);
        let local_to_parent = layout_translation.concatenate(geometry.resolved_transform.transform);
        let parent_box = &world.boxes[parent.index()];
        let relative =
            if world.boxes[child.index()].style.position() == crate::LayoutPosition::Relative {
                crate::inline::relative_atomic_inset_offset(
                    &world.boxes[child.index()].style.taffy,
                    taffy::Size {
                        width: self.geometries[parent.index()].content_box.width,
                        height: self.geometries[parent.index()].content_box.height,
                    },
                    parent_box.style.direction(),
                )
            } else {
                taffy::Point::ZERO
            };
        let normal_flow_translation =
            LayoutTransform2D::translation(location.x - relative.x, location.y - relative.y);
        let parent_is_grid = matches!(
            parent_box.style.display(),
            LayoutDisplay::Grid | LayoutDisplay::InlineGrid
        );
        let margin_to_parent =
            if parent_is_grid && !world.boxes[child.index()].style.is_out_of_flow() {
                // A grid's untransformed floor is its grid area, not the original
                // border box of an item overflowing its track. Keep the item's
                // margins in the same transformed physical space as its border.
                local_to_parent
            } else {
                normal_flow_translation
            };
        let mut contribution = local_to_parent
            .map_rect(visual_overflow.union(geometry.border_box))
            .bounding_rect()
            .union(
                margin_to_parent
                    .map_rect(geometry.margin_box)
                    .bounding_rect(),
            );
        if establishes_scroll_container(world, parent)
            && !world.boxes[child.index()].style.is_out_of_flow()
            && !parent_is_grid
        {
            // End padding extends the direct in-flow margin box, not its
            // escaping descendants or transformed visual bounds. A grid
            // item's overflow does not grow its containing grid area.
            let padding = parent_box.final_layout.padding;
            let padded = extend_end_padding(
                geometry.margin_box,
                padding,
                parent_box.style.reversed_scroll_axes(),
            );
            contribution =
                contribution.union(normal_flow_translation.map_rect(padded).bounding_rect());
        }
        contribution
    }
}

fn overflow_parent<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> Option<LayoutBoxId>
where
    N: Copy + Debug + Eq + Hash,
{
    if id == world.root || world.boxes[id.index()].style.is_fixed_positioned() {
        return None;
    }
    world.boxes[id.index()].layout_parent.or(Some(world.root))
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
    // Taffy's scrollable_overflow_rect is relative to a logical scroll origin,
    // not the physical top-left. It also already aggregates descendants.
    // Reusing its right/bottom here both mirrors mixed-direction overflow and
    // double counts it when resolve_one adds the physical descendants below.
    // Start with this box's scrollport and own inline fragments; aggregate
    // descendant bounds exactly once in the physical coordinate system.
    let mut local_overflow = local_scrollport;
    if is_root {
        local_overflow = local_overflow.union(padding_box);
    }
    if let Some(taffy::DetailedLayoutInfo::Grid(grid)) = world.native_grid_details.get(&id) {
        // Track positions already include physical border/padding/gaps and
        // alignment. Empty or undersized items must not erase a large grid
        // area, and overflowing items must not enlarge that untransformed area.
        let reversed = layout_box.style.reversed_scroll_axes();
        let padding = if establishes_scroll_container(world, id) {
            layout.padding
        } else {
            taffy::Rect::ZERO
        };
        let (left, right) = grid_track_overflow_axis(
            &grid.columns.positions,
            local_overflow.x,
            local_overflow.right(),
            if reversed.x {
                padding.left
            } else {
                padding.right
            },
            reversed.x,
        );
        let (top, bottom) = grid_track_overflow_axis(
            &grid.rows.positions,
            local_overflow.y,
            local_overflow.bottom(),
            if reversed.y {
                padding.top
            } else {
                padding.bottom
            },
            reversed.y,
        );
        local_overflow = LayoutRect::new(left, top, right - left, bottom - top);
    }
    if let Some(context) = world.inline_layouts[id.index()].as_ref() {
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
        let inline_bounds = |rect| {
            let rect = offset_rect(rect, origin);
            if establishes_scroll_container(world, id) {
                extend_end_padding(
                    rect,
                    layout.padding,
                    layout_box.style.reversed_scroll_axes(),
                )
            } else {
                rect
            }
        };
        let rtl = context
            .laid_out
            .as_ref()
            .is_some_and(|layout| layout.is_rtl());
        for line in &context.fragments.lines {
            local_overflow = local_overflow.union(inline_bounds(line.scrollable_rect(rtl)));
        }
        for fragment in &context.fragments.text {
            let line = &context.fragments.lines[fragment.line_index];
            let rect = inline_scrollable_rect(fragment.rect, line, rtl);
            local_overflow = local_overflow.union(inline_bounds(rect));
        }
        // These are structural inline boxes. Atomic boxes and their descendants
        // contribute their own overflow through child_contribution instead.
        for fragment in &context.fragments.boxes {
            let line = &context.fragments.lines[fragment.line_index];
            let rect = inline_scrollable_rect(fragment.rect, line, rtl);
            local_overflow = local_overflow.union(inline_bounds(rect));
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
    }
}

fn inline_scrollable_rect(
    mut rect: LayoutRect,
    line: &InlineLineFragment,
    rtl: bool,
) -> LayoutRect {
    let scrollable = line.scrollable_rect(rtl);
    if scrollable != line.rect {
        let left = rect.x.max(scrollable.x);
        let right = rect.right().min(scrollable.right());
        rect.x = left.min(right);
        rect.width = (right - left).max(0.0);
    }
    rect
}

fn grid_track_overflow_axis(
    tracks: &[taffy::Line<f32>],
    start: f32,
    end: f32,
    end_padding: f32,
    reversed: bool,
) -> (f32, f32) {
    tracks.iter().fold((start, end), |(start, end), track| {
        (
            start.min(track.start - if reversed { end_padding } else { 0.0 }),
            end.max(track.end + if reversed { 0.0 } else { end_padding }),
        )
    })
}

fn extend_end_padding(
    mut rect: LayoutRect,
    padding: taffy::Rect<f32>,
    reversed: taffy::Point<bool>,
) -> LayoutRect {
    let x = if reversed.x {
        padding.left
    } else {
        padding.right
    };
    let y = if reversed.y {
        padding.top
    } else {
        padding.bottom
    };
    if reversed.x {
        rect.x -= x;
    }
    if reversed.y {
        rect.y -= y;
    }
    rect.width += x;
    rect.height += y;
    rect
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

fn clips_overflow<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    if id == world.root {
        world.viewport_scroll_policy.clips_overflow()
    } else if world.is_viewport_defining_body(id) {
        false
    } else {
        world.boxes[id.index()].style.clips_overflow()
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
