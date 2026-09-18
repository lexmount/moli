//! Recover once from the source's error. Replacement errors pass downstream;
//! the recovery subscription does not retain the exhausted source or catcher.

use moli_webapi_declare::WebApiObject;

use super::{
    callbacks, from,
    observer::{self, Notification},
    state::*,
    subscribe_internal, subscriber_complete, subscriber_error, subscriber_next,
};
use crate::{abort_signal_route::ResolvedAbortSignal, util::set_private_value, webidl};

const SOURCE: &str = "__moliObservableCatchSource";
const CALLBACK: &str = "__moliObservableCatchCallback";
const DOWNSTREAM: &str = "__moliCatchSubscriber";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.catch")]
struct CatchArgs {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
}

pub(super) fn catch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CatchArgs>(scope, &args) else {
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
struct CatchObserver<'scope> {
    #[webapi(slot = observer::NATIVE_KIND)]
    kind: i32,
    #[webapi(slot = DOWNSTREAM)]
    downstream: v8::Local<'scope, v8::Object>,
}

fn subscribe_upstream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
    observer: v8::Local<'s, v8::Object>,
) {
    if active(scope, subscriber) {
        // Replace this edge when recovery starts. A pending terminal Promise
        // keeps the replacement producer alive without retaining the old one.
        set_private_value(scope, subscriber, UPSTREAM_OBSERVER, observer.into());
    }
    let signal = object_slot(scope, subscriber, SIGNAL)
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
        .expect("catch Subscriber signal");
    subscribe_internal(scope, observable, observer, Some(signal));
}

pub(super) fn subscribe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(source) = object_slot(scope, observable, SOURCE) else {
        return false;
    };
    let callback = object_slot(scope, observable, CALLBACK).expect("Observable catcher");
    let Some(observer) = CatchObserver::new(observer::CATCH_SOURCE, subscriber)
        .bind(scope)
        .ok()
    else {
        return true;
    };
    set_private_value(scope, observer, CALLBACK, callback.into());
    subscribe_upstream(scope, source, subscriber, observer);
    true
}

fn recover<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    downstream: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    let callback = object_slot(scope, observer, CALLBACK).expect("catch callback");
    // Source.error already closed its Subscriber before notifying us. Neither
    // its producer nor the one-shot callback belongs to the replacement graph.
    for slot in [observer::SUBSCRIBER, CALLBACK] {
        set_private_value(scope, observer, slot, v8::undefined(scope).into());
    }
    let result = match callbacks::invoke_value(scope, callback, &[error]) {
        Ok(result) => result,
        Err(error) => {
            subscriber_error(scope, downstream, error);
            return;
        }
    };
    let (inner, error) = {
        v8::tc_scope!(let scope, scope);
        let inner = from::convert(scope, result);
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
    let Some(inner_observer) = CatchObserver::new(observer::CATCH_INNER, downstream)
        .bind(scope)
        .ok()
    else {
        return;
    };
    // Cancellation inside the callback/conversion still initializes a script
    // replacement with an inactive Subscriber, following pre-aborted subscribe.
    subscribe_upstream(scope, inner, downstream, inner_observer);
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
    kind: i32,
) {
    let downstream = object_slot(scope, observer, DOWNSTREAM).expect("catch downstream");
    match notification {
        Notification::Next(value) => subscriber_next(scope, downstream, value),
        Notification::Error(error) if kind == observer::CATCH_SOURCE => {
            recover(scope, observer, downstream, error);
        }
        Notification::Error(error) => subscriber_error(scope, downstream, error),
        Notification::Complete => subscriber_complete(scope, downstream),
    }
}
