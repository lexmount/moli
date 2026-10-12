use super::*;
use crate::dom::native::{DomHost, DomMutationEffects};

#[derive(Default)]
pub(super) struct MouseInputState {
    presses: std::collections::BTreeMap<i32, DomHandle>,
    element_under_mouse: Option<DomHandle>,
}

pub(crate) enum MouseReleaseFollowUp {
    ActivateViaClick(DomHandle),
    Auxiliary,
}

impl JsContextHost {
    #[cfg(test)]
    pub(crate) fn has_mouse_press(&self) -> bool {
        !self.mouse_input.presses.is_empty()
    }

    pub(crate) fn begin_mouse_press(&mut self, handle: DomHandle, button: i32) {
        self.mouse_input.presses.insert(button, handle);
    }

    pub(crate) fn clear_mouse_button_press(&mut self, button: i32) {
        self.mouse_input.presses.remove(&button);
    }

    pub(crate) fn clear_mouse_presses(&mut self) {
        self.mouse_input.presses.clear();
    }

    pub(crate) fn retain_mouse_presses(&mut self, buttons: i32, released_button: Option<i32>) {
        self.mouse_input.presses.retain(|button, _| {
            Some(*button) == released_button || buttons & mouse_button_mask(*button) != 0
        });
    }

    pub(crate) fn clear_mouse_input_state(&mut self) {
        self.mouse_input = MouseInputState::default();
    }

    pub(crate) fn set_mouse_event_target(&mut self, target: DomHandle) {
        self.mouse_input.element_under_mouse = Some(target);
    }

    pub(crate) fn mouse_event_target(&self) -> Option<DomHandle> {
        self.mouse_input.element_under_mouse
    }

    pub(crate) fn note_mouse_input_target_removals(
        &mut self,
        dom: &DomHost,
        effects: &DomMutationEffects,
    ) {
        if self.mouse_input.presses.is_empty() && self.mouse_input.element_under_mouse.is_none() {
            return;
        }
        // These facts include removals followed by reinsertion, independently
        // of MutationObserver registration. Unlike click targeting, removal
        // uses shadow-including DOM ancestry: removing a slot does not remove
        // its assigned light children. A moved press is itself a removed root.
        for mutation in effects.style().child_list_mutations() {
            for &root in mutation.removed_nodes() {
                self.mouse_input
                    .presses
                    .retain(|_, handle| !in_shadow_including_subtree(dom, root, *handle));
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
        self.mouse_input.presses.get(&button)?;
        match button {
            0 => self
                .take_mouse_click_target(released, button, captured_click_target)
                .map(MouseReleaseFollowUp::ActivateViaClick),
            1..=4 => Some(MouseReleaseFollowUp::Auxiliary),
            _ => {
                self.clear_mouse_button_press(button);
                None
            }
        }
    }

    pub(crate) fn take_mouse_click_target(
        &mut self,
        released: DomHandle,
        button: i32,
        captured_click_target: Option<DomHandle>,
    ) -> Option<DomHandle> {
        let pressed = self.mouse_input.presses.remove(&button)?;
        match button {
            0..=4 => {
                let dom = self.dom_host();
                if !dom.is_connected(pressed) {
                    return None;
                }
                let target = if let Some(target) = captured_click_target {
                    target
                } else {
                    if dom.owner_document_handle(pressed) != dom.owner_document_handle(released) {
                        return None;
                    }
                    common_flat_tree_ancestor(dom, pressed, released)?
                };
                Some(target)
            }
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

fn common_flat_tree_ancestor(
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

fn mouse_button_mask(button: i32) -> i32 {
    match button {
        0 => 1,
        1 => 4,
        2 => 2,
        3 => 8,
        4 => 16,
        _ => 0,
    }
}
