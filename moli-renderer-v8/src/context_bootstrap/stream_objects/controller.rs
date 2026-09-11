use super::*;
use crate::context_bootstrap::stream_adapter::{
    EnqueueChunkError, enqueue_byte_chunk, maybe_pull_stream, prepare_readable_byte_stream_close,
    readable_byte_stream_byob_request, readable_stream_is_byte_stream,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ReadableStreamDefaultController)]
struct ReadableStreamControllerObjectDeclaration<'scope> {
    #[webapi(slot = STREAM_CONTROLLER_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = STREAM_CONTROLLER_ALGORITHMS_SLOT)]
    algorithms: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ReadableByteStreamController)]
struct ReadableByteStreamControllerObjectDeclaration<'scope> {
    #[webapi(slot = STREAM_CONTROLLER_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = STREAM_CONTROLLER_ALGORITHMS_SLOT)]
    algorithms: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TransformStreamDefaultController)]
struct TransformStreamControllerObjectDeclaration<'scope> {
    #[webapi(slot = STREAM_CONTROLLER_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = STREAM_CONTROLLER_WRITABLE_STREAM_SLOT)]
    writable: v8::Local<'scope, v8::Object>,
    /// The single promise shared by the transform's flush/cancel terminal
    /// algorithms. Once one side starts finishing, the other side observes
    /// this exact promise instead of invoking another terminal callback.
    #[webapi(
        slot,
        name = TRANSFORM_STREAM_CONTROLLER_FINISH_PROMISE_SLOT,
        init = "undefined"
    )]
    finish_promise: (),
    /// Owner-local resolver residence for `finish_promise`. This is separate
    /// from the promise value so only the terminal algorithm that claimed the
    /// residence can settle it.
    #[webapi(
        slot,
        name = TRANSFORM_STREAM_CONTROLLER_FINISH_RESIDENCE_SLOT,
        init = "null"
    )]
    finish_residence: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WritableStreamDefaultController)]
struct WritableStreamControllerObjectDeclaration<'scope> {
    #[webapi(slot = STREAM_CONTROLLER_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = STREAM_CONTROLLER_SIGNAL_SLOT)]
    stored_signal: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ReadableStreamDefaultController, enumerable)]
struct ReadableStreamDefaultControllerPrototypeDeclaration {
    #[webapi(method, length = 0, callback = readable_stream_controller_enqueue_callback)]
    enqueue: (),
    #[webapi(method, length = 0, callback = readable_stream_controller_close_callback)]
    close: (),
    #[webapi(method, length = 0, callback = readable_stream_controller_error_callback)]
    error: (),
    #[webapi(accessor_property, getter = readable_stream_controller_desired_size_getter)]
    desired_size: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ReadableByteStreamController, enumerable)]
struct ReadableByteStreamControllerPrototypeDeclaration {
    #[webapi(method, length = 1, callback = readable_stream_controller_enqueue_callback)]
    enqueue: (),
    #[webapi(method, length = 0, callback = readable_stream_controller_close_callback)]
    close: (),
    #[webapi(method, length = 0, callback = readable_stream_controller_error_callback)]
    error: (),
    #[webapi(accessor_property, getter = readable_stream_controller_desired_size_getter)]
    desired_size: (),
    #[webapi(accessor_property, getter = readable_byte_stream_controller_byob_request_getter)]
    byob_request: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TransformStreamDefaultController, enumerable)]
struct TransformStreamDefaultControllerPrototypeDeclaration {
    #[webapi(method, length = 0, callback = readable_stream_controller_enqueue_callback)]
    enqueue: (),
    #[webapi(method, length = 0, callback = transform_stream_controller_error_callback)]
    error: (),
    #[webapi(method, length = 0, callback = transform_stream_controller_terminate_callback)]
    terminate: (),
    #[webapi(accessor_property, getter = readable_stream_controller_desired_size_getter)]
    desired_size: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WritableStreamDefaultController, enumerable)]
