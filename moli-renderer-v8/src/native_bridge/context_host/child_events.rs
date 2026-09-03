use super::{
    JsContextHost, OwnerDispatchScope, child_frame_runtime::WINDOW_EVENT_HANDLER_PROPERTIES,
};
use crate::{
    context_bootstrap::{EventHandlerType, apply_event_handler_return_value, event_is_error_event},
    document_runtime::DomHandle,
    document_runtime::EventTargetHandle,
    exception_reporting::invoke_event_handler,
    frame_owner_model::{FrameDocumentTaskOwner, LocalWindowId},
    host::{
        ChildWindowEventTarget, DispatchStatus, create_host_event, event_dispatch_status,
        invoke_prepared_before_unload_event_handler, invoke_prepared_event_callback,
    },
    native_bridge::{
        ACTIVE_CHILD_WINDOW_HANDLE_SLOT, EventCallbackId, PreparedEventCallback,
        element::EventAttributeHandlerScope, element::compile_event_attribute_handler_for_owner,
    },
    util::{get_private_value, set_private_value, v8_string, v8str},
};
use std::{collections::HashSet, convert::TryFrom};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChildWindowEventRegistrationKind {
    EventListener,
    EventHandlerProperty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChildWindowEventRegistrationId(u64);

pub(super) struct ChildWindowEventListenerEntry {
    registration_id: ChildWindowEventRegistrationId,
    callback_id: EventCallbackId,
    registration_kind: ChildWindowEventRegistrationKind,
    local_window_id: Option<LocalWindowId>,
    capture: bool,
    once: bool,
}

struct ChildWindowEventListenerSnapshot {
    callback_id: EventCallbackId,
    registration_kind: ChildWindowEventRegistrationKind,
    local_window_id: Option<LocalWindowId>,
    once: bool,
}

#[derive(Clone, Copy)]
struct ChildWindowEventDispatchSlot {
    registration_id: ChildWindowEventRegistrationId,
}

struct ReadyChildWindowEventListenerInvocation {
    registration_id: ChildWindowEventRegistrationId,
    registration_kind: ChildWindowEventRegistrationKind,
    once: bool,
    target: ChildWindowEventTarget,
    callback: PreparedEventCallback,
}

impl JsContextHost {
    fn child_window_event_requires_runtime_dispatch(
        &self,
        handle: DomHandle,
        event_type: &str,
    ) -> bool {
        if self.child_window_proxy_records.has_live_window(handle)
            || self
                .child_window_event_listeners
                .get(&handle)
                .and_then(|listeners| listeners.get(event_type))
                .is_some_and(|listeners| !listeners.is_empty())
        {
            return true;
        }
        let body_attribute = match event_type {
            "load" => "onload",
            "storage" => "onstorage",
            _ => return false,
        };
        self.child_browsing_context_document_handle(handle)
            .into_iter()
            .flat_map(|document| {
                self.dom_host()
                    .elements_by_tag_name(document, "body", false)
            })
            .any(|body| {
                self.dom_host()
                    .get_attribute(body, body_attribute)
                    .is_some_and(|source| !source.trim().is_empty())
            })
    }

    fn allocate_child_window_event_registration_id(&mut self) -> ChildWindowEventRegistrationId {
        self.next_child_window_event_registration_id = self
            .next_child_window_event_registration_id
            .checked_add(1)
            .expect("child window event registration id overflow");
        ChildWindowEventRegistrationId(self.next_child_window_event_registration_id)
    }

    pub(in crate::native_bridge::context_host) fn clear_child_window_document_event_state(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        if let Some(window) = self.child_window_proxy_records.live_window(scope, handle) {
            let null = v8::null(scope).into();
            for name in WINDOW_EVENT_HANDLER_PROPERTIES {
                let _ = window.set(scope, v8str(scope, name).into(), null);
            }
        }
        self.clear_child_window_event_listeners(handle);
    }

    pub(crate) fn child_window_event_listener_callback_ids(
        &self,
        handle: DomHandle,
        event_type: &str,
        capture: bool,
    ) -> Vec<EventCallbackId> {
        self.child_window_event_listeners
            .get(&handle)
            .and_then(|target_map| target_map.get(event_type))
            .into_iter()
            .flatten()
            .filter(|entry| {
                entry.registration_kind == ChildWindowEventRegistrationKind::EventListener
                    && entry.capture == capture
            })
            .map(|entry| entry.callback_id)
            .collect()
    }

    pub(crate) fn child_window_inspector_listener_snapshots(
        &self,
        target: ChildWindowEventTarget,
    ) -> Vec<crate::host::EventListenerInspectorSnapshot> {
        if !self.child_window_event_target_is_current(target) {
            return Vec::new();
        }
        self.child_window_event_listeners
            .get(&target.child_handle())
            .into_iter()
            .flat_map(|target_map| target_map.iter())
            .flat_map(|(event_type, entries)| {
                entries
                    .iter()
                    .filter(|entry| entry.local_window_id == Some(target.owner().local_window_id))
                    .map(|entry| crate::host::EventListenerInspectorSnapshot {
                        registration_id: entry.registration_id.0,
                        event_type: event_type.clone(),
                        callback_id: entry.callback_id,
                        capture: entry.capture,
                        once: entry.once,
                        passive: false,
                    })
            })
            .collect()
    }

    pub(crate) fn insert_child_window_event_listener(
        &mut self,
        target: ChildWindowEventTarget,
        event_type: &str,
        callback_id: EventCallbackId,
        capture: bool,
        once: bool,
    ) {
        let registration_id = self.allocate_child_window_event_registration_id();
        self.child_window_event_listeners
            .entry(target.child_handle())
            .or_default()
            .entry(event_type.to_owned())
            .or_default()
            .push(ChildWindowEventListenerEntry {
                registration_id,
                callback_id,
                registration_kind: ChildWindowEventRegistrationKind::EventListener,
                local_window_id: Some(target.owner().local_window_id),
                capture,
                once,
            });
    }

    pub(crate) fn set_child_window_event_handler_property<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        handler_name: &str,
        handler: Option<v8::Local<'s, v8::Function>>,
        callback_relevant_context: v8::Local<'s, v8::Context>,
    ) {
        let Some(event_type) = child_window_event_type_from_handler_name(handler_name) else {
            return;
        };
        let Some(handler) = handler else {
            if let Some(callback_id) =
                self.remove_child_window_event_handler_property(handle, event_type)
            {
                self.release_event_callback(callback_id);
            }
            return;
        };
        let owner = self.frame_owner_current_child_snapshot(handle);
        let local_window_id = owner.as_ref().map(|owner| owner.local_window_id);
        let incumbent_context = scope
            .get_incumbent_context()
            .unwrap_or_else(|| scope.get_current_context());
        let callback = v8::Local::<v8::Object>::from(handler);
        let callback_id = self.register_event_callback(
            scope,
            callback,
            callback_relevant_context,
            incumbent_context,
        );
        let previous_callback_id = self
            .child_window_event_listeners
            .get_mut(&handle)
            .and_then(|target_map| target_map.get_mut(event_type))
            .and_then(|entries| {
                entries.iter_mut().find(|entry| {
                    entry.registration_kind
                        == ChildWindowEventRegistrationKind::EventHandlerProperty
                })
            })
            .map(|entry| {
                let previous_callback_id = entry.callback_id;
                entry.callback_id = callback_id;
                entry.local_window_id = local_window_id;
                previous_callback_id
            });
        if let Some(previous_callback_id) = previous_callback_id {
            self.release_event_callback(previous_callback_id);
            return;
        }
        let registration_id = self.allocate_child_window_event_registration_id();
        self.child_window_event_listeners
            .entry(handle)
            .or_default()
            .entry(event_type.to_owned())
            .or_default()
            .push(ChildWindowEventListenerEntry {
                registration_id,
                callback_id,
                registration_kind: ChildWindowEventRegistrationKind::EventHandlerProperty,
                local_window_id,
                capture: false,
                once: false,
            });
    }

    pub(crate) fn child_window_event_handler_property_value<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        handler_name: &str,
    ) -> Option<v8::Local<'s, v8::Value>> {
        let event_type = child_window_event_type_from_handler_name(handler_name)?;
        self.child_window_event_listeners
            .get(&handle)
            .and_then(|target_map| target_map.get(event_type))
            .and_then(|entries| {
                entries.iter().find_map(|entry| {
                    (entry.registration_kind
                        == ChildWindowEventRegistrationKind::EventHandlerProperty)
                        .then(|| self.event_callback_value(scope, entry.callback_id))
                        .flatten()
                })
            })
    }

    fn remove_child_window_event_handler_property(
        &mut self,
        handle: DomHandle,
        event_type: &str,
    ) -> Option<EventCallbackId> {
        let target_map = self.child_window_event_listeners.get_mut(&handle)?;
        let entries = target_map.get_mut(event_type)?;
        let callback_id = entries
            .iter()
            .find(|entry| {
                entry.registration_kind == ChildWindowEventRegistrationKind::EventHandlerProperty
            })
            .map(|entry| entry.callback_id);
        entries.retain(|entry| {
            entry.registration_kind != ChildWindowEventRegistrationKind::EventHandlerProperty
        });
        if entries.is_empty() {
            target_map.shift_remove(event_type);
        }
        if target_map.is_empty() {
            self.child_window_event_listeners.remove(&handle);
        }
        callback_id
    }

    pub(crate) fn current_child_window_event_target(
        &self,
        child_handle: DomHandle,
    ) -> Option<ChildWindowEventTarget> {
        let owner = self.frame_owner_current_child_snapshot(child_handle)?;
        Some(ChildWindowEventTarget::new(
            child_handle,
            FrameDocumentTaskOwner::new(
                owner.scheduler_lane_id,
                owner.local_window_id,
                owner.document_id,
            ),
        ))
    }

    pub(crate) fn child_window_event_target_is_current(
        &self,
        target: ChildWindowEventTarget,
    ) -> bool {
        self.frame_owner_store
            .child_document_task_owner_is_current(target.child_handle(), target.owner())
    }

    pub(crate) fn child_window_event_target_wrapper<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: ChildWindowEventTarget,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if !self.child_window_event_target_is_current(target) {
            return None;
        }
        let current_context = scope.get_current_context();
        let current_global = current_context.global(scope);
        if self
            .window_execution_context_identity_for_v8_context(scope, current_context)
            .is_some_and(|identity| {
                identity.owner()
                    == crate::native_bridge::WindowExecutionContextOwner::Frame(
                        target.owner().local_window_id,
                    )
                    && identity.dispatch_scope()
                        == crate::native_bridge::OwnerDispatchScope::Child(target.child_handle())
            })
        {
            return Some(current_global);
        }
        self.child_browsing_context_window_wrapper(scope, target.child_handle())
    }

    pub(crate) fn dispatch_child_document_event_for_owner<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        document: v8::Local<'s, v8::Object>,
        event_type: &str,
        bubbles: bool,
        cancelable: bool,
    ) -> bool {
        let Some(snapshot) = self.frame_owner_current_child_snapshot(child_handle) else {
            return false;
        };
        if snapshot.scheduler_lane_id != owner.scheduler_lane_id
            || snapshot.local_window_id != owner.local_window_id
            || snapshot.document_id != owner.document_id
            || self.child_browsing_context_document_handle(child_handle)
                != Some(snapshot.document_handle)
        {
            return false;
        }
        let Ok(event) = create_host_event(
            scope,
            event_type,
            document.into(),
            document.into(),
            bubbles,
            cancelable,
        ) else {
            return false;
        };
        let host_ptr = self as *mut JsContextHost;
        self.dispatch_public_event(
            scope,
            host_ptr,
            EventTargetHandle::Node(snapshot.document_handle),
            event,
        )
        .is_ok()
    }

    pub(crate) fn call_child_window_event_path_listeners<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: ChildWindowEventTarget,
        event_type: &str,
        event: v8::Local<'s, v8::Object>,
        capture_only: bool,
        at_target: bool,
    ) -> DispatchStatus {
        if !self.child_window_event_target_is_current(target) {
            return DispatchStatus::StopPropagation;
        }
        let Some(_window) = self.child_window_event_target_wrapper(scope, target) else {
            return DispatchStatus::StopPropagation;
        };
        if at_target && !capture_only {
            return DispatchStatus::Continue;
        }
        let registration_kind =
            (!at_target && capture_only).then_some(ChildWindowEventRegistrationKind::EventListener);
        let capture = (!at_target).then_some(capture_only);
        let dispatch_slots = self.child_window_event_dispatch_slots(
            target.child_handle(),
            event_type,
            registration_kind,
            capture,
        );
        if dispatch_slots.is_empty() {
            return DispatchStatus::Continue;
        }

        let previous_active_child_window =
            enter_child_window_event_dispatch(scope, target.child_handle());
        self.push_child_subresource_request_scope(target.child_handle());
        let mut status = DispatchStatus::Continue;
        for slot in dispatch_slots {
            let Some(ready) = self.prepare_child_window_event_listener_invocation(
                scope,
                target.child_handle(),
                event_type,
                slot,
            ) else {
                if let Some(callback_id) = self.remove_child_window_event_registration_by_id(
                    target.child_handle(),
                    event_type,
                    slot.registration_id,
                ) {
                    self.unregister_abort_target_listener(callback_id);
                    self.release_event_callback(callback_id);
                }
                continue;
            };
            if ready.once
                && let Some(callback_id) = self.remove_child_window_event_registration_by_id(
                    target.child_handle(),
                    event_type,
                    ready.registration_id,
                )
            {
                self.unregister_abort_target_listener(callback_id);
                self.release_event_callback(callback_id);
            }
            let registration_kind = ready.registration_kind;
            let (invoked, returned) =
                self.invoke_ready_child_window_event_listener(scope, ready, event_type, event);
            if registration_kind == ChildWindowEventRegistrationKind::EventHandlerProperty {
                apply_child_window_event_handler_return(scope, event_type, event, returned);
            }
            let callback_status = event_dispatch_status(scope, event);
            if !invoked || !self.child_window_event_target_is_current(target) {
                status = DispatchStatus::StopPropagation;
                break;
            }
            if callback_status == DispatchStatus::StopImmediate {
                status = DispatchStatus::StopImmediate;
                break;
            }
            if callback_status == DispatchStatus::StopPropagation {
                status = DispatchStatus::StopPropagation;
            }
        }
        self.pop_child_subresource_request_scope();
        restore_child_window_event_dispatch(scope, previous_active_child_window);
        status
    }

    pub(crate) fn dispatch_child_window_event<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        event_type: &str,
        event: v8::Local<'s, v8::Object>,
    ) {
        if !self.child_window_event_requires_runtime_dispatch(handle, event_type) {
            return;
        }
        let Some(window) = self.child_browsing_context_window_wrapper(scope, handle) else {
            return;
        };
        let Some(dispatch_target) = self.current_child_window_event_target(handle) else {
            return;
        };
        let previous_active_child_window = enter_child_window_event_dispatch(scope, handle);
        self.push_child_subresource_request_scope(handle);
        let target = if event_type == "unload" {
            self.child_browsing_context_document_wrapper(scope, handle)
                .map(Into::into)
                .unwrap_or_else(|| window.into())
        } else {
            window.into()
        };
        let _ = event.set(scope, v8str(scope, "target").into(), target);
        let _ = event.set(scope, v8str(scope, "currentTarget").into(), window.into());

        if event_type == "load" {
            install_child_body_load_attribute_handler_if_needed(scope, self, handle);
        }

        if event_type == "storage" {
            dispatch_child_body_storage_attribute(scope, window, event);
        }

        let dispatch_slots = self.child_window_event_dispatch_slots(handle, event_type, None, None);
        for slot in dispatch_slots {
            let Some(ready) = self
                .prepare_child_window_event_listener_invocation(scope, handle, event_type, slot)
            else {
                if let Some(callback_id) = self.remove_child_window_event_registration_by_id(
                    handle,
                    event_type,
                    slot.registration_id,
                ) {
                    self.unregister_abort_target_listener(callback_id);
                    self.release_event_callback(callback_id);
                }
                continue;
            };
            if ready.once
                && let Some(callback_id) = self.remove_child_window_event_registration_by_id(
                    handle,
                    event_type,
                    ready.registration_id,
                )
            {
                self.unregister_abort_target_listener(callback_id);
                self.release_event_callback(callback_id);
            }
            let registration_kind = ready.registration_kind;
            let (invoked, returned) =
                self.invoke_ready_child_window_event_listener(scope, ready, event_type, event);
            if registration_kind == ChildWindowEventRegistrationKind::EventHandlerProperty {
                apply_child_window_event_handler_return(scope, event_type, event, returned);
            }
            let status = event_dispatch_status(scope, event);
            if !invoked
                || !self.child_window_event_target_is_current(dispatch_target)
                || status == DispatchStatus::StopImmediate
            {
                break;
            }
        }
        self.pop_child_subresource_request_scope();
        restore_child_window_event_dispatch(scope, previous_active_child_window);
    }

    fn child_window_event_dispatch_slots(
        &self,
        handle: DomHandle,
        event_type: &str,
        registration_kind: Option<ChildWindowEventRegistrationKind>,
        capture: Option<bool>,
    ) -> Vec<ChildWindowEventDispatchSlot> {
        self.child_window_event_listeners
            .get(&handle)
            .and_then(|target_map| target_map.get(event_type))
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| {
                        registration_kind.is_none_or(|registration_kind| {
                            entry.registration_kind == registration_kind
                        })
                    })
                    .filter(|entry| capture.is_none_or(|capture| entry.capture == capture))
                    .map(|entry| ChildWindowEventDispatchSlot {
                        registration_id: entry.registration_id,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn prepare_child_window_event_listener_invocation(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        event_type: &str,
        slot: ChildWindowEventDispatchSlot,
    ) -> Option<ReadyChildWindowEventListenerInvocation> {
        let entry = self
            .child_window_event_listeners
            .get(&handle)
            .and_then(|target_map| target_map.get(event_type))?
            .iter()
            .find(|entry| entry.registration_id == slot.registration_id)?;
        let snapshot = ChildWindowEventListenerSnapshot {
            callback_id: entry.callback_id,
            registration_kind: entry.registration_kind,
            local_window_id: entry.local_window_id,
            once: entry.once,
        };
        let target = self.current_child_window_event_target(handle)?;
        if snapshot.local_window_id != Some(target.owner().local_window_id) {
            return None;
        }
        let callback = self.prepare_event_callback(scope, snapshot.callback_id)?;
        Some(ReadyChildWindowEventListenerInvocation {
            registration_id: entry.registration_id,
            registration_kind: snapshot.registration_kind,
            once: snapshot.once,
            target,
            callback,
        })
    }

    fn invoke_ready_child_window_event_listener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        ready: ReadyChildWindowEventListenerInvocation,
        event_type: &str,
        event: v8::Local<'s, v8::Object>,
    ) -> (bool, Option<v8::Global<v8::Value>>) {
        if !self.child_window_event_target_is_current(ready.target) {
            return (false, None);
        }
        if ready.registration_kind == ChildWindowEventRegistrationKind::EventHandlerProperty
            && event_type == "beforeunload"
        {
            invoke_prepared_before_unload_event_handler(
                scope,
                self as *mut JsContextHost,
                EventTargetHandle::ChildWindow(ready.target),
                false,
                event_type,
                &format!("child window {event_type} listener"),
                ready.callback,
                event,
            );
            return (true, None);
        }
        let arguments = child_window_event_callback_arguments(
            scope,
            ready.registration_kind,
            event_type,
            event,
        );
        let returned = invoke_prepared_event_callback(
            scope,
            self as *mut JsContextHost,
            false,
            event_type,
            &format!("child window {event_type} listener"),
            ready.callback,
            EventTargetHandle::ChildWindow(ready.target),
            event,
            &arguments,
        );
        (true, returned)
    }

    fn remove_child_window_event_registration_by_id(
        &mut self,
        handle: DomHandle,
        event_type: &str,
        registration_id: ChildWindowEventRegistrationId,
    ) -> Option<EventCallbackId> {
        let target_map = self.child_window_event_listeners.get_mut(&handle)?;
        let entries = target_map.get_mut(event_type)?;
        let position = entries
            .iter()
            .position(|entry| entry.registration_id == registration_id)?;
        let callback_id = entries.remove(position).callback_id;
        if entries.is_empty() {
            target_map.shift_remove(event_type);
        }
        if target_map.is_empty() {
            self.child_window_event_listeners.remove(&handle);
        }
        Some(callback_id)
    }

    pub(crate) fn remove_child_window_event_listener_by_id(
        &mut self,
        handle: DomHandle,
        event_type: &str,
        callback_id: EventCallbackId,
        capture: bool,
    ) -> bool {
        let registration_id = self
            .child_window_event_listeners
            .get(&handle)
            .and_then(|target_map| target_map.get(event_type))
            .and_then(|entries| {
                entries.iter().find_map(|entry| {
                    (entry.registration_kind == ChildWindowEventRegistrationKind::EventListener
                        && entry.callback_id == callback_id
                        && entry.capture == capture)
                        .then_some(entry.registration_id)
                })
            });
        registration_id.is_some_and(|registration_id| {
            self.remove_child_window_event_registration_by_id(handle, event_type, registration_id)
                .is_some()
        })
    }

    pub(in crate::native_bridge::context_host) fn clear_child_window_event_listeners(
        &mut self,
        handle: DomHandle,
    ) {
        let Some(target_map) = self.child_window_event_listeners.remove(&handle) else {
            return;
        };
        for callback_id in target_map
            .into_values()
            .flatten()
            .map(|entry| entry.callback_id)
        {
            self.unregister_abort_target_listener(callback_id);
            self.release_event_callback(callback_id);
        }
    }

    pub(in crate::native_bridge::context_host) fn retire_child_window_event_callbacks(
        &mut self,
        retired: &HashSet<EventCallbackId>,
    ) {
        for target_map in self.child_window_event_listeners.values_mut() {
            target_map.retain(|_, entries| {
                entries.retain(|entry| !retired.contains(&entry.callback_id));
                !entries.is_empty()
            });
        }
        self.child_window_event_listeners
            .retain(|_, target_map| !target_map.is_empty());
    }

    #[cfg(test)]
    pub(crate) fn child_window_event_callback_identities_for_test(
        &self,
        handle: DomHandle,
        event_type: &str,
    ) -> Vec<(
        Option<crate::native_bridge::WindowExecutionContextIdentity>,
        Option<crate::native_bridge::WindowExecutionContextIdentity>,
    )> {
        self.child_window_event_listeners
            .get(&handle)
            .and_then(|target_map| target_map.get(event_type))
            .into_iter()
            .flatten()
            .filter_map(|entry| self.event_callback_identities_for_test(entry.callback_id))
            .collect()
    }
}

