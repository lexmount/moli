//! Switch inner subscriptions through the existing AbortSignal dependency graph.
//! Reentrant mapper/teardown calls can leave multiple live inner observers, so
//! trace each until its own subscription completes or is cancelled.

use moli_webapi_declare::WebApiObject;

use super::{
    callbacks, from,
    observer::{self, Notification},
    state::*,
    subscribe_internal, subscriber_complete, subscriber_error, subscriber_next,
};
use crate::{
    abort_signal_route::ResolvedAbortSignal,
    util::{get_private_value, set_private_value},
    webidl,
};

const SOURCE: &str = "__moliObservableSwitchMapSource";
const MAPPER: &str = "__moliObservableSwitchMapMapper";
const DOWNSTREAM: &str = "__moliSwitchMapSubscriber";
const OWNER: &str = "__moliSwitchMapSourceObserver";
const INNERS: &str = "__moliSwitchMapInnerObservers";
const CURRENT_SIGNAL: &str = "__moliSwitchMapCurrentSignal";
const OUTER_COMPLETE: &str = "__moliSwitchMapSourceCompleted";
const INNER_SIGNAL: &str = "__moliSwitchMapInnerSignal";
const CLEANUP: &str = "__moliSwitchMapInnerCleanup";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.switchMap")]
struct SwitchMapArgs {
    #[webidl(required, converter = "callback_function")]
    mapper: webidl::WebIdlCallbackFunction,
}

pub(super) fn switch_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SwitchMapArgs>(scope, &args) else {
        return;
    };
    let Some(observable) = new_native_observable(scope, None) else {
        return;
    };
    set_private_value(scope, observable, SOURCE, args.this().into());
    set_callback(scope, observable, MAPPER, parsed.mapper);
    rv.set(observable.into());
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct SourceObserver<'scope> {
    #[webapi(slot = observer::NATIVE_KIND)]
    kind: i32,
    #[webapi(slot = DOWNSTREAM)]
    downstream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = MAPPER)]
    mapper: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct InnerObserver<'scope> {
    #[webapi(slot = observer::NATIVE_KIND)]
    kind: i32,
    #[webapi(slot = OWNER)]
    owner: v8::Local<'scope, v8::Object>,
    #[webapi(slot = INNER_SIGNAL)]
    signal: v8::Local<'scope, v8::Object>,
}

fn signal_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<ResolvedAbortSignal<'s>> {
    object_slot(scope, object, slot).and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
}

pub(super) fn subscribe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(source) = object_slot(scope, observable, SOURCE) else {
        return false;
    };
    let mapper = object_slot(scope, observable, MAPPER).expect("switchMap mapper");
    let Some(observer) = SourceObserver::new(observer::SWITCH_MAP_SOURCE, subscriber, mapper)
        .bind(scope)
        .ok()
    else {
        return true;
    };
    if active(scope, subscriber) {
        set_private_value(scope, subscriber, UPSTREAM_OBSERVER, observer.into());
    }
    let signal = signal_slot(scope, subscriber, SIGNAL).expect("switchMap Subscriber signal");
    subscribe_internal(scope, source, observer, Some(signal));
    true
}

fn release_inner<'s>(scope: &mut v8::PinScope<'s, '_>, inner: v8::Local<'s, v8::Object>) {
    let owner = object_slot(scope, inner, OWNER).expect("switchMap owner");
    let mut inners = list(scope, owner, INNERS);
    inners.retain(|entry| *entry != inner);
    set_list(scope, owner, INNERS, &inners);
    if let Some(signal) = signal_slot(scope, inner, INNER_SIGNAL)
        && let Some(callback) = object_slot(scope, inner, CLEANUP)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        signal.unregister_algorithm(scope, callback);
    }
    // Root the producer through the cancellation snapshot before dropping its
    // persistent edge. Its native cancellation callback is deliberately weak.
    let _producer = object_slot(scope, inner, observer::SUBSCRIBER);
    for slot in [INNER_SIGNAL, CLEANUP, observer::SUBSCRIBER] {
        set_private_value(scope, inner, slot, v8::undefined(scope).into());
    }
}

fn cancelled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let inner = v8::Local::<v8::Object>::try_from(args.data()).expect("switchMap cleanup data");
    release_inner(scope, inner);
}