struct WritableStreamControllerPrototypeDeclaration {
    #[webapi(accessor_property, getter = writable_stream_controller_signal_getter)]
    signal: (),
    #[webapi(method, length = 0, callback = writable_stream_controller_error_callback)]
    error: (),
}

fn readable_stream_controller_enqueue_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(stream) = stream_slot_object(scope, args.this(), STREAM_CONTROLLER_STREAM_SLOT) else {
        throw_type_error(
            scope,
            "Readable stream controller called on incompatible receiver",
        );
        return;
    };
    let byte_stream = readable_stream_is_byte_stream(scope, stream);
    if byte_stream && args.length() < 1 {
        throw_type_error(
            scope,
            "ReadableByteStreamController.enqueue requires a chunk",
        );
        return;
    }
    let chunk = args.get(0);
    let result = if byte_stream {
        enqueue_byte_chunk(scope, stream, chunk)
    } else {
        enqueue_chunk(scope, stream, chunk)
    };
    if let Err(error) = result {
        match error {
            EnqueueChunkError::ClosedOrErrored => {
                throw_type_error(
                    scope,
                    "Cannot enqueue a chunk into a readable stream that is closed or errored",
                );
            }
            EnqueueChunkError::Strategy(mut error) => {
                if let Some(writable) =
                    stream_slot_object(scope, args.this(), STREAM_CONTROLLER_WRITABLE_STREAM_SLOT)
                {
                    error = readable_stream_error(scope, stream).unwrap_or(error);
                    error_transform_stream_with_value(scope, writable, stream, error);
                }
                scope.throw_exception(error);
            }
        }
        return;
    }
    // ReadableStreamDefaultControllerEnqueue and its byte-controller
    // counterpart always finish with CallPullIfNeeded. A pipe changes
    // how a pending read is fulfilled, not this controller invariant.
    maybe_pull_stream(scope, stream);
    rv.set_undefined();
}

fn readable_byte_stream_controller_byob_request_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(stream) = stream_slot_object(scope, args.this(), STREAM_CONTROLLER_STREAM_SLOT) else {
        throw_type_error(
            scope,
            "ReadableByteStreamController.byobRequest called on incompatible receiver",
        );
        return;
    };
    rv.set(readable_byte_stream_byob_request(
        scope,
        args.this(),
        stream,
    ));
}

fn readable_stream_controller_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(stream) = stream_slot_object(scope, args.this(), STREAM_CONTROLLER_STREAM_SLOT) else {
        throw_type_error(
            scope,
            "Readable stream controller called on incompatible receiver",
        );
        return;
    };
    if readable_stream_is_byte_stream(scope, stream)
        && let Err(error) = prepare_readable_byte_stream_close(scope, stream)
    {
        error_stream(scope, stream, error);
        scope.throw_exception(error);
        return;
    }
    if close_stream(scope, stream) {
        rv.set_undefined();
        return;
    }
    throw_type_error(
        scope,
        "Cannot close a readable stream that is closed or errored",
    );
}

fn readable_stream_controller_error_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(stream) = stream_slot_object(scope, args.this(), STREAM_CONTROLLER_STREAM_SLOT) else {
        throw_type_error(
            scope,
            "Readable stream controller called on incompatible receiver",
        );
        return;
    };
    error_stream(scope, stream, args.get(0));
    rv.set_undefined();
}

fn transform_stream_controller_error_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(readable) = stream_slot_object(scope, args.this(), STREAM_CONTROLLER_STREAM_SLOT)
    else {
        throw_type_error(
            scope,
            "TransformStreamDefaultController called on incompatible receiver",
        );
        return;
    };
    let Some(writable) =
        stream_slot_object(scope, args.this(), STREAM_CONTROLLER_WRITABLE_STREAM_SLOT)
    else {
        error_stream(scope, readable, args.get(0));
        rv.set_undefined();
        return;
    };
    error_transform_stream_with_value(scope, writable, readable, args.get(0));
    rv.set_undefined();
}

