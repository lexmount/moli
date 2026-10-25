//! Callback-driven Promise consumers use traced Web IDL callbacks and cancel
//! only their own observer when a visitor or reducer throws.

use super::{
    callbacks,
    observer::{self, Notification},
    promise, signal_arg,
    state::{object_slot, set_callback},
    subscribe_internal,
};
use crate::{
    abort_signal_route::ResolvedAbortSignal,
    util::{get_private_value, set_private_value, v8str},
    webidl,
};

const CALLBACK: &str = "__moliObservableConsumerCallback";
const INDEX: &str = "__moliObservableConsumerIndex";
const HAS_ACCUMULATOR: &str = "__moliObservableHasAccumulator";
// Unlike terminal collection state, a reducer's accumulator and callback remain
// observable through a next() snapshot already being dispatched at cancellation.
// The observer retains them until that snapshot is released, without Rust roots.
const ACCUMULATOR: &str = "__moliObservableAccumulator";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.forEach")]
struct ForEachArgs<'scope> {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
    #[webidl(with = signal_arg)]
    signal: Option<ResolvedAbortSignal<'scope>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.reduce")]
struct ReduceArgs<'scope> {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
    #[webidl(converter = "raw")]
    initial: Option<v8::Local<'scope, v8::Value>>,
    #[webidl(with = signal_arg)]
    signal: Option<ResolvedAbortSignal<'scope>>,
}

pub(super) fn for_each<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ForEachArgs<'s>>(scope, &args) else {
        return;
    };
    if let Some(promise) = consume(
        scope,
        args.this(),
        parsed.callback,
        None,
        parsed.signal,
        observer::FOR_EACH,
    ) {
        rv.set(promise.into());
    }
}

pub(super) fn reduce<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ReduceArgs<'s>>(scope, &args) else {
        return;
    };
    // Web IDL treats undefined for an optional argument without a default as
    // missing. Option<raw> preserves null and other actual seed values.
    if let Some(promise) = consume(
        scope,
        args.this(),
        parsed.callback,
        parsed.initial,
        parsed.signal,
        observer::REDUCE,
    ) {
        rv.set(promise.into());
    }
}

fn consume<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    callback: webidl::WebIdlCallbackFunction,
    initial: Option<v8::Local<'s, v8::Value>>,
    signal: Option<ResolvedAbortSignal<'s>>,
    kind: i32,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let Some((observer, signal)) = promise::new_controlled_observer(scope, kind, resolver, signal)
    else {
        return Some(promise);
    };
    set_callback(scope, observer, CALLBACK, callback);
    if let Some(initial) = initial {
        set_accumulator(scope, observer, initial);
    }
    subscribe_internal(scope, source, observer, Some(signal));
    Some(promise)
}

fn index<'s>(scope: &mut v8::PinScope<'s, '_>, observer: v8::Local<'s, v8::Object>) -> u64 {
    get_private_value(scope, observer, INDEX)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map_or(0, |value| value.u64_value().0)
}

fn increment_index<'s>(scope: &mut v8::PinScope<'s, '_>, observer: v8::Local<'s, v8::Object>) {
    let next = index(scope, observer).wrapping_add(1);
    set_private_value(
        scope,
        observer,
        INDEX,
        v8::BigInt::new_from_u64(scope, next).into(),
    );
}

fn set_accumulator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, observer, ACCUMULATOR, value);
    set_private_value(
        scope,
        observer,
        HAS_ACCUMULATOR,
        v8::Boolean::new(scope, true).into(),
    );
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
    kind: i32,
) {
    let has_accumulator =
        get_private_value(scope, observer, HAS_ACCUMULATOR).is_some_and(|value| value.is_true());
    match notification {
        Notification::Next(value) => {
            // Do not skip a snapshot notification after settlement: a prior
            // observer can cancel/complete the source during this same next().
            if kind == observer::REDUCE && !has_accumulator {
                set_accumulator(scope, observer, value);
                increment_index(scope, observer);
                return;
            }
            let callback =
                object_slot(scope, observer, CALLBACK).expect("Observable consumer callback");
            let accumulator = get_private_value(scope, observer, ACCUMULATOR)
                .unwrap_or_else(|| v8::undefined(scope).into());
            let idx = index(scope, observer);
            let arguments = [
                accumulator,
                value,
                v8::Number::new(scope, idx as f64).into(),
            ];
            let arguments = if kind == observer::REDUCE {
                &arguments[..]
            } else {
                &arguments[1..]
            };
            let result = callbacks::invoke_value(scope, callback, arguments);
            if let Err(error) = result {
                if let Some(resolver) = promise::start_settlement(scope, observer) {
                    resolver.reject(scope, error);
                }
                promise::abort_controller(scope, observer, error);
            }
            // The draft increments after invocation; a reentrant next() sees
            // the current index. Read it again to preserve nested increments.
            increment_index(scope, observer);
            if kind == observer::REDUCE
                && let Ok(value) = result
            {
                set_accumulator(scope, observer, value);
            }
        }
        Notification::Error(error) => {
            if let Some(resolver) = promise::start_settlement(scope, observer) {
                resolver.reject(scope, error);
            }
        }
        Notification::Complete => {
            let value = if kind == observer::REDUCE {
                get_private_value(scope, observer, ACCUMULATOR)
                    .unwrap_or_else(|| v8::undefined(scope).into())
            } else {
                v8::undefined(scope).into()
            };
            if let Some(resolver) = promise::start_settlement(scope, observer) {
                if kind == observer::REDUCE && !has_accumulator {
                    let error = v8::Exception::type_error(
                        scope,
                        v8str(scope, "Reduce of empty Observable with no initial value"),
                    );
                    resolver.reject(scope, error);
                } else {
                    resolver.resolve(scope, value);
                }
            }
        }
    }
}
