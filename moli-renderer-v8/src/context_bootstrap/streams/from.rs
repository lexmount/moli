use super::*;
use crate::util::{get_private_value, throw_type_error, v8str};
use moli_webapi_declare::WebApiObject;

const READABLE_STREAM_FROM_ITERATOR_SLOT: &str = "__moliReadableStreamFromIterator";
const READABLE_STREAM_FROM_NEXT_METHOD_SLOT: &str = "__moliReadableStreamFromNextMethod";
const READABLE_STREAM_FROM_SYNC_ITERATOR_SLOT: &str = "__moliReadableStreamFromSyncIterator";
const READABLE_STREAM_FROM_REACTION_STREAM_SLOT: &str = "__moliReadableStreamFromReactionStream";
const READABLE_STREAM_FROM_REACTION_SYNC_SLOT: &str = "__moliReadableStreamFromReactionSync";

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ReadableStreamFromSourceDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_FROM_ITERATOR_SLOT)]
    iterator: v8::Local<'scope, v8::Object>,
    #[webapi(slot = READABLE_STREAM_FROM_NEXT_METHOD_SLOT)]
    next_method: v8::Local<'scope, v8::Function>,
    #[webapi(slot = READABLE_STREAM_FROM_SYNC_ITERATOR_SLOT)]
    sync_iterator: bool,
    #[webapi(method, callback = readable_stream_from_pull_callback)]
    pull: (),
    #[webapi(method, callback = readable_stream_from_cancel_callback)]
    cancel: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ReadableStreamFromReactionDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_FROM_REACTION_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = READABLE_STREAM_FROM_REACTION_SYNC_SLOT)]
    sync_iterator: bool,
}

struct OpenedAsyncSequence<'s> {
    iterator: v8::Local<'s, v8::Object>,
    next_method: v8::Local<'s, v8::Function>,
    sync_iterator: bool,
}

pub(super) fn readable_stream_from_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() < 1 {
        throw_type_error(
            scope,
            "Failed to execute 'from' on 'ReadableStream': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(sequence) = open_async_sequence(scope, args.get(0)) else {
        return;
    };
    let source = ReadableStreamFromSourceDeclaration::new(
        sequence.iterator,
        sequence.next_method,
        sequence.sync_iterator,
    )
    .bind(scope)
    .expect("ReadableStream.from source declaration should bind");
    let stream = new_readable_stream_object(scope, Some(source), 0.0, None);
    rv.set(stream.into());
}

fn open_async_sequence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<OpenedAsyncSequence<'s>> {
    if value.is_null_or_undefined() {
        throw_type_error(scope, "ReadableStream.from requires an iterable");
        return None;
    }
    let object = value.to_object(scope)?;
    let async_iterator_symbol = v8::Symbol::get_async_iterator(scope);
    let async_iterator_method = object.get(scope, async_iterator_symbol.into())?;
    let (iterator_method, sync_iterator) = if async_iterator_method.is_null_or_undefined() {
        let iterator_symbol = v8::Symbol::get_iterator(scope);
        let iterator_method = object.get(scope, iterator_symbol.into())?;
        if iterator_method.is_null_or_undefined() {
            throw_type_error(scope, "ReadableStream.from requires an iterable");
            return None;
        }
        (iterator_method, true)
    } else {
        (async_iterator_method, false)
    };
    let Ok(iterator_method) = v8::Local::<v8::Function>::try_from(iterator_method) else {
        throw_type_error(
            scope,
            "ReadableStream.from iterator method must be callable",
        );
        return None;
    };
    let iterator = call_sequence_method(scope, iterator_method, value, &[])?;
    let Ok(iterator) = v8::Local::<v8::Object>::try_from(iterator) else {
        throw_type_error(
            scope,
            "ReadableStream.from iterator method must return an object",
        );
        return None;
    };
    let next_method = iterator.get(scope, v8str(scope, "next").into())?;
    let Ok(next_method) = v8::Local::<v8::Function>::try_from(next_method) else {
        throw_type_error(
            scope,
            "ReadableStream.from iterator next method must be callable",
        );
        return None;
    };
    Some(OpenedAsyncSequence {
        iterator,
        next_method,
        sync_iterator,
    })
}