fn child_window_event_type_from_handler_name(name: &str) -> Option<&str> {
    name.strip_prefix("on")
        .filter(|event_type| !event_type.is_empty())
}

fn child_window_event_callback_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration_kind: ChildWindowEventRegistrationKind,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Value>> {
    if registration_kind == ChildWindowEventRegistrationKind::EventHandlerProperty
        && event_type == "error"
        && event_is_error_event(scope, event)
    {
        vec![
            event
                .get(scope, v8str(scope, "message").into())
                .unwrap_or_else(|| v8::undefined(scope).into()),
            event
                .get(scope, v8str(scope, "filename").into())
                .unwrap_or_else(|| v8::undefined(scope).into()),
            event
                .get(scope, v8str(scope, "lineno").into())
                .unwrap_or_else(|| v8::Number::new(scope, 0.0).into()),
            event
                .get(scope, v8str(scope, "colno").into())
                .unwrap_or_else(|| v8::Number::new(scope, 0.0).into()),
            event
                .get(scope, v8str(scope, "error").into())
                .unwrap_or_else(|| v8::null(scope).into()),
        ]
    } else {
        vec![event.into()]
    }
}

fn apply_child_window_event_handler_return<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
    returned: Option<v8::Global<v8::Value>>,
) {
    let Some(returned) = returned else {
        return;
    };
    let returned = v8::Local::new(scope, returned);
    let handler_type = if event_type == "error" && event_is_error_event(scope, event) {
        EventHandlerType::OnErrorEventHandler
    } else {
        EventHandlerType::EventHandler
    };
    apply_event_handler_return_value(scope, event, returned, handler_type);
}

