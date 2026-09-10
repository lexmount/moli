use crate::web_api_interfaces;
use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, rc::Rc};

use crate::context_bootstrap::MessagePortEventListenerId;
use crate::exception_reporting::invoke_callback;
use crate::types::MessagePortId;
use crate::util::{get_private_value, set_private_value, v8_string, v8str};
use crate::webidl;
use moli_webapi_declare::WebApiObject;

use super::global_scope::{
    TimerInfo, get_worker_state, reject_worker_fetches_for_signal, worker_isolate_timer_queues,
};

mod event_listener;

use event_listener::WorkerAbortListener;
pub(crate) use event_listener::{
    worker_abort_signal_add_event_listener_callback, worker_abort_signal_dispatch_event_callback,
    worker_abort_signal_remove_event_listener_callback,
};

const WORKER_ABORT_SIGNAL_ID_SLOT: &str = "__lmWorkerAbortSignalId";
const WORKER_ABORT_CONTROLLER_ID_SLOT: &str = "__lmWorkerAbortControllerId";
const WORKER_ABORT_CONTROLLER_SIGNAL_SLOT: &str = "__lmWorkerAbortControllerSignal";
const WORKER_ABORT_SIGNAL_REASON_SLOT: &str = "__lmWorkerAbortSignalReason";

#[derive(Default)]
pub(super) struct WorkerAbortStore {
    next_signal_id: u32,
    next_controller_id: u32,
    next_listener_id: u64,
    signals: HashMap<u32, WorkerAbortSignalState>,
    controllers: HashMap<u32, u32>,
}

#[derive(Default)]
pub(super) struct WorkerAbortSignalState {
    signal: Option<v8::Global<v8::Object>>,
    aborted: bool,
    reason: Option<v8::Global<v8::Value>>,
    onabort: Option<v8::Global<v8::Function>>,
    listeners: HashMap<String, Vec<WorkerAbortListener>>,
    abort_algorithms: Vec<v8::Global<v8::Function>>,
    linked_message_port_listeners: Vec<WorkerAbortLinkedMessagePortListener>,
    // None for a source; Some (including empty) for a dependent signal's ordered roots.
    source_signals: Option<Vec<u32>>,
    dependent_signals: Vec<u32>,
}

struct WorkerAbortLinkedMessagePortListener {
    port_id: MessagePortId,
    listener_id: MessagePortEventListenerId,
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::AbortSignal)]
struct WorkerAbortSignalObjectDeclaration<'scope> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
}

impl WorkerAbortStore {
    fn alloc_signal_id(&mut self) -> u32 {
        self.next_signal_id = self
            .next_signal_id
            .checked_add(1)
            .expect("worker AbortSignal id space exhausted");
        self.next_signal_id
    }

    fn alloc_controller_id(&mut self) -> u32 {
        self.next_controller_id = self
            .next_controller_id
            .checked_add(1)
            .expect("worker AbortController id space exhausted");
        self.next_controller_id
    }

    pub(super) fn signal_id_from_object<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<u32> {
        get_private_value(scope, object, WORKER_ABORT_SIGNAL_ID_SLOT)
            .and_then(|value| value.number_value(scope))
            .filter(|value| value.is_finite() && *value >= 1.0)
            .map(|value| value as u32)
    }

    fn controller_id_from_object<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<u32> {
        get_private_value(scope, object, WORKER_ABORT_CONTROLLER_ID_SLOT)
            .and_then(|value| value.number_value(scope))
            .filter(|value| value.is_finite() && *value >= 1.0)
            .map(|value| value as u32)
    }

