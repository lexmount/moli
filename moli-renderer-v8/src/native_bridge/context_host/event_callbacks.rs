use super::{JsContextHost, WindowExecutionContextIdentity, WindowExecutionContextOwner};
use moli_webidl_callback::{PreparedWebIdlCallbackInterface, WebIdlCallbackInterface};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct EventCallbackId(u64);

struct EventCallbackRecord {
    callback: WebIdlCallbackInterface,
    relevant_identity: Option<WindowExecutionContextIdentity>,
    #[cfg(test)]
    incumbent_identity: Option<WindowExecutionContextIdentity>,
}

pub(crate) struct PreparedEventCallback {
    callback: PreparedWebIdlCallbackInterface,
    relevant_identity: Option<WindowExecutionContextIdentity>,
}

pub(crate) struct WindowErrorReportingScope {
    host_ptr: *mut JsContextHost,
    owner: WindowExecutionContextOwner,
}

pub(crate) struct ExplicitEventDispatchScope {
    host_ptr: *mut JsContextHost,
}

pub(crate) struct EventCallbackInvocationScope {
    host_ptr: *mut JsContextHost,
    outermost: bool,
}

impl EventCallbackInvocationScope {
    pub(crate) fn is_outermost(&self) -> bool {
        self.outermost
    }
}

impl Drop for EventCallbackInvocationScope {
    fn drop(&mut self) {
        let host = unsafe { &mut *self.host_ptr };
        host.event_callback_invocation_depth = host
            .event_callback_invocation_depth
            .checked_sub(1)
            .expect("event callback invocation scope must be active");
    }
}

impl Drop for ExplicitEventDispatchScope {
    fn drop(&mut self) {
        let host = unsafe { &mut *self.host_ptr };
        host.explicit_event_dispatch_depth = host
            .explicit_event_dispatch_depth
            .checked_sub(1)
            .expect("explicit EventTarget dispatch scope must be active");
    }
}

impl Drop for WindowErrorReportingScope {
    fn drop(&mut self) {
        let removed = unsafe { &mut *self.host_ptr }
            .active_window_error_report_owners
            .remove(&self.owner);
        debug_assert!(removed, "Window error reporting scope must be active");
    }
}

impl PreparedEventCallback {
    pub(crate) fn callback<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Object> {
        self.callback.callback(scope)
    }

    pub(crate) fn relevant_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Context> {
        self.callback.relevant_context(scope)
    }

    pub(crate) fn incumbent_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Context> {
        self.callback.incumbent_context(scope)
    }

    pub(crate) fn relevant_identity(&self) -> Option<WindowExecutionContextIdentity> {
        self.relevant_identity
    }

    pub(crate) fn is_callable(&self) -> bool {
        self.callback.callable_at_conversion()
    }
}

#[derive(Default)]
pub(super) struct EventCallbackRegistry {
    next_id: u64,
    records: HashMap<EventCallbackId, EventCallbackRecord>,
}

impl EventCallbackRegistry {
    fn allocate_id(&mut self) -> EventCallbackId {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("event callback id overflow");
        EventCallbackId(self.next_id)
    }

    fn take_owned_by(&mut self, owner: WindowExecutionContextOwner) -> HashSet<EventCallbackId> {
        let owned = self
            .records
            .iter()
            .filter_map(|(id, record)| {
                record
                    .relevant_identity
                    .is_some_and(|identity| identity.owner() == owner)
                    .then_some(*id)
            })
            .collect::<HashSet<_>>();
        self.records.retain(|id, _| !owned.contains(id));
        owned
    }
}

impl JsContextHost {
    pub(crate) fn enter_event_callback_invocation_scope(&mut self) -> EventCallbackInvocationScope {
        let outermost = self.event_callback_invocation_depth == 0;
        self.event_callback_invocation_depth += 1;
        EventCallbackInvocationScope {
            host_ptr: self,
            outermost,
        }
    }

    pub(crate) fn enter_explicit_event_dispatch_scope(&mut self) -> ExplicitEventDispatchScope {
        self.explicit_event_dispatch_depth += 1;
        ExplicitEventDispatchScope { host_ptr: self }
    }

    pub(crate) fn explicit_event_dispatch_is_active(&self) -> bool {
        self.explicit_event_dispatch_depth != 0
    }

    pub(crate) fn enter_window_error_reporting_scope(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) -> Option<WindowErrorReportingScope> {
        self.active_window_error_report_owners
            .insert(owner)
            .then(|| WindowErrorReportingScope {
                host_ptr: self,
                owner,
            })
    }

