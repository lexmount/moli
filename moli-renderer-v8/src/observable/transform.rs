//! Lazy map/filter producers. Each shared downstream subscription owns one
//! upstream observer, whose callback and index are traced by V8.

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
const CALLBACK: &str = "__moliObservableTransformCallback";
const KIND: &str = "__moliObservableTransformKind";
const DOWNSTREAM: &str = "__moliObservableTransformSubscriber";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable")]
struct TransformArgs {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
}

pub(super) fn map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    create(scope, args, rv, observer::MAP);
}

pub(super) fn filter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    create(scope, args, rv, observer::FILTER);
}

fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: i32,
) {
    let Some(parsed) = webidl::parse_args::<TransformArgs>(scope, &args) else {
        return;
    };
    let Some(observable) = new_native_observable(scope, None) else {
        return;
    };
    set_private_value(scope, observable, SOURCE, args.this().into());
    set_private_value(
        scope,
        observable,
        KIND,
        v8::Integer::new(scope, kind).into(),
    );
    set_callback(scope, observable, CALLBACK, parsed.callback);
    rv.set(observable.into());
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct TransformObserver<'scope> {
    #[webapi(slot = observer::NATIVE_KIND)]
    kind: i32,
    #[webapi(slot = DOWNSTREAM)]
    downstream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = CALLBACK)]
    callback: v8::Local<'scope, v8::Object>,
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
    let callback = object_slot(scope, observable, CALLBACK).expect("Observable transform callback");
    let Some(observer) = TransformObserver::new(kind, subscriber, callback)
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
        Notification::Next(value) => {
            // A captured notification still invokes the callback after another
            // observer cancels this branch. Subscriber.next filters delivery;
            // Subscriber.error reports a callback failure after closure.
            let callback = object_slot(scope, observer, CALLBACK).expect("Transform callback");
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
