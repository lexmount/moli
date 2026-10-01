use crate::event_type::{EventType, EventTypeKey};
use std::collections::{HashMap, HashSet};

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
    new_dependent_abort_signal,
};

const ABORT_SIGNAL_ID_SLOT: &str = "__lmAbortSignalId";
const ABORT_SIGNAL_STATE_SLOT: &str = "__moliAbortSignalState";
const ABORT_SIGNAL_ABORTED_SLOT: &str = "__moliAbortSignalAborted";
const ABORT_SIGNAL_REASON_SLOT: &str = "__moliAbortSignalReason";
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
    if let Some(state) = get_private_value(scope, signal, ABORT_SIGNAL_STATE_SLOT) {
        set_private_value(scope, wrapper, ABORT_SIGNAL_STATE_SLOT, state);
    }
    true
}

#[derive(Default)]
pub(super) struct AbortStore {
    next_signal_id: u32,
    next_controller_id: u32,
    signals: HashMap<u32, AbortSignalState>,
    controllers: HashMap<u32, u32>,
    detached: bool,
}

#[derive(Default)]
struct AbortSignalState {
    signal: Option<crate::util::RealmObjectHandle>,
    aborted: bool,
    detached: bool,
    abort_algorithms: Vec<crate::abort_signal_route::AbortAlgorithm>,
    linked_target_listeners: Vec<AbortLinkedTargetListener>,
    // None for a source; Some (including empty) for a dependent signal's ordered roots.
    source_signals: Option<Vec<u32>>,
    dependent_signals: Vec<u32>,
}

struct AbortLinkedTargetListener {
    target: EventTargetHandle,
    event_type: EventType,
    callback_id: super::EventCallbackId,
    capture: bool,
}

struct AbortSignalDispatch {
    algorithms: Vec<crate::abort_signal_route::AbortAlgorithm>,
    linked_target_listeners: Vec<AbortLinkedTargetListener>,
    dispatch_event: bool,
}

impl AbortStore {
    pub(super) fn clear_for_context_teardown(&mut self) {
        self.detached = true;
        for state in self.signals.values_mut() {
            state.detached = true;
            state.abort_algorithms.clear();
            state.linked_target_listeners.clear();
        }
    }