fn process_next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    if let Some(signal) = signal_slot(scope, observer, CURRENT_SIGNAL) {
        let error = {
            v8::tc_scope!(let scope, scope);
            let reason = crate::native_bridge::abort::abort_error_value(scope);
            signal.abort(scope, reason);
            let error = scope.exception();
            scope.reset();
            error
        };
        // Internal switching must finish even if IteratorClose fails. Explicit
        // author cancellation still uses the AbortController rethrow boundary.
        if let Some(error) = error {
            callbacks::report(scope, observer, error);
        }
    }
    let Some(initial_signal) = ResolvedAbortSignal::new(scope) else {
        return;
    };
    set_private_value(
        scope,
        observer,
        CURRENT_SIGNAL,
        initial_signal.value().into(),
    );
    let downstream = object_slot(scope, observer, DOWNSTREAM).expect("switchMap downstream");
    let mapper = object_slot(scope, observer, MAPPER).expect("switchMap mapper");
    let index = observer::index(scope, observer);
    let index = v8::Number::new(scope, index as f64).into();
    let mapped = match callbacks::invoke_value(scope, mapper, &[value, index]) {
        Ok(mapped) => mapped,
        Err(error) => {
            subscriber_error(scope, downstream, error);
            return;
        }
    };
    observer::increment_index(scope, observer);
    let (inner, error) = {
        v8::tc_scope!(let scope, scope);
        let inner = from::convert(scope, mapped);
        let error = scope.exception();
        scope.reset();
        (inner, error)
    };
    if let Some(error) = error {
        subscriber_error(scope, downstream, error);
        return;
    }
    let Some(inner) = inner else {
        return;
    };
    // The draft passes the current controller by reference: mapper/conversion
    // reentrancy can replace it before subscription. If a reentrant inner also
    // completed, it cleared that reference; the original (now aborted) signal
    // safely initializes this superseded inner as inactive instead.
    let current = signal_slot(scope, observer, CURRENT_SIGNAL).unwrap_or(initial_signal);
    let downstream_signal = signal_slot(scope, downstream, SIGNAL).expect("switchMap signal");
    let Some(signal) = ResolvedAbortSignal::dependent(scope, &[current, downstream_signal]) else {
        return;
    };
    let Some(inner_observer) =
        InnerObserver::new(observer::SWITCH_MAP_INNER, observer, signal.value())
            .bind(scope)
            .ok()
    else {
        return;
    };
    if !signal.is_aborted(scope) {
        let mut inners = list(scope, observer, INNERS);
        inners.push(inner_observer);
        set_list(scope, observer, INNERS, &inners);
        let cleanup = v8::Function::builder(cancelled)
            .data(inner_observer.into())
            .build(scope)
            .expect("switchMap inner cleanup");
        set_private_value(scope, inner_observer, CLEANUP, cleanup.into());
        signal.register_weak_algorithm(scope, cleanup);
    }
    subscribe_internal(scope, inner, inner_observer, Some(signal));
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
    kind: i32,
) {
    let source = if kind == observer::SWITCH_MAP_INNER {
        object_slot(scope, observer, OWNER).expect("switchMap source observer")
    } else {
        observer
    };
    let downstream = object_slot(scope, source, DOWNSTREAM).expect("switchMap downstream");
    match notification {
        Notification::Next(value) if kind == observer::SWITCH_MAP_SOURCE => {
            process_next(scope, source, value);
        }
        Notification::Next(value) => subscriber_next(scope, downstream, value),
        Notification::Error(error) => {
            if kind == observer::SWITCH_MAP_INNER {
                release_inner(scope, observer);
            }
            subscriber_error(scope, downstream, error);
        }
        Notification::Complete => {
            if kind == observer::SWITCH_MAP_SOURCE {
                set_private_value(
                    scope,
                    observer,
                    observer::SUBSCRIBER,
                    v8::undefined(scope).into(),
                );
                set_private_value(
                    scope,
                    source,
                    OUTER_COMPLETE,
                    v8::Boolean::new(scope, true).into(),
                );
                if signal_slot(scope, source, CURRENT_SIGNAL).is_none() {
                    subscriber_complete(scope, downstream);
                }
            } else {
                release_inner(scope, observer);
                if get_private_value(scope, source, OUTER_COMPLETE).is_some_and(|v| v.is_true()) {
                    subscriber_complete(scope, downstream);
                } else {
                    set_private_value(scope, source, CURRENT_SIGNAL, v8::undefined(scope).into());
                }
            }
        }
    }
}
