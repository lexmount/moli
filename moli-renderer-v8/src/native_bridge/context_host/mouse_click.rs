use super::*;
use crate::dom::native::{DomHost, DomMutationEffects};
use moli_layout::{FrozenLayoutTree, LayoutDisplay};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct SyntheticScrollTargetWatch {
    document: DomHandle,
    source: DomHandle,
    box_path: Vec<(DomHandle, LayoutDisplay)>,
    alive: Weak<AtomicBool>,
}

#[derive(Clone, Copy)]
struct MousePress {
    handle: DomHandle,
    button: i32,
}

#[derive(Default)]
pub(super) struct MouseInputState {
    press: Option<MousePress>,
    element_under_mouse: Option<DomHandle>,
    #[cfg(feature = "molio")]
    compat_press_owners: [Option<WindowDocumentTaskTarget>; 5],
}

pub(crate) enum MouseReleaseFollowUp {
    ActivateViaClick(DomHandle),
    DispatchEvent {
        name: &'static str,
        target: DomHandle,
    },
}

impl PageScriptHost {
    pub(crate) fn watch_synthetic_scroll_target(
        &self,
        document: DomHandle,
        source: DomHandle,
    ) -> Option<Arc<AtomicBool>> {
        let box_path = self
            .with_latest_layout_tree_for_document(document, |tree| scroll_box_path(tree, source))
            .flatten()?;
        let alive = Arc::new(AtomicBool::new(true));
        let mut watches = self.synthetic_scroll_targets.borrow_mut();
        watches.retain(|watch| {
            watch
                .alive
                .upgrade()
                .is_some_and(|alive| alive.load(Ordering::Relaxed))
        });
        watches.push(SyntheticScrollTargetWatch {
            document,
            source,
            box_path,
            alive: Arc::downgrade(&alive),
        });
        Some(alive)
    }

    pub(super) fn note_synthetic_scroll_target_layout(
        &self,
        document: DomHandle,
        tree: &FrozenLayoutTree<DomHandle>,
    ) {
        self.synthetic_scroll_targets.borrow_mut().retain(|watch| {
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
                alive.store(false, Ordering::Relaxed);
            }
            survives && alive.load(Ordering::Relaxed)
        });
    }

    pub(crate) fn note_synthetic_scroll_target_removals(
        &mut self,
        dom: &DomHost,
        effects: &DomMutationEffects,
    ) {
        self.synthetic_scroll_targets.get_mut().retain(|watch| {
            let Some(alive) = watch.alive.upgrade() else {
                return false;
            };
            if !alive.load(Ordering::Relaxed) {
                return false;
            }
            // Mutation facts preserve actual removals even when the same node
            // is reinserted before the next wheel, with no observer installed.
            for mutation in effects.style().child_list_mutations() {
                for &root in mutation.removed_nodes() {
                    if in_shadow_including_subtree(dom, root, watch.source) {
                        alive.store(false, Ordering::Relaxed);
                        return false;
                    }
                }
            }
            true
        });
    }

    pub(crate) fn mouse_press_handle(&self) -> Option<DomHandle> {
        self.mouse_input.press.map(|press| press.handle)
    }

    pub(crate) fn mouse_press_button(&self) -> Option<i32> {
        self.mouse_input.press.map(|press| press.button)
    }

    pub(crate) fn begin_mouse_press(&mut self, handle: DomHandle, button: i32) {
        self.mouse_input.press = Some(MousePress { handle, button });
    }

    pub(crate) fn clear_mouse_press(&mut self) {
        self.mouse_input.press = None;
    }

    pub(crate) fn clear_mouse_input_state(&mut self) {
        self.mouse_input = MouseInputState::default();
    }

    #[cfg(feature = "molio")]
    pub(crate) fn set_compat_mouse_press_owner(
        &mut self,
        button: i32,
        owner: Option<WindowDocumentTaskTarget>,
    ) {
        if let Some(slot) = usize::try_from(button)
            .ok()
            .and_then(|index| self.mouse_input.compat_press_owners.get_mut(index))
        {
            *slot = owner;
        }
    }

    #[cfg(feature = "molio")]
    pub(crate) fn take_compat_mouse_press_owners(
        &mut self,
    ) -> [Option<WindowDocumentTaskTarget>; 5] {
        std::mem::take(&mut self.mouse_input.compat_press_owners)
    }

    pub(crate) fn set_mouse_event_target(&mut self, target: Option<DomHandle>) {
        self.mouse_input.element_under_mouse = target;
    }

    pub(crate) fn mouse_event_target(&self) -> Option<DomHandle> {
        self.mouse_input.element_under_mouse
    }

    pub(crate) fn note_mouse_input_target_removals(
        &mut self,
        dom: &DomHost,
        effects: &DomMutationEffects,
    ) {
        if self.mouse_input.press.is_none() && self.mouse_input.element_under_mouse.is_none() {
            return;
        }
        // These facts include removals followed by reinsertion, independently
        // of MutationObserver registration. Unlike click targeting, removal
        // uses shadow-including DOM ancestry: removing a slot does not remove
        // its assigned light children. A moved press is itself a removed root.
        for mutation in effects.style().child_list_mutations() {
            for &root in mutation.removed_nodes() {
                if self
                    .mouse_input
                    .press
                    .is_some_and(|press| in_shadow_including_subtree(dom, root, press.handle))
                {
                    self.clear_mouse_press();
                }
                if self
                    .mouse_input
                    .element_under_mouse
                    .is_some_and(|target| in_shadow_including_subtree(dom, root, target))
                {
                    // Like Blink's HandleRemoveSubtree, retarget immediately
                    // to the old DOM parent, even if the target is reinserted.
                    self.mouse_input.element_under_mouse =
                        std::iter::successors(Some(mutation.target()), |node| {
                            dom.parent_node(*node)
                                .or_else(|| dom.shadow_root_host(*node))
                        })
                        .find(|node| dom.node(*node).is_some_and(|node| node.is_element()));
                }
            }
        }
    }

    pub(crate) fn mouse_release_follow_up(
        &mut self,
        released: DomHandle,
        button: i32,
        captured_click_target: Option<DomHandle>,
    ) -> Option<MouseReleaseFollowUp> {
        let press = self.mouse_input.press.take()?;
        if press.button != button {
            return None;
        }
        match button {
            0 | 1 => {
                let dom = self.dom_host();
                if !dom.is_connected(press.handle) {
                    return None;
                }
                let target = if let Some(target) = captured_click_target {
                    target
                } else {
                    if dom.owner_document_handle(press.handle)
                        != dom.owner_document_handle(released)
                    {
                        return None;
                    }
                    common_flat_tree_ancestor(dom, press.handle, released)?
                };
                Some(if button == 0 {
                    MouseReleaseFollowUp::ActivateViaClick(target)
                } else {
                    MouseReleaseFollowUp::DispatchEvent {
                        name: "auxclick",
                        target,
                    }
                })
            }
            2 if press.handle == released => Some(MouseReleaseFollowUp::DispatchEvent {
                name: "contextmenu",
                target: released,
            }),
            _ => None,
        }
    }
}

