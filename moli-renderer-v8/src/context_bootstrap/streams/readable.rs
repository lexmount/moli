use super::super::stream_adapter::{
    TeeStartError, apply_readable_stream_access_transition, enqueue_byte_chunk,
    install_readable_stream_pipe_to_abort_signal, lock_readable_stream,
    new_lazy_readable_byte_stream_object, new_readable_byte_stream_object,
    new_readable_stream_pipe_owner, prime_readable_stream_pipe_to, readable_stream_access_snapshot,
    register_readable_stream_pipe_owner, register_writable_stream_pipe_owner,
    set_writable_stream_locked, suppress_promise_unhandled_rejection, tee_readable_stream,
};
use super::*;
use crate::context_bootstrap::stream_objects::readable_stream_async_iterator_prototype;
use crate::webidl;
use moli_streams::pipe::{PipeEntryObservation, PipeEntryPlan, PipeOptions};
use moli_streams::readable::{AcquireReaderPlan, CancelEntryPlan, ReadableKind, ReaderKind};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = "ReadableStream AsyncIterator")]
struct ReadableStreamAsyncIteratorObjectDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_ITERATOR_READER_SLOT)]
    reader: v8::Local<'scope, v8::Object>,
    #[webapi(slot = READABLE_STREAM_ITERATOR_CLOSED_SLOT, init = false)]
    closed: (),
    #[webapi(slot = READABLE_STREAM_ITERATOR_PREVENT_CANCEL_SLOT)]
    prevent_cancel: bool,
    #[webapi(slot = READABLE_STREAM_ITERATOR_RETURNING_SLOT, init = false)]
    returning: (),
    #[webapi(slot = READABLE_STREAM_ITERATOR_OPERATION_ACTIVE_SLOT, init = false)]
    operation_active: (),
    #[webapi(slot = READABLE_STREAM_ITERATOR_OPERATIONS_SLOT, init = "array")]
    operations: (),
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ReadableStream.cancel")]
struct ReadableStreamCancelArgs<'s> {
    #[webidl(converter = "raw")]
    reason: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ReadableStream.pipeThrough")]
struct ReadableStreamPipeThroughArgs<'s> {
    #[webidl(required, name = "transform")]
    transform: v8::Local<'s, v8::Object>,
    #[webidl(index = 1, converter = "raw")]
    options: Option<v8::Local<'s, v8::Value>>,
}

#[derive(Default)]
struct StreamPipeOptions<'s> {
    core: PipeOptions,
    signal: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ReadableStream.values")]
struct ReadableStreamValuesArgs {
    #[webidl(with = readable_stream_iterator_options_arg)]
    options: ReadableStreamIteratorOptions,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ReadableStreamIteratorOptions")]
struct ReadableStreamIteratorOptions {
    #[webidl(default = false)]
    prevent_cancel: bool,
}

pub(crate) fn is_readable_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    moli_webapi_declare::implements_interface(scope, object, "ReadableStream")
}

pub(crate) fn new_readable_stream_from_array_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::ArrayBuffer>,
    len: usize,
) -> Option<v8::Local<'s, v8::Object>> {
    let stream = new_readable_byte_stream_object(scope);
    if len != 0 {
        let source = v8::Uint8Array::new(scope, buffer, 0, len)?;
        let mut bytes = vec![0; len];
        let written = source.copy_contents(&mut bytes);
        bytes.truncate(written);
        let chunk = crate::context_bootstrap::shared::new_uint8_array_from_bytes(scope, bytes)?;
        let _ = enqueue_byte_chunk(scope, stream, chunk.into());
    }
    close_stream(scope, stream);
    Some(stream)
}

pub(crate) fn new_readable_stream_from_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    new_lazy_readable_byte_stream_object(scope, source)
}

pub(in crate::context_bootstrap) fn readable_stream_get_reader_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = args.this();
    if !is_readable_stream_object(scope, stream) {
        throw_type_error(
            scope,
            "ReadableStream.getReader called on incompatible receiver",
        );
        return;
    }
    let Some(reader_kind) = readable_stream_get_reader_mode(scope, &args) else {
        return;
    };
    let stream_kind = ReadableKind::from_byte_stream(readable_stream_is_byte_stream(scope, stream));
    let transition = match readable_stream_access_snapshot(scope, stream)
        .plan_get_reader(reader_kind, stream_kind)
    {
        AcquireReaderPlan::RejectLocked => {
            throw_type_error(scope, "ReadableStream is locked");
            return;
        }
        AcquireReaderPlan::RejectIncompatibleByob => {
            throw_type_error(scope, "ReadableStream is not a byte stream");
            return;
        }
        AcquireReaderPlan::Acquire(transition) => transition,
    };
    let reader = match reader_kind {
        ReaderKind::Default => new_readable_stream_reader_object(scope, stream),
        ReaderKind::Byob => new_readable_stream_byob_reader_object(scope, stream),
    };
    let Some(reader) = reader else {
        rv.set_undefined();
        return;
    };
    apply_readable_stream_access_transition(scope, stream, transition);
    rv.set(reader.into());
}

