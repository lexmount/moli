//! Shared lifetime and single-settlement state for native Promise observers.

use moli_webapi_declare::WebApiObject;

use super::{callbacks, observer, state::object_slot};
use crate::{
    abort_signal_route::ResolvedAbortSignal,
    util::{get_private_value, set_private_value},
};

const RESOLVER: &str = "__moliObservablePromiseResolver";
const REJECTION_SIGNAL: &str = "__moliObservableRejectionSignal";
const REJECTION_ALGORITHM: &str = "__moliObservableRejectionAlgorithm";
const SETTLED: &str = "__moliObservablePromiseSettled";
const PROMISE_OBSERVER: &str = "__moliObservablePromiseObserver";
pub(super) const VALUE: &str = "__moliObservablePromiseValue";

#[derive(WebApiObject)]
#[webapi(plain)]
struct PromiseObserver<'scope> {
    #[webapi(slot = observer::NATIVE_KIND)]
    kind: i32,
    #[webapi(slot = RESOLVER)]
    resolver: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SETTLED)]
    settled: bool,
}

pub(super) fn new_observer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: i32,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    signal: Option<ResolvedAbortSignal<'s>>,
    caller_signal: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(signal) = signal.filter(|signal| signal.is_aborted(scope)) {
        let reason = signal.reason(scope);
        resolver.reject(scope, reason);
        return None;
    }
    let observer = PromiseObserver::new(kind, resolver.into(), false)
        .bind(scope)
        .ok()?;
    let promise = resolver.get_promise(scope);
    // A reachable pending Promise retains its observer. Without an external
    // signal or producer, this cycle remains entirely V8-traced and collectible.
    set_private_value(scope, promise.into(), PROMISE_OBSERVER, observer.into());
    if let Some(signal) = signal {
        let algorithm = v8::Function::builder(aborted)
            .data(observer.into())
            .build(scope)
            .expect("Observable Promise abort algorithm");
        set_private_value(scope, observer, REJECTION_SIGNAL, signal.value().into());
        set_private_value(scope, observer, REJECTION_ALGORITHM, algorithm.into());
        if caller_signal {
            // Like subscribe({signal}), caller-driven cancellation remains
            // an owner while this operation is pending.
            signal.register_algorithm(scope, algorithm);
        } else {
            signal.register_weak_rethrowing_algorithm(scope, algorithm);
        }
    }
    Some(observer)
}

pub(super) fn is_settled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, observer, SETTLED).is_some_and(|value| value.is_true())
}

pub(super) fn start_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    if is_settled(scope, observer) {
        return None;
    }
    // Resolve can execute a then getter that reenters the producer or aborts
    // its signal. A Promise assimilating a thenable is still Pending, so its
    // V8 state is not an already-resolved flag.
    set_private_value(
        scope,
        observer,
        SETTLED,
        v8::Boolean::new(scope, true).into(),
    );
    if let Some(algorithm) = object_slot(scope, observer, REJECTION_ALGORITHM)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let signal = object_slot(scope, observer, REJECTION_SIGNAL)
            .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))?;
        signal.unregister_algorithm(scope, algorithm);
        set_private_value(
            scope,
            observer,
            REJECTION_ALGORITHM,
            v8::undefined(scope).into(),
        );
    }
    let resolver = object_slot(scope, observer, RESOLVER).expect("Observable Promise resolver");
    // SAFETY: this private slot is populated only with PromiseResolver::new;
    // it is never read from a public property or supplied by script.
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
    let promise = resolver.get_promise(scope);
    set_private_value(
        scope,
        promise.into(),
        PROMISE_OBSERVER,
        v8::undefined(scope).into(),
    );
    set_private_value(
        scope,
        observer,
        observer::SUBSCRIBER,
        v8::undefined(scope).into(),
    );
    // Operators read their final value before settling. Release accumulated
    // values on rejection or abort too, even while the result Promise survives.
    set_private_value(scope, observer, VALUE, v8::undefined(scope).into());
    Some(resolver)
}

fn aborted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let observer =
        v8::Local::<v8::Object>::try_from(args.data()).expect("Observable Promise abort data");
    if callbacks::is_current(scope, observer)
        && let Some(resolver) = start_settlement(scope, observer)
    {
        resolver.reject(scope, args.get(0));
    }
}