    fn init_signal(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        signal: v8::Local<'_, v8::Object>,
        aborted: bool,
        reason: Option<v8::Local<'_, v8::Value>>,
    ) -> u32 {
        let signal_id = self.alloc_signal_id();
        let mut state = WorkerAbortSignalState {
            aborted,
            ..WorkerAbortSignalState::default()
        };
        state.signal = Some(v8::Global::new(scope, signal));
        if let Some(reason) = reason {
            state.reason = Some(v8::Global::new(scope, reason));
        }
        self.signals.insert(signal_id, state);
        set_private_value(
            scope,
            signal,
            WORKER_ABORT_SIGNAL_ID_SLOT,
            v8::Number::new(scope, signal_id as f64).into(),
        );
        if let Some(reason) = reason {
            set_private_value(scope, signal, WORKER_ABORT_SIGNAL_REASON_SLOT, reason);
        }
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
            WORKER_ABORT_CONTROLLER_ID_SLOT,
            v8::Number::new(scope, controller_id as f64).into(),
        );
        set_private_value(
            scope,
            controller,
            WORKER_ABORT_CONTROLLER_SIGNAL_SLOT,
            signal.into(),
        );
    }

    fn signal_state(&self, id: u32) -> Option<&WorkerAbortSignalState> {
        self.signals.get(&id)
    }

    fn signal_state_mut(&mut self, id: u32) -> Option<&mut WorkerAbortSignalState> {
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

    fn signal_reason<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        Self::signal_id_from_object(scope, signal)
            .and_then(|id| self.signal_state(id))
            .and_then(|state| state.reason.as_ref())
            .map(|reason| v8::Local::new(scope, reason))
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

    fn register_abort_algorithm<'s>(
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

    fn unregister_abort_algorithm<'s>(
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
            .push(WorkerAbortLinkedMessagePortListener {
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

fn abort_worker_signal<'s>(
    store: &Rc<RefCell<WorkerAbortStore>>,
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(signal_id) = WorkerAbortStore::signal_id_from_object(scope, signal) else {
        return;
    };
    let signals_to_abort = {
        let mut store = store.borrow_mut();
        let Some(state) = store.signal_state_mut(signal_id) else {
            return;
        };
        if state.aborted {
            return;
        }
        state.aborted = true;
        state.reason = Some(v8::Global::new(scope, reason));
        set_private_value(scope, signal, WORKER_ABORT_SIGNAL_REASON_SLOT, reason);
        let dependent_signals = state.dependent_signals.clone();

        // Mark the complete dependency list before callbacks can reenter abort().
        let mut signals_to_abort = vec![(signal_id, signal)];
        for dependent_signal_id in dependent_signals {
            let Some(state) = store.signal_state_mut(dependent_signal_id) else {
                continue;
            };
            if state.aborted {
                continue;
            }
            state.aborted = true;
            state.reason = Some(v8::Global::new(scope, reason));
            if let Some(signal) = &state.signal {
                let signal = v8::Local::new(scope, signal);
                set_private_value(scope, signal, WORKER_ABORT_SIGNAL_REASON_SLOT, reason);
                signals_to_abort.push((dependent_signal_id, signal));
            }
        }
        signals_to_abort
    };
    for (signal_id, signal) in signals_to_abort {
        run_worker_abort_steps(store, scope, signal, signal_id, reason);
    }
}

fn run_worker_abort_steps<'s>(
    store: &Rc<RefCell<WorkerAbortStore>>,
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    signal_id: u32,
    reason: v8::Local<'s, v8::Value>,
) {
    let (abort_algorithms, linked_message_port_listeners) = {
        let mut store = store.borrow_mut();
        let Some(state) = store.signal_state_mut(signal_id) else {
            return;
        };
        (
            std::mem::take(&mut state.abort_algorithms),
            std::mem::take(&mut state.linked_message_port_listeners),
        )
    };
    reject_worker_fetches_for_signal(scope, signal_id, reason);
    invoke_worker_abort_algorithms(scope, signal, reason, abort_algorithms);
    for linked in linked_message_port_listeners {
        crate::worker::remove_worker_message_port_event_listener_by_id(
            scope,
            linked.port_id,
            linked.listener_id,
        );
    }
    // Snapshot at dispatch time, after earlier signals and abort algorithms.
    let Some(dispatch_snapshot) = store
        .borrow()
        .signal_state(signal_id)
        .map(|state| state.dispatch_snapshot("abort"))
    else {
        return;
    };
    event_listener::dispatch_abort(store, scope, signal, signal_id, dispatch_snapshot);
}

fn invoke_worker_abort_algorithms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
    abort_algorithms: Vec<v8::Global<v8::Function>>,
) {
    for algorithm in abort_algorithms {
        let algorithm = v8::Local::new(scope, &algorithm);
        let _ = invoke_callback(
            scope,
            "Worker AbortSignal abort algorithm",
            algorithm,
            signal.into(),
            &[reason],
        );
    }
}

fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

