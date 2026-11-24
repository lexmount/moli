use super::mouse_click::in_shadow_including_subtree;
use super::*;
use crate::dom::native::{DomHost, DomMutationEffects};
use moli_layout::{FrozenLayoutTree, LayoutDisplay, LayoutListMarkerPosition};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

#[derive(PartialEq, Eq)]
struct ScrollBoxShape {
    source: DomHandle,
    display: LayoutDisplay,
    list_marker_position: LayoutListMarkerPosition,
}

pub(super) struct WheelTargetWatch {
    document: DomHandle,
    owner: WindowDocumentOwner,
    source: DomHandle,
    box_path: Vec<ScrollBoxShape>,
    alive: Weak<Cell<bool>>,
}

impl PageScriptHost {
    pub(crate) fn watch_wheel_scroll_target(
        &self,
        document: DomHandle,
        owner: WindowDocumentTaskTarget,
        source: DomHandle,
    ) -> Option<Rc<Cell<bool>>> {
        let box_path = self
            .with_latest_layout_tree_for_document(document, |tree| scroll_box_path(tree, source))
            .flatten()?;
        let alive = Rc::new(Cell::new(true));
        let mut watches = self.wheel_scroll_targets.borrow_mut();
        watches.retain(|watch| watch.alive.upgrade().is_some_and(|alive| alive.get()));
        watches.push(WheelTargetWatch {
            document,
            owner: owner.owner(),
            source,
            box_path,
            alive: Rc::downgrade(&alive),
        });
        Some(alive)
    }

    pub(super) fn note_wheel_scroll_target_layout(
        &self,
        document: DomHandle,
        tree: &FrozenLayoutTree<DomHandle>,
    ) {
        self.wheel_scroll_targets.borrow_mut().retain(|watch| {
            let Some(alive) = watch.alive.upgrade() else {
                return false;
            };
            let member = self
                .dom_host()
                .dom()
                .document_element_handle_for_document(watch.document)
                .and_then(|root| tree.tree_for_root(root));
            let survives = match member {
                Some(member) => {
                    scroll_box_path(member, watch.source).as_ref() == Some(&watch.box_path)
                }
                None => document != watch.document && document != self.document_handle(),
            };
            if !survives {
                alive.set(false);
            }
            survives && alive.get()
        });
    }

    pub(super) fn retire_wheel_scroll_targets(&mut self, owner: WindowDocumentOwner) {
        self.wheel_scroll_targets.get_mut().retain(|watch| {
            let Some(alive) = watch.alive.upgrade() else {
                return false;
            };
            if watch.owner == owner {
                alive.set(false);
                return false;
            }
            alive.get()
        });
    }

    pub(crate) fn note_wheel_scroll_target_removals(
        &mut self,
        dom: &DomHost,
        effects: &DomMutationEffects,
    ) {
        self.wheel_scroll_targets.get_mut().retain(|watch| {
            let Some(alive) = watch.alive.upgrade() else {
                return false;
            };
            if !alive.get() {
                return false;
            }
            // Mutation facts preserve actual removals even when the same node
            // is reinserted before the next wheel, with no observer installed.
            for mutation in effects.style().child_list_mutations() {
                for &root in mutation.removed_nodes() {
                    if in_shadow_including_subtree(dom, root, watch.source) {
                        alive.set(false);
                        return false;
                    }
                }
            }
            true
        });
    }
}

fn scroll_box_path(
    tree: &FrozenLayoutTree<DomHandle>,
    source: DomHandle,
) -> Option<Vec<ScrollBoxShape>> {
    let scroller = tree
        .boxes
        .iter()
        .find(|layout_box| layout_box.principal_source == Some(source))?;
    if source != tree.source_root() && !scroller.scroll_extent.is_scroll_container {
        return None;
    }
    // Each publication numbers its boxes again. Compare the source-backed
    // boxes that establish this scroller's structure using published styles.
    let path = std::iter::successors(Some(scroller), |layout_box| {
        tree.boxes.get(layout_box.structural_parent?.index())
    })
    .filter_map(|layout_box| {
        Some(ScrollBoxShape {
            source: layout_box.principal_source?,
            display: layout_box.display,
            list_marker_position: layout_box.list_marker_position,
        })
    })
    .collect();
    Some(path)
}