fn readable_stream_get_reader_mode(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<ReaderKind> {
    let options = args.get(0);
    if options.is_null_or_undefined() {
        return Some(ReaderKind::Default);
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(options) else {
        throw_type_error(
            scope,
            "ReadableStream.getReader options must be a dictionary",
        );
        return None;
    };
    let mode_key = v8str(scope, "mode");
    let mode = options.get(scope, mode_key.into())?;
    if mode.is_undefined() {
        return Some(ReaderKind::Default);
    }
    let mode = mode.to_string(scope)?;
    let mode = mode.to_rust_string_lossy(scope);
    if mode == "byob" {
        Some(ReaderKind::Byob)
    } else {
        throw_type_error(
            scope,
            "ReadableStream.getReader mode must be undefined or 'byob'",
        );
        None
    }
}

pub(in crate::context_bootstrap) fn readable_stream_cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_readable_stream_object(scope, args.this()) {
        let error = v8::Exception::type_error(
            scope,
            v8str(
                scope,
                "ReadableStream.cancel called on incompatible receiver",
            ),
        );
        if let Some(promise) = rejected_promise_value(scope, error) {
            rv.set(promise);
        }
        return;
    }
    let Some(parsed) = webidl::parse_args::<ReadableStreamCancelArgs<'s>>(scope, &args) else {
        return;
    };
    match readable_stream_access_snapshot(scope, args.this()).plan_cancel_entry() {
        CancelEntryPlan::RejectLocked => {
            let error =
                v8::Exception::type_error(scope, v8str(scope, "Cannot cancel a locked stream"));
            if let Some(promise) = rejected_promise_value(scope, error) {
                rv.set(promise);
            } else {
                rv.set_undefined();
            }
            return;
        }
        CancelEntryPlan::Continue => {}
    }
    let reason = parsed.reason.unwrap_or_else(|| v8::undefined(scope).into());
    if let Some(promise) = cancel_readable_stream(scope, args.this(), reason) {
        rv.set(promise);
    } else {
        rv.set_undefined();
    }
}

pub(in crate::context_bootstrap) fn readable_stream_pipe_through_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = args.this();
    if !is_readable_stream_object(scope, stream) {
        throw_type_error(
            scope,
            "ReadableStream.pipeThrough called on incompatible receiver",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ReadableStreamPipeThroughArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(readable) = readable_writable_pair_readable(scope, parsed.transform) else {
        return;
    };
    let Some(writable) = readable_writable_pair_writable(scope, parsed.transform) else {
        return;
    };
    let options = match parse_stream_pipe_options(scope, parsed.options) {
        Ok(options) => options,
        Err(error) => {
            scope.throw_exception(error);
            return;
        }
    };
    let source_locked = readable_stream_locked(scope, stream);
    let destination_locked = writable_stream_locked(scope, writable);
    match PipeEntryObservation::new(source_locked, destination_locked).plan() {
        PipeEntryPlan::RejectSourceLocked => {
            throw_type_error(scope, "Cannot pipe a locked stream");
            return;
        }
        PipeEntryPlan::RejectDestinationLocked => {
            throw_type_error(scope, "Cannot pipe to a locked stream");
            return;
        }
        PipeEntryPlan::Start => {}
    }
    if let Some(pipe_promise) = start_readable_stream_pipe_to(scope, stream, writable, options) {
        suppress_promise_unhandled_rejection(scope, pipe_promise);
    }
    rv.set(readable.into());
}

fn readable_writable_pair_readable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pair: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let value = pair.get(scope, v8str(scope, "readable").into())?;
    let Ok(readable) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(
            scope,
            "ReadableWritablePair.readable must be a ReadableStream",
        );
        return None;
    };
    if !is_readable_stream_object(scope, readable) {
        throw_type_error(
            scope,
            "ReadableWritablePair.readable must be a ReadableStream",
        );
        return None;
    }
    Some(readable)
}

fn readable_writable_pair_writable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pair: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let value = pair.get(scope, v8str(scope, "writable").into())?;
    let Ok(writable) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(
            scope,
            "ReadableWritablePair.writable must be a WritableStream",
        );
        return None;
    };
    if !is_writable_stream_object(scope, writable) {
        throw_type_error(
            scope,
            "ReadableWritablePair.writable must be a WritableStream",
        );
        return None;
    }
    Some(writable)
}