fn create_signal_with_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype_source: v8::Local<'_, v8::Object>,
    store: &mut WorkerAbortStore,
    aborted: bool,
    reason: Option<v8::Local<'_, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = prototype_source
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let signal = WorkerAbortSignalObjectDeclaration { prototype }
        .bind(scope)
        .ok()?;
    store.init_signal(scope, signal, aborted, reason);
    Some(signal)
}

fn worker_abort_store(scope: &mut v8::PinScope<'_, '_>) -> Option<Rc<RefCell<WorkerAbortStore>>> {
    get_worker_state(scope).map(|state| state.borrow().abort.clone())
}

pub(crate) fn worker_abort_signal_aborted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> bool {
    worker_abort_store(scope)
        .map(|store| store.borrow().signal_aborted(scope, signal))
        .unwrap_or(false)
}

pub(crate) fn worker_abort_signal_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    WorkerAbortStore::signal_id_from_object(scope, signal)
}

pub(crate) fn worker_abort_signal_reason<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    worker_abort_store(scope).and_then(|store| store.borrow().signal_reason(scope, signal))
}

pub(crate) fn register_worker_abort_signal_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    algorithm: v8::Local<'s, v8::Function>,
) -> bool {
    let Some(store) = worker_abort_store(scope) else {
        return false;
    };
    store
        .borrow_mut()
        .register_abort_algorithm(scope, signal, algorithm)
}

pub(crate) fn unregister_worker_abort_signal_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    algorithm: v8::Local<'s, v8::Function>,
) -> bool {
    let Some(store) = worker_abort_store(scope) else {
        return false;
    };
    store
        .borrow_mut()
        .unregister_abort_algorithm(scope, signal, algorithm)
}

pub(crate) fn register_worker_abort_signal_message_port_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    port_id: MessagePortId,
    listener_id: MessagePortEventListenerId,
) -> bool {
    let Some(store) = worker_abort_store(scope) else {
        return false;
    };
    store
        .borrow_mut()
        .register_message_port_listener(scope, signal, port_id, listener_id)
}

pub(crate) fn abort_worker_signal_by_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal_id: u32,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(store) = worker_abort_store(scope) else {
        return;
    };
    let Some(signal) = store.borrow().signal_object(scope, signal_id) else {
        return;
    };
    abort_worker_signal(&store, scope, signal, reason);
}

fn worker_abort_signal_onabort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Function>> {
    worker_abort_store(scope)
        .and_then(|state| {
            WorkerAbortStore::signal_id_from_object(scope, signal).and_then(|id| {
                state
                    .borrow()
                    .signal_state(id)
                    .and_then(|s| s.onabort.clone())
            })
        })
        .map(|onabort| v8::Local::new(scope, &onabort))
}

fn set_worker_abort_signal_onabort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(store) = worker_abort_store(scope) else {
        return;
    };
    let Some(signal_id) = WorkerAbortStore::signal_id_from_object(scope, signal) else {
        return;
    };
    if let Some(signal_state) = store.borrow_mut().signal_state_mut(signal_id) {
        signal_state.onabort = v8::Local::<v8::Function>::try_from(value)
            .ok()
            .map(|function| v8::Global::new(scope, function));
    }
}

pub(super) fn worker_dom_exception_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    name: &str,
) -> v8::Local<'s, v8::Value> {
    crate::context_bootstrap::new_dom_exception_value(scope, message, name)
}

pub(super) fn worker_abort_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    worker_dom_exception_value(scope, "The operation was aborted.", "AbortError")
}

fn worker_timeout_error_value<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    worker_dom_exception_value(scope, "signal timed out", "TimeoutError")
}

pub(crate) fn worker_abort_controller_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        scope.throw_exception(v8::Exception::type_error(
            scope,
            v8str(
                scope,
                "Failed to construct 'AbortController': Please use the 'new' operator.",
            ),
        ));
        return;
    }
    let Some(store) = worker_abort_store(scope) else {
        rv.set_undefined();
        return;
    };
    let global = scope.get_current_context().global(scope);
    let Some(signal_ctor) = global
        .get(scope, v8str(scope, "AbortSignal").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set(args.this().into());
        return;
    };
    let Some(signal) =
        create_signal_with_prototype(scope, signal_ctor, &mut store.borrow_mut(), false, None)
    else {
        rv.set(args.this().into());
        return;
    };
    store
        .borrow_mut()
        .init_controller(scope, args.this(), signal);
    rv.set(args.this().into());
}

