use std::{collections::HashMap, fmt::Debug, hash::Hash, mem::size_of, num::NonZeroU32};

use crate::{
    FrozenLayoutBox, LayoutBoxGeometry, LayoutError, LayoutOutputBoxId, LayoutPoint,
    LayoutScrollExtent, LayoutScrollbarGeometry, LayoutWorld, style::ResolvedLayoutTransform,
};

use super::{
    coordinates::{BoxPlacement, resolve_coordinate_spaces},
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
    fn new<N>(
        root: LayoutOutputBoxId,
        placements: &[BoxPlacement],
        extents: &[LayoutScrollExtent],
        scroll_sources: &[(LayoutOutputBoxId, N)],
    ) -> Option<Self> {
        let mut sources = scroll_sources
            .iter()
            .map(|&(id, _)| id)
            .filter(|&id| id != root)
            .peekable();
        sources.peek()?;
        let mut child_spaces = Vec::with_capacity(placements.len());
        let mut spaces = Vec::new();
        for (index, placement) in placements.iter().enumerate() {
            let parent = placement.parent.and_then(|id| child_spaces[id.index()]);
            let own = sources
                .next_if(|id| id.index() == index)
                .map(|box_id| {
                    spaces.push(LocalScrollSpace {
                        box_id,
                        parent,
                        published_offset: extents[index].applied_offset,
                        translation: ScrollTranslation::default(),
                    });
                    let index = u32::try_from(spaces.len()).expect("scroll space fits box ID");
                    NonZeroU32::new(index).expect("one-based scroll space")
                })
                .or(parent);
            child_spaces.push(own);
        }
        // For a box's own coordinates use its parent's space. Descending
        // order keeps each parent's child space available until its own turn.
        for (index, placement) in placements.iter().enumerate().rev() {
            child_spaces[index] = placement.parent.and_then(|id| child_spaces[id.index()]);
        }
        Some(Self {
            box_spaces: child_spaces,
            spaces,
            has_translation: false,
        })
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
        let mut parents_precede_children = true;
        for index in 0..boxes.len() {
            let (base, constraints) = placement(index)?;
            parents_precede_children &= base.parent.is_none_or(|parent| parent.index() < index);
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
        if parents_precede_children && projection.sticky.is_empty() {
            projection.local = LocalScrollSpaces::new(
                projection.root,
                &projection.placements,
                extents,
                &projection.scroll_sources,
            );
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
