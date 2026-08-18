use std::collections::{HashMap, HashSet};

use moli_layout::{
    FrozenLayoutTree, LayoutIntersectionGeometry, LayoutOutputBoxId, LayoutQuad, LayoutRect,
    LayoutTransform2D,
};

use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

use super::source_view::native_flat_parent;

/// Document-owned state for `content-visibility:auto` display locks.
///
/// The DOM element is the durable identity. Layout boxes are rebuilt for each
/// pass and only publish the capability and geometry needed by the post-layout
/// observation step. A newly observed auto context starts locked, matching
/// Blink's display-lock lifecycle, and can expose its children after its own
/// principal box has been observed near the viewport.
#[derive(Clone, Default)]
pub(crate) struct AutoDisplayLockState {
    locked: HashMap<DomHandle, bool>,
}

impl AutoDisplayLockState {
    /// Lock input for a new layout epoch. A newly styled auto context begins
    /// locked until its principal box receives the first post-layout viewport
    /// observation.
    pub(super) fn is_locked_for_layout(&self, element: DomHandle) -> bool {
        self.locked.get(&element).copied().unwrap_or(true)
    }

    /// State published by the most recent successful rendering demand.
    /// Absence means that no rendered principal box has established a lock.
    pub(crate) fn published_is_locked(&self, element: DomHandle) -> bool {
        self.locked.get(&element).copied().unwrap_or(false)
    }

    pub(super) fn contains(&self, element: DomHandle) -> bool {
        self.locked.contains_key(&element)
    }

    pub(crate) fn retain_connected(&mut self, runtime: &JsContextHost) {
        self.locked
            .retain(|element, _| runtime.dom_host().is_connected(*element));
    }

    /// Reconciles one document's requested auto contexts after layout.
    ///
    /// Each context receives at most one viewport observation per rendering
    /// demand. If an observation changes its lock state, the caller rebuilds
    /// the complete document tree. This gives nested locks a stable sequence
    /// without an arbitrary retry cap: each additional pass must expose and
    /// observe at least one previously undelivered context.
    pub(super) fn observe_layout(
        &mut self,
        runtime: &JsContextHost,
        root: DomHandle,
        document: DomHandle,
        tree: &FrozenLayoutTree<DomHandle>,
        requests: &DisplayLockStyleRequests,
        delivered: &mut HashSet<DomHandle>,
    ) -> bool {
        for element in &requests.non_auto {
            self.locked.remove(element);
        }

        let activation = DisplayLockActivationSnapshot::read(runtime, document);
        let mut changed = false;
        for element in &requests.auto {
            let Some(box_id) = tree
                .source_output(*element)
                .and_then(|output| output.principal_box)
            else {
                // `display:none` and `display:contents` can request `auto` but
                // have no principal box on which a lock can live. Clear any
                // previously published state instead of letting a stale lock
                // hide descendants after a display-type change.
                self.locked.insert(*element, false);
                delivered.insert(*element);
                continue;
            };
            let Some(box_geometry) = tree.box_geometry(box_id) else {
                self.locked.insert(*element, false);
                delivered.insert(*element);
                continue;
            };

            // Applicability is a LayoutObject decision, not a computed-style
            // decision. Persist the force-unlocked result so CSSOM consumers
            // share the same answer as box construction.
            if !box_geometry.display_lock_eligible {
                self.locked.insert(*element, false);
                delivered.insert(*element);
                continue;
            }
            if !delivered.insert(*element) {
                continue;
            }

            let should_lock = !activation.requires_unlock(runtime, root, *element)
                && !display_lock_box_intersects_viewport(tree, box_id, *element);
            let was_locked = self.is_locked_for_layout(*element);
            self.locked.insert(*element, should_lock);
            changed |= was_locked != should_lock;
        }
        changed
    }
}

/// Styles resolved during one layout pass, separated from the persistent lock
/// state. Descendants below a locked context are intentionally absent: they
/// become observable only after an ancestor unlocks and reconstructs them.
#[derive(Default)]
pub(super) struct DisplayLockStyleRequests {
    auto: HashSet<DomHandle>,
    non_auto: HashSet<DomHandle>,
}

impl DisplayLockStyleRequests {
    pub(super) fn record(&mut self, element: DomHandle, requests_auto: bool) {
        if requests_auto {
            self.non_auto.remove(&element);
            self.auto.insert(element);
        } else {
            self.auto.remove(&element);
            self.non_auto.insert(element);
        }
    }
}

struct DisplayLockActivationSnapshot {
    focused: Option<DomHandle>,
    fragment_target: Option<DomHandle>,
    selection: Option<crate::native_bridge::DocumentSelectionSnapshot>,
    top_layer: Vec<DomHandle>,
}

