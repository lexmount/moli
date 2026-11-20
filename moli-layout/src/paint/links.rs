//! Print-only projection of flat-tree links onto the existing box/line fragments.
use std::{collections::HashMap, fmt::Debug, hash::Hash};

use crate::{
    LayoutFragmentId, LayoutFragmentKind, LayoutPoint, LayoutSource, PaintLink, PaintLinkTarget,
    PaintSnapshot, projection::OutputProjection,
};

pub(super) struct LinkProjection {
    targets: Vec<Option<PaintLinkTarget>>,
}

impl LinkProjection {
    pub(super) fn new<N, S>(projection: &OutputProjection<'_, N>, source: &S) -> Self
    where
        N: Copy + Debug + Eq + Hash,
        S: LayoutSource<NodeId = N>,
    {
        let mut cache: HashMap<N, Option<PaintLinkTarget>> = HashMap::new();
        let targets = projection
            .world
            .boxes
            .iter()
            .map(|layout_box| {
                let mut current = layout_box.source.or(layout_box.owner);
                let mut ancestors = Vec::new();
                let mut target = None;
                while let Some(node) = current {
                    if let Some(cached) = cache.get(&node) {
                        target = cached.clone();
                        break;
                    }
                    ancestors.push(node);
                    if let Some(link) = source.hyperlink(node) {
                        let destination = link.destination.and_then(|node| {
                            let id = projection.world.source_box(node).or_else(|| {
                                projection
                                    .world
                                    .display_contents_mapping
                                    .get(&node)?
                                    .first()
                                    .copied()
                            })?;
                            let fragment = projection.boxes[id.index()].fragments.first()?;
                            let fragment = &projection.fragments[fragment.index()];
                            let matrix = projection.coordinate_spaces
                                [fragment.coordinate_space.index()]
                            .paint
                            .local_transform();
                            Some(
                                matrix
                                    .map_point(LayoutPoint::new(fragment.rect.x, fragment.rect.y)),
                            )
                        });
                        target = Some(PaintLinkTarget {
                            url: link.url,
                            destination,
                        });
                        break;
                    }
                    current = source.flat_parent(node);
                }
                for node in ancestors {
                    cache.insert(node, target.clone());
                }
                target
            })
            .collect();
        Self { targets }
    }

    pub(super) fn append<N>(
        &self,
        projection: &OutputProjection<'_, N>,
        fragment: LayoutFragmentId,
        snapshot: &mut PaintSnapshot,
    ) where
        N: Copy + Debug + Eq + Hash,
    {
        let fragment = &projection.fragments[fragment.index()];
        let box_id = match fragment.kind {
            LayoutFragmentKind::Box { box_id }
            | LayoutFragmentKind::InlineBox { box_id, .. }
            | LayoutFragmentKind::Text { box_id, .. } => box_id,
            LayoutFragmentKind::Line { .. } => return,
        };
        if !fragment.role.has_display_geometry()
            || !projection.world.boxes[box_id.index()].style.is_visible()
            || fragment.rect.width <= 0.0
            || fragment.rect.height <= 0.0
        {
            return;
        }
        let Some(mut target) = self.targets[box_id.index()].clone() else {
            return;
        };
        target.destination = target
            .destination
            .map(|p| snapshot.viewport_to_surface.map_point(p));
        let matrix = projection.coordinate_spaces[fragment.coordinate_space.index()]
            .paint
            .local_transform();
        snapshot.links.push(PaintLink {
            before_fragment: snapshot.fragments.len(),
            rect: fragment.rect,
            transform: snapshot.viewport_to_surface.concatenate(matrix),
            target,
        });
    }
}