fn scroll_box_path(
    tree: &FrozenLayoutTree<DomHandle>,
    source: DomHandle,
) -> Option<Vec<(DomHandle, LayoutDisplay)>> {
    let scroller = tree
        .boxes
        .iter()
        .find(|layout_box| layout_box.principal_source == Some(source))?;
    if source != tree.source_root() && !scroller.scroll_extent.is_scroll_container {
        return None;
    }
    // Each publication numbers its boxes again. Compare the source-backed
    // boxes that establish this scroller's structure and their display types.
    Some(
        std::iter::successors(Some(scroller), |layout_box| {
            tree.boxes.get(layout_box.structural_parent?.index())
        })
        .filter_map(|layout_box| Some((layout_box.principal_source?, layout_box.display)))
        .collect(),
    )
}

fn in_shadow_including_subtree(dom: &DomHost, root: DomHandle, target: DomHandle) -> bool {
    std::iter::successors(Some(target), |node| {
        dom.parent_node(*node)
            .or_else(|| dom.shadow_root_host(*node))
    })
    .any(|ancestor| ancestor == root)
}

pub(crate) fn common_flat_tree_ancestor(
    dom: &DomHost,
    pressed: DomHandle,
    released: DomHandle,
) -> Option<DomHandle> {
    if pressed == released {
        return Some(pressed);
    }
    let pressed_ancestors =
        std::iter::successors(Some(pressed), |node| dom.flat_tree_parent(*node))
            .collect::<Vec<_>>();
    let released_ancestors =
        std::iter::successors(Some(released), |node| dom.flat_tree_parent(*node))
            .collect::<Vec<_>>();
    pressed_ancestors
        .iter()
        .rev()
        .zip(released_ancestors.iter().rev())
        .take_while(|(pressed, released)| pressed == released)
        .last()
        .map(|(ancestor, _)| *ancestor)
}
