//! Native conversion producers. Inputs and iterator records live in V8-traced
//! slots, so an Observable does not add a Rust root for its input or subscriber.

mod asynchronous;

use moli_webapi_declare::WebApiObject;

use super::{
    callbacks::is_current, state::*, subscriber_complete, subscriber_error, subscriber_next,
};
use crate::{
    abort_signal_route::ResolvedAbortSignal,
    util::{get_private_value, set_private_value, v8str},
    web_api_interfaces, webidl,
};

const INPUT: &str = "__moliObservableFromInput";
const KIND: &str = "__moliObservableFromKind";
const SUBSCRIBER: &str = "__moliObservableIteratorSubscriber";
const ITERATOR: &str = "__moliObservableIterator";
const NEXT_METHOD: &str = "__moliObservableIteratorNext";
const ASYNCHRONOUS: &str = "__moliObservableIteratorAsync";
const SYNC_FALLBACK: &str = "__moliObservableIteratorSyncFallback";
const ITERATOR_ABORT: &str = "__moliObservableIteratorAbort";

const SYNC: i32 = 0;
const ASYNC: i32 = 1;
const PROMISE: i32 = 2;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.from")]
struct FromArgs<'scope> {
    #[webidl(required, converter = "raw")]
    value: v8::Local<'scope, v8::Value>,
}

pub(super) fn from<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<FromArgs<'s>>(scope, &args) else {
        return;
    };
    if let Some(observable) = convert(scope, parsed.value) {
        rv.set(observable.into());
    }
}

pub(super) fn convert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = require_object(scope, value, "Observable.from requires an Object")?;
    if web_api_interfaces::Observable::is_instance(scope, object) {
        return Some(object);
    }
    // Conversion only probes the protocols. Each fresh subscription must get
    // the method again, and async iteration takes priority over sync/Promise.
    let asynchronous = v8::Symbol::get_async_iterator(scope);
    let synchronous = v8::Symbol::get_iterator(scope);
    let kind = if get_method(scope, object, asynchronous.into())?.is_some() {
        ASYNC
    } else if get_method(scope, object, synchronous.into())?.is_some() {
        SYNC
    } else if value.is_promise() {
        PROMISE
    } else {
        webidl::throw_type_error(
            scope,
            "Observable.from requires an Observable, iterable or Promise",
        );
        return None;
    };
    let observable = new_native_observable(scope, None)?;
    set_private_value(scope, observable, INPUT, value);
    set_private_value(
        scope,
        observable,
        KIND,
        v8::Integer::new(scope, kind).into(),
    );
    Some(observable)
}

pub(super) fn subscribe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(input) = object_slot(scope, observable, INPUT) else {
        return false;
    };
    let kind = get_private_value(scope, observable, KIND)
        .and_then(|value| value.int32_value(scope))
        .expect("Observable conversion kind");
    if kind == PROMISE {
        // Even an already-aborted observer handles the source rejection. A late
        // error is reported through Subscriber.error, not left unhandled here.
        asynchronous::subscribe_promise(scope, input, subscriber);
    } else if active(scope, subscriber) {
        match try_js(scope, |scope| {
            open_iterator(scope, input, subscriber, kind == ASYNC)
        }) {
            Ok(Some(runner)) if active(scope, subscriber) && is_current(scope, subscriber) => {
                let algorithm = v8::Function::builder(abort_iterator)
                    .data(runner.into())
                    .build(scope)
                    .expect("Observable iterator abort algorithm");
                set_private_value(scope, runner, ITERATOR_ABORT, algorithm.into());
                signal(scope, subscriber).register_weak_rethrowing_algorithm(scope, algorithm);
                if kind == ASYNC {
                    asynchronous::next(scope, runner);
                } else {
                    run_sync(scope, runner);
                }
            }
            Err(error) => subscriber_error(scope, subscriber, error),
            _ => {}
        }
    }
    true
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct IteratorSubscription<'scope> {
    #[webapi(slot = SUBSCRIBER)]
    subscriber: v8::Local<'scope, v8::Object>,
    #[webapi(slot = ITERATOR)]
    iterator: v8::Local<'scope, v8::Object>,
    #[webapi(slot = NEXT_METHOD)]
    next_method: v8::Local<'scope, v8::Value>,
    #[webapi(slot = ASYNCHRONOUS)]
    asynchronous: bool,
    #[webapi(slot = SYNC_FALLBACK)]
    sync_fallback: bool,
}

fn open_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
    asynchronous: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let method = if asynchronous {
        let key = v8::Symbol::get_async_iterator(scope);
        get_method(scope, input, key.into())?
    } else {
        None
    };
    let fallback = asynchronous && method.is_none();
    let method = match method {
        Some(method) => Some(method),
        None => {
            let key = v8::Symbol::get_iterator(scope);
            get_method(scope, input, key.into())?
        }
    };
    let Some(method) = method else {
        webidl::throw_type_error(scope, "Object must be iterable");
        return None;
    };
    let iterator = method.call_as_function(scope, input.into(), &[])?;
    let iterator = require_object(scope, iterator, "Iterator method must return an Object")?;
    // GetIteratorFromMethod caches [[NextMethod]] during iterator creation.
    // In particular a next getter exception is synchronous, including async
    // iterables; only failures while calling next are promise rejections.
    let next_method = iterator.get(scope, v8str(scope, "next").into())?;
    IteratorSubscription::new(subscriber, iterator, next_method, asynchronous, fallback)
        .bind(scope)
        .ok()
}

