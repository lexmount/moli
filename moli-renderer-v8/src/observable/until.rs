//! takeUntil subscribes to its notifier before its source. Both observers use
//! the downstream signal, and its traced upstream link keeps both alive.

use moli_webapi_declare::WebApiObject;

use super::{
    from,
    observer::{self, Notification},
    state::*,
    subscribe_internal, subscriber_complete, subscriber_error, subscriber_next,
};
use crate::{abort_signal_route::ResolvedAbortSignal, util::set_private_value, webidl};

const SOURCE: &str = "__moliObservableUntilSource";
const NOTIFIER: &str = "__moliObservableUntilNotifier";
const DOWNSTREAM: &str = "__moliObservableUntilSubscriber";
const NOTIFIER_OBSERVER: &str = "__moliObservableUntilNotifierObserver";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.takeUntil")]
struct UntilArgs<'scope> {
    #[webidl(required, converter = "raw")]
    value: v8::Local<'scope, v8::Value>,
}

pub(super) fn take_until<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<UntilArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(notifier) = from::convert(scope, parsed.value) else {
        return;
    };
    let Some(observable) = new_native_observable(scope, None) else {
        return;
    };
    set_private_value(scope, observable, SOURCE, args.this().into());
    set_private_value(scope, observable, NOTIFIER, notifier.into());
    rv.set(observable.into());
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct UntilObserver<'scope> {
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
    let notifier = object_slot(scope, observable, NOTIFIER).expect("takeUntil notifier");
    let Some(source_observer) = UntilObserver::new(observer::UNTIL_SOURCE, subscriber)
        .bind(scope)
        .ok()
    else {
        return true;
    };
    let Some(notifier_observer) = UntilObserver::new(observer::UNTIL_NOTIFIER, subscriber)
        .bind(scope)
        .ok()
    else {
        return true;
    };
    if active(scope, subscriber) {
        // A pending terminal Promise must retain both producers. Reuse the
        // downstream link that close() roots and clears before running abort.
        set_private_value(
            scope,
            source_observer,
            NOTIFIER_OBSERVER,
            notifier_observer.into(),
        );
        set_private_value(scope, subscriber, UPSTREAM_OBSERVER, source_observer.into());
    }
    let signal = object_slot(scope, subscriber, SIGNAL)
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
        .expect("takeUntil Subscriber signal");
    subscribe_internal(scope, notifier, notifier_observer, Some(signal));
    if active(scope, subscriber) {
        // A synchronous notifier next/error (or cancellation during its
        // initialization) prevents source initialization altogether.
        subscribe_internal(scope, source, source_observer, Some(signal));
    }
    true
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
    kind: i32,
) {
    let downstream = object_slot(scope, observer, DOWNSTREAM).expect("takeUntil downstream");
    if kind == observer::UNTIL_NOTIFIER {
        match notification {
            Notification::Next(_) | Notification::Error(_) => {
                subscriber_complete(scope, downstream)
            }
            Notification::Complete => {
                // Completion without a value does not stop the source. Stop
                // retaining the exhausted notifier while that source is live.
                if let Some(source_observer) = object_slot(scope, downstream, UPSTREAM_OBSERVER) {
                    set_private_value(
                        scope,
                        source_observer,
                        NOTIFIER_OBSERVER,
                        v8::undefined(scope).into(),
                    );
                }
            }
        }
    } else {
        match notification {
            Notification::Next(value) => subscriber_next(scope, downstream, value),
            Notification::Error(error) => subscriber_error(scope, downstream, error),
            Notification::Complete => subscriber_complete(scope, downstream),
        }
    }
}
