//! Lazy map/filter/take/drop producers. Each shared downstream subscription owns
//! one upstream observer, whose callback or remaining count is traced by V8.

use moli_webapi_declare::WebApiObject;

use super::{
    callbacks,
    observer::{self, Notification},
    state::*,
    subscribe_internal, subscriber_complete, subscriber_error, subscriber_next,
};
use crate::{
    abort_signal_route::ResolvedAbortSignal,
    util::{get_private_value, set_private_value},
    webidl,
};

const SOURCE: &str = "__moliObservableTransformSource";
const PARAMETER: &str = "__moliObservableTransformParameter";
const KIND: &str = "__moliObservableTransformKind";
const DOWNSTREAM: &str = "__moliObservableTransformSubscriber";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable")]
struct TransformArgs {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable")]
struct CountArgs {
    #[webidl(required, converter = "unsigned_long_long")]
    amount: u64,
}

pub(super) fn map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    create_callback(scope, args, rv, observer::MAP);
}

pub(super) fn filter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    create_callback(scope, args, rv, observer::FILTER);
}

pub(super) fn take<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    create_count(scope, args, rv, observer::TAKE);
}

pub(super) fn drop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    create_count(scope, args, rv, observer::DROP);
}

fn create_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: i32,
) {
    let Some(parsed) = webidl::parse_args::<TransformArgs>(scope, &args) else {
        return;
    };
    let Some(observable) = new_transform(scope, args.this(), kind) else {
        return;
    };
    set_callback(scope, observable, PARAMETER, parsed.callback);
    rv.set(observable.into());
}

fn create_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: i32,
) {
    let Some(parsed) = webidl::parse_args::<CountArgs>(scope, &args) else {
        return;
    };
    let Some(observable) = new_transform(scope, args.this(), kind) else {
        return;
    };
    set_count(scope, observable, parsed.amount);
    rv.set(observable.into());
}

fn new_transform<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    kind: i32,
) -> Option<v8::Local<'s, v8::Object>> {
    let observable = new_native_observable(scope, None)?;
    set_private_value(scope, observable, SOURCE, source.into());
    set_private_value(
        scope,
        observable,
        KIND,
        v8::Integer::new(scope, kind).into(),
    );
    Some(observable)
}

fn count<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> u64 {
    get_private_value(scope, object, PARAMETER)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .expect("Observable transform count")
        .u64_value()
        .0
}

fn set_count<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>, count: u64) {
    // Keep every bit after WebIDL conversion, including negative inputs modulo
    // 2^64; storing a Number would round u64::MAX back up to 2^64.
    set_private_value(
        scope,
        object,
        PARAMETER,
        v8::BigInt::new_from_u64(scope, count).into(),
    );
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct TransformObserver<'scope> {
    #[webapi(slot = observer::NATIVE_KIND)]
    kind: i32,
    #[webapi(slot = DOWNSTREAM)]
    downstream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = PARAMETER)]
    parameter: v8::Local<'scope, v8::Value>,
}

pub(super) fn subscribe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(source) = object_slot(scope, observable, SOURCE) else {
        return false;
    };
    let kind = get_private_value(scope, observable, KIND)
        .and_then(|value| value.int32_value(scope))
        .expect("Observable transform kind");
    if kind == observer::TAKE && count(scope, observable) == 0 {
        // take(0) completes synchronously without ever subscribing upstream.
        subscriber_complete(scope, subscriber);
        return true;
    }
    let parameter = get_private_value(scope, observable, PARAMETER).expect("Transform parameter");
    // Count state belongs to the subscription, not the reusable Observable.
    let Some(observer) = TransformObserver::new(kind, subscriber, parameter)
        .bind(scope)
        .ok()
    else {
        return true;
    };
    if active(scope, subscriber) {
        // The downstream (e.g. retained by a pending toArray() Promise) must
        // keep the upstream producer alive even if both Observables are GCed.
        // The whole graph is V8-traced, including its cancellation callback.
        set_private_value(scope, subscriber, UPSTREAM_OBSERVER, observer.into());
    }
    let signal = object_slot(scope, subscriber, SIGNAL)
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
        .expect("Transform Subscriber signal");
    // Even an already-closed downstream subscribes with its aborted signal:
    // a script producer is initialized with an inactive Subscriber, per subscribe.
    subscribe_internal(scope, source, observer, Some(signal));
    true
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
    kind: i32,
) {
    let downstream = object_slot(scope, observer, DOWNSTREAM).expect("Transform downstream");
    match notification {
        Notification::Next(value) if kind == observer::TAKE => {
            subscriber_next(scope, downstream, value);
            // Delivery may reenter next() and consume more values. Read the
            // current remaining count after delivery, as in the draft algorithm.
            let remaining = count(scope, observer).wrapping_sub(1);
            set_count(scope, observer, remaining);
            if remaining == 0 {
                subscriber_complete(scope, downstream);
            }
        }
        Notification::Next(value) if kind == observer::DROP => {
            let remaining = count(scope, observer);
            if remaining > 0 {
                set_count(scope, observer, remaining - 1);
            } else {
                subscriber_next(scope, downstream, value);
            }
        }
        Notification::Next(value) => {
            // A captured notification still invokes the callback after another
            // observer cancels this branch. Subscriber.next filters delivery;
            // Subscriber.error reports a callback failure after closure.
            let callback = object_slot(scope, observer, PARAMETER).expect("Transform callback");
            let index = observer::index(scope, observer);
            let index = v8::Number::new(scope, index as f64).into();
            match callbacks::invoke_value(scope, callback, &[value, index]) {
                Ok(result) => {
                    // Like the draft's Promise consumers, increment after
                    // callback invocation, and before delivering downstream.
                    observer::increment_index(scope, observer);
                    if kind == observer::MAP {
                        subscriber_next(scope, downstream, result);
                    } else if result.boolean_value(scope) {
                        subscriber_next(scope, downstream, value);
                    }
                }
                Err(error) => subscriber_error(scope, downstream, error),
            }
        }
        Notification::Error(error) => subscriber_error(scope, downstream, error),
        Notification::Complete => subscriber_complete(scope, downstream),
    }
}
