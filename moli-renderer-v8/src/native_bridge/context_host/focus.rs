use super::{JsContextHost, OwnerDispatchScope, PendingWindowMessageEndpoint};
use crate::document_runtime::DomHandle;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_WINDOW_FOCUS_ORDER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Eq, PartialEq)]
enum WindowFocusIdentity {
    Frame(crate::frame_owner_model::FrameSchedulerLaneId),
    Popup(u64),
}

pub(super) struct WindowFocusHistory {
    identity: WindowFocusIdentity,
    top_document: DomHandle,
    focused: bool,
    last_order: Option<u64>,
}

mod navigation;
pub(crate) use navigation::SequentialFocusStartingPoint;

#[derive(Default)]
pub(super) struct DocumentFocusChangeState {
    focused_area: Option<DomHandle>,
    sequential_starting_point: Option<SequentialFocusStartingPoint>,
    epoch: u64,
    last_change_was_automatic_fixup: bool,
}

impl JsContextHost {
    pub(crate) fn note_window_focus_changes(&mut self) {
        let mut windows = vec![OwnerDispatchScope::Top];
        windows.extend(
            self.child_browsing_context_handles_in_document_order()
                .into_iter()
                .map(OwnerDispatchScope::Child),
        );
        let mut popups = self
            .lightweight_popup_browsing_contexts
            .keys()
            .copied()
            .collect::<Vec<_>>();
        popups.sort_unstable();
        windows.extend(popups.into_iter().map(OwnerDispatchScope::LightweightPopup));
        let mut live = std::collections::HashSet::new();
        let mut changed_roots = std::collections::HashSet::new();
        for dispatch in windows {
            let Some(target) =
                self.current_window_document_task_target_for_dispatch_scope(dispatch)
            else {
                continue;
            };
            let document = match dispatch {
                OwnerDispatchScope::Top => Some(self.document_handle()),
                OwnerDispatchScope::Child(frame) => {
                    self.child_browsing_context_document_handle(frame)
                }
                OwnerDispatchScope::LightweightPopup(id) => {
                    self.lightweight_popup_document_handle(id)
                }
            };
            let Some(document) = document else {
                continue;
            };
            let Some(top_document) = self.top_level_document_for_document(document) else {
                continue;
            };
            let identity = match target.owner() {
                crate::window_document_identity::WindowDocumentOwner::Frame(owner) => {
                    WindowFocusIdentity::Frame(owner.scheduler_lane_id)
                }
                crate::window_document_identity::WindowDocumentOwner::LightweightPopup(owner) => {
                    WindowFocusIdentity::Popup(owner.popup_id())
                }
            };
            let focused = self.document_has_focus(document);
            live.insert(dispatch);
            let entry = self
                .window_focus_history
                .entry(dispatch)
                .or_insert(WindowFocusHistory {
                    identity,
                    top_document,
                    focused: false,
                    last_order: None,
                });
            if entry.identity != identity {
                *entry = WindowFocusHistory {
                    identity,
                    top_document,
                    focused: false,
                    last_order: None,
                };
            }
            if focused != entry.focused {
                changed_roots.insert(top_document);
            }
            entry.top_document = top_document;
            entry.focused = focused;
        }
        self.window_focus_history.retain(|dispatch, entry| {
            if live.contains(dispatch) {
                return true;
            }
            if entry.focused {
                changed_roots.insert(entry.top_document);
            }
            false
        });
        let order = self
            .window_focus_history
            .values()
            .any(|entry| entry.focused && changed_roots.contains(&entry.top_document))
            .then(|| {
                NEXT_WINDOW_FOCUS_ORDER
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                        value.checked_add(1)
                    })
                    .expect("Window focus order exhausted")
            });
        for entry in self.window_focus_history.values_mut() {
            if entry.focused && changed_roots.contains(&entry.top_document) {
                // A focused frame and its focused ancestors share one focus
                // time; creation order breaks ties within that chain.
                entry.last_order = order;
            }
        }
    }

    pub(crate) fn window_last_focus_order(&self, dispatch: OwnerDispatchScope) -> Option<u64> {
        let entry = self.window_focus_history.get(&dispatch)?;
        let target = self.current_window_document_task_target_for_dispatch_scope(dispatch)?;
        let identity = match target.owner() {
            crate::window_document_identity::WindowDocumentOwner::Frame(owner) => {
                WindowFocusIdentity::Frame(owner.scheduler_lane_id)
            }
            crate::window_document_identity::WindowDocumentOwner::LightweightPopup(owner) => {
                WindowFocusIdentity::Popup(owner.popup_id())
            }
        };
        (entry.identity == identity)
            .then_some(entry.last_order)
            .flatten()
    }

    pub(crate) fn document_has_focus(&self, document: DomHandle) -> bool {
        if !self.document_activity().focused || !self.document_is_fully_active(document) {
            return false;
        }
        let mut document = document;
        loop {
            match self.window_endpoint_for_document(document) {
                Some(PendingWindowMessageEndpoint::TopWindow)
                | Some(PendingWindowMessageEndpoint::LightweightPopup(_)) => return true,
                Some(PendingWindowMessageEndpoint::ChildWindow(container)) => {
                    let Some(parent) = self.dom_host().owner_document_handle(container) else {
                        return false;
                    };
                    // Inactive Documents retain their local focused area. Every
                    // ancestor must still select this child's navigable.
                    if self.document_focused_area(parent) != Some(container) {
                        return false;
                    }
                    document = parent;
                }
                None => return false,
            }
        }
    }

    pub(crate) fn focused_viewport_document(&self) -> Option<DomHandle> {
        self.focused_viewport_document.filter(|document| {
            self.active_element_handle().is_none()
                && self.top_level_document_for_document(*document).is_some()
        })
    }

    pub(crate) fn set_focused_viewport_document(&mut self, document: Option<DomHandle>) {
        // An unfocused element leaves a concrete Document viewport. Its iframe
        // remains the active element exposed by ancestor Documents during blur.
        self.focused_viewport_document = document;
    }

    pub(crate) fn clear_disconnected_document_focus(&mut self) {
        let mut changed = false;
        if self
            .focused_viewport_document
            .is_some_and(|document| self.top_level_document_for_document(document).is_none())
        {
            self.focused_viewport_document = None;
            changed = true;
        }
        let documents = self
            .document_focus_changes
            .iter()
            .filter_map(|(document, state)| {
                state
                    .focused_area
                    .filter(|area| {
                        !self.dom_host().is_connected(*area)
                            || self.dom_host().owner_document_handle(*area) != Some(*document)
                    })
                    .map(|area| (*document, area))
            })
            .collect::<Vec<_>>();
        for (document, area) in documents {
            // A retained iframe handle must not revive its former parent's
            // focus when script inserts the iframe again after removal.
            changed |= self.clear_document_focus_record(document, area);
        }
        if changed {
            self.note_window_focus_changes();
        }
    }

    pub(crate) fn note_inserted_autofocus_candidates(&mut self, roots: &[DomHandle]) {
        for &root in roots {
            let Some(document) = self.dom_host().owner_document_handle(root) else {
                continue;
            };
            let Some(top_document) = self.top_level_document_for_document(document) else {
                continue;
            };
            if self.autofocus_processed(top_document)
                || !self.document_allows_autofocus(document)
                || !self.queue_autofocus_candidates_in_subtree(top_document, root)
            {
                continue;
            }
            self.queue_top_document_post_parse_autofocus(top_document);
        }
    }

    pub(super) fn queue_top_document_post_parse_autofocus(&mut self, document: DomHandle) {
        // Parsing admits after DOMContentLoaded and its checkpoint. Later
        // insertion or child stylesheet completion also publishes rendering work.
        if document == self.document_handle() {
            if self.dom_content_loaded_dispatched()
                && let Some(owner) = self.current_main_document_task_owner()
            {
                let _ = self.queue_main_document_post_parse_autofocus(owner);
            }
        } else if let Some(popup_id) = self.lightweight_popup_id_for_document_handle(document) {
            self.queue_lightweight_popup_post_parse_autofocus(popup_id);
        }
    }

    pub(crate) fn autofocus_document_has_blocking_stylesheets(&self, document: DomHandle) -> bool {
        if let Some(child) = self.child_browsing_context_host_for_document_handle(document) {
            return self
                .current_child_document_task_owner(child)
                .is_some_and(|owner| {
                    self.frame_document_blocking_stylesheets
                        .has_pending(owner.document_owner())
                });
        }
        self.has_blocking_stylesheets_for_document(document)
    }

    pub(crate) fn autofocus_ancestor_has_target(&self, mut document: DomHandle) -> bool {
        loop {
            if self.dom_host().document_target_element(document).is_some() {
                return true;
            }
            let Some(PendingWindowMessageEndpoint::ChildWindow(container)) =
                self.window_endpoint_for_document(document)
            else {
                return false;
            };
            let Some(parent) = self.dom_host().owner_document_handle(container) else {
                return false;
            };
            document = parent;
        }
    }

    pub(crate) fn document_focused_area(&self, document: DomHandle) -> Option<DomHandle> {
        self.document_focus_changes
            .get(&document)
            .and_then(|state| state.focused_area)
            .filter(|handle| {
                self.dom_host().is_connected(*handle)
                    && self.dom_host().owner_document_handle(*handle) == Some(document)
            })
    }

    pub(crate) fn clear_document_focused_area_for_fixup(
        &mut self,
        document: DomHandle,
        area: DomHandle,
    ) {
        if self.clear_document_focus_record(document, area) {
            self.note_window_focus_changes();
        }
    }

    fn clear_document_focus_record(&mut self, document: DomHandle, area: DomHandle) -> bool {
        if let Some(state) = self.document_focus_changes.get_mut(&document)
            && state.focused_area == Some(area)
        {
            state.focused_area = None;
            state.epoch = state.epoch.wrapping_add(1);
            // Automatic fixup clears "focus changed during ongoing navigation",
            // even after an earlier focus() call. Later author focus changes
            // still suppress the navigation's focus reset.
            state.last_change_was_automatic_fixup = true;
            return true;
        }
        false
    }

    pub(crate) fn top_level_document_for_document(
        &self,
        mut document: DomHandle,
    ) -> Option<DomHandle> {
        loop {
            match self.window_endpoint_for_document(document)? {
                PendingWindowMessageEndpoint::TopWindow
                | PendingWindowMessageEndpoint::LightweightPopup(_) => return Some(document),
                PendingWindowMessageEndpoint::ChildWindow(container) => {
                    if !self.dom_host().is_connected(container) {
                        return None;
                    }
                    document = self.dom_host().owner_document_handle(container)?;
                }
            }
        }
    }

    pub(crate) fn focus_change_epoch(&self, document: DomHandle) -> u64 {
        self.document_focus_changes
            .get(&document)
            .map_or(0, |state| state.epoch)
    }

    pub(crate) fn focus_changed_since(&self, document: DomHandle, epoch: u64) -> bool {
        self.document_focus_changes
            .get(&document)
            .is_some_and(|state| state.epoch != epoch && !state.last_change_was_automatic_fixup)
    }

    pub(crate) fn note_document_focused_area(
        &mut self,
        document: DomHandle,
        area: Option<DomHandle>,
    ) {
        let state = self.document_focus_changes.entry(document).or_default();
        if let Some(area) = area {
            state.sequential_starting_point = Some(SequentialFocusStartingPoint::Element(area));
        }
        if state.focused_area != area {
            state.focused_area = area;
            state.epoch = state.epoch.wrapping_add(1);
            state.last_change_was_automatic_fixup = false;
        }
    }

    pub(crate) fn sequential_focus_starting_point(
        &self,
        document: DomHandle,
    ) -> Option<SequentialFocusStartingPoint> {
        self.document_focus_changes
            .get(&document)
            .and_then(|state| state.sequential_starting_point)
            .filter(|point| {
                let handle = point.container();
                self.dom_host()
                    .node(handle)
                    .is_some_and(|node| node.is_connected())
                    && self.dom_host().owner_document_handle(handle) == Some(document)
            })
    }

    pub(crate) fn set_sequential_focus_starting_point(
        &mut self,
        document: DomHandle,
        target: Option<DomHandle>,
    ) {
        self.document_focus_changes
            .entry(document)
            .or_default()
            .sequential_starting_point = target.map(SequentialFocusStartingPoint::Element);
    }

    pub(crate) fn mark_focus_changed(
        &mut self,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
    ) {
        // Only Documents in the new focus chain change their focused area.
        // Focusing a parent does not change the focused area retained by an
        // inactive child, or cancel that child's pending navigation focus reset.
        if let Some(mut next) = next {
            while let Some(document) = self.dom_host().owner_document_handle(next) {
                self.note_document_focused_area(document, Some(next));
                let Some(container) =
                    self.child_browsing_context_host_for_document_handle(document)
                else {
                    break;
                };
                next = container;
            }
        } else if let Some(document) =
            previous.and_then(|handle| self.dom_host().owner_document_handle(handle))
        {
            self.note_document_focused_area(document, None);
        }
        self.note_window_focus_changes();
    }

    pub(crate) fn window_endpoint_for_document(
        &self,
        document_handle: DomHandle,
    ) -> Option<PendingWindowMessageEndpoint> {
        if document_handle == self.document_handle() {
            return Some(PendingWindowMessageEndpoint::TopWindow);
        }
        if let Some(popup_id) = self.lightweight_popup_id_for_document_handle(document_handle) {
            return Some(PendingWindowMessageEndpoint::LightweightPopup(popup_id));
        }
        self.child_browsing_context_host_for_document_handle(document_handle)
            .map(PendingWindowMessageEndpoint::ChildWindow)
    }

    pub(crate) fn scroll_window_endpoint_to(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        endpoint: PendingWindowMessageEndpoint,
        x: f64,
        y: f64,
    ) {
        if matches!(endpoint, PendingWindowMessageEndpoint::LightweightPopup(_)) {
            return;
        }
        let dispatch_scope = endpoint.dispatch_scope();
        let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) else {
            return;
        };
        let Some((_, context)) = self.window_execution_context(scope, owner, dispatch_scope) else {
            return;
        };
        let context = v8::Global::new(scope, context);
        let context = v8::Local::new(scope, &context);
        let target_scope = &mut v8::ContextScope::new(scope, context);
        crate::window_host::scroll_window_to(target_scope, self, x, y);
    }
}
