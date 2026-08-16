//! Scroll-into-view and intersection geometry derived from the retained tree.

use std::{collections::HashSet, fmt::Debug, hash::Hash};

use super::{
    model::{
        LayoutClipChainId, LayoutFragmentId, LayoutFragmentKind, LayoutOutputBoxId, LayoutQuad,
        LayoutRect, LayoutTransform2D,
    },
    query::{
        LayoutIntersectionGeometry, LayoutScrollContainerKind, LayoutScrollContainerMetrics,
        LayoutScrollIntoViewGeometry,
    },
    tree::FrozenLayoutTree,
};

impl<N> FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub fn scroll_into_view_geometry_for_source(
        &self,
        source: N,
    ) -> Option<LayoutScrollIntoViewGeometry<N>> {
        let output = self.source_output(source)?;
        let (target_box, target_rects) = if let Some(box_id) = output.principal_box {
            let mut rects = self.client_rects_for_source(source);
            if rects.is_empty() {
                rects = self.content_quads_for_source(source);
            }
            (box_id, rects)
        } else if let Some(box_id) = output
            .fragments
            .iter()
            .find_map(|fragment| self.fragment_box(*fragment))
        {
            (box_id, self.content_quads_for_source(source))
        } else {
            output.scroll_proxy_boxes.iter().find_map(|box_id| {
                let rects = self.scroll_target_rects_for_box(*box_id);
                (!rects.is_empty()).then_some((*box_id, rects))
            })?
        };
        if target_rects.is_empty() {
            return None;
        }
        let mut candidate = self
            .box_geometry(target_box)
            .and_then(|geometry| geometry.layout_parent.or(geometry.parent));
        let mut seen = HashSet::new();
        let mut scroll_containers = Vec::new();
        while let Some(box_id) = candidate {
            let geometry = self.box_geometry(box_id)?;
            let extent = self.scroll_extent(box_id)?;
            if extent.is_scroll_container
                && let Some(container_source) = self
                    .boxes
                    .get(box_id.index())
                    .and_then(|layout_box| layout_box.geometry_source)
                && Some(container_source) != self.document_scrolling_element
                && seen.insert(container_source)
                && let Some(metrics) = self.element_metrics_for_source(container_source)
            {
                scroll_containers.push(LayoutScrollContainerMetrics {
                    source: container_source,
                    kind: LayoutScrollContainerKind::Element,
                    metrics,
                });
            }
            candidate = geometry.layout_parent.or(geometry.parent);
        }
        let viewport_source = self
            .document_scrolling_element
            .unwrap_or(self.document_element);
        if seen.insert(viewport_source)
            && let Some(metrics) = self.viewport_scroll_metrics_for_source(viewport_source)
        {
            scroll_containers.push(LayoutScrollContainerMetrics {
                source: viewport_source,
                kind: LayoutScrollContainerKind::Viewport,
                metrics,
            });
        }
        Some(LayoutScrollIntoViewGeometry {
            target_rects,
            scroll_containers,
        })
    }

    fn fragment_box(&self, fragment: LayoutFragmentId) -> Option<LayoutOutputBoxId> {
        match self.fragment(fragment)?.kind {
            LayoutFragmentKind::Box { box_id }
            | LayoutFragmentKind::InlineBox { box_id, .. }
            | LayoutFragmentKind::LineBreak { box_id, .. }
            | LayoutFragmentKind::Text { box_id, .. } => Some(box_id),
            LayoutFragmentKind::Line { .. } => None,
        }
    }

    fn scroll_target_rects_for_box(&self, box_id: LayoutOutputBoxId) -> Vec<LayoutQuad> {
        let Some(geometry) = self.box_geometry(box_id) else {
            return Vec::new();
        };
        let rects = geometry
            .fragments
            .iter()
            .filter_map(|fragment| self.fragment(*fragment))
            .filter(|fragment| match fragment.kind {
                LayoutFragmentKind::Box {
                    box_id: fragment_box,
                }
                | LayoutFragmentKind::InlineBox {
                    box_id: fragment_box,
                    ..
                }
                | LayoutFragmentKind::LineBreak {
                    box_id: fragment_box,
                    ..
                }
                | LayoutFragmentKind::Text {
                    box_id: fragment_box,
                    ..
                } => fragment_box == box_id,
                LayoutFragmentKind::Line { .. } => false,
            })
            .filter_map(|fragment| {
                self.coordinate_space(fragment.coordinate_space)
                    .map(|space| space.local_to_viewport.map_rect(fragment.rect))
            })
            .collect::<Vec<_>>();
        if !rects.is_empty() {
            return rects;
        }
        self.coordinate_space(geometry.coordinate_space)
            .map(|space| vec![space.local_to_viewport.map_rect(geometry.border_box)])
            .unwrap_or_default()
    }

    pub fn intersection_geometry(
        &self,
        target: N,
        root: Option<N>,
    ) -> Option<LayoutIntersectionGeometry> {
        let target_output = self.source_output(target);
        let target_box = target_output
            .as_ref()
            .and_then(|output| output.principal_box);
        let root_box = root.and_then(|source| self.source_output(source)?.principal_box);
        let root_is_layout_ancestor = match (target_box, root_box, root) {
            (_, _, None) => true,
            (Some(target_box), Some(root_box), Some(_)) => {
                self.box_is_layout_descendant_of(target_box, root_box)
            }
            _ => false,
        };
        let root_clips_overflow = root_box
            .and_then(|root_box| self.scroll_extent(root_box))
            .is_some_and(|extent| extent.clips_overflow);
        let root_rect = if root.is_none() {
            LayoutTransform2D::IDENTITY.map_rect(LayoutRect::new(
                0.0,
                0.0,
                self.viewport.css_width as f32,
                self.viewport.css_height as f32,
            ))
        } else if let Some((geometry, extent, space)) = root_box.and_then(|root_box| {
            let geometry = self.box_geometry(root_box)?;
            Some((
                geometry,
                self.scroll_extent(root_box)?,
                self.coordinate_space(geometry.coordinate_space)?,
            ))
        }) {
            let local_rect = if extent.clips_overflow {
                extent.scrollport
            } else {
                geometry.border_box
            };
            space.local_to_viewport.map_rect(local_rect)
        } else {
            LayoutTransform2D::IDENTITY.map_rect(LayoutRect::ZERO)
        };

        let mut clip_ids = HashSet::new();
        let mut ancestor_clips = Vec::new();
        let mut add_clip_chain = |mut clip: Option<LayoutClipChainId>| {
            while let Some(id) = clip {
                let Some(node) = self.clip_chain.get(id.index()) else {
                    break;
                };
                if node.owner == root_box {
                    break;
                }
                if node.owner.is_some()
                    && clip_ids.insert(id)
                    && let Some(space) = self.coordinate_space(node.coordinate_space)
                {
                    ancestor_clips.push(space.local_to_viewport.map_rect(node.rect));
                }
                clip = node.parent;
            }
        };
        if let Some(target_output) = target_output.as_ref() {
            for fragment in target_output
                .fragments
                .iter()
                .filter_map(|id| self.fragment(*id))
            {
                add_clip_chain(fragment.clip_chain);
            }
        }
        if target_output
            .as_ref()
            .is_none_or(|output| output.fragments.is_empty())
            && let Some(target_box) = target_box
            && let Some(geometry) = self.box_geometry(target_box)
        {
            add_clip_chain(geometry.clip_chain);
        }
        let target_visible = target_box
            .and_then(|id| self.box_geometry(id))
            .is_some_and(|geometry| geometry.visible);
        Some(LayoutIntersectionGeometry {
            target_rects: self.client_rects_for_source(target),
            root_rect,
            ancestor_clips,
            target_has_layout: target_box.is_some(),
            target_visible,
            root_clips_overflow,
            root_is_layout_ancestor,
        })
    }

    fn box_is_layout_descendant_of(
        &self,
        mut candidate: LayoutOutputBoxId,
        ancestor: LayoutOutputBoxId,
    ) -> bool {
        loop {
            if candidate == ancestor {
                return true;
            }
            let Some(parent) = self
                .box_geometry(candidate)
                .and_then(|geometry| geometry.layout_parent.or(geometry.parent))
            else {
                return false;
            };
            candidate = parent;
        }
    }
}
