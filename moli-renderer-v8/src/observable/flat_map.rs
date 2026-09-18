//! Serial flattening keeps raw source values queued until the active inner
//! subscription completes. Queue cells and both observers are V8-traced.

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

const SOURCE: &str = "__moliObservableFlatMapSource";
const MAPPER: &str = "__moliObservableFlatMapMapper";
const DOWNSTREAM: &str = "__moliFlatMapSubscriber";
const OWNER: &str = "__moliFlatMapSourceObserver";
const INNER: &str = "__moliFlatMapInnerObserver";
const BUSY: &str = "__moliFlatMapActiveInner";
const OUTER_COMPLETE: &str = "__moliFlatMapSourceCompleted";
const HEAD: &str = "__moliFlatMapQueueHead";
const TAIL: &str = "__moliFlatMapQueueTail";
const VALUE: &str = "__moliFlatMapQueuedValue";
const NEXT: &str = "__moliFlatMapQueueNext";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.flatMap")]
struct FlatMapArgs {
    #[webidl(required, converter = "callback_function")]
    mapper: webidl::WebIdlCallbackFunction,
}

pub(super) fn flat_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FlatMapArgs>(scope, &args) else {
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
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct QueueEntry<'scope> {
    #[webapi(slot = VALUE)]
    value: v8::Local<'scope, v8::Value>,
}

pub(super) fn subscribe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(source) = object_slot(scope, observable, SOURCE) else {
        return false;
    };
    let mapper = object_slot(scope, observable, MAPPER).expect("flatMap mapper");
    let Some(observer) = SourceObserver::new(observer::FLAT_MAP_SOURCE, subscriber, mapper)
        .bind(scope)
        .ok()
    else {
        return true;
    };
    if active(scope, subscriber) {
        set_private_value(scope, subscriber, UPSTREAM_OBSERVER, observer.into());
    }
    let signal = signal(scope, subscriber);
    subscribe_internal(scope, source, observer, Some(signal));
    true
}

fn signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    subscriber: v8::Local<'s, v8::Object>,
) -> ResolvedAbortSignal<'s> {
    object_slot(scope, subscriber, SIGNAL)
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
        .expect("flatMap Subscriber signal")
}

fn flag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    slot: &str,
) -> bool {
    get_private_value(scope, observer, slot).is_some_and(|value| value.is_true())
}

fn enqueue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let entry = QueueEntry::new(value)
        .bind(scope)
        .expect("flatMap queue entry");
    if let Some(tail) = object_slot(scope, observer, TAIL) {
        set_private_value(scope, tail, NEXT, entry.into());
    } else {
        set_private_value(scope, observer, HEAD, entry.into());
    }
    set_private_value(scope, observer, TAIL, entry.into());
}

fn dequeue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let head = object_slot(scope, observer, HEAD)?;
    let value = get_private_value(scope, head, VALUE).expect("flatMap queued value");
    let next = object_slot(scope, head, NEXT);
    let next_value = next.map_or_else(|| v8::undefined(scope).into(), Into::into);
    set_private_value(scope, observer, HEAD, next_value);
    if next.is_none() {
        set_private_value(scope, observer, TAIL, v8::undefined(scope).into());
    }
    Some(value)
}

fn process_next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let downstream = object_slot(scope, observer, DOWNSTREAM).expect("flatMap downstream");
    let mapper = object_slot(scope, observer, MAPPER).expect("flatMap mapper");
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
    let Some(inner_observer) = InnerObserver::new(observer::FLAT_MAP_INNER, observer)
        .bind(scope)
        .ok()
    else {
        return;
    };
    set_private_value(scope, observer, INNER, inner_observer.into());
    // Conversion may cancel downstream. The source still receives its fresh,
    // inactive Subscriber, just as an explicitly pre-aborted subscribe does.
    let signal = signal(scope, downstream);
    subscribe_internal(scope, inner, inner_observer, Some(signal));
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
    kind: i32,
) {
    let source_observer = if kind == observer::FLAT_MAP_INNER {
        object_slot(scope, observer, OWNER).expect("flatMap source observer")
    } else {
        observer
    };
    let downstream = object_slot(scope, source_observer, DOWNSTREAM).expect("flatMap downstream");
    match notification {
        Notification::Next(value) if kind == observer::FLAT_MAP_SOURCE => {
            if flag(scope, source_observer, BUSY) {
                enqueue(scope, source_observer, value);
            } else {
                // Set before invoking mapper: reentrant source.next queues its
                // raw value and does not overlap the current mapper or inner.
                set_private_value(
                    scope,
                    source_observer,
                    BUSY,
                    v8::Boolean::new(scope, true).into(),
                );
                process_next(scope, source_observer, value);
            }
        }
        Notification::Next(value) => subscriber_next(scope, downstream, value),
        Notification::Error(error) => subscriber_error(scope, downstream, error),
        Notification::Complete => {
            set_private_value(
                scope,
                observer,
                observer::SUBSCRIBER,
                v8::undefined(scope).into(),
            );
            if kind == observer::FLAT_MAP_SOURCE {
                set_private_value(
                    scope,
                    source_observer,
                    OUTER_COMPLETE,
                    v8::Boolean::new(scope, true).into(),
                );
                if !flag(scope, source_observer, BUSY) {
                    subscriber_complete(scope, downstream);
                }
            } else {
                set_private_value(scope, source_observer, INNER, v8::undefined(scope).into());
                if let Some(value) = dequeue(scope, source_observer) {
                    // Run before this complete() returns, including when the
                    // next inner completes synchronously and reenters here.
                    process_next(scope, source_observer, value);
                } else {
                    set_private_value(
                        scope,
                        source_observer,
                        BUSY,
                        v8::Boolean::new(scope, false).into(),
                    );
                    if flag(scope, source_observer, OUTER_COMPLETE) {
                        subscriber_complete(scope, downstream);
                    }
                }
            }
        }
    }
}