fn install_child_body_load_attribute_handler_if_needed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    handle: DomHandle,
) {
    let Some(source) = host
        .child_browsing_context_document_handle(handle)
        .and_then(|document| {
            host.dom_host()
                .elements_by_tag_name(document, "body", false)
                .into_iter()
                .next()
        })
        .and_then(|body| host.dom_host().get_attribute(body, "onload"))
        .filter(|source| !source.trim().is_empty())
    else {
        return;
    };
    let Ok(context) = host.ensure_prebootstrapped_child_default_context(scope, handle) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(window) = host.child_window_proxy_records.live_window(scope, handle) else {
        return;
    };
    if let Some(current) = window.get(scope, v8str(scope, "onload").into())
        && !current.is_null_or_undefined()
    {
        return;
    }
    let host_ptr: *mut JsContextHost = host;
    let Some(handler) = compile_event_attribute_handler_for_owner(
        scope,
        host_ptr,
        OwnerDispatchScope::Child(handle),
        source.as_ref(),
        EventAttributeHandlerScope::ChildWindow,
    ) else {
        let _ = window.set(scope, v8str(scope, "onload").into(), v8::null(scope).into());
        return;
    };
    let _ = window.set(scope, v8str(scope, "onload").into(), handler.into());
}

fn enter_child_window_event_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: DomHandle,
) -> v8::Local<'s, v8::Value> {
    let global = scope.get_current_context().global(scope);
    let previous = get_private_value(scope, global, ACTIVE_CHILD_WINDOW_HANDLE_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let handle_value = v8::BigInt::new_from_u64(scope, handle.index() as u64);
    set_private_value(
        scope,
        global,
        ACTIVE_CHILD_WINDOW_HANDLE_SLOT,
        handle_value.into(),
    );
    previous
}

