use super::*;
use crate::util::ecmascript_promise_resolve;

const ITERATOR: &str = "__moliReadableStreamSyncIterator";
const DONE: &str = "__moliReadableStreamSyncIteratorDone";
const CLOSE_ON_REJECTION: &str = "__moliReadableStreamSyncIteratorCloseOnRejection";

#[derive(WebApiObject)]
#[webapi(plain)]
struct SyncContinuationDeclaration<'scope> {
    #[webapi(slot = ITERATOR)]
    iterator: v8::Local<'scope, v8::Object>,
    #[webapi(slot = DONE)]
    done: bool,
    #[webapi(slot = CLOSE_ON_REJECTION)]
    close_on_rejection: bool,
}

pub(super) fn adapt_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    result: v8::Local<'s, v8::Value>,
    close_on_rejection: bool,
) -> Option<v8::Local<'s, v8::Promise>> {
    let Ok(result) = v8::Local::<v8::Object>::try_from(result) else {
        throw_type_error(scope, "Synchronous iterator result must be an object");
        return None;
    };
    // AsyncFromSyncIteratorContinuation reads both properties synchronously,
    // including the final value, and awaits the value rather than the result.
    let done = result
        .get(scope, v8str(scope, "done").into())?
        .boolean_value(scope);
    let value = result.get(scope, v8str(scope, "value").into())?;
    let resolved = {
        v8::tc_scope!(let scope, scope);
        let promise = ecmascript_promise_resolve(scope, value);
        if let Some(error) = scope.exception() {
            scope.reset();
            Err(error)
        } else {
            Ok(promise)
        }
    };
    let value_promise = match resolved {
        Ok(promise) => promise?,
        Err(error) => {
            if !done && close_on_rejection {
                close_preserving_error(scope, iterator);
            }
            scope.throw_exception(error);
            return None;
        }
    };
    let continuation = SyncContinuationDeclaration::new(iterator, done, close_on_rejection)
        .bind(scope)
        .ok()?;
    let fulfilled = v8::Function::builder(value_fulfilled)
        .data(continuation.into())
        .build(scope)?;
    let rejected = v8::Function::builder(value_rejected)
        .data(continuation.into())
        .build(scope)?;
    value_promise.then2(scope, fulfilled, rejected)
}

pub(super) fn iterator_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    done: bool,
) -> v8::Local<'s, v8::Object> {
    let result = v8::Object::new(scope);
    result.create_data_property(scope, v8str(scope, "value").into(), value);
    result.create_data_property(
        scope,
        v8str(scope, "done").into(),
        v8::Boolean::new(scope, done).into(),
    );
    result
}

fn value_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Object>::try_from(args.data()).expect("sync iterator continuation");
    let done = get_private_value(scope, data, DONE).is_some_and(|value| value.boolean_value(scope));
    rv.set(iterator_result(scope, args.get(0), done).into());
}

fn value_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Object>::try_from(args.data()).expect("sync iterator continuation");
    let done = get_private_value(scope, data, DONE).is_some_and(|value| value.boolean_value(scope));
    let close = get_private_value(scope, data, CLOSE_ON_REJECTION)
        .is_some_and(|value| value.boolean_value(scope));
    if !done && close {
        let iterator = get_private_value(scope, data, ITERATOR)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .expect("sync iterator");
        close_preserving_error(scope, iterator);
    }
    scope.throw_exception(args.get(0));
}

fn close_preserving_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
) {
    v8::tc_scope!(let scope, scope);
    if let Some(Some(method)) = sequence_return_method(scope, iterator)
        && let Some(result) = call_sequence_method(scope, method.into(), iterator.into(), &[])
        && !result.is_object()
    {
        throw_type_error(
            scope,
            "Synchronous iterator return result must be an object",
        );
    }
    // IteratorClose with a throw completion preserves the original error even
    // when getting/calling return or validating its result throws.
    if scope.has_caught() {
        scope.reset();
    }
}
