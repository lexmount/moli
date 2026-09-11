use crate::web_api_interfaces;
use std::collections::{HashMap, HashSet};

use super::super::document_runtime::EventTargetHandle;
use super::super::util::{get_private_value, set_private_value, v8_string, v8str};
use crate::context_bootstrap::{MessagePortEventListenerId, new_dom_exception_value};
use crate::types::MessagePortId;
use moli_webapi_declare::WebApiObject;

mod controller;
mod event;
mod signal;
mod statics;

pub(crate) use controller::{
    abort_controller_abort_callback, abort_controller_constructor_callback,
    abort_controller_signal_getter_callback,
};
use event::dispatch_abort;
pub(crate) use signal::{
    abort_signal_aborted_getter_callback, abort_signal_add_event_listener_callback,
    abort_signal_dispatch_event_callback, abort_signal_onabort_getter_callback,
    abort_signal_onabort_setter_callback, abort_signal_reason_getter_callback,
    abort_signal_remove_event_listener_callback, abort_signal_throw_if_aborted_callback,
};
pub(crate) use statics::{
    abort_signal_any_callback, abort_signal_static_abort_callback, abort_signal_timeout_callback,
};

const ABORT_SIGNAL_ID_SLOT: &str = "__lmAbortSignalId";
const ABORT_CONTROLLER_ID_SLOT: &str = "__lmAbortControllerId";
const ABORT_CONTROLLER_SIGNAL_SLOT: &str = "__lmAbortControllerSignal";

#[derive(Default)]
pub(super) struct AbortStore {
    next_signal_id: u32,
    next_controller_id: u32,
    signals: HashMap<u32, AbortSignalState>,
    controllers: HashMap<u32, u32>,
}

#[derive(Default)]
struct AbortSignalState {
    signal: Option<v8::Global<v8::Object>>,
    aborted: bool,
    reason: Option<v8::Global<v8::Value>>,
    onabort: Option<v8::Global<v8::Function>>,
    listeners: HashMap<String, Vec<AbortListener>>,
    abort_algorithms: Vec<v8::Global<v8::Function>>,
    linked_target_listeners: Vec<AbortLinkedTargetListener>,
    linked_message_port_listeners: Vec<AbortLinkedMessagePortListener>,
    dependent_signals: Vec<u32>,
}

#[derive(Clone, Copy)]
pub(super) struct AbortListener {
    pub(super) callback_id: super::EventCallbackId,
    pub(super) capture: bool,
    pub(super) once: bool,
    pub(super) passive: bool,
}

pub(super) struct PreparedAbortListener {
    pub(super) callback: super::PreparedEventCallback,
    pub(super) passive: bool,
}

#[derive(Default)]
pub(super) struct AbortDispatchSnapshot {
    listeners: Vec<AbortListener>,
    onabort: Option<v8::Global<v8::Function>>,
}

impl AbortSignalState {
    fn take_dispatch_snapshot(&mut self, event_type: &str) -> AbortDispatchSnapshot {
        let listeners = self.listeners.get(event_type).cloned().unwrap_or_default();
        let onabort = if event_type == "abort" {
            self.onabort.clone()
        } else {
            None
        };
        AbortDispatchSnapshot { listeners, onabort }
    }
}

struct AbortLinkedTargetListener {
    target: EventTargetHandle,
    event_type: String,
    callback_id: super::EventCallbackId,
    capture: bool,
}

struct AbortLinkedMessagePortListener {
    port_id: MessagePortId,
    listener_id: MessagePortEventListenerId,
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::AbortSignal)]
struct AbortSignalObjectDeclaration<'scope> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
}

impl AbortStore {
    fn alloc_signal_id(&mut self) -> u32 {
        self.next_signal_id = self
            .next_signal_id
            .checked_add(1)
            .expect("AbortSignal id space exhausted");
        self.next_signal_id
    }

    fn alloc_controller_id(&mut self) -> u32 {
        self.next_controller_id = self
            .next_controller_id
            .checked_add(1)
            .expect("AbortController id space exhausted");
        self.next_controller_id
    }

