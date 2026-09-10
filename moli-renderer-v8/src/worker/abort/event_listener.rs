use std::{cell::RefCell, rc::Rc};

use moli_webidl_callback::{PreparedWebIdlCallbackInterface, WebIdlCallbackInterface};

use super::{WorkerAbortSignalState, WorkerAbortStore, local_object_in_scope, worker_abort_store};
use crate::callback_invocation::{CallbackInvocation, CallbackInvocationOutcome, CallbackInvoker};
use crate::context_bootstrap::abort_signal_events;
use crate::context_bootstrap::{
    EVENT_PASSIVE_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT, construct_original_event,
    event_internal_bool_flag, set_event_internal_flag,
};
use crate::exception_reporting::{CallbackExceptionLogLevel, invoke_callback};
use crate::util::v8str;
use crate::webidl;

/// Identity of one EventListener registration inside a worker run.
///
/// It never crosses the worker boundary. `WorkerAbortStore` is owned by the
/// worker isolate, so destroying or restarting that run retires every id and
/// callback context together.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct WorkerAbortListenerId(u64);

/// Signal-local registration state.
///
/// The callback owns its conversion-time relevant/incumbent contexts. The
/// surrounding record owns only EventTarget policy: order, duplicate identity,
/// capture, once, and passive.
pub(super) struct WorkerAbortListener {
    id: WorkerAbortListenerId,
    callback: WebIdlCallbackInterface,
    capture: bool,
    once: bool,
    passive: bool,
}

/// An immutable view of which registrations existed when dispatch started.
///
/// Only ids are copied. Every id is claimed again immediately before
/// invocation so removal by an earlier listener is observable, while listeners
/// added during dispatch wait for the next dispatch.
pub(super) struct WorkerAbortDispatchSnapshot {
    listener_ids: Vec<WorkerAbortListenerId>,
    onabort: Option<v8::Global<v8::Function>>,
}

struct PreparedWorkerAbortListener {
    callback: PreparedWebIdlCallbackInterface,
    passive: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AbortSignal.addEventListener")]
struct WorkerAbortAddEventListenerArgs {
    #[webidl(required)]
    event_type: String,
    #[webidl(required, converter = "callback_interface", nullable)]
    listener: Option<WebIdlCallbackInterface>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AbortSignal.removeEventListener")]
struct WorkerAbortRemoveEventListenerArgs {
    #[webidl(required)]
    event_type: String,
    #[webidl(required, converter = "callback_interface", nullable)]
    listener: Option<WebIdlCallbackInterface>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AbortSignal.dispatchEvent")]
struct WorkerAbortDispatchEventArgs<'s> {
    #[webidl(required)]
    event: v8::Local<'s, v8::Value>,
}

impl WorkerAbortSignalState {
    pub(super) fn dispatch_snapshot(&self, event_type: &str) -> WorkerAbortDispatchSnapshot {
        let listener_ids = self
            .listeners
            .get(event_type)
            .into_iter()
            .flatten()
            .map(|listener| listener.id)
            .collect();
        let onabort = (event_type == "abort")
            .then(|| self.onabort.clone())
            .flatten();
        WorkerAbortDispatchSnapshot {
            listener_ids,
            onabort,
        }
    }
}

impl WorkerAbortStore {
    fn allocate_listener_id(&mut self) -> WorkerAbortListenerId {
        self.next_listener_id = self
            .next_listener_id
            .checked_add(1)
            .expect("worker AbortSignal listener id space exhausted");
        WorkerAbortListenerId(self.next_listener_id)
    }

    fn register_event_listener(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        signal_id: u32,
        event_type: String,
        callback: WebIdlCallbackInterface,
        options: webidl::EventListenerOptions,
    ) -> bool {
        let callback_value = callback.value(scope);
        let Ok(callback_object) = v8::Local::<v8::Object>::try_from(callback_value) else {
            return false;
        };
        let Some(state) = self.signal_state(signal_id) else {
            return false;
        };
        if state
            .listeners
            .get(&event_type)
            .into_iter()
            .flatten()
            .any(|listener| {
                listener.capture == options.capture
                    && listener.callback.matches(scope, callback_object)
            })
        {
            return false;
        }

        let id = self.allocate_listener_id();
        let state = self
            .signal_state_mut(signal_id)
            .expect("validated worker AbortSignal state must remain resident");
        state
            .listeners
            .entry(event_type)
            .or_default()
            .push(WorkerAbortListener {
                id,
                callback,
                capture: options.capture,
                once: options.once,
                passive: options.passive.unwrap_or(false),
            });
        true
    }

    fn remove_event_listener(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        signal_id: u32,
        event_type: &str,
        callback: &WebIdlCallbackInterface,
        capture: bool,
    ) -> bool {
        let Some(state) = self.signal_state_mut(signal_id) else {
            return false;
        };
        let Some(listeners) = state.listeners.get_mut(event_type) else {
            return false;
        };
        let callback = callback.value(scope);
        let Ok(callback) = v8::Local::<v8::Object>::try_from(callback) else {
            return false;
        };
        let before = listeners.len();
        listeners.retain(|listener| {
            listener.capture != capture || !listener.callback.matches(scope, callback)
        });
        let removed = listeners.len() != before;
        if listeners.is_empty() {
            state.listeners.remove(event_type);
        }
        removed
    }

