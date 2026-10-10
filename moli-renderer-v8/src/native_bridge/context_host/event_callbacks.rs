use super::{JsContextHost, WindowExecutionContextIdentity, WindowExecutionContextOwner};
use moli_webidl_callback::{PreparedWebIdlCallbackInterface, WebIdlCallbackInterface};
use std::collections::{HashMap, HashSet};
use std::{
    cell::RefCell,
    ops::DerefMut,
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct EventCallbackId(u64);

struct EventCallbackRecord {
    callback: WebIdlCallbackInterface,
    relevant_identity: Option<WindowExecutionContextIdentity>,
    relevant_host: Option<Weak<RefCell<JsContextHost>>>,
    #[cfg(test)]
    incumbent_identity: Option<WindowExecutionContextIdentity>,
}

pub(crate) struct PreparedEventCallback {
    callback: PreparedWebIdlCallbackInterface,
    relevant_identity: Option<WindowExecutionContextIdentity>,
    relevant_host: Option<Rc<RefCell<JsContextHost>>>,
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

    pub(crate) fn execution_context_host(
        &self,
        target_host: *mut JsContextHost,
    ) -> *mut JsContextHost {
        self.relevant_host
            .as_ref()
            .map_or(target_host, |host| host.as_ptr())
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
                if record.relevant_host.is_some() {
                    return None;
                }
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
        self.set_registered_event_handler_property_with_contexts(
            scope,
            target,
            event_type,
            handler,
            target_context,
            target_context,
        );
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
        let relevant_host = crate::util::context_host_ptr_from_context_slot(relevant_context)
            .filter(|pointer| !std::ptr::eq(*pointer, self))
            .and_then(|pointer| {
                self.page_script_environment()?
                    .related_document_hosts()
                    .into_iter()
                    .find(|host| host.as_ptr() == pointer)
            });
        let identity_host = relevant_host
            .as_ref()
            .map_or(self as *const Self, |host| host.as_ptr());
        let relevant_identity = unsafe { &*identity_host }
            .window_execution_context_identity_for_v8_context(scope, relevant_context);
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
                relevant_host: relevant_host.as_ref().map(Rc::downgrade),
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
        let relevant_host = if let Some(owner) = &record.relevant_host {
            let owner = owner.upgrade()?;
            if record.relevant_identity.is_some_and(|identity| {
                !unsafe { &*owner.as_ptr() }.window_execution_context_identity_is_current(identity)
            }) {
                return None;
            }
            Some(owner)
        } else {
            if record.relevant_identity.is_some_and(|identity| {
                !self.window_execution_context_identity_is_current(identity)
            }) {
                return None;
            }
            None
        };
        Some(PreparedEventCallback {
            callback: record.callback.prepare(scope),
            relevant_identity: record.relevant_identity,
            relevant_host,
        })
    }

    pub(crate) fn adopt_node_event_target_from(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        source: &Rc<RefCell<JsContextHost>>,
        source_handle: crate::document_runtime::DomHandle,
        handle: crate::document_runtime::DomHandle,
    ) {
        let source_ptr = source.as_ptr();
        let target_ptr = self as *mut Self;
        if source_ptr == target_ptr {
            return;
        }
        let mut callbacks = HashMap::new();
        for snapshot in unsafe { &*source_ptr }.inspector_event_listener_snapshots(
            crate::document_runtime::EventTargetHandle::Node(source_handle),
        ) {
            let old_id = snapshot.callback_id;
            if callbacks.contains_key(&old_id) {
                continue;
            }
            let Some(record) = unsafe { &*source_ptr }.event_callbacks.records.get(&old_id) else {
                continue;
            };
            let callback_object = v8::Local::<v8::Object>::try_from(record.callback.value(scope))
                .expect("event callback object");
            let relevant_context = record.callback.relevant_context(scope);
            let incumbent_context = record.callback.incumbent_context(scope);
            let callback = WebIdlCallbackInterface::new(
                scope,
                callback_object,
                relevant_context,
                incumbent_context,
            );
            let relevant_host = record
                .relevant_host
                .clone()
                .unwrap_or_else(|| Rc::downgrade(source));
            let relevant_host = relevant_host
                .upgrade()
                .is_none_or(|host| host.as_ptr() != target_ptr)
                .then_some(relevant_host);
            let new_id = self.event_callbacks.allocate_id();
            self.event_callbacks.records.insert(
                new_id,
                EventCallbackRecord {
                    callback,
                    relevant_identity: record.relevant_identity,
                    relevant_host,
                    #[cfg(test)]
                    incumbent_identity: record.incumbent_identity,
                },
            );
            callbacks.insert(old_id, new_id);
        }
        let source = unsafe { &mut *source_ptr };
        self.deref_mut()
            .adopt_event_target_from(source.deref_mut(), source_handle, handle, |id| {
                callbacks.get(&id).copied()
            });
        if let Some(environment) = self.page_script_environment() {
            let hosts = environment.related_document_hosts();
            if let Some(target) = hosts.iter().find(|host| host.as_ptr() == target_ptr) {
                for host in &hosts {
                    let pointer = host.as_ptr();
                    unsafe { &mut *pointer }
                        .native_bridge_mut()
                        .abort
                        .retarget_adopted_event_listeners(
                            pointer,
                            source_ptr,
                            crate::document_runtime::EventTargetHandle::Node(source_handle),
                            Rc::downgrade(target),
                            crate::document_runtime::EventTargetHandle::Node(handle),
                            &callbacks,
                        );
                }
            }
        }
        for id in callbacks.into_keys() {
            source.release_event_callback(id);
        }
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
        for callback_id in retired {
            self.unregister_abort_target_listener(callback_id);
        }
    }
}
