use std::{collections::HashMap, fmt::Debug, hash::Hash, mem::size_of, num::NonZeroU32};

use crate::{
    FrozenLayoutBox, LayoutBoxGeometry, LayoutError, LayoutOutputBoxId, LayoutPoint,
    LayoutScrollExtent, LayoutScrollbarGeometry, LayoutWorld, PaintPixelGrid,
    style::ResolvedLayoutTransform,
};

use super::{
    coordinates::{BoxPlacement, ResolvedCoordinates, resolve_coordinate_spaces},
    sticky::StickyConstraints,
};

/// Stable inputs for reprojecting a single frozen Document. Local box and
/// fragment geometry, clip chains and scroll ranges stay in the frozen tree.
pub(crate) struct ScrollProjection<N> {
    root: LayoutOutputBoxId,
    placements: Vec<BoxPlacement>,
    layout_origins: Vec<LayoutPoint>,
    sticky: HashMap<usize, StickyConstraints>,
    scroll_sources: Vec<(LayoutOutputBoxId, N)>,
    local: Option<LocalScrollSpaces>,
}

/// A local scroll moves a whole coordinate branch by the same viewport
/// translation. Box matrices stay at their published offsets.
struct LocalScrollSpaces {
    box_spaces: Vec<Option<NonZeroU32>>,
    spaces: Vec<LocalScrollSpace>,
    has_translation: bool,
}

struct LocalScrollSpace {
    box_id: LayoutOutputBoxId,
    parent: Option<NonZeroU32>,
    published_offset: LayoutPoint,
    translation: ScrollTranslation,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ScrollTranslation {
    pub(crate) visual: [f64; 2],
    pub(crate) untransformed: [f64; 2],
}

impl LocalScrollSpaces {
    fn new(box_count: usize) -> Self {
        Self {
            box_spaces: vec![None; box_count],
            spaces: Vec::new(),
            has_translation: false,
        }
    }

    fn resolve_box(
        &mut self,
        index: usize,
        parent: Option<LayoutOutputBoxId>,
        own_offset: Option<LayoutPoint>,
    ) {
        // Coordinate resolution visits the parent before any of its children.
        let parent = parent.and_then(|id| self.box_spaces[id.index()]);
        self.box_spaces[index] = if let Some(published_offset) = own_offset {
            self.spaces.push(LocalScrollSpace {
                box_id: LayoutOutputBoxId::from_index(index),
                parent,
                published_offset,
                translation: ScrollTranslation::default(),
            });
            let space = u32::try_from(self.spaces.len()).expect("scroll space fits box ID");
            Some(NonZeroU32::new(space).expect("one-based scroll space"))
        } else {
            parent
        };
    }

    fn finish(mut self) -> Self {
        // A scroll container translates its descendants. Its own coordinates
        // use the space inherited from its parent.
        for space in &self.spaces {
            self.box_spaces[space.box_id.index()] = space.parent;
        }
        self
    }

    fn refresh<N>(
        &mut self,
        boxes: &[FrozenLayoutBox<N>],
        offsets: &HashMap<LayoutOutputBoxId, LayoutPoint>,
    ) {
        self.has_translation = false;
        for index in 0..self.spaces.len() {
            let space = &self.spaces[index];
            let mut translation = space.parent.map_or_else(ScrollTranslation::default, |id| {
                self.spaces[id.get() as usize - 1].translation
            });
            let layout_box = &boxes[space.box_id.index()];
            let offset = offsets
                .get(&space.box_id)
                .copied()
                .unwrap_or(layout_box.scroll_extent.applied_offset);
            let dx = f64::from(space.published_offset.x) - f64::from(offset.x);
            let dy = f64::from(space.published_offset.y) - f64::from(offset.y);
            if dx != 0.0 || dy != 0.0 {
                self.has_translation = true;
                let [a, b, c, d, _, _] = layout_box.coordinate_space.local_to_viewport.coefficients;
                translation.visual[0] += a * dx + c * dy;
                translation.visual[1] += b * dx + d * dy;
                translation.untransformed[0] += dx;
                translation.untransformed[1] += dy;
            }
            self.spaces[index].translation = translation;
        }
    }

    fn reset<N>(
        &mut self,
        boxes: &[FrozenLayoutBox<N>],
        offsets: &HashMap<LayoutOutputBoxId, LayoutPoint>,
    ) {
        self.has_translation = false;
        for space in &mut self.spaces {
            space.published_offset = offsets
                .get(&space.box_id)
                .copied()
                .unwrap_or(boxes[space.box_id.index()].scroll_extent.applied_offset);
            space.translation = ScrollTranslation::default();
        }
    }
}

impl<N: Copy + Debug + Eq + Hash> ScrollProjection<N> {
    pub(super) fn new(
        world: &LayoutWorld<N>,
        boxes: &[LayoutBoxGeometry],
        extents: &[LayoutScrollExtent],
        placement: impl Fn(usize) -> Result<(BoxPlacement, Option<StickyConstraints>), LayoutError>,
    ) -> Result<Self, LayoutError> {
        let mut projection = Self {
            root: world.output_id(world.root),
            placements: Vec::with_capacity(boxes.len()),
            layout_origins: Vec::new(),
            sticky: HashMap::new(),
            scroll_sources: Vec::new(),
            local: None,
        };
        for index in 0..boxes.len() {
            let (base, constraints) = placement(index)?;
            projection.placements.push(base);
            if let Some(constraints) = constraints {
                projection.sticky.insert(index, constraints);
            }
            if extents[index].is_scroll_container
                && let Some(source) = world.boxes[index].source()
            {
                projection
                    .scroll_sources
                    .push((LayoutOutputBoxId::from_index(index), source));
            }
        }
        if !projection.sticky.is_empty() {
            projection.layout_origins = boxes
                .iter()
                .map(|geometry| geometry.layout_origin_in_document)
                .collect();
        }
        Ok(projection)
    }

