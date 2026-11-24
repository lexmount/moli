use super::*;
use crate::dom::native::{DomHost, DomMutationEffects};
use std::{
    cell::RefCell,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

struct SyntheticScrollTargetWatch {
    source: DomHandle,
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
    synthetic_scroll_targets: RefCell<Vec<SyntheticScrollTargetWatch>>,
}

pub(crate) enum MouseReleaseFollowUp {
    ActivateViaClick(DomHandle),
    DispatchEvent {
        name: &'static str,
        target: DomHandle,
    },
}

impl PageScriptHost {
    pub(crate) fn watch_synthetic_scroll_target(&self, source: DomHandle) -> Arc<AtomicBool> {
        let alive = Arc::new(AtomicBool::new(true));
        let mut watches = self.mouse_input.synthetic_scroll_targets.borrow_mut();
        watches.retain(|watch| {
            watch
                .alive
                .upgrade()
                .is_some_and(|alive| alive.load(Ordering::Relaxed))
        });
        watches.push(SyntheticScrollTargetWatch {
            source,
            alive: Arc::downgrade(&alive),
        });
        alive
    }

    pub(crate) fn note_synthetic_scroll_target_removals(
        &mut self,
        dom: &DomHost,
        effects: &DomMutationEffects,
    ) {
        self.mouse_input
            .synthetic_scroll_targets
            .get_mut()
            .retain(|watch| {
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
        // A drag can reset mouse state while a wheel gesture remains active.
        // Its removal watches belong to the gesture's independent lifetime.
        self.mouse_input = MouseInputState {
            synthetic_scroll_targets: std::mem::take(&mut self.mouse_input.synthetic_scroll_targets),
            ..MouseInputState::default()
        };
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