pub(crate) fn worker_abort_controller_signal_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match get_private_value(scope, args.this(), WORKER_ABORT_CONTROLLER_SIGNAL_SLOT) {
        Some(value) => rv.set(value),
        None => {
            scope.throw_exception(v8::Exception::type_error(
                scope,
                v8str(scope, "Illegal invocation"),
            ));
        }
    }
}

pub(crate) fn worker_abort_controller_abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(store) = worker_abort_store(scope) else {
        rv.set_undefined();
        return;
    };
    let controller = args.this();
    let Some(controller_id) = WorkerAbortStore::controller_id_from_object(scope, controller) else {
        rv.set_undefined();
        return;
    };
    let Some(signal) = get_private_value(scope, controller, WORKER_ABORT_CONTROLLER_SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let reason = if args.length() > 0 && !args.get(0).is_undefined() {
        args.get(0)
    } else {
        worker_abort_error_value(scope)
    };
    let Some(signal_id) = store.borrow().controllers.get(&controller_id).copied() else {
        rv.set_undefined();
        return;
    };
    if store
        .borrow()
        .signal_state(signal_id)
        .is_some_and(|signal_state| signal_state.aborted)
    {
        rv.set_undefined();
        return;
    }
    abort_worker_signal(&store, scope, signal, reason);
    rv.set_undefined();
}

pub(crate) fn worker_abort_signal_static_abort_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(store) = worker_abort_store(scope) else {
        rv.set_null();
        return;
    };
    let reason = if args.length() > 0 && !args.get(0).is_undefined() {
        Some(args.get(0))
    } else {
        Some(worker_abort_error_value(scope))
    };
    let Some(signal) =
        create_signal_with_prototype(scope, args.this(), &mut store.borrow_mut(), true, reason)
    else {
        rv.set_null();
        return;
    };
    rv.set(signal.into());
}

pub(crate) fn worker_abort_signal_timeout_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(store) = worker_abort_store(scope) else {
        rv.set_null();
        return;
    };
    let delay_ms = u64::from(webidl::non_negative_milliseconds_arg(
        scope,
        &args,
        0,
        "AbortSignal.timeout",
    ));
    let Some(signal) =
        create_signal_with_prototype(scope, args.this(), &mut store.borrow_mut(), false, None)
    else {
        rv.set_null();
        return;
    };
    let Some(signal_id) = WorkerAbortStore::signal_id_from_object(scope, signal) else {
        rv.set_null();
        return;
    };
    let Some(callback) = v8::FunctionTemplate::builder(worker_abort_signal_timeout_fire_callback)
        .data(v8::Number::new(scope, signal_id as f64).into())
        .build(scope)
        .get_function(scope)
    else {
        rv.set(signal.into());
        return;
    };
    let timer = TimerInfo {
        id: {
            let Some(state) = get_worker_state(scope) else {
                rv.set(signal.into());
                return;
            };
            let mut state = state.borrow_mut();
            state.next_timer_id += 1;
            state.next_timer_id
        },
        callback: super::timer_callback::WorkerTimerCallback::browser_function(scope, callback),
        delay_ms,
        is_interval: false,
        extra_args: Vec::new(),
    };
    if let Some(timers) = worker_isolate_timer_queues(scope) {
        timers.push_pending(timer);
    }
    rv.set(signal.into());
}

pub(crate) fn worker_abort_signal_any_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(store) = worker_abort_store(scope) else {
        rv.set_null();
        return;
    };
    let Some(signal) =
        create_signal_with_prototype(scope, args.this(), &mut store.borrow_mut(), false, None)
    else {
        rv.set_null();
        return;
    };
    let Some(composite_signal_id) = WorkerAbortStore::signal_id_from_object(scope, signal) else {
        rv.set_null();
        return;
    };
    let signals = match collect_abort_signal_iterable(scope, args.get(0)) {
        Ok(signals) => signals,
        Err(message) => {
            if let Some(message) = v8_string(scope, &message) {
                scope.throw_exception(v8::Exception::type_error(scope, message));
            }
            return;
        }
    };
    for source_signal in &signals {
        let Some(source_signal_id) = WorkerAbortStore::signal_id_from_object(scope, *source_signal)
        else {
            continue;
        };
        let Some(reason) = store
            .borrow()
            .signal_state(source_signal_id)
            .filter(|signal_state| signal_state.aborted)
            .and_then(|signal_state| signal_state.reason.as_ref())
            .map(|reason| v8::Local::new(scope, reason))
        else {
            continue;
        };
        abort_worker_signal(&store, scope, signal, reason);
        rv.set(signal.into());
        return;
    }
    store.borrow_mut().set_signal_sources(
        composite_signal_id,
        signals
            .into_iter()
            .filter_map(|signal| WorkerAbortStore::signal_id_from_object(scope, signal)),
    );
    rv.set(signal.into());
}

