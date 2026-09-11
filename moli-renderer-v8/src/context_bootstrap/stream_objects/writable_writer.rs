use super::*;
use crate::util::v8str;
use crate::web_api_interfaces;
use crate::webidl;
use moli_streams::writable::{AcquireWriterPlan, DesiredSizePlan, WriterWriteEntryPlan};
use moli_webapi_declare::WebApiObject;

const WRITER_LOCK_RELEASED_MESSAGE: &str = "WritableStreamDefaultWriter lock released";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WritableStreamDefaultWriter)]
struct WritableStreamWriterObjectDeclaration<'scope> {
    #[webapi(slot = WRITABLE_STREAM_WRITER_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
    #[webapi(slot = WRITABLE_STREAM_WRITER_READY_PROMISE_SLOT, init = "undefined")]
    ready_promise: (),
    #[webapi(slot = WRITABLE_STREAM_WRITER_CLOSED_PROMISE_SLOT, init = "undefined")]
    closed_promise: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WritableStreamDefaultWriter")]
struct WritableStreamWriterConstructorArgs<'s> {
    #[webidl(required)]
    stream: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WritableStreamDefaultWriter.write")]
struct WritableStreamWriterWriteArgs<'s> {
    #[webidl(converter = "raw")]
    chunk: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WritableStreamDefaultWriter.abort")]
struct WritableStreamWriterAbortArgs<'s> {
    #[webidl(converter = "raw")]
    reason: Option<v8::Local<'s, v8::Value>>,
}

pub(in crate::context_bootstrap) fn writable_stream_default_writer_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'WritableStreamDefaultWriter': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<WritableStreamWriterConstructorArgs<'s>>(scope, &args)
    else {
        return;
    };
    if !is_writable_stream_object(scope, parsed.stream) {
        throw_type_error(
            scope,
            "WritableStreamDefaultWriter constructor requires a WritableStream",
        );
        return;
    }
    if writable_stream_snapshot(scope, parsed.stream).plan_acquire_writer()
        == AcquireWriterPlan::RejectLocked
    {
        throw_type_error(
            scope,
            "WritableStreamDefaultWriter constructor requires an unlocked stream",
        );
        return;
    }
    WritableStreamWriterObjectDeclaration::new(parsed.stream)
        .initialize(scope, args.this())
        .expect("WritableStreamDefaultWriter declaration should initialize constructed object");
    acquire_writable_stream_writer(scope, args.this(), parsed.stream);
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn writable_stream_writer_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !writable_stream_writer_is_branded(scope, args.this()) {
        set_rejected_writer_lock_released_promise(scope, &mut rv);
        return;
    }
    if let Some(stream) = stream_slot_object(scope, args.this(), WRITABLE_STREAM_WRITER_STREAM_SLOT)
    {
        if let Some(error_promise) = writable_stream_writer_write_error_promise(scope, stream) {
            rv.set(error_promise);
            return;
        }
        let Some(parsed) = webidl::parse_args::<WritableStreamWriterWriteArgs<'s>>(scope, &args)
        else {
            return;
        };
        let chunk = parsed.chunk.unwrap_or_else(|| v8::undefined(scope).into());
        if let Some(write_result) =
            writable_stream_writer_write_internal(scope, args.this(), stream, chunk)
        {
            if write_result.is_promise() {
                rv.set(write_result);
            } else {
                set_resolved_promise(scope, &mut rv, write_result);
            }
            return;
        }
        set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
        return;
    }
    set_rejected_writer_lock_released_promise(scope, &mut rv);
}

pub(in crate::context_bootstrap) fn writable_stream_writer_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !writable_stream_writer_is_branded(scope, args.this()) {
        set_rejected_writer_lock_released_promise(scope, &mut rv);
        return;
    }
    if let Some(stream) = stream_slot_object(scope, args.this(), WRITABLE_STREAM_WRITER_STREAM_SLOT)
    {
        if let Some(close_result) = writable_stream_close_internal(scope, stream) {
            rv.set(close_result);
        } else {
            set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
        }
        return;
    }
    set_rejected_writer_lock_released_promise(scope, &mut rv);
}