fn readable_stream_from_pull_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.this();
    let Some(iterator) = get_private_value(scope, source, READABLE_STREAM_FROM_ITERATOR_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        throw_type_error(scope, "ReadableStream.from source has no iterator");
        return;
    };
    let Some(next_method) = get_private_value(scope, source, READABLE_STREAM_FROM_NEXT_METHOD_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        throw_type_error(scope, "ReadableStream.from source has no next method");
        return;
    };
    let Ok(controller) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(scope, "ReadableStream.from pull has no controller");
        return;
    };
    let Some(stream) = stream_slot_object(scope, controller, STREAM_CONTROLLER_STREAM_SLOT) else {
        throw_type_error(scope, "ReadableStream.from pull has an invalid controller");
        return;
    };
    let Some(next_result) = call_sequence_method(scope, next_method, iterator.into(), &[]) else {
        return;
    };
    let Some(next_promise) = promise_resolved_with(scope, next_result) else {
        return;
    };
    let sync_iterator = get_private_value(scope, source, READABLE_STREAM_FROM_SYNC_ITERATOR_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    let reaction = ReadableStreamFromReactionDeclaration::new(stream, sync_iterator)
        .bind(scope)
        .expect("ReadableStream.from reaction declaration should bind");
    let Some(on_fulfilled) = v8::Function::builder(readable_stream_from_next_fulfilled_callback)
        .data(reaction.into())
        .build(scope)
    else {
        return;
    };
    if let Some(promise) = next_promise.then(scope, on_fulfilled) {
        rv.set(promise.into());
    }
}

fn readable_stream_from_next_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((stream, sync_iterator)) = readable_stream_from_reaction(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let Ok(result) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(
            scope,
            "ReadableStream.from iterator next result must be an object",
        );
        return;
    };
    let Some(done) = result.get(scope, v8str(scope, "done").into()) else {
        return;
    };
    if done.boolean_value(scope) {
        let _ = close_stream(scope, stream);
        rv.set_undefined();
        return;
    }
    let Some(value) = result.get(scope, v8str(scope, "value").into()) else {
        return;
    };
    if !sync_iterator {
        enqueue_readable_stream_from_value(scope, stream, value);
        rv.set_undefined();
        return;
    }
    let Some(value_promise) = promise_resolved_with(scope, value) else {
        return;
    };
    let Some(on_fulfilled) = v8::Function::builder(readable_stream_from_value_fulfilled_callback)
        .data(stream.into())
        .build(scope)
    else {
        return;
    };
    if let Some(promise) = value_promise.then(scope, on_fulfilled) {
        rv.set(promise.into());
    }
}

fn readable_stream_from_value_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(stream) = v8::Local::<v8::Object>::try_from(args.data()) {
        enqueue_readable_stream_from_value(scope, stream, args.get(0));
    }
    rv.set_undefined();
}

fn enqueue_readable_stream_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    match enqueue_chunk(scope, stream, value) {
        Ok(()) | Err(EnqueueChunkError::ClosedOrErrored) => {}
        Err(EnqueueChunkError::Strategy(error)) => {
            scope.throw_exception(error);
        }
    }
}

fn readable_stream_from_cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.this();
    let Some(iterator) = get_private_value(scope, source, READABLE_STREAM_FROM_ITERATOR_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let Some(return_method) = iterator.get(scope, v8str(scope, "return").into()) else {
        return;
    };
    if return_method.is_null_or_undefined() {
        rv.set_undefined();
        return;
    }
    let Ok(return_method) = v8::Local::<v8::Function>::try_from(return_method) else {
        throw_type_error(
            scope,
            "ReadableStream.from iterator return method must be callable",
        );
        return;
    };
    let reason = if args.length() > 0 {
        args.get(0)
    } else {
        v8::undefined(scope).into()
    };
    let Some(return_result) =
        call_sequence_method(scope, return_method, iterator.into(), &[reason])
    else {
        return;
    };
    let Some(return_promise) = promise_resolved_with(scope, return_result) else {
        return;
    };
    let Some(on_fulfilled) =
        v8::Function::builder(readable_stream_from_return_fulfilled_callback).build(scope)
    else {
        return;
    };
    if let Some(promise) = return_promise.then(scope, on_fulfilled) {
        rv.set(promise.into());
    }
}

fn readable_stream_from_return_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let result = args.get(0);
    if !result.is_object() {
        throw_type_error(
            scope,
            "ReadableStream.from iterator return result must be an object",
        );
        return;
    }
    rv.set_undefined();
}

fn readable_stream_from_reaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, bool)> {
    let reaction = v8::Local::<v8::Object>::try_from(value).ok()?;
    let stream = get_private_value(scope, reaction, READABLE_STREAM_FROM_REACTION_STREAM_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let sync_iterator = get_private_value(scope, reaction, READABLE_STREAM_FROM_REACTION_SYNC_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    Some((stream, sync_iterator))
}

fn promise_resolved_with<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        return Some(promise);
    }
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    (resolver.resolve(scope, value) == Some(true)).then_some(promise)
}

fn call_sequence_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    method: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    match call_function_result(scope, method, receiver, arguments) {
        Ok(result) => result,
        Err(error) => {
            scope.throw_exception(error);
            None
        }
    }
}
