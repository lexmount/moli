//! Internal observer steps share notification ordering with script observers,
//! while script callbacks keep their typed Web IDL invocation boundary.

use super::{
    callbacks, collect, consume, first, inspect, invoke_and_report, state::*, transform, until,
};
use crate::util::{get_private_value, set_private_value};

pub(super) const NATIVE_KIND: &str = "__moliObservableNativeObserver";
pub(super) const FIRST: i32 = 1;
pub(super) const LAST: i32 = 2;
pub(super) const TO_ARRAY: i32 = 3;
pub(super) const FOR_EACH: i32 = 4;
pub(super) const REDUCE: i32 = 5;
pub(super) const SOME: i32 = 6;
pub(super) const EVERY: i32 = 7;
pub(super) const FIND: i32 = 8;
pub(super) const MAP: i32 = 9;
pub(super) const FILTER: i32 = 10;
pub(super) const TAKE: i32 = 11;
pub(super) const DROP: i32 = 12;
pub(super) const UNTIL_SOURCE: i32 = 13;
pub(super) const UNTIL_NOTIFIER: i32 = 14;
pub(super) const INSPECT: i32 = 15;
pub(super) const SUBSCRIBER: &str = "__moliObservableNativeSubscriber";
const INDEX: &str = "__moliObservableCallbackIndex";

pub(super) fn index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> u64 {
    get_private_value(scope, observer, INDEX)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map_or(0, |value| value.u64_value().0)
}

pub(super) fn increment_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) {
    let next = index(scope, observer).wrapping_add(1);
    set_private_value(
        scope,
        observer,
        INDEX,
        v8::BigInt::new_from_u64(scope, next).into(),
    );
}

#[derive(Clone, Copy)]
pub(super) enum Notification<'s> {
    Next(v8::Local<'s, v8::Value>),
    Error(v8::Local<'s, v8::Value>),
    Complete,
}

pub(super) fn is_native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, observer, NATIVE_KIND).is_some_and(|value| value.is_int32())
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
) {
    if let Some(kind) = get_private_value(scope, observer, NATIVE_KIND)
        .filter(|value| value.is_int32())
        .and_then(|value| value.int32_value(scope))
    {
        if !callbacks::is_current(scope, observer) {
            return;
        }
        let Some(context) = observer.get_creation_context(scope) else {
            return;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let exception = {
            v8::tc_scope!(let scope, scope);
            match kind {
                FIRST => first::notify(scope, observer, notification),
                LAST | TO_ARRAY => collect::notify(scope, observer, notification, kind),
                FOR_EACH | REDUCE | SOME | EVERY | FIND => {
                    consume::notify(scope, observer, notification, kind);
                }
                MAP | FILTER | TAKE | DROP => {
                    transform::notify(scope, observer, notification, kind)
                }
                UNTIL_SOURCE | UNTIL_NOTIFIER => until::notify(scope, observer, notification, kind),
                INSPECT => inspect::notify(scope, observer, notification),
                _ => unreachable!("unknown native Observable observer"),
            }
            let exception = scope.exception();
            scope.reset();
            exception
        };
        // Internal observer steps cannot throw through Subscriber.next/error/
        // complete. In particular, first() has already resolved its Promise
        // before a throwing iterator return() is encountered during cancellation.
        if let Some(exception) = exception {
            callbacks::report(scope, observer, exception);
        }
        return;
    }
    match notification {
        Notification::Next(value) => {
            if let Some(callback) = object_slot(scope, observer, NEXT) {
                invoke_and_report(scope, callback, &[value]);
            }
        }
        Notification::Error(error) => {
            if let Some(callback) = object_slot(scope, observer, ERROR) {
                invoke_and_report(scope, callback, &[error]);
            } else {
                callbacks::report_default_error(scope, error);
            }
        }
        Notification::Complete => {
            if let Some(callback) = object_slot(scope, observer, COMPLETE) {
                invoke_and_report(scope, callback, &[]);
            }
        }
    }
}