pub(in crate::context_bootstrap) fn writable_stream_writer_abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !writable_stream_writer_is_branded(scope, args.this()) {
        set_rejected_writer_lock_released_promise(scope, &mut rv);
        return;
    }
    if let Some(stream) = stream_slot_object(scope, args.this(), WRITABLE_STREAM_WRITER_STREAM_SLOT)
    {
        let Some(parsed) = webidl::parse_args::<WritableStreamWriterAbortArgs<'s>>(scope, &args)
        else {
            return;
        };
        let reason = parsed.reason.unwrap_or_else(|| v8::undefined(scope).into());
        if let Some(promise) = writable_stream_abort_promise(scope, stream, reason) {
            rv.set(promise);
        } else {
            set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
        }
        return;
    }
    set_rejected_writer_lock_released_promise(scope, &mut rv);
}

pub(in crate::context_bootstrap) fn writable_stream_writer_release_lock_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let writer = args.this();
    if !writable_stream_writer_is_branded(scope, writer) {
        throw_type_error(
            scope,
            "WritableStreamDefaultWriter.releaseLock called on incompatible receiver",
        );
        return;
    }
    if let Some(stream) = stream_slot_object(scope, writer, WRITABLE_STREAM_WRITER_STREAM_SLOT) {
        let release_error = writer_lock_released_type_error(scope);
        release_writable_stream_writer(scope, writer, stream, release_error);
    }
    rv.set_undefined();
}

fn writer_lock_released_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    v8::Exception::type_error(scope, v8str(scope, WRITER_LOCK_RELEASED_MESSAGE))
}

fn set_rejected_writer_lock_released_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let error = writer_lock_released_type_error(scope);
    if let Some(promise) = rejected_promise_value(scope, error) {
        rv.set(promise);
    } else {
        rv.set_undefined();
    }
}

pub(in crate::context_bootstrap) fn writable_stream_writer_desired_size_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !writable_stream_writer_is_branded(scope, args.this()) {
        throw_type_error(
            scope,
            "WritableStreamDefaultWriter.desiredSize called on incompatible receiver",
        );
        return;
    }
    let Some(stream) = stream_slot_object(scope, args.this(), WRITABLE_STREAM_WRITER_STREAM_SLOT)
    else {
        let error = writer_lock_released_type_error(scope);
        scope.throw_exception(error);
        return;
    };
    match writable_stream_snapshot(scope, stream).plan_desired_size() {
        DesiredSizePlan::Null => rv.set(v8::null(scope).into()),
        DesiredSizePlan::Zero => rv.set(v8::Number::new(scope, 0.0).into()),
        DesiredSizePlan::Value(desired_size) => {
            rv.set(v8::Number::new(scope, desired_size).into());
        }
    }
}

pub(in crate::context_bootstrap) fn writable_stream_writer_ready_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !writable_stream_writer_is_branded(scope, args.this()) {
        set_rejected_writer_lock_released_promise(scope, &mut rv);
        return;
    }
    if let Some(promise) = writable_stream_writer_ready_promise_value(scope, args.this()) {
        rv.set(promise);
    }
}

pub(in crate::context_bootstrap) fn writable_stream_writer_closed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !writable_stream_writer_is_branded(scope, args.this()) {
        set_rejected_writer_lock_released_promise(scope, &mut rv);
        return;
    }
    if let Some(promise) = writable_stream_writer_closed_promise_value(scope, args.this()) {
        rv.set(promise);
    }
}

fn writable_stream_writer_write_error_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    match writable_stream_snapshot(scope, stream).plan_writer_write_entry() {
        WriterWriteEntryPlan::RejectStoredError => writable_stream_stored_error(scope, stream)
            .and_then(|error| rejected_promise_value(scope, error)),
        WriterWriteEntryPlan::Continue => None,
    }
}

pub(in crate::context_bootstrap) fn new_writable_stream_writer_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let writer = WritableStreamWriterObjectDeclaration::new(stream)
        .bind(scope)
        .ok()?;
    acquire_writable_stream_writer(scope, writer, stream);
    Some(writer)
}

fn writable_stream_writer_is_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::WritableStreamDefaultWriter::is_instance(scope, writer)
}
