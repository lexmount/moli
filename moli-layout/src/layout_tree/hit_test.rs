//! Temporary hit-test candidates and caret projections derived per query.

use std::{collections::HashSet, fmt::Debug, hash::Hash};

use super::{
    model::{
        LayoutBoxModel, LayoutClipChainId, LayoutCoordinateSpaceId, LayoutFragmentId,
        LayoutFragmentKind, LayoutOutputBoxId, LayoutPoint, LayoutQuad, LayoutRect,
        LayoutTransform2D,
    },
    tree::FrozenLayoutTree,
};

/// One front-to-back hit-test candidate.
#[derive(Clone, Debug, PartialEq)]
struct LayoutHitTestEntry<N> {
    source: N,
    fragment: LayoutFragmentId,
    coordinate_space: LayoutCoordinateSpaceId,
    clip_chain: Option<LayoutClipChainId>,
    local_rect: LayoutRect,
    paint_order: u32,
    is_text: bool,
    pointer_events: bool,
}

/// Result of resolving a point against hit candidates derived from the tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutHit<N> {
    pub source: N,
    /// The real provider identifies the exact fragment. Explicit mock
    /// providers can return a source-only hit without manufacturing a
    /// tree-local fragment identity.
    pub fragment: Option<LayoutFragmentId>,
    pub local_point: LayoutPoint,
    pub is_text: bool,
    /// The exact physical content box in the hit fragment's own coordinate
    /// space.
    ///
    /// Unlike `box_model`, this rectangle has not been projected through CSS
    /// transforms. Embedded-content consumers use it to enter the child frame
    /// without reconstructing the iframe's used size from authored CSS.
    pub local_content_box: Option<LayoutRect>,
    /// Converts a point from this tree's viewport into the hit fragment's
    /// coordinate space. This is retained on the short-lived hit result so
    /// frame and native-scrollbar routing can compose the same inverse
    /// transforms that produced `local_point`.
    pub viewport_to_local: LayoutTransform2D,
    /// Box geometry copied from the same frozen tree when the hit source owns a
    /// CSS box. Consumers use it for source-dependent follow-up work such as
    /// descending through a transformed child-frame content box without
    /// forcing a second parent-document pass.
    pub box_model: Option<LayoutBoxModel>,
}

/// Caret geometry resolved from the same text fragments and coordinate spaces
/// as Range geometry and hit testing.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutCaretPosition<N> {
    pub source: N,
    /// Present when `source` owns a rendered text fragment. The offset uses
    /// the source node's UTF-16 code-unit coordinate space.
    pub utf16_offset: Option<usize>,
    pub rect: LayoutQuad,
    /// Source boxes from the selected fragment towards the construction root.
    /// This lets tree-scope retargeting use the same pass without retaining
    /// output-local box identifiers or forcing a follow-up layout.
    pub ancestor_boxes: Vec<(N, LayoutBoxModel)>,
}