pub(crate) fn worker_abort_signal_aborted_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(worker_abort_signal_aborted(scope, args.this()));
}

pub(crate) fn worker_abort_signal_reason_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(reason) = worker_abort_signal_reason(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    rv.set(reason);
}

pub(crate) fn worker_abort_signal_onabort_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(onabort) = worker_abort_signal_onabort(scope, args.this()) else {
        rv.set_null();
        return;
    };
    rv.set(onabort.into());
}

pub(crate) fn worker_abort_signal_onabort_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() > 0 {
        set_worker_abort_signal_onabort(scope, args.this(), args.get(0));
    } else {
        set_worker_abort_signal_onabort(scope, args.this(), v8::undefined(scope).into());
    }
    rv.set_undefined();
}

pub(crate) fn worker_abort_signal_throw_if_aborted_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let signal = args.this();
    let Some(reason) =
        worker_abort_store(scope).and_then(|store| store.borrow().signal_reason(scope, signal))
    else {
        rv.set_undefined();
        return;
    };
    if !worker_abort_signal_aborted(scope, signal) {
        rv.set_undefined();
        return;
    }
    // DOM requires throwing the stored abort reason itself. In particular, a
    // string reason remains a string rather than being wrapped in `Error`.
    scope.throw_exception(reason);
}

fn collect_abort_signal_iterable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterable: v8::Local<'s, v8::Value>,
) -> Result<Vec<v8::Local<'s, v8::Object>>, String> {
    if iterable.is_null_or_undefined() {
        return Err(
            "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
        );
    }
    let Ok(iterable_object) = v8::Local::<v8::Object>::try_from(iterable) else {
        return Err(
            "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
        );
    };
    let iterator_symbol = v8::Symbol::get_iterator(scope);
    let Some(iterator_method) = iterable_object
        .get(scope, iterator_symbol.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Err(
            "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
        );
    };
    let Some(iterator) = iterator_method
        .call(scope, iterable, &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Err(
            "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
        );
    };
    let Some(next_method) = iterator
        .get(scope, v8str(scope, "next").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Err(
            "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
        );
    };
    let mut signals = Vec::new();
    loop {
        let Some(step) = next_method
            .call(scope, iterator.into(), &[])
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            return Err(
                "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
            );
        };
        let done = step
            .get(scope, v8str(scope, "done").into())
            .is_some_and(|value| value.boolean_value(scope));
        if done {
            break;
        }
        let Some(value) = step.get(scope, v8str(scope, "value").into()) else {
            return Err(
                "Failed to execute 'any' on 'AbortSignal': iterable yielded a non-AbortSignal value."
                    .to_owned(),
            );
        };
        let Ok(signal) = v8::Local::<v8::Object>::try_from(value) else {
            return Err(
                "Failed to execute 'any' on 'AbortSignal': iterable yielded a non-AbortSignal value."
                    .to_owned(),
            );
        };
        if WorkerAbortStore::signal_id_from_object(scope, signal).is_none() {
            return Err(
                "Failed to execute 'any' on 'AbortSignal': iterable yielded a non-AbortSignal value."
                    .to_owned(),
            );
        }
        signals.push(signal);
    }
    Ok(signals)
}

fn worker_abort_signal_timeout_fire_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(store) = worker_abort_store(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(signal_id) = args
        .data()
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 1.0)
        .map(|value| value as u32)
    else {
        rv.set_undefined();
        return;
    };
    let Some(signal) = store.borrow().signal_object(scope, signal_id) else {
        rv.set_undefined();
        return;
    };
    let reason = worker_timeout_error_value(scope);
    abort_worker_signal(&store, scope, signal, reason);
    rv.set_undefined();
}
