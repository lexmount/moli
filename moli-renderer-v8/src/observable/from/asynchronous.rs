use super::*;

const ADAPTER_RUNNER: &str = "__moliObservableAsyncAdapterRunner";
const ADAPTER_DONE: &str = "__moliObservableAsyncAdapterDone";
const CLOSE_ON_REJECTION: &str = "__moliObservableAsyncAdapterCloseOnRejection";

pub(super) fn subscribe_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
) {
    let promise = v8::Local::<v8::Promise>::try_from(input).expect("Observable Promise input");
    let fulfilled = v8::Function::builder(promise_fulfilled)
        .data(subscriber.into())
        .build(scope);
    let rejected = v8::Function::builder(promise_rejected)
        .data(subscriber.into())
        .build(scope);
    if let (Some(fulfilled), Some(rejected)) = (fulfilled, rejected) {
        promise.then2(scope, fulfilled, rejected);
    }
}

fn promise_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let subscriber = v8::Local::<v8::Object>::try_from(args.data()).expect("Promise subscriber");
    subscriber_next(scope, subscriber, args.get(0));
    subscriber_complete(scope, subscriber);
}

fn promise_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let subscriber = v8::Local::<v8::Object>::try_from(args.data()).expect("Promise subscriber");
    subscriber_error(scope, subscriber, args.get(0));
}

pub(super) fn next<'s>(scope: &mut v8::PinScope<'s, '_>, runner: v8::Local<'s, v8::Object>) {
    let subscriber = slot(scope, runner, SUBSCRIBER);
    if !active(scope, subscriber) || !is_current(scope, subscriber) {
        return;
    }
    let result = try_js(scope, |scope| {
        let result = iterator_next(scope, runner)?;
        if flag(scope, runner, SYNC_FALLBACK) {
            adapt_sync_result(scope, runner, result, true)
        } else {
            resolved_with(scope, result.into())
        }
    });
    let promise = match result {
        Ok(promise) => promise,
        Err(error) => rejected_with(scope, error),
    };
    if let Some(promise) = promise {
        let fulfilled = v8::Function::builder(next_fulfilled)
            .data(runner.into())
            .build(scope);
        let rejected = v8::Function::builder(next_rejected)
            .data(runner.into())
            .build(scope);
        if let (Some(fulfilled), Some(rejected)) = (fulfilled, rejected) {
            promise.then2(scope, fulfilled, rejected);
        }
    }
}

fn next_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let runner = v8::Local::<v8::Object>::try_from(args.data()).expect("Async iterator reaction");
    let subscriber = slot(scope, runner, SUBSCRIBER);
    if !is_current(scope, subscriber) {
        return;
    }
    // An already queued result still has its done/value getters evaluated after
    // cancellation. Subscriber.next suppresses delivery; next() suppresses the
    // subsequent pull. Do not put an active check before read_result.
    match try_js(scope, |scope| read_result(scope, args.get(0))) {
        Ok(Some(Some(value))) => {
            subscriber_next(scope, subscriber, value);
            next(scope, runner);
        }
        Ok(Some(None)) => finish(scope, runner),
        Err(error) => fail(scope, runner, error),
        Ok(None) => {}
    }
}

fn next_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let runner = v8::Local::<v8::Object>::try_from(args.data()).expect("Async iterator reaction");
    fail(scope, runner, args.get(0));
}

pub(super) fn close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runner: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let result = try_js(scope, |scope| {
        let iterator = slot(scope, runner, ITERATOR);
        let Some(method) = get_method(scope, iterator, v8str(scope, "return").into())? else {
            return Some(None);
        };
        // Observable's shipped async-iterator contract forwards the abort
        // reason (also covered by observable-from.any.js). Sync close has no
        // argument. Neither path calls an author-replaced Subscriber method.
        let result = method.call_as_function(scope, iterator.into(), &[reason])?;
        if flag(scope, runner, SYNC_FALLBACK) {
            let result = require_object(scope, result, "Iterator return() must return an Object")?;
            adapt_sync_result(scope, runner, result, false).map(Some)
        } else {
            resolved_with(scope, result).map(Some)
        }
    });
    match result {
        Ok(Some(Some(promise))) => {
            if let Some(fulfilled) = v8::Function::new(scope, close_fulfilled) {
                // Leave rejection unhandled: it belongs to async iterator close,
                // after the subscriber has already been cancelled.
                promise.then(scope, fulfilled);
            }
        }
        Err(error) => {
            rejected_with(scope, error);
        }
        _ => {}
    }
}

