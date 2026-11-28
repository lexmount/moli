use super::{PageScriptHost, PendingWindowMessageEndpoint};
use crate::document_runtime::{DomHandle, EventTargetHandle};
use crate::native_bridge::element::construct_focus_event;

impl PageScriptHost {
    pub(crate) fn document_has_focus(&self, document: DomHandle) -> bool {
        if !self.document_activity().focused || !self.document_is_fully_active(document) {
            return false;
        }
        let container = match self.window_endpoint_for_document(document) {
            Some(PendingWindowMessageEndpoint::TopWindow)
            | Some(PendingWindowMessageEndpoint::LightweightPopup(_)) => return true,
            Some(PendingWindowMessageEndpoint::ChildWindow(container)) => container,
            None => return false,
        };
        let Some(focused) = self.focused_frame_owner() else {
            return false;
        };
        focused == container
            || self
                .dom_host()
                .owner_document_handle(focused)
                .is_some_and(|owner| {
                    self.child_browsing_context_host_is_ancestor_of_document(container, owner)
                })
    }

    pub(crate) fn focused_frame_owner(&self) -> Option<DomHandle> {
        let PendingWindowMessageEndpoint::ChildWindow(handle) = self.focused_window else {
            return None;
        };
        (self.dom_host().is_connected(handle)
            && self
                .dom_host()
                .owner_document_handle(handle)
                .is_some_and(|document| self.document_is_fully_active(document)))
        .then_some(handle)
    }

    fn window_endpoint_for_focus_target(
        &self,
        handle: DomHandle,
    ) -> Option<PendingWindowMessageEndpoint> {
        let dom = self.dom_host();
        if dom.is_html_element_named(handle, "iframe") || dom.is_html_element_named(handle, "frame")
        {
            Some(PendingWindowMessageEndpoint::ChildWindow(handle))
        } else {
            self.window_endpoint_for_document(dom.owner_document_handle(handle)?)
        }
    }

    pub(crate) fn is_focused_window(&self, window: PendingWindowMessageEndpoint) -> bool {
        self.focused_window == window
    }

    /// Complete the Window transition before installing the element focus.
    /// Return the requested endpoint when Window callbacks ran.
    pub(crate) fn focus_window_for_element(
        scope: &mut v8::PinScope<'_, '_>,
        runtime_ptr: *mut Self,
        handle: DomHandle,
    ) -> Option<PendingWindowMessageEndpoint> {
        let runtime = unsafe { &mut *runtime_ptr };
        let next = runtime.window_endpoint_for_focus_target(handle)?;
        let previous = if runtime.focused_window == next {
            None
        } else {
            let previous = std::mem::replace(&mut runtime.focused_window, next);
            // A blur listener can redirect focus before the selected Window
            // has had its own focus event.
            std::mem::replace(&mut runtime.focused_window_notified, false).then_some(previous)
        };
        if let Some(previous) = previous {
            Self::dispatch_window_focus_event(scope, runtime_ptr, previous, "blur");
        }
        let runtime = unsafe { &mut *runtime_ptr };
        let dispatch_focus = runtime.is_focused_window(next)
            && !std::mem::replace(&mut runtime.focused_window_notified, true);
        if dispatch_focus {
            Self::dispatch_window_focus_event(scope, runtime_ptr, next, "focus");
        }
        (previous.is_some() || dispatch_focus).then_some(next)
    }

    fn dispatch_window_focus_event(
        scope: &mut v8::PinScope<'_, '_>,
        runtime_ptr: *mut Self,
        endpoint: PendingWindowMessageEndpoint,
        event_type: &str,
    ) {
        let event = construct_focus_event(scope, event_type, None, false)
            .expect("Window focus transition must materialize its FocusEvent");
        let runtime = unsafe { &mut *runtime_ptr };
        match endpoint {
            PendingWindowMessageEndpoint::TopWindow => {
                let _ = runtime.dispatch_public_event_best_effort(
                    scope,
                    runtime_ptr,
                    EventTargetHandle::Window,
                    event,
                    "window focus event",
                );
            }
            PendingWindowMessageEndpoint::ChildWindow(handle) => {
                runtime.dispatch_child_window_event(scope, handle, event_type, event);
            }
            PendingWindowMessageEndpoint::LightweightPopup(popup_id) => {
                let _ = runtime
                    .dispatch_lightweight_popup_window_event(scope, popup_id, event_type, event);
            }
        }
    }

    pub(crate) fn unfocus_frame_owner(&mut self, handle: DomHandle) {
        if self.focused_window == PendingWindowMessageEndpoint::ChildWindow(handle) {
            self.focused_window = self
                .dom_host()
                .owner_document_handle(handle)
                .and_then(|document| self.window_endpoint_for_document(document))
                .unwrap_or(PendingWindowMessageEndpoint::TopWindow);
            self.focused_window_notified = true;
        }
    }

    pub(super) fn clear_focused_window(&mut self, endpoint: PendingWindowMessageEndpoint) {
        if self.focused_window == endpoint {
            self.focused_window = PendingWindowMessageEndpoint::TopWindow;
            self.focused_window_notified = true;
        }
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
