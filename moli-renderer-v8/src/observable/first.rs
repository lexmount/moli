use super::{
    observer::{self, Notification},
    promise, signal_arg, subscribe_internal,
};
use crate::{abort_signal_route::ResolvedAbortSignal, util::v8str, webidl};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.first")]
struct FirstArgs<'scope> {
    #[webidl(with = signal_arg)]
    signal: Option<ResolvedAbortSignal<'scope>>,
}

pub(super) fn first<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FirstArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let Some((observer, signal)) =
        promise::new_controlled_observer(scope, observer::FIRST, resolver, parsed.signal)
    else {
        return;
    };
    subscribe_internal(scope, args.this(), observer, Some(signal));
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
) {
    match notification {
        Notification::Next(value) => {
            if let Some(resolver) = promise::start_settlement(scope, observer) {
                resolver.resolve(scope, value);
            }
            // Abort after resolving, including when a then getter reenters
            // next(). This removes just this observer from a shared producer.
            promise::abort_controller(scope, observer, v8::undefined(scope).into());
        }
        Notification::Error(error) => {
            if let Some(resolver) = promise::start_settlement(scope, observer) {
                resolver.reject(scope, error);
            }
        }
        Notification::Complete => {
            if let Some(resolver) = promise::start_settlement(scope, observer) {
                let error =
                    v8::Exception::range_error(scope, v8str(scope, "No values in Observable"));
                resolver.reject(scope, error);
            }
        }
    }
}
