//! Promise operators that retain values until the source completes.

use super::{
    observer::{self, Notification},
    promise, signal_arg,
    state::object_slot,
    subscribe_internal,
};
use crate::{
    abort_signal_route::ResolvedAbortSignal,
    util::{get_private_value, set_private_value, v8str},
    webidl,
};

const HAS_VALUE: &str = "__moliObservableHasLastValue";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable")]
struct CollectArgs<'scope> {
    #[webidl(with = signal_arg)]
    signal: Option<ResolvedAbortSignal<'scope>>,
}

pub(super) fn last<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    collect(scope, args, rv, observer::LAST);
}

pub(super) fn to_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    collect(scope, args, rv, observer::TO_ARRAY);
}

fn collect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: i32,
) {
    let Some(parsed) = webidl::parse_args::<CollectArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    let Some(observer) = promise::new_observer(scope, kind, resolver, parsed.signal, true) else {
        return;
    };
    if kind == observer::TO_ARRAY {
        let values = v8::Array::new(scope, 0);
        set_private_value(scope, observer, promise::VALUE, values.into());
    }
    // These operators do not cancel on next(), and subscribe directly with the
    // caller's signal. Adding a dependent signal would change abort ordering.
    subscribe_internal(scope, args.this(), observer, parsed.signal);
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
    kind: i32,
) {
    if promise::is_settled(scope, observer) {
        return;
    }
    match notification {
        Notification::Next(value) if kind == observer::TO_ARRAY => {
            let values = object_slot(scope, observer, promise::VALUE)
                .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
                .expect("Observable values");
            let Some(key) = v8::String::new(scope, &values.length().to_string()) else {
                return;
            };
            // This private, dense array represents an Infra list until
            // completion. Append without invoking inherited index setters.
            values.create_data_property(scope, key.into(), value);
        }
        Notification::Next(value) => {
            set_private_value(scope, observer, promise::VALUE, value);
            set_private_value(
                scope,
                observer,
                HAS_VALUE,
                v8::Boolean::new(scope, true).into(),
            );
        }
        Notification::Error(error) => {
            if let Some(resolver) = promise::start_settlement(scope, observer) {
                resolver.reject(scope, error);
            }
        }
        Notification::Complete => {
            let has_value = kind == observer::TO_ARRAY
                || get_private_value(scope, observer, HAS_VALUE)
                    .is_some_and(|value| value.is_true());
            let value = get_private_value(scope, observer, promise::VALUE)
                .unwrap_or_else(|| v8::undefined(scope).into());
            if let Some(resolver) = promise::start_settlement(scope, observer) {
                if has_value {
                    resolver.resolve(scope, value);
                } else {
                    let error =
                        v8::Exception::range_error(scope, v8str(scope, "No values in Observable"));
                    resolver.reject(scope, error);
                }
            }
        }
    }
}
