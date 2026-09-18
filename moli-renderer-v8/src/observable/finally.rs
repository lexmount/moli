//! Finalizers are downstream teardowns: upstream cancellation finishes first,
//! and callback exceptions are reported without replacing terminal notifications.

use moli_webapi_declare::WebApiObject;

use super::{
    observer::{self, Notification},
    state::*,
    subscribe_internal, subscriber_add_teardown, subscriber_complete, subscriber_error,
    subscriber_next,
};
use crate::{abort_signal_route::ResolvedAbortSignal, util::set_private_value, webidl};

const SOURCE: &str = "__moliObservableFinallySource";
const CALLBACK: &str = "__moliObservableFinallyCallback";
const DOWNSTREAM: &str = "__moliFinallySubscriber";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.finally")]
struct FinallyArgs {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
}

pub(super) fn finally<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FinallyArgs>(scope, &args) else {
        return;
    };
    let Some(observable) = new_native_observable(scope, None) else {
        return;
    };
    set_private_value(scope, observable, SOURCE, args.this().into());
    set_callback(scope, observable, CALLBACK, parsed.callback);
    rv.set(observable.into());
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct FinallyObserver<'scope> {
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
    let callback = object_slot(scope, observable, CALLBACK).expect("Observable finalizer");
    // Adding a teardown to an already-cancelled subscription invokes it now,
    // before initializing the source with its inactive Subscriber.
    subscriber_add_teardown(scope, subscriber, callback);
    let Some(observer) = FinallyObserver::new(observer::FINALLY, subscriber)
        .bind(scope)
        .ok()
    else {
        return true;
    };
    if active(scope, subscriber) {
        set_private_value(scope, subscriber, UPSTREAM_OBSERVER, observer.into());
    }
    let signal = object_slot(scope, subscriber, SIGNAL)
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
        .expect("Finally Subscriber signal");
    subscribe_internal(scope, source, observer, Some(signal));
    true
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
) {
    let downstream = object_slot(scope, observer, DOWNSTREAM).expect("Finally downstream");
    match notification {
        Notification::Next(value) => subscriber_next(scope, downstream, value),
        Notification::Error(error) => subscriber_error(scope, downstream, error),
        Notification::Complete => subscriber_complete(scope, downstream),
    }
}