impl<N> FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub fn hit_test(
        &self,
        viewport_point: LayoutPoint,
        ignore_pointer_events_none: bool,
    ) -> Option<LayoutHit<N>> {
        self.hit_test_entries().into_iter().find_map(|entry| {
            self.hit_for_entry(&entry, viewport_point, ignore_pointer_events_none)
        })
    }

    pub fn hit_test_all(
        &self,
        viewport_point: LayoutPoint,
        ignore_pointer_events_none: bool,
    ) -> Vec<LayoutHit<N>> {
        let mut seen = HashSet::new();
        let mut hits = Vec::new();
        for entry in self.hit_test_entries() {
            let Some(hit) = self.hit_for_entry(&entry, viewport_point, ignore_pointer_events_none)
            else {
                continue;
            };
            if seen.insert(hit.source) {
                hits.push(hit);
            }
        }
        hits
    }

    /// Resolves a native classic-scrollbar control before ordinary DOM hit
    /// testing. Scrollbar controls are user-agent chrome and do not dispatch a
    /// pointer/mouse target into the scrolled content.
    pub fn scrollbar_hit_test(
        &self,
        viewport_point: LayoutPoint,
    ) -> Option<crate::LayoutScrollbarHit<N>> {
        let mut candidates = self
            .boxes
            .iter()
            .enumerate()
            .filter_map(|(index, layout_box)| {
                if !layout_box.visible || !layout_box.pointer_events {
                    return None;
                }
                let source = layout_box.geometry_source.or(layout_box.hit_source)?;
                let paint_order = layout_box
                    .fragments
                    .iter()
                    .filter_map(|id| self.fragment(*id)?.paint_order)
                    .max()
                    .unwrap_or(0);
                Some((std::cmp::Reverse(paint_order), index, source))
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.0);

        for (_, index, source) in candidates {
            let layout_box = &self.boxes[index];
            let is_root = layout_box.id == self.root_box;
            if !is_root && !self.point_passes_clip_chain(viewport_point, layout_box.clip_chain) {
                continue;
            }
            let local_to_viewport = if is_root {
                LayoutTransform2D::IDENTITY
            } else {
                layout_box.coordinate_space.local_to_viewport
            };
            let Some(viewport_to_local) = local_to_viewport.inverse() else {
                continue;
            };
            let local_point = viewport_to_local.map_point(viewport_point);
            for scrollbar in [
                layout_box.scroll_extent.vertical_scrollbar,
                layout_box.scroll_extent.horizontal_scrollbar,
            ] {
                let Some(scrollbar) = scrollbar else {
                    continue;
                };
                let Some(part) = scrollbar.part_at(local_point) else {
                    continue;
                };
                return Some(crate::LayoutScrollbarHit {
                    source,
                    scrollbar,
                    part,
                    local_point,
                    viewport_to_local,
                });
            }
        }
        None
    }

    pub fn caret_position(&self, viewport_point: LayoutPoint) -> Option<LayoutCaretPosition<N>> {
        let entries = self.hit_test_entries();
        let top_entry = entries
            .iter()
            .find(|entry| self.hit_for_entry(entry, viewport_point, true).is_some())?;
        let top_box = self.fragment_box_id(top_entry.fragment)?;
        let text_entry = entries
            .iter()
            .filter(|entry| entry.is_text)
            .filter(|entry| {
                self.fragment_box_id(entry.fragment)
                    .is_some_and(|box_id| self.box_is_construction_descendant_of(box_id, top_box))
            })
            .filter_map(|entry| {
                self.hit_entry_distance_to_point(entry, viewport_point)
                    .map(|distance| (entry, distance))
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(entry, _)| entry);
        if let Some(entry) = text_entry {
            return self.caret_position_for_text_entry(entry, viewport_point);
        }

        let fragment = self.fragment(top_entry.fragment)?;
        let space = self.coordinate_space(top_entry.coordinate_space)?;
        let inverse = space.local_to_viewport.inverse()?;
        let local_point = inverse.map_point(viewport_point);
        let caret_x = if local_point.x <= fragment.rect.x + fragment.rect.width / 2.0 {
            fragment.rect.x
        } else {
            fragment.rect.right()
        };
        let rect = space.local_to_viewport.map_rect(LayoutRect::new(
            caret_x,
            fragment.rect.y,
            0.0,
            fragment.rect.height,
        ));
        Some(LayoutCaretPosition {
            source: top_entry.source,
            utf16_offset: None,
            rect,
            ancestor_boxes: self.ancestor_box_models(top_box),
        })
    }

    /// Builds the front-to-back hit candidates for one query.
    ///
    /// Paint order, source provenance, transforms, and clips are canonical
    /// tree data. The duplicated candidate vector is deliberately temporary.
    fn hit_test_entries(&self) -> Vec<LayoutHitTestEntry<N>> {
        let mut entries = self
            .fragments
            .iter()
            .filter_map(|fragment| {
                let paint_order = fragment.paint_order?;
                let (box_id, is_text) = match fragment.kind {
                    LayoutFragmentKind::Box { box_id }
                    | LayoutFragmentKind::InlineBox { box_id, .. } => (box_id, false),
                    LayoutFragmentKind::Text { box_id, .. } => (box_id, true),
                    LayoutFragmentKind::Line { .. } => return None,
                };
                let layout_box = self.boxes.get(box_id.index())?;
                if !layout_box.visible {
                    return None;
                }
                Some(LayoutHitTestEntry {
                    source: layout_box.hit_source?,
                    fragment: fragment.id,
                    coordinate_space: fragment.coordinate_space,
                    clip_chain: fragment.clip_chain,
                    local_rect: fragment.rect,
                    paint_order,
                    is_text,
                    pointer_events: layout_box.pointer_events,
                })
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.paint_order));
        entries
    }

    fn caret_position_for_text_entry(
        &self,
        entry: &LayoutHitTestEntry<N>,
        viewport_point: LayoutPoint,
    ) -> Option<LayoutCaretPosition<N>> {
        let fragment = self.fragment(entry.fragment)?;
        let LayoutFragmentKind::Text {
            box_id,
            source_utf16_range,
            rtl,
            ..
        } = &fragment.kind
        else {
            return None;
        };
        let space = self.coordinate_space(entry.coordinate_space)?;
        let local_point = space.local_to_viewport.inverse()?.map_point(viewport_point);
        let source_len = source_utf16_range
            .end
            .saturating_sub(source_utf16_range.start);
        let on_left_half = local_point.x <= fragment.rect.x + fragment.rect.width * 0.5;
        let at_source_start = if *rtl { !on_left_half } else { on_left_half };
        let fragment_offset = if at_source_start { 0 } else { source_len };
        let caret_x = if at_source_start == *rtl {
            fragment.rect.right()
        } else {
            fragment.rect.x
        };
        Some(LayoutCaretPosition {
            source: entry.source,
            utf16_offset: Some(source_utf16_range.start + fragment_offset),
            rect: space.local_to_viewport.map_rect(LayoutRect::new(
                caret_x,
                fragment.rect.y,
                0.0,
                fragment.rect.height,
            )),
            ancestor_boxes: self.ancestor_box_models(*box_id),
        })
    }

    fn hit_entry_distance_to_point(
        &self,
        entry: &LayoutHitTestEntry<N>,
        viewport_point: LayoutPoint,
    ) -> Option<f64> {
        let space = self.coordinate_space(entry.coordinate_space)?;
        let local_point = space.local_to_viewport.inverse()?.map_point(viewport_point);
        let nearest_local = LayoutPoint::new(
            local_point
                .x
                .clamp(entry.local_rect.x, entry.local_rect.right()),
            local_point
                .y
                .clamp(entry.local_rect.y, entry.local_rect.bottom()),
        );
        let nearest_viewport = space.local_to_viewport.map_point(nearest_local);
        if !self.point_passes_clip_chain(nearest_viewport, entry.clip_chain) {
            return None;
        }
        let dx = f64::from(nearest_viewport.x - viewport_point.x);
        let dy = f64::from(nearest_viewport.y - viewport_point.y);
        Some(dx * dx + dy * dy)
    }

    fn fragment_box_id(&self, fragment: LayoutFragmentId) -> Option<LayoutOutputBoxId> {
        match self.fragment(fragment)?.kind {
            LayoutFragmentKind::Box { box_id }
            | LayoutFragmentKind::InlineBox { box_id, .. }
            | LayoutFragmentKind::Text { box_id, .. } => Some(box_id),
            LayoutFragmentKind::Line { owner, .. } => Some(owner),
        }
    }

    fn box_is_construction_descendant_of(
        &self,
        mut candidate: LayoutOutputBoxId,
        ancestor: LayoutOutputBoxId,
    ) -> bool {
        loop {
            if candidate == ancestor {
                return true;
            }
            let Some(parent) = self.box_geometry(candidate).and_then(|box_| box_.parent) else {
                return false;
            };
            candidate = parent;
        }
    }

    fn ancestor_box_models(&self, mut box_id: LayoutOutputBoxId) -> Vec<(N, LayoutBoxModel)> {
        let mut seen = HashSet::new();
        let mut ancestors = Vec::new();
        loop {
            if let Some(source) = self
                .boxes
                .get(box_id.index())
                .and_then(|layout_box| layout_box.geometry_source)
                && seen.insert(source)
                && let Some(model) = self.box_model_for_source(source)
            {
                ancestors.push((source, model));
            }
            let Some(parent) = self.box_geometry(box_id).and_then(|box_| box_.parent) else {
                break;
            };
            box_id = parent;
        }
        ancestors
    }

    fn hit_for_entry(
        &self,
        entry: &LayoutHitTestEntry<N>,
        viewport_point: LayoutPoint,
        ignore_pointer_events_none: bool,
    ) -> Option<LayoutHit<N>> {
        if !ignore_pointer_events_none && !entry.pointer_events {
            return None;
        }
        if !self.point_passes_clip_chain(viewport_point, entry.clip_chain) {
            return None;
        }
        let inverse = self
            .coordinate_space(entry.coordinate_space)?
            .local_to_viewport
            .inverse()?;
        let local_point = inverse.map_point(viewport_point);
        let local_content_box = self
            .fragment(entry.fragment)
            .and_then(|fragment| fragment.box_model.map(|model| model.content))
            .or_else(|| {
                let box_id = self.fragment_box_id(entry.fragment)?;
                let geometry = self.box_geometry(box_id)?;
                Some(geometry.content_box)
            });
        entry.local_rect.contains(local_point).then_some(LayoutHit {
            source: entry.source,
            fragment: Some(entry.fragment),
            local_point,
            is_text: entry.is_text,
            local_content_box,
            viewport_to_local: inverse,
            box_model: self.box_model_for_source(entry.source),
        })
    }
}

impl<N> FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
    fn point_passes_clip_chain(
        &self,
        viewport_point: LayoutPoint,
        mut clip: Option<LayoutClipChainId>,
    ) -> bool {
        while let Some(id) = clip {
            let Some(node) = self.clip_chain.get(id.index()) else {
                return false;
            };
            let Some(inverse) = self
                .coordinate_space(node.coordinate_space)
                .and_then(|space| space.local_to_viewport.inverse())
            else {
                return false;
            };
            if !node.rect.contains(inverse.map_point(viewport_point)) {
                return false;
            }
            clip = node.parent;
        }
        true
    }
}