impl DisplayLockActivationSnapshot {
    fn read(runtime: &JsContextHost, document: DomHandle) -> Self {
        let focused = runtime
            .active_element_handle()
            .filter(|element| runtime.dom_host().owner_document_handle(*element) == Some(document));
        Self {
            focused,
            fragment_target: runtime.dom_host().document_target_element(document),
            selection: runtime.document_selection_snapshot(document),
            top_layer: top_layer_elements(runtime, document),
        }
    }

    fn requires_unlock(
        &self,
        runtime: &JsContextHost,
        root: DomHandle,
        element: DomHandle,
    ) -> bool {
        self.focused
            .is_some_and(|focused| is_inclusive_flat_ancestor(runtime, root, element, focused))
            || self
                .fragment_target
                .is_some_and(|target| is_inclusive_flat_ancestor(runtime, root, element, target))
            || self.selection.is_some_and(|selection| {
                selection_intersects_element(runtime, root, selection, element)
            })
            || self
                .top_layer
                .iter()
                .copied()
                .any(|top_layer| is_inclusive_flat_ancestor(runtime, root, element, top_layer))
    }
}

fn top_layer_elements(runtime: &JsContextHost, document: DomHandle) -> Vec<DomHandle> {
    runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .filter_map(|node| {
            if !node.is_connected()
                || runtime.dom_host().owner_document_handle(node.id()) != Some(document)
            {
                return None;
            }
            let element = node.as_element()?;
            (element.popover_open()
                || (element.is_html_element("dialog")
                    && element.dialog_modal()
                    && element.attribute("open").is_some()))
            .then_some(node.id())
        })
        .collect()
}

fn is_inclusive_flat_ancestor(
    runtime: &JsContextHost,
    root: DomHandle,
    ancestor: DomHandle,
    mut candidate: DomHandle,
) -> bool {
    let host = runtime.dom_host();
    loop {
        if candidate == ancestor {
            return true;
        }
        let Some(parent) = native_flat_parent(host, root, candidate) else {
            return false;
        };
        candidate = parent;
    }
}

fn selection_intersects_element(
    runtime: &JsContextHost,
    root: DomHandle,
    mut selection: crate::native_bridge::DocumentSelectionSnapshot,
    element: DomHandle,
) -> bool {
    let host = runtime.dom_host();
    if crate::range_boundary::point_order_in_dom(
        host,
        selection.start.container,
        selection.start.offset,
        selection.end.container,
        selection.end.offset,
    ) == Some(std::cmp::Ordering::Greater)
    {
        std::mem::swap(&mut selection.start, &mut selection.end);
    }
    if is_inclusive_flat_ancestor(runtime, root, element, selection.start.container)
        || is_inclusive_flat_ancestor(runtime, root, element, selection.end.container)
    {
        return true;
    }

    let Some(parent) = host
        .node(element)
        .and_then(crate::dom::native::Node::parent_node)
    else {
        return false;
    };
    let Some(index) = host.child_index(parent, element) else {
        return false;
    };
    let Ok(start_offset) = u32::try_from(index) else {
        return false;
    };
    let Some(end_offset) = start_offset.checked_add(1) else {
        return false;
    };
    let element_starts_before_selection_end = crate::range_boundary::point_order_in_dom(
        host,
        parent,
        start_offset,
        selection.end.container,
        selection.end.offset,
    ) == Some(std::cmp::Ordering::Less);
    let element_ends_after_selection_start = crate::range_boundary::point_order_in_dom(
        host,
        parent,
        end_offset,
        selection.start.container,
        selection.start.offset,
    ) == Some(std::cmp::Ordering::Greater);
    element_starts_before_selection_end && element_ends_after_selection_start
}

/// Blink applies the display-lock observer margin in target-local coordinates
/// before mapping authored transforms. Preserve that ordering instead of
/// expanding an already transformed viewport-space bounding box.
fn display_lock_box_intersects_viewport(
    tree: &FrozenLayoutTree<DomHandle>,
    box_id: LayoutOutputBoxId,
    element: DomHandle,
) -> bool {
    let Some(geometry) = tree.intersection_geometry(element, None) else {
        return false;
    };
    if !geometry.target_has_layout || !geometry.root_is_layout_ancestor {
        return false;
    }
    let root_rect = geometry.root_rect.bounding_rect();
    let Some(layout_box) = tree.box_geometry(box_id) else {
        return false;
    };
    let Some(space) = tree.coordinate_space(layout_box.coordinate_space) else {
        return false;
    };
    let expanded_target =
        expanded_display_lock_target(layout_box.border_box, root_rect, space.local_to_viewport);
    intersects_display_lock_viewport(&geometry, [expanded_target])
}