fn restore_child_window_event_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    previous: v8::Local<'s, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, ACTIVE_CHILD_WINDOW_HANDLE_SLOT, previous);
}

fn dispatch_child_body_storage_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) {
    let Some(document) = window
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(body) = document
        .get(scope, v8str(scope, "body").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(get_attribute) = body
        .get(scope, v8str(scope, "getAttribute").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let Some(source_value) =
        get_attribute.call(scope, body.into(), &[v8str(scope, "onstorage").into()])
    else {
        return;
    };
    if source_value.is_null_or_undefined() {
        return;
    }
    let Some(source) = source_value.to_string(scope) else {
        return;
    };
    let source = source.to_rust_string_lossy(scope);
    if source.trim().is_empty() {
        return;
    }
    let wrapped = format!("(function(window){{with(window){{{source}}}}})");
    let Some(script_source) = v8_string(scope, &wrapped) else {
        return;
    };
    let Some(handler) = v8::Script::compile(scope, script_source, None)
        .and_then(|script| script.run(scope))
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let event_key = v8str(scope, "event");
    let previous_event = window
        .get(scope, event_key.into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = window.set(scope, event_key.into(), event.into());
    let _ = invoke_event_handler(
        scope,
        "child body onstorage",
        handler,
        body.into(),
        &[window.into()],
    );
    let _ = window.set(scope, event_key.into(), previous_event);
}
