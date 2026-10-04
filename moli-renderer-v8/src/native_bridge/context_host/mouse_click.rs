use super::*;
use crate::dom::native::{DomHost, DomMutationEffects};

#[derive(Clone, Copy)]
struct MousePress {
    handle: DomHandle,
    button: i32,
}

#[derive(Default)]
pub(super) struct MouseInputState {
    press: Option<MousePress>,
    element_under_mouse: Option<DomHandle>,
}

pub(crate) enum MouseReleaseFollowUp {
    ActivateViaClick(DomHandle),
    Auxiliary,
}

impl JsContextHost {
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
        let press = self.mouse_input.press?;
        if press.button != button {
            self.clear_mouse_press();
            return None;
        }
        match button {
            0 => self
                .take_mouse_click_target(released, button, captured_click_target)
                .map(MouseReleaseFollowUp::ActivateViaClick),
            1..=4 => Some(MouseReleaseFollowUp::Auxiliary),
            _ => {
                self.clear_mouse_press();
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
        let press = self.mouse_input.press.take()?;
        if press.button != button {
            return None;
        }
        match button {
            0..=4 => {
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