fn close_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    require_object(
        scope,
        args.get(0),
        "Iterator return() must resolve to an Object",
    );
}

fn resolved_with<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    // Web IDL's "a promise resolved with" preserves a native Promise and does
    // not consult its public then or constructor properties.
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        return Some(promise);
    }
    let resolver = v8::PromiseResolver::new(scope)?;
    resolver.resolve(scope, value)?;
    Some(resolver.get_promise(scope))
}

fn rejected_with<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    resolver.reject(scope, error)?;
    Some(resolver.get_promise(scope))
}

// GetIterator(async) falls back to CreateAsyncFromSyncIterator. That adapter
// awaits each value, including the final value, before exposing an iterator
// result. It also closes the sync iterator if awaiting a non-final value fails.
#[derive(WebApiObject)]
#[webapi(plain)]
struct SyncContinuation<'scope> {
    #[webapi(slot = ADAPTER_RUNNER)]
    runner: v8::Local<'scope, v8::Object>,
    #[webapi(slot = ADAPTER_DONE)]
    done: bool,
    #[webapi(slot = CLOSE_ON_REJECTION)]
    close_on_rejection: bool,
}

fn adapt_sync_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runner: v8::Local<'s, v8::Object>,
    result: v8::Local<'s, v8::Object>,
    close_on_rejection: bool,
) -> Option<v8::Local<'s, v8::Promise>> {
    let done = result
        .get(scope, v8str(scope, "done").into())?
        .boolean_value(scope);
    let value = result.get(scope, v8str(scope, "value").into())?;
    let value_promise = match try_js(scope, |scope| promise_resolve(scope, value)) {
        Ok(promise) => promise?,
        Err(error) => {
            if !done && close_on_rejection {
                // IteratorClose with a throw completion preserves that error,
                // even if return itself throws or returns a primitive.
                let _ = try_js(scope, |scope| close_sync(scope, runner));
            }
            scope.throw_exception(error);
            return None;
        }
    };
    let data = SyncContinuation::new(runner, done, close_on_rejection)
        .bind(scope)
        .ok()?;
    let fulfilled = v8::Function::builder(sync_fulfilled)
        .data(data.into())
        .build(scope)?;
    let rejected = v8::Function::builder(sync_rejected)
        .data(data.into())
        .build(scope)?;
    value_promise.then2(scope, fulfilled, rejected)
}

fn promise_resolve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    // Unlike Web IDL's promise wrapping, the ECMAScript async-from-sync adapter
    // uses PromiseResolve(%Promise%, value), including Get(value, constructor).
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        let constructor = promise.get(scope, v8str(scope, "constructor").into())?;
        let global = scope.get_current_context().global(scope);
        let intrinsic = crate::util::registered_intrinsic_constructor(scope, global, "Promise")?;
        if constructor.strict_equals(intrinsic.into()) {
            return Some(promise);
        }
    }
    let resolver = v8::PromiseResolver::new(scope)?;
    resolver.resolve(scope, value)?;
    Some(resolver.get_promise(scope))
}

fn sync_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data =
        v8::Local::<v8::Object>::try_from(args.data()).expect("Async-from-sync continuation");
    let done = flag(scope, data, ADAPTER_DONE);
    let result = v8::Object::new(scope);
    result.create_data_property(scope, v8str(scope, "value").into(), args.get(0));
    result.create_data_property(
        scope,
        v8str(scope, "done").into(),
        v8::Boolean::new(scope, done).into(),
    );
    rv.set(result.into());
}

fn sync_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data =
        v8::Local::<v8::Object>::try_from(args.data()).expect("Async-from-sync continuation");
    let runner = slot(scope, data, ADAPTER_RUNNER);
    let subscriber = slot(scope, runner, SUBSCRIBER);
    if !is_current(scope, subscriber) {
        return;
    }
    if !flag(scope, data, ADAPTER_DONE) && flag(scope, data, CLOSE_ON_REJECTION) {
        let _ = try_js(scope, |scope| close_sync(scope, runner));
    }
    scope.throw_exception(args.get(0));
}
