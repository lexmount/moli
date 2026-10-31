//! Inspect callbacks run before forwarding notifications. Its abort callback
//! observes consumer cancellation and is removed before producer termination.

use moli_webapi_declare::WebApiObject;

use super::{
    dictionary, invoke, invoke_and_report,
    observer::{self, Notification},
    observer_callbacks,
    state::*,
    subscribe_internal, subscriber_complete, subscriber_error, subscriber_next,
};
use crate::{abort_signal_route::ResolvedAbortSignal, util::set_private_value, webidl};

const SOURCE: &str = "__moliObservableInspectSource";
const INSPECTOR: &str = "__moliObservableInspector";
const SUBSCRIBE: &str = "__moliInspectorSubscribe";
const ABORT: &str = "__moliInspectorAbort";
const DOWNSTREAM: &str = "__moliInspectorSubscriber";
const ABORT_HANDLER: &str = "__moliInspectorAbortHandler";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.inspect")]
struct InspectArgs<'scope> {
    #[webidl(with = inspector_arg)]
    inspector: v8::Local<'scope, v8::Object>,
}

fn inspector_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    observer_callbacks(
        scope,
        dictionary(args.get(index), "ObservableInspector must be an object")?,
        "Observable.inspect",
        "ObservableInspector",
        &[
            ("abort", ABORT),
            ("complete", COMPLETE),
            ("error", ERROR),
            ("next", NEXT),
            ("subscribe", SUBSCRIBE),
        ],
    )
}

pub(super) fn inspect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<InspectArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(observable) = new_native_observable(scope, None) else {
        return;
    };
    set_private_value(scope, observable, SOURCE, args.this().into());
    set_private_value(scope, observable, INSPECTOR, parsed.inspector.into());
    rv.set(observable.into());
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct InspectObserver<'scope> {
    #[webapi(slot = observer::NATIVE_KIND)]
    kind: i32,
    #[webapi(slot = DOWNSTREAM)]
    downstream: v8::Local<'scope, v8::Object>,
}

pub(super) fn subscribe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(source) = object_slot(scope, observable, SOURCE) else {
        return false;
    };
    let inspector = object_slot(scope, observable, INSPECTOR).expect("Observable inspector");
    if let Some(callback) = object_slot(scope, inspector, SUBSCRIBE)
        && let Some(error) = invoke(scope, callback, &[])
    {
        subscriber_error(scope, subscriber, error);
        return true;
    }
    let Some(observer) = InspectObserver::new(observer::INSPECT, subscriber)
        .bind(scope)
        .ok()
    else {
        return true;
    };
    // The subscription retains its notification callbacks, not the reusable
    // inspector dictionary or its already-invoked subscribe callback.
    for slot in [NEXT, ERROR, COMPLETE] {
        if let Some(callback) = object_slot(scope, inspector, slot) {
            set_private_value(scope, observer, slot, callback.into());
        }
    }
    if active(scope, subscriber) {
        set_private_value(scope, subscriber, UPSTREAM_OBSERVER, observer.into());
    }
    let signal = object_slot(scope, subscriber, SIGNAL)
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
        .expect("Inspect Subscriber signal");
    if !signal.is_aborted(scope)
        && let Some(callback) = object_slot(scope, inspector, ABORT)
    {
        let algorithm = v8::Function::builder(aborted)
            .data(observer.into())
            .build(scope)
            .expect("Inspect abort algorithm");
        set_private_value(scope, observer, ABORT, callback.into());
        set_private_value(scope, observer, ABORT_HANDLER, algorithm.into());
        // Register before upstream cancellation so the hook runs before
        // source cleanup. The observer traces the hook, allowing abandoned
        // subscription graphs to be collected without a Rust signal root.
        signal.register_weak_algorithm(scope, algorithm);
    }
    // Pre-aborted subscribers still initialize a script source with an inactive
    // Subscriber, even when the inspector's subscribe callback just cancelled it.
    subscribe_internal(scope, source, observer, Some(signal));
    true
}

fn remove_abort_handler<'s>(scope: &mut v8::PinScope<'s, '_>, observer: v8::Local<'s, v8::Object>) {
    if let Some(algorithm) = object_slot(scope, observer, ABORT_HANDLER)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let downstream = object_slot(scope, observer, DOWNSTREAM).expect("Inspect downstream");
        let signal = object_slot(scope, downstream, SIGNAL)
            .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
            .expect("Inspect Subscriber signal");
        signal.unregister_algorithm(scope, algorithm);
        set_private_value(scope, observer, ABORT_HANDLER, v8::undefined(scope).into());
        set_private_value(scope, observer, ABORT, v8::undefined(scope).into());
    }
}

fn aborted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let observer = v8::Local::<v8::Object>::try_from(args.data()).expect("Inspect abort data");
    let callback = object_slot(scope, observer, ABORT);
    remove_abort_handler(scope, observer);
    if let Some(callback) = callback {
        // A consumer has already closed downstream. Report hook exceptions in
        // the callback's realm while continuing upstream cancellation.
        invoke_and_report(scope, callback, &[args.get(0)]);
    }
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
) {
    let downstream = object_slot(scope, observer, DOWNSTREAM).expect("Inspect downstream");
    let (slot, arguments) = match notification {
        Notification::Next(value) => (NEXT, Some(value)),
        Notification::Error(error) => {
            remove_abort_handler(scope, observer);
            (ERROR, Some(error))
        }
        Notification::Complete => {
            remove_abort_handler(scope, observer);
            (COMPLETE, None)
        }
    };
    if let Some(callback) = object_slot(scope, observer, slot)
        && let Some(error) = invoke(scope, callback, arguments.as_slice())
    {
        remove_abort_handler(scope, observer);
        subscriber_error(scope, downstream, error);
        return;
    }
    match notification {
        Notification::Next(value) => subscriber_next(scope, downstream, value),
        Notification::Error(error) => subscriber_error(scope, downstream, error),
        Notification::Complete => subscriber_complete(scope, downstream),
    }
}