    pub(crate) fn register_target_event_listener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: crate::document_runtime::EventTargetHandle,
        event_type: &str,
        callback: v8::Local<'s, v8::Object>,
        relevant_context: v8::Local<'s, v8::Context>,
        incumbent_context: v8::Local<'s, v8::Context>,
        capture: bool,
        once: bool,
        passive: bool,
    ) -> Option<EventCallbackId> {
        let candidates = match target {
            crate::document_runtime::EventTargetHandle::ChildWindow(target) => {
                if !self.child_window_event_target_is_current(target) {
                    return None;
                }
                self.child_window_event_listener_callback_ids(
                    target.child_handle(),
                    event_type,
                    capture,
                )
            }
            crate::document_runtime::EventTargetHandle::Window
            | crate::document_runtime::EventTargetHandle::Node(_) => {
                self.event_listener_callback_ids(target, event_type, capture)
            }
        };
        if candidates
            .into_iter()
            .any(|id| self.event_callback_matches(scope, id, callback))
        {
            return None;
        }

        let callback_id =
            self.register_event_callback(scope, callback, relevant_context, incumbent_context);
        match target {
            crate::document_runtime::EventTargetHandle::ChildWindow(target) => {
                self.insert_child_window_event_listener(
                    target,
                    event_type,
                    callback_id,
                    capture,
                    once,
                );
            }
            crate::document_runtime::EventTargetHandle::Window
            | crate::document_runtime::EventTargetHandle::Node(_) => {
                let registration = crate::host::EventListenerRegistration::new(
                    scope,
                    callback_id,
                    callback,
                    capture,
                    once,
                    passive,
                );
                self.insert_event_listener(target, event_type, registration);
            }
        }
        Some(callback_id)
    }

    pub(crate) fn remove_registered_event_listener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: crate::document_runtime::EventTargetHandle,
        event_type: &str,
        callback: v8::Local<'s, v8::Object>,
        capture: bool,
    ) -> bool {
        let candidates = match target {
            crate::document_runtime::EventTargetHandle::ChildWindow(target) => self
                .child_window_event_listener_callback_ids(
                    target.child_handle(),
                    event_type,
                    capture,
                ),
            crate::document_runtime::EventTargetHandle::Window
            | crate::document_runtime::EventTargetHandle::Node(_) => {
                self.event_listener_callback_ids(target, event_type, capture)
            }
        };
        let Some(callback_id) = candidates
            .into_iter()
            .find(|id| self.event_callback_matches(scope, *id, callback))
        else {
            return false;
        };
        let removed = match target {
            crate::document_runtime::EventTargetHandle::ChildWindow(target) => self
                .remove_child_window_event_listener_by_id(
                    target.child_handle(),
                    event_type,
                    callback_id,
                    capture,
                ),
            crate::document_runtime::EventTargetHandle::Window
            | crate::document_runtime::EventTargetHandle::Node(_) => {
                self.remove_event_listener_by_id(target, event_type, callback_id, capture)
            }
        };
        if removed {
            self.unregister_abort_target_listener(callback_id);
            self.release_event_callback(callback_id);
        }
        removed
    }

    pub(crate) fn set_registered_event_handler_property<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: crate::document_runtime::EventTargetHandle,
        event_type: &str,
        handler: Option<v8::Local<'s, v8::Function>>,
    ) {
        let relevant_context = handler
            .and_then(|handler| v8::Local::<v8::Object>::from(handler).get_creation_context(scope))
            .unwrap_or_else(|| scope.get_current_context());
        let incumbent_context = scope
            .get_incumbent_context()
            .unwrap_or_else(|| scope.get_current_context());
        self.set_registered_event_handler_property_with_contexts(
            scope,
            target,
            event_type,
            handler,
            relevant_context,
            incumbent_context,
        );
    }

    pub(crate) fn set_registered_content_attribute_event_handler_property<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: crate::document_runtime::EventTargetHandle,
        event_type: &str,
        handler: Option<v8::Local<'s, v8::Function>>,
        target_context: v8::Local<'s, v8::Context>,
    ) {
        let callback_id = handler.map(|handler| {
            self.register_event_callback(scope, handler.into(), target_context, target_context)
        });
        if let Some(previous) =
            self.set_compiled_event_handler_property(target, event_type, callback_id)
        {
            self.release_event_callback(previous);
        }
    }

    fn set_registered_event_handler_property_with_contexts<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: crate::document_runtime::EventTargetHandle,
        event_type: &str,
        handler: Option<v8::Local<'s, v8::Function>>,
        relevant_context: v8::Local<'s, v8::Context>,
        incumbent_context: v8::Local<'s, v8::Context>,
    ) {
        let callback_id = handler.map(|handler| {
            self.register_event_callback(scope, handler.into(), relevant_context, incumbent_context)
        });
        if let Some(previous) = self.set_event_handler_property(target, event_type, callback_id) {
            self.release_event_callback(previous);
        }
    }

    pub(crate) fn registered_event_handler_property_value<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        target: crate::document_runtime::EventTargetHandle,
        event_type: &str,
    ) -> Option<v8::Local<'s, v8::Value>> {
        match self.event_handler_property_callback_id(target, event_type)? {
            Some(callback_id) => self.event_callback_value(scope, callback_id),
            None => Some(v8::null(scope).into()),
        }
    }

    pub(crate) fn register_event_callback<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: v8::Local<'s, v8::Object>,
        relevant_context: v8::Local<'s, v8::Context>,
        incumbent_context: v8::Local<'s, v8::Context>,
    ) -> EventCallbackId {
        let callback =
            WebIdlCallbackInterface::new(scope, callback, relevant_context, incumbent_context);
        self.register_webidl_event_callback(scope, callback)
    }

    /// Registers an already converted EventListener callback.
    ///
    /// EventTarget-like surfaces whose target storage is not the DOM event
    /// registry (for example AbortSignal) still share this callback residence.
    /// Their target-specific store owns ordering/options and keeps only the
    /// returned id.
    pub(crate) fn register_webidl_event_callback(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        callback: WebIdlCallbackInterface,
    ) -> EventCallbackId {
        let relevant_context = callback.relevant_context(scope);
        let relevant_identity =
            self.window_execution_context_identity_for_v8_context(scope, relevant_context);
        #[cfg(test)]
        let incumbent_identity = {
            let incumbent_context = callback.incumbent_context(scope);
            self.window_execution_context_identity_for_v8_context(scope, incumbent_context)
        };
        let id = self.event_callbacks.allocate_id();
        let previous = self.event_callbacks.records.insert(
            id,
            EventCallbackRecord {
                callback,
                relevant_identity,
                #[cfg(test)]
                incumbent_identity,
            },
        );
        debug_assert!(previous.is_none());
        id
    }

    pub(crate) fn release_event_callback(&mut self, id: EventCallbackId) {
        self.event_callbacks.records.remove(&id);
    }

    pub(in crate::native_bridge) fn clear_event_callbacks_for_document_replacement(
        &mut self,
        document_handle: crate::document_runtime::DomHandle,
        clear_window: bool,
    ) {
        let retired =
            self.clear_event_state_for_document_replacement(document_handle, clear_window);
        self.release_retired_event_callbacks(retired);
    }

    fn release_retired_event_callbacks(&mut self, retired: HashSet<EventCallbackId>) {
        self.bridge
            .abort
            .unregister_signal_event_callbacks(&retired);
        for callback_id in retired {
            self.unregister_abort_target_listener(callback_id);
            self.release_event_callback(callback_id);
        }
    }

    pub(crate) fn event_callback_matches<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: EventCallbackId,
        callback: v8::Local<'s, v8::Object>,
    ) -> bool {
        self.event_callbacks
            .records
            .get(&id)
            .is_some_and(|record| record.callback.matches(scope, callback))
    }

    pub(crate) fn event_callback_value<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: EventCallbackId,
    ) -> Option<v8::Local<'s, v8::Value>> {
        self.event_callbacks
            .records
            .get(&id)
            .map(|record| record.callback.value(scope))
    }

    pub(crate) fn event_callback_relevant_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: EventCallbackId,
    ) -> Option<v8::Local<'s, v8::Context>> {
        self.event_callbacks
            .records
            .get(&id)
            .map(|record| record.callback.relevant_context(scope))
    }

    #[cfg(test)]
    pub(crate) fn event_callback_identities_for_test(
        &self,
        id: EventCallbackId,
    ) -> Option<(
        Option<WindowExecutionContextIdentity>,
        Option<WindowExecutionContextIdentity>,
    )> {
        self.event_callbacks
            .records
            .get(&id)
            .map(|record| (record.relevant_identity, record.incumbent_identity))
    }

    pub(crate) fn prepare_event_callback(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: EventCallbackId,
    ) -> Option<PreparedEventCallback> {
        let record = self.event_callbacks.records.get(&id)?;
        if record
            .relevant_identity
            .is_some_and(|identity| !self.window_execution_context_identity_is_current(identity))
        {
            return None;
        }
        Some(PreparedEventCallback {
            callback: record.callback.prepare(scope),
            relevant_identity: record.relevant_identity,
        })
    }

    pub(in crate::native_bridge::context_host) fn retire_event_callbacks_for_execution_context(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) {
        let retired = self.event_callbacks.take_owned_by(owner);
        if retired.is_empty() {
            return;
        }
        self.remove_event_callback_registrations(&retired);
        self.retire_child_window_event_callbacks(&retired);
        self.remove_message_port_event_callbacks(&retired);
        self.bridge
            .abort
            .unregister_signal_event_callbacks(&retired);
        for callback_id in retired {
            self.unregister_abort_target_listener(callback_id);
        }
    }
}