    pub(crate) fn retire_context<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Context>,
    ) {
        for state in self.signals.values_mut() {
            let Some(handle) = state.signal.as_mut() else {
                continue;
            };
            let Some(signal) = handle.to_local(scope) else {
                continue;
            };
            if signal.get_creation_context(scope) != Some(context) {
                continue;
            }
            handle.retain_in_realm(scope);
            state.detached = true;
            state.abort_algorithms.clear();
            state.linked_target_listeners.clear();
        }
    }

    fn signal_passive_state<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        get_private_value(scope, signal, ABORT_SIGNAL_STATE_SLOT)
            .and_then(|state| v8::Local::<v8::Object>::try_from(state).ok())
    }

    fn signal_aborted_from_object<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> bool {
        Self::signal_passive_state(scope, signal)
            .and_then(|state| get_private_value(scope, state, ABORT_SIGNAL_ABORTED_SLOT))
            .is_some_and(|value| value.is_true())
    }

    fn signal_reason_from_object<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        let state = Self::signal_passive_state(scope, signal)?;
        get_private_value(scope, state, ABORT_SIGNAL_REASON_SLOT)
    }

    fn publish_signal_abort<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        reason: v8::Local<'s, v8::Value>,
    ) {
        if let Some(state) = Self::signal_passive_state(scope, signal) {
            set_private_value(
                scope,
                state,
                ABORT_SIGNAL_ABORTED_SLOT,
                v8::Boolean::new(scope, true).into(),
            );
            set_private_value(scope, state, ABORT_SIGNAL_REASON_SLOT, reason);
        }
    }

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
        Self::signal_id_from_object(scope, object).is_some()
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
            detached: self.detached,
            ..AbortSignalState::default()
        };
        state.signal = Some(if aborted {
            crate::util::RealmObjectHandle::weak(scope, signal)
        } else {
            crate::util::RealmObjectHandle::new(scope, signal)
        });
        self.signals.insert(signal_id, state);
        let passive_state = v8::Object::new(scope);
        set_private_value(
            scope,
            passive_state,
            ABORT_SIGNAL_ABORTED_SLOT,
            v8::Boolean::new(scope, aborted).into(),
        );
        set_private_value(
            scope,
            passive_state,
            ABORT_SIGNAL_REASON_SLOT,
            reason.unwrap_or_else(|| v8::undefined(scope).into()),
        );
        set_private_value(scope, signal, ABORT_SIGNAL_STATE_SLOT, passive_state.into());
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
        let signal_id = Self::signal_id_from_object(scope, signal)
            .expect("AbortController signal was initialized by its native factory");
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
            .and_then(|signal| signal.to_local(scope))
    }

    pub(super) fn signal_aborted<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> bool {
        Self::signal_aborted_from_object(scope, signal)
    }

    pub(super) fn signal_reason<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        Self::signal_reason_from_object(scope, signal)
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
        if state.detached {
            return false;
        }
        state
            .abort_algorithms
            .push(crate::abort_signal_route::AbortAlgorithm::new(
                scope, algorithm,
            ));
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
            candidate
                .prepare(scope)
                .is_some_and(|candidate| !candidate.strict_equals(algorithm.into()))
        });
        true
    }

    pub(super) fn register_target_listener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        target: EventTargetHandle,
        event_type: &(impl EventTypeKey + ?Sized),
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
                event_type: event_type.to_event_type(),
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

    fn prepare_signal_abort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        reason: v8::Local<'s, v8::Value>,
    ) -> Option<Vec<(u32, v8::Local<'s, v8::Object>)>> {
        let signal_id = Self::signal_id_from_object(scope, signal)?;
        if Self::signal_aborted_from_object(scope, signal) {
            return None;
        }
        Self::publish_signal_abort(scope, signal, reason);
        let state = self.signal_state_mut(signal_id)?;
        if state.aborted {
            return None;
        }
        state.aborted = true;
        // The dispatch snapshot roots this signal through all synchronous abort
        // steps. Its stored reason may reference the signal or a captured stack;
        // the completed registry must not turn that V8 graph into a Rust root.
        state.signal = Some(crate::util::RealmObjectHandle::weak(scope, signal));
        let dependent_signals = state.dependent_signals.clone();
        // Publish every reason before any author callback can reenter abort.
        let mut signals = vec![(signal_id, signal)];
        for dependent_signal_id in dependent_signals {
            let Some(state) = self.signal_state_mut(dependent_signal_id) else {
                continue;
            };
            if state.aborted {
                continue;
            }
            state.aborted = true;
            if let Some(signal) = state
                .signal
                .as_ref()
                .and_then(|signal| signal.to_local(scope))
            {
                Self::publish_signal_abort(scope, signal, reason);
                state.signal = Some(crate::util::RealmObjectHandle::weak(scope, signal));
                signals.push((dependent_signal_id, signal));
            }
        }
        Some(signals)
    }

    fn take_signal_abort_steps(&mut self, signal_id: u32) -> Option<AbortSignalDispatch> {
        let state = self.signal_state_mut(signal_id)?;
        Some(AbortSignalDispatch {
            algorithms: std::mem::take(&mut state.abort_algorithms),
            linked_target_listeners: std::mem::take(&mut state.linked_target_listeners),
            dispatch_event: !state.detached,
        })
    }

    fn set_signal_sources(
        &mut self,
        dependent_signal_id: u32,
        input_signal_ids: impl IntoIterator<Item = u32>,
    ) {
        let mut sources = Vec::new();
        let mut seen = HashSet::new();
        for input_signal_id in input_signal_ids {
            let Some(state) = self.signal_state(input_signal_id) else {
                continue;
            };
            let roots = state
                .source_signals
                .as_deref()
                .unwrap_or(std::slice::from_ref(&input_signal_id));
            for &source_signal_id in roots {
                if seen.insert(source_signal_id) {
                    sources.push(source_signal_id);
                }
            }
        }
        for &source_signal_id in &sources {
            if let Some(state) = self.signal_state_mut(source_signal_id) {
                state.dependent_signals.push(dependent_signal_id);
            }
        }
        if let Some(state) = self.signal_state_mut(dependent_signal_id) {
            state.source_signals = Some(sources);
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
    let signals = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .abort
        .prepare_signal_abort(scope, signal, reason);
    let Some(signals) = signals else {
        return;
    };
    // Complete the snapshot even if IteratorClose throws: every dependent
    // is already marked aborted and cannot be retried.
    let mut first_error = None;
    for (signal_id, signal) in signals {
        let dispatch = unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .abort
            .take_signal_abort_steps(signal_id);
        let Some(dispatch) = dispatch else {
            continue;
        };
        let error = {
            v8::tc_scope!(let scope, scope);
            if event::invoke_abort_algorithms(scope, signal, reason, dispatch.algorithms) {
                for linked in dispatch.linked_target_listeners {
                    unsafe { &mut *host_ptr }.remove_registered_event_listener_by_id(
                        linked.target,
                        &linked.event_type,
                        linked.callback_id,
                        linked.capture,
                    );
                }
                // Snapshot listeners after earlier signals and abort algorithms.
                if dispatch.dispatch_event {
                    abort_signal_events::dispatch_abort(scope, signal);
                }
                None
            } else {
                let Some(error) = scope.exception() else {
                    return;
                };
                scope.reset();
                Some(error)
            }
        };
        if first_error.is_none() {
            first_error = error;
        }
    }
    if let Some(error) = first_error {
        scope.throw_exception(error);
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

pub(crate) fn create_signal<'s>(
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
