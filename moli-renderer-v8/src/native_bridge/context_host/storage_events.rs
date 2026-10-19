use super::*;
use crate::{
    context_bootstrap::{construct_original_storage_event_utf16, mark_event_trusted},
    document_runtime::EventTargetHandle,
    page_task_queue::RendererPageStorageEventData,
    util::v8str,
};

impl JsContextHost {
    pub(super) fn attach_window_storage(&mut self, target: WindowTaskTarget) {
        let (storage, session_override) = match target.dispatch_scope() {
            OwnerDispatchScope::Top => (self.top_document_storage_context(), None),
            OwnerDispatchScope::Child(handle) => {
                let Some(storage) = self.storage_context_for_child_browsing_context(handle) else {
                    return;
                };
                (storage, None)
            }
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                let Some(storage) = self.storage_context_for_lightweight_popup(popup_id) else {
                    return;
                };
                (
                    storage,
                    self.lightweight_popup_session_storage_store(popup_id),
                )
            }
        };
        if moli_storage_key::serialized_storage_key_has_opaque_origin(
            &storage.storage_key().serialized_storage_key(),
        ) {
            self.window_storage.get_mut().retire(target.owner());
            return;
        }
        self.window_storage.get_mut().attach(
            target,
            storage.web_storage_area_key(),
            session_override,
        );
    }

    pub(crate) fn window_storage_area(
        &mut self,
        target: WindowTaskTarget,
        is_session: bool,
    ) -> Option<crate::context_bootstrap::WebStorageArea> {
        if !self.window_execution_context_owner_is_current(target.owner(), target.dispatch_scope())
            || self.browsing_context_is_closed()
        {
            return None;
        }
        if let Some(area) = self.window_storage.borrow().area(target, is_session) {
            return Some(area);
        }
        self.attach_window_storage(target);
        self.window_storage.borrow().area(target, is_session)
    }

    /// Like Blink's Window storage controller, a listener establishes both
    /// subscriptions even when no Storage getter has ever been evaluated.
    pub(super) fn subscribe_window_storage_listener(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        target: EventTargetHandle,
        event_type: &str,
    ) {
        if event_type != "storage" {
            return;
        }
        let dispatch_scope = match target {
            EventTargetHandle::Window => crate::native_bridge::active_lightweight_popup_id(scope)
                .map(OwnerDispatchScope::LightweightPopup)
                .unwrap_or(OwnerDispatchScope::Top),
            EventTargetHandle::ChildWindow(target) => {
                OwnerDispatchScope::Child(target.child_handle())
            }
            EventTargetHandle::Node(_) => return,
        };
        if let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) {
            self.attach_window_storage(WindowTaskTarget::new(dispatch_scope, owner));
        }
    }

    /// Dispatch one StorageEvent after the Page arbiter has authorized its
    /// exact LocalDOMWindow. This method resolves that Window's default realm;
    /// it does not perform a second current/stale decision.
    pub(crate) fn dispatch_authorized_storage_event_delivery(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowTaskTarget,
        data: &RendererPageStorageEventData,
    ) -> bool {
        match target.dispatch_scope() {
            OwnerDispatchScope::Top => {
                let Some((_, context)) =
                    self.window_execution_context(scope, target.owner(), target.dispatch_scope())
                else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                let global = scope.get_current_context().global(scope);
                let Some(event) = self.storage_event_for_target(scope, global, data) else {
                    return false;
                };
                self.dispatch_public_event(scope, host_ptr, EventTargetHandle::Window, event)
                    .is_ok()
            }
            OwnerDispatchScope::Child(handle) => {
                if self
                    .ensure_prebootstrapped_child_default_context(scope, handle)
                    .is_err()
                {
                    return false;
                }
                let Some((_, context)) =
                    self.window_execution_context(scope, target.owner(), target.dispatch_scope())
                else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                let Some(window) =
                    self.existing_child_browsing_context_window_wrapper(scope, handle)
                else {
                    return false;
                };
                let Some(event) = self.storage_event_for_target(scope, window, data) else {
                    return false;
                };
                self.dispatch_child_window_event(scope, handle, "storage", event);
                true
            }
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                if !self.ensure_lightweight_popup_execution_context(scope, popup_id) {
                    return false;
                }
                let Some((_, context)) =
                    self.window_execution_context(scope, target.owner(), target.dispatch_scope())
                else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
                    return false;
                };
                let Some(event) = self.storage_event_for_target(scope, window, data) else {
                    return false;
                };
                self.dispatch_lightweight_popup_window_event(scope, popup_id, "storage", event);
                true
            }
        }
    }

    fn storage_event_for_target<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        target: v8::Local<'s, v8::Object>,
        data: &RendererPageStorageEventData,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let storage_name = if data.is_session() {
            "sessionStorage"
        } else {
            "localStorage"
        };
        let storage_area = target.get(scope, v8str(scope, storage_name).into());
        let event = construct_original_storage_event_utf16(
            scope,
            "storage",
            data.key(),
            data.old_value(),
            data.new_value(),
            data.url(),
            storage_area,
        )?;
        mark_event_trusted(scope, event);
        Some(event)
    }
}