fn signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    subscriber: v8::Local<'s, v8::Object>,
) -> ResolvedAbortSignal<'s> {
    let signal = object_slot(scope, subscriber, SIGNAL).expect("Subscriber signal");
    ResolvedAbortSignal::resolve(scope, signal).expect("Subscriber AbortSignal owner")
}

fn slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runner: v8::Local<'s, v8::Object>,
    key: &str,
) -> v8::Local<'s, v8::Object> {
    object_slot(scope, runner, key).expect("Observable iterator record")
}

fn flag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runner: v8::Local<'s, v8::Object>,
    key: &str,
) -> bool {
    get_private_value(scope, runner, key).is_some_and(|value| value.is_true())
}

fn clear_abort<'s>(scope: &mut v8::PinScope<'s, '_>, runner: v8::Local<'s, v8::Object>) {
    if let Some(algorithm) = object_slot(scope, runner, ITERATOR_ABORT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let subscriber = slot(scope, runner, SUBSCRIBER);
        signal(scope, subscriber).unregister_algorithm(scope, algorithm);
        set_private_value(scope, runner, ITERATOR_ABORT, v8::undefined(scope).into());
    }
}

fn fail<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runner: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    // Iteration failure/exhaustion must not close the iterator again when
    // Subscriber.error/complete aborts its internal signal.
    clear_abort(scope, runner);
    let subscriber = slot(scope, runner, SUBSCRIBER);
    subscriber_error(scope, subscriber, error);
}

fn finish<'s>(scope: &mut v8::PinScope<'s, '_>, runner: v8::Local<'s, v8::Object>) {
    clear_abort(scope, runner);
    let subscriber = slot(scope, runner, SUBSCRIBER);
    subscriber_complete(scope, subscriber);
}

fn iterator_next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runner: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let iterator = slot(scope, runner, ITERATOR);
    let method = get_private_value(scope, runner, NEXT_METHOD).expect("Iterator next method");
    let method = callable(scope, method)?;
    let result = method.call_as_function(scope, iterator.into(), &[])?;
    require_object(scope, result, "Iterator next() must return an Object")
}

fn read_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: v8::Local<'s, v8::Value>,
) -> Option<Option<v8::Local<'s, v8::Value>>> {
    let result = require_object(scope, result, "Iterator next() must resolve to an Object")?;
    let done = result.get(scope, v8str(scope, "done").into())?;
    if done.boolean_value(scope) {
        Some(None)
    } else {
        result.get(scope, v8str(scope, "value").into()).map(Some)
    }
}

fn run_sync<'s>(scope: &mut v8::PinScope<'s, '_>, runner: v8::Local<'s, v8::Object>) {
    let subscriber = slot(scope, runner, SUBSCRIBER);
    while active(scope, subscriber) && is_current(scope, subscriber) {
        let result = try_js(scope, |scope| {
            let result = iterator_next(scope, runner)?;
            read_result(scope, result.into())
        });
        match result {
            Ok(Some(Some(value))) => subscriber_next(scope, subscriber, value),
            Ok(Some(None)) => {
                finish(scope, runner);
                return;
            }
            Err(error) => {
                fail(scope, runner, error);
                return;
            }
            Ok(None) => return,
        }
    }
}

fn abort_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let runner = v8::Local::<v8::Object>::try_from(args.data()).expect("Iterator abort data");
    let subscriber = slot(scope, runner, SUBSCRIBER);
    clear_abort(scope, runner);
    if !is_current(scope, subscriber) {
        return;
    }
    if flag(scope, runner, ASYNCHRONOUS) {
        asynchronous::close(scope, runner, args.get(0));
    } else {
        close_sync(scope, runner);
    }
}

fn close_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runner: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let iterator = slot(scope, runner, ITERATOR);
    if let Some(method) = get_method(scope, iterator, v8str(scope, "return").into())? {
        let result = method.call_as_function(scope, iterator.into(), &[])?;
        require_object(scope, result, "Iterator return() must return an Object")?;
    }
    Some(())
}

fn require_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    message: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    match v8::Local::<v8::Object>::try_from(value) {
        Ok(object) => Some(object),
        Err(_) => {
            webidl::throw_type_error(scope, message);
            None
        }
    }
}

fn callable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && object.is_callable()
    {
        Some(object)
    } else {
        webidl::throw_type_error(scope, "Iterator method must be callable");
        None
    }
}

fn get_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Value>,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    let value = object.get(scope, key)?;
    if value.is_null_or_undefined() {
        Some(None)
    } else {
        callable(scope, value).map(Some)
    }
}

fn try_js<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: impl FnOnce(&mut v8::PinScope<'s, '_>) -> Option<T>,
) -> Result<Option<T>, v8::Local<'s, v8::Value>> {
    v8::tc_scope!(let scope, scope);
    let result = operation(scope);
    if let Some(error) = scope.exception() {
        scope.reset();
        Err(error)
    } else {
        Ok(result)
    }
}