    pub(super) fn signal_id_from_object<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<u32> {
        get_private_value(scope, object, ABORT_SIGNAL_ID_SLOT)
            .and_then(|value| value.number_value(scope))
            .filter(|value| value.is_finite() && *value >= 1.0)
            .map(|value| value as u32)
    }

    pub(super) fn is_signal_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> bool {
        Self::signal_id_from_object(scope, object)
            .and_then(|id| self.signal_state(id))
            .is_some()
    }

    fn controller_id_from_object<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<u32> {
        get_private_value(scope, object, ABORT_CONTROLLER_ID_SLOT)
            .and_then(|value| value.number_value(scope))
            .filter(|value| value.is_finite() && *value >= 1.0)
            .map(|value| value as u32)
    }

    fn define_hidden_value(
        scope: &mut v8::PinScope<'_, '_>,
        object: v8::Local<'_, v8::Object>,
        key: &str,
        value: v8::Local<'_, v8::Value>,
    ) {
        let Some(key) = v8_string(scope, key) else {
            return;
        };
        let _ =
            object.define_own_property(scope, key.into(), value, v8::PropertyAttribute::DONT_ENUM);
    }

    fn init_signal(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        signal: v8::Local<'_, v8::Object>,
        aborted: bool,
        reason: Option<v8::Local<'_, v8::Value>>,
    ) -> u32 {
        let signal_id = self.alloc_signal_id();
        let mut state = AbortSignalState {
            aborted,
            ..AbortSignalState::default()
        };
        state.signal = Some(v8::Global::new(scope, signal));
        if let Some(reason) = reason {
            state.reason = Some(v8::Global::new(scope, reason));
        }
        self.signals.insert(signal_id, state);
        set_private_value(
            scope,
            signal,
            ABORT_SIGNAL_ID_SLOT,
            v8::Number::new(scope, signal_id as f64).into(),
        );
        signal_id
    }

    fn init_controller(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        controller: v8::Local<'_, v8::Object>,
        signal: v8::Local<'_, v8::Object>,
    ) {
        let signal_id = self.init_signal(scope, signal, false, None);
        let controller_id = self.alloc_controller_id();
        self.controllers.insert(controller_id, signal_id);
        set_private_value(
            scope,
            controller,
            ABORT_CONTROLLER_ID_SLOT,
            v8::Number::new(scope, controller_id as f64).into(),
        );
        set_private_value(
            scope,
            controller,
            ABORT_CONTROLLER_SIGNAL_SLOT,
            signal.into(),
        );
    }

    fn signal_state(&self, id: u32) -> Option<&AbortSignalState> {
        self.signals.get(&id)
    }

    fn signal_state_mut(&mut self, id: u32) -> Option<&mut AbortSignalState> {
        self.signals.get_mut(&id)
    }