fn transform_stream_controller_terminate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(readable) = stream_slot_object(scope, args.this(), STREAM_CONTROLLER_STREAM_SLOT)
    else {
        throw_type_error(
            scope,
            "TransformStreamDefaultController called on incompatible receiver",
        );
        return;
    };
    let Some(writable) =
        stream_slot_object(scope, args.this(), STREAM_CONTROLLER_WRITABLE_STREAM_SLOT)
    else {
        let _ = close_stream(scope, readable);
        rv.set_undefined();
        return;
    };
    terminate_transform_stream(scope, writable, readable);
    rv.set_undefined();
}

fn writable_stream_controller_error_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(stream) = stream_slot_object(scope, args.this(), STREAM_CONTROLLER_STREAM_SLOT) else {
        throw_type_error(
            scope,
            "WritableStreamDefaultController called on incompatible receiver",
        );
        return;
    };
    error_writable_stream_with_value(scope, stream, args.get(0));
    rv.set_undefined();
}

fn writable_stream_controller_signal_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match stream_slot_object(scope, args.this(), STREAM_CONTROLLER_SIGNAL_SLOT) {
        Some(signal) => rv.set(signal.into()),
        None => throw_type_error(
            scope,
            "WritableStreamDefaultController.signal called on incompatible receiver",
        ),
    }
}

fn readable_stream_controller_desired_size_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(stream) = stream_slot_object(scope, args.this(), STREAM_CONTROLLER_STREAM_SLOT) else {
        throw_type_error(
            scope,
            "Readable stream controller called on incompatible receiver",
        );
        return;
    };
    if readable_stream_error(scope, stream).is_some() {
        rv.set(v8::null(scope).into());
        return;
    }
    if readable_stream_closed(scope, stream) {
        rv.set(v8::Number::new(scope, 0.0).into());
        return;
    }
    let high_water_mark =
        stream_slot_number(scope, stream, READABLE_STREAM_HWM_SLOT).unwrap_or(1.0);
    let total_size = readable_stream_queue_total_size(scope, stream);
    let desired_size =
        moli_streams::strategy::StrategySnapshot::new(high_water_mark, total_size).desired_size();
    rv.set(v8::Number::new(scope, desired_size).into());
}

pub(in crate::context_bootstrap) fn new_readable_stream_controller_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    algorithms: v8::Local<'s, v8::Value>,
    byte_stream: bool,
) -> v8::Local<'s, v8::Object> {
    if byte_stream {
        return ReadableByteStreamControllerObjectDeclaration::new(stream, algorithms)
            .bind(scope)
            .expect("ReadableByteStreamController declaration should bind");
    }
    ReadableStreamControllerObjectDeclaration::new(stream, algorithms)
        .bind(scope)
        .expect("ReadableStreamDefaultController declaration should bind")
}

pub(in crate::context_bootstrap) fn new_transform_stream_controller_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    readable: v8::Local<'s, v8::Object>,
    writable: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    TransformStreamControllerObjectDeclaration::new(readable, writable)
        .bind(scope)
        .expect("TransformStreamDefaultController declaration should bind")
}

pub(in crate::context_bootstrap) fn new_writable_stream_controller_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    let abort_controller = global
        .get(scope, v8str(scope, "AbortController").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .and_then(|constructor| constructor.new_instance(scope, &[]))
        .expect("AbortController must be installed before WritableStream");
    let signal = abort_controller
        .get(scope, v8str(scope, "signal").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("AbortController must expose its signal");
    WritableStreamControllerObjectDeclaration::new(stream, signal)
        .bind(scope)
        .expect("WritableStreamDefaultController declaration should bind")
}

pub(in crate::context_bootstrap) fn install_stream_controller_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "ReadableStreamDefaultController" => {
            ReadableStreamDefaultControllerPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "ReadableByteStreamController" => {
            ReadableByteStreamControllerPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "TransformStreamDefaultController" => {
            TransformStreamDefaultControllerPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "WritableStreamDefaultController" => {
            WritableStreamControllerPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}