fn parse_stream_pipe_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
) -> Result<StreamPipeOptions<'s>, v8::Local<'s, v8::Value>> {
    let Some(value) = value else {
        return Ok(StreamPipeOptions::default());
    };
    if value.is_null_or_undefined() {
        return Ok(StreamPipeOptions::default());
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        return Err(v8::Exception::type_error(
            scope,
            v8str(scope, "StreamPipeOptions must be an object"),
        ));
    };
    let prevent_abort = stream_pipe_option_bool(scope, options, "preventAbort")?;
    let prevent_cancel = stream_pipe_option_bool(scope, options, "preventCancel")?;
    let prevent_close = stream_pipe_option_bool(scope, options, "preventClose")?;
    let signal = stream_pipe_signal(scope, options)?;
    Ok(StreamPipeOptions {
        core: PipeOptions::new(prevent_close, prevent_abort, prevent_cancel),
        signal,
    })
}

fn stream_pipe_option_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<bool, v8::Local<'s, v8::Value>> {
    let value = stream_pipe_option_value(scope, options, name)?;
    Ok(value.is_some_and(|value| value.boolean_value(scope)))
}

fn stream_pipe_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
) -> Result<Option<v8::Local<'s, v8::Object>>, v8::Local<'s, v8::Value>> {
    let Some(value) = stream_pipe_option_value(scope, options, "signal")? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    let Ok(signal) = v8::Local::<v8::Object>::try_from(value) else {
        return Err(v8::Exception::type_error(
            scope,
            v8str(scope, "StreamPipeOptions.signal must be an AbortSignal"),
        ));
    };
    if !stream_pipe_signal_is_abort_signal(scope, signal) {
        return Err(v8::Exception::type_error(
            scope,
            v8str(scope, "StreamPipeOptions.signal must be an AbortSignal"),
        ));
    }
    Ok(Some(signal))
}

fn stream_pipe_signal_is_abort_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> bool {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && unsafe { &mut *host_ptr }.is_abort_signal(scope, signal)
    {
        return true;
    }
    crate::worker::abort::worker_abort_signal_id(scope, signal).is_some()
}

fn stream_pipe_option_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    let Some(key) = v8_string(&scope, name) else {
        return Ok(None);
    };
    match options.get(&scope, key.into()) {
        Some(value) => Ok(Some(value)),
        None if scope.has_caught() => Err(scope
            .exception()
            .unwrap_or_else(|| v8::undefined(&scope).into())),
        None => Ok(None),
    }
}

fn start_readable_stream_pipe_to<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    writable: v8::Local<'s, v8::Object>,
    options: StreamPipeOptions<'s>,
) -> Option<v8::Local<'s, v8::Value>> {
    let (promise, owner) = new_readable_stream_pipe_owner(scope, stream, writable, options.core)?;
    register_readable_stream_pipe_owner(scope, stream, owner);
    if !lock_readable_stream(scope, stream) {
        return None;
    }
    set_writable_stream_locked(scope, writable, true);
    register_writable_stream_pipe_owner(scope, writable, owner);
    if let Some(signal) = options.signal {
        install_readable_stream_pipe_to_abort_signal(scope, stream, signal);
    }
    prime_readable_stream_pipe_to(scope, stream);
    Some(promise.into())
}

/// Connect a browser-owned cross-realm readable endpoint to a writable
/// destination. Transfer admission has already established that both streams
/// are unlocked; the ordinary pipe owner remains responsible for all later
/// close/error/backpressure settlement.
pub(super) fn start_internal_readable_stream_pipe_to<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    writable: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    debug_assert!(!readable_stream_access_snapshot(scope, stream).locked());
    debug_assert!(!writable_stream_snapshot(scope, writable).locked());
    start_readable_stream_pipe_to(scope, stream, writable, StreamPipeOptions::default())
}