    fn signal_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: u32,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.signal_state(id)
            .and_then(|state| state.signal.as_ref())
            .map(|signal| v8::Local::new(scope, signal))
    }

    pub(super) fn signal_aborted<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> bool {
        Self::signal_id_from_object(scope, signal)
            .and_then(|id| self.signal_state(id))
            .is_some_and(|state| state.aborted)
    }

    pub(super) fn signal_reason<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        Self::signal_id_from_object(scope, signal)
            .and_then(|id| self.signal_state(id))
            .and_then(|state| state.reason.as_ref())
            .map(|reason| v8::Local::new(scope, reason))
    }

    pub(super) fn listener_callback_ids(
        &self,
        signal_id: u32,
        event_type: &str,
        capture: bool,
    ) -> Vec<super::EventCallbackId> {
        self.signal_state(signal_id)
            .and_then(|state| state.listeners.get(event_type))
            .into_iter()
            .flatten()
            .filter(|listener| listener.capture == capture)
            .map(|listener| listener.callback_id)
            .collect()
    }

    pub(super) fn register_listener(
        &mut self,
        signal_id: u32,
        event_type: &str,
        callback_id: super::EventCallbackId,
        capture: bool,
        once: bool,
        passive: bool,
    ) -> bool {
        let Some(state) = self.signal_state_mut(signal_id) else {
            return false;
        };
        state
            .listeners
            .entry(event_type.to_owned())
            .or_default()
            .push(AbortListener {
                callback_id,
                capture,
                once,
                passive,
            });
        true
    }

    pub(super) fn unregister_listener_by_id(
        &mut self,
        signal_id: u32,
        event_type: &str,
        callback_id: super::EventCallbackId,
        capture: bool,
    ) -> bool {
        let Some(state) = self.signal_state_mut(signal_id) else {
            return false;
        };
        let mut remove_entry = false;
        let mut removed = false;
        if let Some(listeners) = state.listeners.get_mut(event_type) {
            listeners.retain(|candidate| {
                let matches = candidate.callback_id == callback_id && candidate.capture == capture;
                removed |= matches;
                !matches
            });
            remove_entry = listeners.is_empty();
        }
        if remove_entry {
            state.listeners.remove(event_type);
        }
        removed
    }

    pub(super) fn claim_listener_for_dispatch(
        &mut self,
        signal_id: u32,
        event_type: &str,
        callback_id: super::EventCallbackId,
    ) -> Option<AbortListener> {
        let state = self.signal_state_mut(signal_id)?;
        let listener = state
            .listeners
            .get(event_type)?
            .iter()
            .find(|listener| listener.callback_id == callback_id)
            .copied()?;
        if listener.once {
            let _ = self.unregister_listener_by_id(
                signal_id,
                event_type,
                callback_id,
                listener.capture,
            );
        }
        Some(listener)
    }

    pub(super) fn unregister_signal_event_callbacks(
        &mut self,
        callback_ids: &HashSet<super::EventCallbackId>,
    ) {
        for state in self.signals.values_mut() {
            state.listeners.retain(|_, listeners| {
                listeners.retain(|listener| !callback_ids.contains(&listener.callback_id));
                !listeners.is_empty()
            });
        }
    }

    pub(crate) fn register_abort_algorithm<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        algorithm: v8::Local<'s, v8::Function>,
    ) -> bool {
        let Some(signal_id) = Self::signal_id_from_object(scope, signal) else {
            return false;
        };
        let Some(state) = self.signal_state_mut(signal_id) else {
            return false;
        };
        state
            .abort_algorithms
            .push(v8::Global::new(scope, algorithm));
        true
    }

    pub(crate) fn unregister_abort_algorithm<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        algorithm: v8::Local<'s, v8::Function>,
    ) -> bool {
        let Some(signal_id) = Self::signal_id_from_object(scope, signal) else {
            return false;
        };
        let Some(state) = self.signal_state_mut(signal_id) else {
            return false;
        };
        state.abort_algorithms.retain(|candidate| {
            let candidate = v8::Local::new(scope, candidate);
            !candidate.strict_equals(algorithm.into())
        });
        true
    }

    pub(super) fn register_target_listener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        target: EventTargetHandle,
        event_type: &str,
        callback_id: super::EventCallbackId,
        capture: bool,
    ) {
        let Some(signal_id) = Self::signal_id_from_object(scope, signal) else {
            return;
        };
        let Some(state) = self.signal_state_mut(signal_id) else {
            return;
        };
        state
            .linked_target_listeners
            .push(AbortLinkedTargetListener {
                target,
                event_type: event_type.to_owned(),
                callback_id,
                capture,
            });
    }

    pub(super) fn unregister_target_listener(&mut self, callback_id: super::EventCallbackId) {
        for state in self.signals.values_mut() {
            state
                .linked_target_listeners
                .retain(|linked| linked.callback_id != callback_id);
        }
    }

    pub(super) fn register_message_port_listener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        port_id: MessagePortId,
        listener_id: MessagePortEventListenerId,
    ) -> bool {
        let Some(signal_id) = Self::signal_id_from_object(scope, signal) else {
            return false;
        };
        let Some(state) = self.signal_state_mut(signal_id) else {
            return false;
        };
        state
            .linked_message_port_listeners
            .push(AbortLinkedMessagePortListener {
                port_id,
                listener_id,
            });
        true
    }

    pub(super) fn unregister_message_port_listener(
        &mut self,
        port_id: MessagePortId,
        listener_id: MessagePortEventListenerId,
    ) {
        for state in self.signals.values_mut() {
            state
                .linked_message_port_listeners
                .retain(|linked| linked.port_id != port_id || linked.listener_id != listener_id);
        }
    }

    pub(super) fn abort_signal<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host: &mut super::JsContextHost,
        signal: v8::Local<'s, v8::Object>,
        reason: v8::Local<'s, v8::Value>,
    ) {
        let Some(signal_id) = Self::signal_id_from_object(scope, signal) else {
            return;
        };
        let Some((
            abort_algorithms,
            dispatch_snapshot,
            linked_target_listeners,
            linked_message_port_listeners,
            dependent_signals,
        )) = ({
            let Some(state) = self.signal_state_mut(signal_id) else {
                return;
            };
            if state.aborted {
                return;
            }
            state.aborted = true;
            state.reason = Some(v8::Global::new(scope, reason));
            let abort_algorithms = std::mem::take(&mut state.abort_algorithms);
            let linked_target_listeners = std::mem::take(&mut state.linked_target_listeners);
            let linked_message_port_listeners =
                std::mem::take(&mut state.linked_message_port_listeners);
            let dependent_signals = state.dependent_signals.clone();
            let dispatch_snapshot = state.take_dispatch_snapshot("abort");
            Some((
                abort_algorithms,
                dispatch_snapshot,
                linked_target_listeners,
                linked_message_port_listeners,
                dependent_signals,
            ))
        })
        else {
            return;
        };
        event::invoke_abort_algorithms(scope, signal, reason, abort_algorithms);
        for linked in linked_message_port_listeners {
            host.remove_message_port_event_listener_after_signal_abort(
                linked.port_id,
                linked.listener_id,
            );
        }
        dispatch_abort(
            scope,
            host as *mut super::JsContextHost,
            signal,
            signal_id,
            dispatch_snapshot,
        );
        for linked in linked_target_listeners {
            host.remove_registered_event_listener_by_id(
                linked.target,
                &linked.event_type,
                linked.callback_id,
                linked.capture,
            );
        }
        for dependent_signal_id in dependent_signals {
            let Some(dependent_signal) = self.signal_object(scope, dependent_signal_id) else {
                continue;
            };
            host.abort_signal(scope, dependent_signal, reason);
        }
    }

    pub(super) fn link_dependent_signal(
        &mut self,
        source_signal_id: u32,
        dependent_signal_id: u32,
    ) {
        let Some(state) = self.signal_state_mut(source_signal_id) else {
            return;
        };
        if !state.dependent_signals.contains(&dependent_signal_id) {
            state.dependent_signals.push(dependent_signal_id);
        }
    }
}

pub(crate) fn dom_exception_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    name: &str,
) -> v8::Local<'s, v8::Value> {
    new_dom_exception_value(scope, message, name)
}

pub(crate) fn abort_error_value<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    dom_exception_value(scope, "The operation was aborted.", "AbortError")
}

pub(super) fn timeout_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    dom_exception_value(scope, "signal timed out", "TimeoutError")
}

fn create_signal_with_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype_source: v8::Local<'_, v8::Object>,
    host: &mut super::JsContextHost,
    aborted: bool,
    reason: Option<v8::Local<'_, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = prototype_source
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let signal = AbortSignalObjectDeclaration::new(prototype)
        .bind(scope)
        .ok()?;
    host.native_bridge_mut()
        .abort
        .init_signal(scope, signal, aborted, reason);
    Some(signal)
}