    pub(super) fn resolve_initial(
        &mut self,
        world: &LayoutWorld<N>,
        viewport_scroll: LayoutPoint,
        extents: &[LayoutScrollExtent],
        paint_grid: Option<PaintPixelGrid>,
    ) -> Result<ResolvedCoordinates, LayoutError> {
        let mut local = (self.sticky.is_empty()
            && self.scroll_sources.iter().any(|&(id, _)| id != self.root))
        .then(|| LocalScrollSpaces::new(self.placements.len()));
        let coordinates = resolve_coordinate_spaces(
            self.root,
            viewport_scroll,
            self.placements.len(),
            paint_grid,
            |index| Ok(self.placement(index)),
            |id| {
                let extent = &extents[id.index()];
                extent.is_scroll_container.then_some(extent.applied_offset)
            },
            |index, parent| {
                if let Some(local) = &mut local {
                    let extent = &extents[index];
                    let owns_space = index != self.root.index()
                        && extent.is_scroll_container
                        && world.boxes[index].source().is_some();
                    local.resolve_box(index, parent, owns_space.then_some(extent.applied_offset));
                }
            },
        )?;
        self.local = local.map(LocalScrollSpaces::finish);
        Ok(coordinates)
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
            + self.scroll_sources.capacity() * size_of::<(LayoutOutputBoxId, N)>()
            + self.local.as_ref().map_or(0, |local| {
                local.box_spaces.capacity() * size_of::<Option<NonZeroU32>>()
                    + local.spaces.capacity() * size_of::<LocalScrollSpace>()
            })
    }

    #[inline]
    pub(crate) fn local_translation(&self, box_index: usize) -> Option<&ScrollTranslation> {
        let local = self.local.as_ref()?;
        if !local.has_translation {
            return None;
        }
        let id = local.box_spaces[box_index]?;
        Some(&local.spaces[id.get() as usize - 1].translation)
    }

    pub(crate) fn refresh(
        &mut self,
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
        if let Some(local) = &mut self.local
            && *viewport_scroll == LayoutPoint::ZERO
            && new_viewport_scroll == LayoutPoint::ZERO
        {
            local.refresh(boxes, &offsets);
        } else {
            let coordinates = resolve_coordinate_spaces(
                self.root,
                new_viewport_scroll,
                boxes.len(),
                None,
                |index| Ok(self.placement(index)),
                |id| {
                    let extent = &boxes[id.index()].scroll_extent;
                    extent
                        .is_scroll_container
                        .then(|| offsets.get(&id).copied().unwrap_or(extent.applied_offset))
                },
                |_, _| {},
            )?;
            for (index, ((layout_box, coordinate), adjustment)) in boxes
                .iter_mut()
                .zip(coordinates.spaces.into_iter().skip(1))
                .zip(coordinates.position_adjustments)
                .enumerate()
            {
                layout_box.coordinate_space = coordinate.layout.into();
                if let Some(origin) = self.layout_origins.get(index) {
                    layout_box.geometry.layout_origin_in_document =
                        LayoutPoint::new(origin.x + adjustment.x, origin.y + adjustment.y);
                }
            }
            if let Some(local) = &mut self.local {
                local.reset(boxes, &offsets);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_local_scroll_spaces_follow_parents_with_later_indices() {
        let id = LayoutOutputBoxId::from_index;
        // The target is in the inner scroller. Neither scroller appears before
        // its descendants in storage.
        let parents = [None, Some(id(2)), Some(id(3)), Some(id(0))];
        let mut local = LocalScrollSpaces::new(parents.len());
        resolve_coordinate_spaces(
            id(0),
            LayoutPoint::ZERO,
            parents.len(),
            None,
            |index| {
                Ok((
                    BoxPlacement {
                        parent: parents[index],
                        location: LayoutPoint::ZERO,
                        transform: ResolvedLayoutTransform::IDENTITY,
                        fixed: false,
                    },
                    None,
                ))
            },
            |_| None,
            |index, parent| {
                let offset = (index >= 2).then_some(LayoutPoint::ZERO);
                local.resolve_box(index, parent, offset);
            },
        )
        .unwrap();
        let local = local.finish();
        assert_eq!(local.spaces[0].box_id, id(3));
        assert_eq!(local.spaces[1].box_id, id(2));
        assert_eq!(local.spaces[1].parent, NonZeroU32::new(1));
        assert_eq!(
            local.box_spaces,
            vec![None, NonZeroU32::new(2), NonZeroU32::new(1), None]
        );
    }
}