fn expanded_display_lock_target(
    target: LayoutRect,
    root: LayoutRect,
    local_to_viewport: LayoutTransform2D,
) -> LayoutQuad {
    let horizontal_margin = root.width.max(0.0) * 1.5;
    let vertical_margin = root.height.max(0.0) * 1.5;
    local_to_viewport.map_rect(LayoutRect::new(
        target.x - horizontal_margin,
        target.y - vertical_margin,
        target.width + horizontal_margin * 2.0,
        target.height + vertical_margin * 2.0,
    ))
}

/// Clips the expanded target through every overflow ancestor before testing
/// the implicit viewport root. Inclusive edges follow IntersectionObserver's
/// zero-area intersection semantics.
fn intersects_display_lock_viewport(
    geometry: &LayoutIntersectionGeometry,
    expanded_targets: impl IntoIterator<Item = LayoutQuad>,
) -> bool {
    if !geometry.target_has_layout || !geometry.root_is_layout_ancestor {
        return false;
    }
    let root_rect = geometry.root_rect.bounding_rect();
    expanded_targets.into_iter().any(|target| {
        let expanded = target.bounding_rect();
        geometry
            .ancestor_clips
            .iter()
            .copied()
            .map(moli_layout::LayoutQuad::bounding_rect)
            .try_fold(expanded, intersect_inclusive)
            .and_then(|clipped| intersect_inclusive(clipped, root_rect))
            .is_some()
    })
}

fn intersect_inclusive(a: LayoutRect, b: LayoutRect) -> Option<LayoutRect> {
    if !a.x.is_finite()
        || !a.y.is_finite()
        || !a.width.is_finite()
        || !a.height.is_finite()
        || !b.x.is_finite()
        || !b.y.is_finite()
        || !b.width.is_finite()
        || !b.height.is_finite()
        || a.width < 0.0
        || a.height < 0.0
        || b.width < 0.0
        || b.height < 0.0
    {
        return None;
    }
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    (right >= left && bottom >= top).then(|| LayoutRect::new(left, top, right - left, bottom - top))
}

#[cfg(test)]
mod tests {
    use moli_layout::{LayoutIntersectionGeometry, LayoutRect, LayoutTransform2D};

    use super::{expanded_display_lock_target, intersects_display_lock_viewport};

    fn geometry(target: LayoutRect) -> LayoutIntersectionGeometry {
        LayoutIntersectionGeometry {
            target_rects: vec![LayoutTransform2D::IDENTITY.map_rect(target)],
            root_rect: LayoutTransform2D::IDENTITY
                .map_rect(LayoutRect::new(0.0, 0.0, 100.0, 100.0)),
            ancestor_clips: Vec::new(),
            target_has_layout: true,
            target_visible: true,
            root_clips_overflow: true,
            root_is_layout_ancestor: true,
        }
    }

    #[test]
    fn display_lock_observation_uses_blink_target_margin() {
        let root = LayoutRect::new(0.0, 0.0, 100.0, 100.0);
        for (x, expected) in [(249.0, true), (250.0, true), (251.0, false)] {
            let geometry = geometry(LayoutRect::new(x, 20.0, 10.0, 10.0));
            let target = expanded_display_lock_target(
                LayoutRect::new(x, 20.0, 10.0, 10.0),
                root,
                LayoutTransform2D::IDENTITY,
            );
            assert_eq!(
                intersects_display_lock_viewport(&geometry, [target]),
                expected
            );
        }
    }

    #[test]
    fn display_lock_target_margin_precedes_authored_transforms() {
        let root = LayoutRect::new(0.0, 0.0, 100.0, 100.0);
        let geometry = geometry(LayoutRect::new(300.0, 20.0, 20.0, 20.0));
        let target = expanded_display_lock_target(
            LayoutRect::new(150.0, 10.0, 10.0, 10.0),
            root,
            LayoutTransform2D::scale(2.0, 2.0),
        );
        assert!(intersects_display_lock_viewport(&geometry, [target]));
    }

    #[test]
    fn display_lock_observation_applies_every_ancestor_clip() {
        let mut geometry = geometry(LayoutRect::new(50.0, 50.0, 10.0, 10.0));
        geometry.ancestor_clips = vec![
            LayoutTransform2D::IDENTITY.map_rect(LayoutRect::new(0.0, 0.0, 40.0, 100.0)),
            LayoutTransform2D::IDENTITY.map_rect(LayoutRect::new(60.0, 0.0, 40.0, 100.0)),
        ];
        let target = expanded_display_lock_target(
            LayoutRect::new(50.0, 50.0, 10.0, 10.0),
            LayoutRect::new(0.0, 0.0, 100.0, 100.0),
            LayoutTransform2D::IDENTITY,
        );
        assert!(!intersects_display_lock_viewport(&geometry, [target]));
    }
}
