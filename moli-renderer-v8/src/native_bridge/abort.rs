use std::collections::HashMap;

use super::super::document_runtime::EventTargetHandle;
use super::super::util::{get_private_value, set_private_value};
use crate::context_bootstrap::new_dom_exception_value;

mod controller;
mod event;
mod signal;
mod statics;

use crate::context_bootstrap::{abort_signal, abort_signal_events};
pub(crate) use controller::{
    abort_controller_abort_callback, abort_controller_constructor_callback,
    abort_controller_signal_getter_callback,
};
pub(crate) use signal::{
    abort_signal_aborted_getter_callback, abort_signal_reason_getter_callback,
    abort_signal_throw_if_aborted_callback,
};
pub(crate) use statics::{
    abort_signal_any_callback, abort_signal_static_abort_callback, abort_signal_timeout_callback,
};

const ABORT_SIGNAL_ID_SLOT: &str = "__lmAbortSignalId";
const ABORT_CONTROLLER_ID_SLOT: &str = "__lmAbortControllerId";
const ABORT_CONTROLLER_SIGNAL_SLOT: &str = "__lmAbortControllerSignal";

pub(crate) fn bind_signal_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    wrapper: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(id) = get_private_value(scope, signal, ABORT_SIGNAL_ID_SLOT) else {
        return false;
    };
    set_private_value(scope, wrapper, ABORT_SIGNAL_ID_SLOT, id);
    true
}

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
    abort_algorithms: Vec<v8::Global<v8::Function>>,
    linked_target_listeners: Vec<AbortLinkedTargetListener>,
    dependent_signals: Vec<u32>,
}

struct AbortLinkedTargetListener {
    target: EventTargetHandle,
    event_type: String,
    callback_id: super::EventCallbackId,
    capture: bool,
}

struct AbortSignalDispatch {
    algorithms: Vec<v8::Global<v8::Function>>,
    linked_target_listeners: Vec<AbortLinkedTargetListener>,
    dependent_signals: Vec<u32>,
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

    fn init_signal<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
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
        abort_signal_events::initialize(scope, signal);
        signal_id
    }

    fn init_controller<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        controller: v8::Local<'_, v8::Object>,
        signal: v8::Local<'s, v8::Object>,
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

    fn take_signal_abort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        reason: v8::Local<'s, v8::Value>,
    ) -> Option<AbortSignalDispatch> {
        let signal_id = Self::signal_id_from_object(scope, signal)?;
        let state = self.signal_state_mut(signal_id)?;
        if state.aborted {
            return None;
        }
        state.aborted = true;
        state.reason = Some(v8::Global::new(scope, reason));
        Some(AbortSignalDispatch {
            algorithms: std::mem::take(&mut state.abort_algorithms),
            linked_target_listeners: std::mem::take(&mut state.linked_target_listeners),
            dependent_signals: state.dependent_signals.clone(),
        })
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

/// Aborting a signal invokes author algorithms and listeners synchronously.
/// Only owned dispatch data may cross those calls; neither the host nor its
/// AbortStore can remain borrowed, including while aborting dependent signals.
pub(crate) fn abort_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let dispatch = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .abort
        .take_signal_abort(scope, signal, reason);
    let Some(dispatch) = dispatch else {
        return;
    };
    event::invoke_abort_algorithms(scope, signal, reason, dispatch.algorithms);
    abort_signal_events::dispatch_abort(scope, signal);
    for linked in dispatch.linked_target_listeners {
        unsafe { &mut *host_ptr }.remove_registered_event_listener_by_id(
            linked.target,
            &linked.event_type,
            linked.callback_id,
            linked.capture,
        );
    }
    for dependent_signal_id in dispatch.dependent_signals {
        let signal = unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .abort
            .signal_object(scope, dependent_signal_id);
        if let Some(signal) = signal {
            abort_signal(scope, signal, reason);
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

fn create_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut super::JsContextHost,
    aborted: bool,
    reason: Option<v8::Local<'_, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let signal = abort_signal::new_signal(scope)?;
    host.native_bridge_mut()
        .abort
        .init_signal(scope, signal, aborted, reason);
    Some(signal)
}
