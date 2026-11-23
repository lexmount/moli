use std::{collections::HashMap, fmt::Debug, hash::Hash, mem::size_of};

use crate::{
    FrozenLayoutBox, LayoutBoxGeometry, LayoutBoxId, LayoutError, LayoutPoint, LayoutScrollExtent,
    LayoutScrollbarGeometry, LayoutWorld, style::ResolvedLayoutTransform,
};

use super::{
    coordinates::{BoxPlacement, resolve_coordinate_spaces},
    sticky::StickyConstraints,
};

/// Stable inputs for reprojecting a single frozen Document. Local box and
/// fragment geometry, clip chains and scroll ranges stay in the frozen tree.
pub(crate) struct ScrollProjection<N> {
    root: LayoutBoxId,
    placements: Vec<BoxPlacement>,
    layout_origins: Vec<LayoutPoint>,
    sticky: HashMap<usize, StickyConstraints>,
    scroll_sources: Vec<(LayoutBoxId, N)>,
}

impl<N: Copy + Debug + Eq + Hash> ScrollProjection<N> {
    pub(super) fn new(
        world: &LayoutWorld<N>,
        boxes: &[LayoutBoxGeometry],
        extents: &[LayoutScrollExtent],
        placement: impl Fn(usize) -> Result<(BoxPlacement, Option<StickyConstraints>), LayoutError>,
    ) -> Result<Self, LayoutError> {
        let mut projection = Self {
            root: world.root,
            placements: Vec::with_capacity(boxes.len()),
            layout_origins: Vec::with_capacity(boxes.len()),
            sticky: HashMap::new(),
            scroll_sources: Vec::new(),
        };
        for (index, geometry) in boxes.iter().enumerate() {
            let (base, constraints) = placement(index)?;
            projection.placements.push(base);
            projection
                .layout_origins
                .push(geometry.layout_origin_in_document);
            if let Some(constraints) = constraints {
                projection.sticky.insert(index, constraints);
            }
            if extents[index].is_scroll_container
                && let Some(source) = world.boxes[index].source()
            {
                projection
                    .scroll_sources
                    .push((LayoutBoxId::from_index(index), source));
            }
        }
        Ok(projection)
    }

    pub(super) fn placement(&self, index: usize) -> (BoxPlacement, Option<StickyConstraints>) {
        (self.placements[index], self.sticky.get(&index).copied())
    }

    pub(super) fn resolved_transform(&self, index: usize) -> ResolvedLayoutTransform {
        self.placements[index].transform
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.placements.capacity() * size_of::<BoxPlacement>()
            + self.layout_origins.capacity() * size_of::<LayoutPoint>()
            + self.sticky.capacity() * size_of::<(usize, StickyConstraints)>()
            + self.scroll_sources.capacity() * size_of::<(LayoutBoxId, N)>()
    }

    pub(crate) fn refresh(
        &self,
        boxes: &mut [FrozenLayoutBox<N>],
        viewport_scroll: &mut LayoutPoint,
        scroll_offset: &impl Fn(N) -> LayoutPoint,
    ) -> Result<bool, LayoutError> {
        let mut offsets = HashMap::new();
        for &(id, source) in &self.scroll_sources {
            let layout_box = &boxes[id.index()];
            let extent = &layout_box.scroll_extent;
            let css_offset = scroll_offset(source);
            let zoom = layout_box.effective_zoom;
            let requested = LayoutPoint::new(css_offset.x * zoom, css_offset.y * zoom);
            let applied =
                clamp_scroll_offset(requested, extent.minimum_offset, extent.maximum_offset);
            if applied != extent.applied_offset {
                offsets.insert(id, applied);
            }
        }
        if offsets.is_empty() {
            return Ok(false);
        }
        let new_viewport_scroll = offsets.get(&self.root).copied().unwrap_or(*viewport_scroll);
        let coordinates = resolve_coordinate_spaces(
            self.root,
            new_viewport_scroll,
            boxes.len(),
            false,
            |index| Ok(self.placement(index)),
            |id| {
                let extent = &boxes[id.index()].scroll_extent;
                extent
                    .is_scroll_container
                    .then(|| offsets.get(&id).copied().unwrap_or(extent.applied_offset))
            },
        )?;
        for (index, ((layout_box, coordinate), adjustment)) in boxes
            .iter_mut()
            .zip(coordinates.spaces.into_iter().skip(1))
            .zip(coordinates.position_adjustments)
            .enumerate()
        {
            layout_box.coordinate_space = coordinate.layout.into();
            let origin = self.layout_origins[index];
            layout_box.geometry.layout_origin_in_document =
                LayoutPoint::new(origin.x + adjustment.x, origin.y + adjustment.y);
        }
        for (id, offset) in offsets {
            apply_scroll_offset(&mut boxes[id.index()].scroll_extent, offset);
        }
        *viewport_scroll = new_viewport_scroll;
        Ok(true)
    }
}

pub(super) fn clamp_scroll_offset(
    requested: LayoutPoint,
    minimum: LayoutPoint,
    maximum: LayoutPoint,
) -> LayoutPoint {
    let clamp = |value: f32, min, max| {
        let finite = if value.is_finite() { value } else { 0.0 };
        finite.clamp(min, max)
    };
    LayoutPoint::new(
        clamp(requested.x, minimum.x, maximum.x),
        clamp(requested.y, minimum.y, maximum.y),
    )
}

fn apply_scroll_offset(extent: &mut LayoutScrollExtent, offset: LayoutPoint) {
    extent.applied_offset = offset;
    let update = |bar: &mut LayoutScrollbarGeometry, value, visible, total| {
        *bar = LayoutScrollbarGeometry::new(
            bar.axis,
            bar.frame,
            bar.minimum_offset,
            bar.maximum_offset,
            value,
            visible,
            total,
        );
    };
    if let Some(bar) = &mut extent.horizontal_scrollbar {
        update(
            bar,
            offset.x,
            extent.scrollport.width,
            extent.scroll_size.width,
        );
    }
    if let Some(bar) = &mut extent.vertical_scrollbar {
        update(
            bar,
            offset.y,
            extent.scrollport.height,
            extent.scroll_size.height,
        );
    }
}