pub(in crate::context_bootstrap) fn readable_stream_pipe_to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = args.this();
    if !is_readable_stream_object(scope, stream) {
        set_rejected_pipe_to_type_error(scope, &mut rv, "Cannot pipe an invalid ReadableStream");
        return;
    }
    if args.length() < 1 {
        set_rejected_pipe_to_type_error(
            scope,
            &mut rv,
            "ReadableStream.pipeTo requires a destination",
        );
        return;
    }
    let Ok(writable) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        set_rejected_pipe_to_type_error(
            scope,
            &mut rv,
            "ReadableStream.pipeTo destination must be an object",
        );
        return;
    };
    if !is_writable_stream_object(scope, writable) {
        set_rejected_pipe_to_type_error(scope, &mut rv, "Cannot pipe to an invalid WritableStream");
        return;
    }
    let options_value = (args.length() > 1 && !args.get(1).is_undefined()).then(|| args.get(1));
    let options = match parse_stream_pipe_options(scope, options_value) {
        Ok(options) => options,
        Err(error) => {
            set_rejected_pipe_to_error(scope, &mut rv, error);
            return;
        }
    };
    let source_locked = readable_stream_locked(scope, stream);
    let destination_locked = writable_stream_locked(scope, writable);
    match PipeEntryObservation::new(source_locked, destination_locked).plan() {
        PipeEntryPlan::RejectSourceLocked => {
            set_rejected_pipe_to_type_error(scope, &mut rv, "Cannot pipe a locked stream");
            return;
        }
        PipeEntryPlan::RejectDestinationLocked => {
            set_rejected_pipe_to_type_error(scope, &mut rv, "Cannot pipe to a locked stream");
            return;
        }
        PipeEntryPlan::Start => {}
    }
    if let Some(promise) = start_readable_stream_pipe_to(scope, stream, writable, options) {
        rv.set(promise);
    }
}

pub(in crate::context_bootstrap) fn readable_stream_tee_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = args.this();
    if !is_readable_stream_object(scope, stream) {
        throw_type_error(scope, "ReadableStream.tee called on incompatible receiver");
        return;
    }
    match tee_readable_stream(scope, stream) {
        Ok(result) => rv.set(result.into()),
        Err(TeeStartError::Locked) => throw_type_error(scope, "Cannot tee a locked stream"),
        Err(TeeStartError::Unavailable) => rv.set_undefined(),
    }
}

fn set_rejected_pipe_to_type_error(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    message: &str,
) {
    let message = v8::String::new(scope, message).expect("pipeTo TypeError message");
    let error = v8::Exception::type_error(scope, message);
    set_rejected_pipe_to_error(scope, rv, error);
}

fn set_rejected_pipe_to_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    error: v8::Local<'s, v8::Value>,
) {
    if let Some(promise) = rejected_promise_value(scope, error) {
        rv.set(promise);
    } else {
        rv.set_undefined();
    }
}

pub(in crate::context_bootstrap) fn readable_stream_async_iterator_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = args.this();
    if !is_readable_stream_object(scope, stream) {
        throw_type_error(
            scope,
            "ReadableStream.values called on incompatible receiver",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ReadableStreamValuesArgs>(scope, &args) else {
        return;
    };
    let transition = match readable_stream_access_snapshot(scope, stream).plan_get_reader(
        ReaderKind::Default,
        ReadableKind::from_byte_stream(readable_stream_is_byte_stream(scope, stream)),
    ) {
        AcquireReaderPlan::RejectLocked => {
            throw_type_error(scope, "ReadableStream is locked");
            return;
        }
        AcquireReaderPlan::Acquire(transition) => transition,
        AcquireReaderPlan::RejectIncompatibleByob => unreachable!("default readers are compatible"),
    };
    let Some(reader) = new_readable_stream_reader_object(scope, stream) else {
        rv.set_undefined();
        return;
    };
    apply_readable_stream_access_transition(scope, stream, transition);
    let Some(iterator) = new_readable_stream_async_iterator_object(
        scope,
        stream,
        reader,
        parsed.options.prevent_cancel,
    ) else {
        release_readable_stream_reader(scope, reader);
        rv.set_undefined();
        return;
    };
    rv.set(iterator.into());
}

fn new_readable_stream_async_iterator_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reader: v8::Local<'s, v8::Object>,
    prevent_cancel: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let relevant_context = stream.get_creation_context(scope)?;
    if relevant_context == scope.get_current_context() {
        let prototype = readable_stream_async_iterator_prototype(scope)?;
        return ReadableStreamAsyncIteratorObjectDeclaration::new(
            reader,
            prevent_cancel,
            prototype,
        )
        .bind(scope)
        .ok();
    }
    let reader = v8::Global::new(scope, reader);
    let iterator = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let reader = v8::Local::new(target_scope, &reader);
        let prototype = readable_stream_async_iterator_prototype(target_scope)?;
        let iterator =
            ReadableStreamAsyncIteratorObjectDeclaration::new(reader, prevent_cancel, prototype)
                .bind(target_scope)
                .ok()?;
        v8::Global::new(target_scope, iterator)
    };
    Some(v8::Local::new(scope, &iterator))
}

fn readable_stream_iterator_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<ReadableStreamIteratorOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("ReadableStream.values", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}

pub(in crate::context_bootstrap) fn readable_stream_locked_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_readable_stream_object(scope, args.this()) {
        throw_type_error(
            scope,
            "ReadableStream.locked called on incompatible receiver",
        );
        return;
    }
    let locked = readable_stream_locked(scope, args.this());
    rv.set(v8::Boolean::new(scope, locked).into());
}