    fn claim_event_listener(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        signal_id: u32,
        event_type: &str,
        listener_id: WorkerAbortListenerId,
    ) -> Option<PreparedWorkerAbortListener> {
        let state = self.signal_state_mut(signal_id)?;
        let listeners = state.listeners.get_mut(event_type)?;
        let index = listeners
            .iter()
            .position(|listener| listener.id == listener_id)?;
        let callback = listeners[index].callback.prepare(scope);
        let passive = listeners[index].passive;
        if listeners[index].once {
            listeners.remove(index);
        }
        if listeners.is_empty() {
            state.listeners.remove(event_type);
        }
        Some(PreparedWorkerAbortListener { callback, passive })
    }
}

pub(crate) fn worker_abort_signal_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(store) = worker_abort_store(scope) else {
        rv.set_undefined();
        return;
    };
    let signal = args.this();
    let Some(signal_id) = WorkerAbortStore::signal_id_from_object(scope, signal) else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<WorkerAbortAddEventListenerArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let Some(listener) = parsed.listener else {
        rv.set_undefined();
        return;
    };
    let options = webidl::event_listener_options(scope, &args, 2, true);
    store.borrow_mut().register_event_listener(
        scope,
        signal_id,
        parsed.event_type,
        listener,
        options,
    );
    rv.set_undefined();
}

pub(crate) fn worker_abort_signal_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(store) = worker_abort_store(scope) else {
        rv.set_undefined();
        return;
    };
    let signal = args.this();
    let Some(signal_id) = WorkerAbortStore::signal_id_from_object(scope, signal) else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<WorkerAbortRemoveEventListenerArgs>(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let Some(listener) = parsed.listener else {
        rv.set_undefined();
        return;
    };
    let capture = webidl::event_listener_options(scope, &args, 2, true).capture;
    store.borrow_mut().remove_event_listener(
        scope,
        signal_id,
        &parsed.event_type,
        &listener,
        capture,
    );
    rv.set_undefined();
}

pub(crate) fn worker_abort_signal_dispatch_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(store) = worker_abort_store(scope) else {
        rv.set_bool(false);
        return;
    };
    let signal = args.this();
    let Some(parsed) = webidl::parse_args::<WorkerAbortDispatchEventArgs<'s>>(scope, &args) else {
        rv.set_bool(false);
        return;
    };
    let Some(signal_id) = WorkerAbortStore::signal_id_from_object(scope, signal) else {
        rv.set_bool(false);
        return;
    };
    let Some((event, event_type)) =
        abort_signal_events::prepare_script_dispatch(scope, parsed.event)
    else {
        rv.set_bool(false);
        return;
    };

    let default_prevented_key = v8str(scope, "defaultPrevented");
    let signal = local_object_in_scope(scope, signal);
    let event = local_object_in_scope(scope, event);
    let dispatch_snapshot = store
        .borrow()
        .signal_state(signal_id)
        .map(|state| state.dispatch_snapshot(&event_type));
    if let Some(dispatch_snapshot) = dispatch_snapshot {
        dispatch_event_callbacks(
            &store,
            scope,
            signal,
            signal_id,
            &event_type,
            dispatch_snapshot,
            event,
        );
    }
    let result = event
        .get(scope, default_prevented_key.into())
        .is_none_or(|value| !value.boolean_value(scope));
    rv.set_bool(result);
}

pub(super) fn dispatch_abort<'s>(
    store: &Rc<RefCell<WorkerAbortStore>>,
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    signal_id: u32,
    dispatch_snapshot: WorkerAbortDispatchSnapshot,
) {
    let context = signal
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(event) = construct_original_event(scope, "abort") else {
        return;
    };
    dispatch_event_callbacks(
        store,
        scope,
        signal,
        signal_id,
        "abort",
        dispatch_snapshot,
        event,
    );
}

#[allow(clippy::too_many_arguments)]
fn dispatch_event_callbacks<'s>(
    store: &Rc<RefCell<WorkerAbortStore>>,
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    signal_id: u32,
    event_type: &str,
    dispatch_snapshot: WorkerAbortDispatchSnapshot,
    event: v8::Local<'s, v8::Object>,
) {
    if !abort_signal_events::begin_dispatch(scope, signal, event) {
        abort_signal_events::finish_dispatch(scope, event);
        return;
    }
    for listener_id in dispatch_snapshot.listener_ids {
        let listener =
            store
                .borrow_mut()
                .claim_event_listener(scope, signal_id, event_type, listener_id);
        let Some(listener) = listener else {
            continue;
        };
        set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, listener.passive);
        invoke_worker_abort_event_listener(
            scope,
            &format!("Worker AbortSignal {event_type} listener"),
            listener.callback,
            signal,
            event,
        );
        set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, false);
        if event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT) {
            break;
        }
    }

    if !event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT)
        && let Some(onabort) = dispatch_snapshot.onabort
    {
        let onabort = v8::Local::new(scope, &onabort);
        let _ = invoke_callback(
            scope,
            "Worker AbortSignal.onabort",
            onabort,
            signal.into(),
            &[event.into()],
        );
    }
    abort_signal_events::finish_dispatch(scope, event);
}

fn invoke_worker_abort_event_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback_name: &str,
    callback: PreparedWebIdlCallbackInterface,
    signal: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) {
    let arguments = [event.into()];
    let invocation = CallbackInvocation::new(
        callback.callback(scope),
        signal.into(),
        callback.relevant_context(scope),
        callback.incumbent_context(scope),
        callback.callable_at_conversion(),
        "handleEvent",
        &arguments,
        None,
    );
    if let CallbackInvocationOutcome::Threw(report) = CallbackInvoker::invoke(
        scope,
        "event listener",
        "worker AbortSignal listener threw",
        CallbackExceptionLogLevel::Debug,
        callback_name,
        invocation,
    ) {
        let _ = crate::worker::dispatch_current_worker_callback_exception(scope, *report);
    }
}
