use moli_webapi_declare::WebApiObject;

use super::{
    callbacks,
    observer::{self, NATIVE_KIND, Notification},
    signal_arg,
    state::object_slot,
    subscribe_internal,
};
use crate::{
    abort_signal_route::ResolvedAbortSignal,
    util::{get_private_value, set_private_value, v8str},
    webidl,
};

const RESOLVER: &str = "__moliObservableFirstResolver";
const CONTROLLER_SIGNAL: &str = "__moliObservableFirstControllerSignal";
const REJECTION_SIGNAL: &str = "__moliObservableFirstRejectionSignal";
const REJECTION_ALGORITHM: &str = "__moliObservableFirstRejectionAlgorithm";
const SETTLED: &str = "__moliObservableFirstSettled";
const PROMISE_OBSERVER: &str = "__moliObservablePromiseObserver";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.first")]
struct FirstArgs<'scope> {
    #[webidl(with = signal_arg)]
    signal: Option<ResolvedAbortSignal<'scope>>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct FirstObserver<'scope> {
    #[webapi(slot = NATIVE_KIND)]
    kind: i32,
    #[webapi(slot = RESOLVER)]
    resolver: v8::Local<'scope, v8::Object>,
    #[webapi(slot = CONTROLLER_SIGNAL)]
    controller_signal: v8::Local<'scope, v8::Object>,
    #[webapi(slot = REJECTION_SIGNAL)]
    rejection_signal: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SETTLED)]
    settled: bool,
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
    let Some(controller) = ResolvedAbortSignal::new(scope) else {
        return;
    };
    let mut sources = vec![controller];
    sources.extend(parsed.signal);
    let Some(signal) = ResolvedAbortSignal::dependent(scope, &sources) else {
        return;
    };
    if signal.is_aborted(scope) {
        let reason = signal.reason(scope);
        resolver.reject(scope, reason);
        return;
    }
    let observer = FirstObserver::new(
        observer::FIRST,
        resolver.into(),
        controller.value(),
        signal.value(),
        false,
    )
    .bind(scope)
    .expect("Observable.first observer");
    // A reachable pending Promise retains its observer. Without an external
    // signal or producer, this cycle remains entirely V8-traced and collectible.
    set_private_value(scope, promise.into(), PROMISE_OBSERVER, observer.into());
    let algorithm = v8::Function::builder(aborted)
        .data(observer.into())
        .build(scope)
        .expect("Observable.first abort algorithm");
    set_private_value(scope, observer, REJECTION_ALGORITHM, algorithm.into());
    if parsed.signal.is_some() {
        // As with an ordinary subscribe({signal}), caller-driven cancellation
        // remains an owner while its observer is pending.
        signal.register_algorithm(scope, algorithm);
    } else {
        signal.register_weak_rethrowing_algorithm(scope, algorithm);
    }
    subscribe_internal(scope, args.this(), observer, Some(signal));
}

fn start_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    if get_private_value(scope, observer, SETTLED).is_some_and(|value| value.is_true()) {
        return None;
    }
    // Lock the result before Resolve can run a then getter and reenter the
    // producer or abort its signal. Promise::state can still be Pending while
    // assimilating the first value, so it is not an already-resolved flag.
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
    let resolver = object_slot(scope, observer, RESOLVER).expect("Observable.first resolver");
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
    Some(resolver)
}

pub(super) fn notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    notification: Notification<'s>,
) {
    match notification {
        Notification::Next(value) => {
            if let Some(resolver) = start_settlement(scope, observer) {
                resolver.resolve(scope, value);
            }
            // Abort after resolving, including when a then getter reenters
            // next(). This removes just this observer from a shared producer.
            if let Some(signal) = object_slot(scope, observer, CONTROLLER_SIGNAL)
                .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
            {
                let reason = crate::native_bridge::abort::abort_error_value(scope);
                signal.abort(scope, reason);
            }
        }
        Notification::Error(error) => {
            if let Some(resolver) = start_settlement(scope, observer) {
                resolver.reject(scope, error);
            }
        }
        Notification::Complete => {
            if let Some(resolver) = start_settlement(scope, observer) {
                let error =
                    v8::Exception::range_error(scope, v8str(scope, "No values in Observable"));
                resolver.reject(scope, error);
            }
        }
    }
}

fn aborted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let observer =
        v8::Local::<v8::Object>::try_from(args.data()).expect("Observable.first abort data");
    if callbacks::is_current(scope, observer)
        && let Some(resolver) = start_settlement(scope, observer)
    {
        resolver.reject(scope, args.get(0));
    }
}
