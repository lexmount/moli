use super::*;
use crate::util::v8str;
use crate::web_api_interfaces;
use crate::webidl;
use moli_streams::readable::{
    AcquireReaderPlan, ReadableKind, ReaderKind, ReaderReleaseSnapshot, ReleaseReaderPlan,
    ReleasedReaderClosedPromisePlan,
};
use moli_webapi_declare::WebApiObject;

const READER_LOCK_RELEASED_MESSAGE: &str = "ReadableStreamDefaultReader lock released";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ReadableStreamDefaultReader)]
struct ReadableStreamReaderObjectDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_READER_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ReadableStreamBYOBReader)]
struct ReadableStreamByobReaderObjectDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_READER_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ReadableStreamDefaultReader.cancel")]
struct ReadableStreamReaderCancelArgs<'s> {
    #[webidl(converter = "raw")]
    reason: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ReadableStreamDefaultReader")]
struct ReadableStreamReaderConstructorArgs<'s> {
    #[webidl(required, converter = "raw")]
    stream: v8::Local<'s, v8::Value>,
}

pub(in crate::context_bootstrap) fn readable_stream_default_reader_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ReadableStreamDefaultReader': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ReadableStreamReaderConstructorArgs<'s>>(scope, &args)
    else {
        return;
    };
    let Ok(stream) = v8::Local::<v8::Object>::try_from(parsed.stream) else {
        throw_type_error(
            scope,
            "ReadableStreamDefaultReader constructor can only accept readable streams",
        );
        return;
    };
    if !is_readable_stream_object(scope, stream) {
        throw_type_error(
            scope,
            "ReadableStreamDefaultReader constructor can only accept readable streams",
        );
        return;
    }
    let transition = match readable_stream_access_snapshot(scope, stream).plan_reader_constructor(
        ReaderKind::Default,
        ReadableKind::from_byte_stream(readable_stream_is_byte_stream(scope, stream)),
    ) {
        AcquireReaderPlan::RejectLocked => {
            throw_type_error(
                scope,
                "ReadableStreamDefaultReader constructor can only accept readable streams that are not yet locked to a reader",
            );
            return;
        }
        AcquireReaderPlan::Acquire(transition) => transition,
        AcquireReaderPlan::RejectIncompatibleByob => {
            unreachable!("default readers are compatible")
        }
    };
    ReadableStreamReaderObjectDeclaration::new(stream)
        .initialize(scope, args.this())
        .expect("ReadableStreamDefaultReader declaration should initialize constructed object");
    apply_readable_stream_access_transition(scope, stream, transition);
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn readable_stream_byob_reader_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ReadableStreamBYOBReader': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ReadableStreamReaderConstructorArgs<'s>>(scope, &args)
    else {
        return;
    };
    let Ok(stream) = v8::Local::<v8::Object>::try_from(parsed.stream) else {
        throw_type_error(
            scope,
            "ReadableStreamBYOBReader constructor requires a readable byte stream",
        );
        return;
    };
    if !is_readable_stream_object(scope, stream) {
        throw_type_error(
            scope,
            "ReadableStreamBYOBReader constructor requires a readable byte stream",
        );
        return;
    }
    let transition = match readable_stream_access_snapshot(scope, stream).plan_reader_constructor(
        ReaderKind::Byob,
        ReadableKind::from_byte_stream(readable_stream_is_byte_stream(scope, stream)),
    ) {
        AcquireReaderPlan::RejectIncompatibleByob => {
            throw_type_error(
                scope,
                "ReadableStreamBYOBReader constructor requires a readable byte stream",
            );
            return;
        }
        AcquireReaderPlan::RejectLocked => {
            throw_type_error(
                scope,
                "ReadableStreamBYOBReader constructor requires an unlocked stream",
            );
            return;
        }
        AcquireReaderPlan::Acquire(transition) => transition,
    };
    ReadableStreamByobReaderObjectDeclaration::new(stream)
        .initialize(scope, args.this())
        .expect("ReadableStreamBYOBReader declaration should initialize constructed object");
    apply_readable_stream_access_transition(scope, stream, transition);
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn readable_stream_reader_read_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !readable_stream_reader_is_branded(scope, args.this()) {
        let reason = reader_lock_released_type_error(scope);
        if let Some(promise) = rejected_promise_value(scope, reason) {
            rv.set(promise);
        }
        return;
    }
    let Some(stream) = stream_slot_object(scope, args.this(), READABLE_STREAM_READER_STREAM_SLOT)
    else {
        let reason = reader_lock_released_type_error(scope);
        if let Some(promise) = rejected_promise_value(scope, reason) {
            rv.set(promise);
        } else {
            rv.set_undefined();
        }
        return;
    };
    if let Some(promise) = read_from_stream_as_promise(scope, stream) {
        rv.set(promise.into());
    } else {
        let done = done_result(scope);
        set_resolved_promise(scope, &mut rv, done.into());
    }
}

pub(in crate::context_bootstrap) fn readable_stream_byob_reader_read_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !readable_stream_reader_is_branded(scope, args.this()) {
        let reason = reader_lock_released_type_error(scope);
        if let Some(promise) = rejected_promise_value(scope, reason) {
            rv.set(promise);
        }
        return;
    }
    let Some(stream) = stream_slot_object(scope, args.this(), READABLE_STREAM_READER_STREAM_SLOT)
    else {
        let reason = reader_lock_released_type_error(scope);
        if let Some(promise) = rejected_promise_value(scope, reason) {
            rv.set(promise);
        }
        return;
    };
    if v8::Local::<v8::ArrayBufferView>::try_from(args.get(0)).is_err() {
        let reason =
            v8::Exception::type_error(scope, v8str(scope, "BYOB read requires an ArrayBufferView"));
        if let Some(promise) = rejected_promise_value(scope, reason) {
            rv.set(promise);
        }
        return;
    }
    let minimum_elements = match readable_stream_byob_read_minimum(scope, &args) {
        Ok(value) => value,
        Err(reason) => {
            if let Some(promise) = rejected_promise_value(scope, reason) {
                rv.set(promise);
            }
            return;
        }
    };
    if let Some(promise) =
        read_into_byte_stream_as_promise(scope, stream, args.get(0), minimum_elements)
    {
        rv.set(promise.into());
    }
}

fn readable_stream_byob_read_minimum<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<usize, v8::Local<'s, v8::Value>> {
    if args.length() < 2 || args.get(1).is_null_or_undefined() {
        return Ok(1);
    }
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let Some(options) = args.get(1).to_object(&scope) else {
        return Err(scope
            .exception()
            .unwrap_or_else(|| v8::undefined(&scope).into()));
    };
    let value = match options.get(&scope, v8str(&scope, "min").into()) {
        Some(value) => value,
        None if scope.has_caught() => {
            return Err(scope
                .exception()
                .unwrap_or_else(|| v8::undefined(&scope).into()));
        }
        None => return Ok(1),
    };
    if value.is_undefined() {
        return Ok(1);
    }
    let converted = match webidl::convert::<webidl::EnforceRangeUnsignedLongLong>(
        &mut scope,
        value,
        webidl::Context::member("ReadableStreamBYOBReaderReadOptions", "min"),
    ) {
        Ok(value) => value,
        Err(_) if scope.has_caught() => {
            return Err(scope
                .exception()
                .unwrap_or_else(|| v8::undefined(&scope).into()));
        }
        Err(error) => {
            let message = error.to_string();
            let message = v8::String::new(&scope, &message)
                .unwrap_or_else(|| v8str(&scope, "Invalid BYOB read minimum"));
            return Err(v8::Exception::type_error(&scope, message));
        }
    };
    usize::try_from(u64::from(converted)).map_err(|_| {
        v8::Exception::range_error(
            &scope,
            v8str(&scope, "BYOB read minimum is too large for this platform"),
        )
    })
}

pub(in crate::context_bootstrap) fn readable_stream_reader_release_lock_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !readable_stream_reader_is_branded(scope, args.this()) {
        throw_type_error(
            scope,
            "ReadableStream reader releaseLock called on incompatible receiver",
        );
        return;
    }
    release_readable_stream_reader(scope, args.this());
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn release_readable_stream_reader<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) {
    let Some(stream) = stream_slot_object(scope, reader, READABLE_STREAM_READER_STREAM_SLOT) else {
        let ReleaseReaderPlan::AlreadyReleased = ReaderReleaseSnapshot::detached().plan() else {
            unreachable!("a detached reader release is a no-op")
        };
        return;
    };
    let pending_closed_entry = stream_slot_object(
        scope,
        reader,
        READABLE_STREAM_READER_CLOSED_PROMISE_ENTRY_SLOT,
    );
    let plan = ReaderReleaseSnapshot::attached(
        readable_stream_access_snapshot(scope, stream),
        ReadableKind::from_byte_stream(readable_stream_is_byte_stream(scope, stream)),
        readable_stream_closed(scope, stream),
        pending_closed_entry.is_some(),
    )
    .plan();
    let ReleaseReaderPlan::Release {
        access,
        release_byte_controller,
        closed_promise,
    } = plan
    else {
        unreachable!("an attached reader must release its stream")
    };

    let release_reason = reader_lock_released_type_error(scope);
    if release_byte_controller {
        release_byte_stream_reader(scope, stream);
    }
    reject_pending_read_requests(scope, stream, release_reason);
    apply_readable_stream_access_transition(scope, stream, access);

    match closed_promise {
        ReleasedReaderClosedPromisePlan::RejectExisting => {
            if let Some(entry) = pending_closed_entry {
                remove_pending_closed_promise(scope, stream, entry);
                reject_pending_read(scope, entry, release_reason);
                suppress_pending_read_unhandled_rejection(scope, entry);
            }
        }
        ReleasedReaderClosedPromisePlan::ReplaceWithRejected => {
            if let Some(entry) = pending_closed_entry {
                remove_pending_closed_promise(scope, stream, entry);
            }
            replace_released_reader_closed_promise(scope, reader, release_reason);
        }
        ReleasedReaderClosedPromisePlan::CreateRejected => {
            replace_released_reader_closed_promise(scope, reader, release_reason);
        }
    }
    set_stream_slot_value(
        scope,
        reader,
        READABLE_STREAM_READER_CLOSED_PROMISE_ENTRY_SLOT,
        v8::undefined(scope).into(),
    );
    set_stream_slot_value(
        scope,
        reader,
        READABLE_STREAM_READER_STREAM_SLOT,
        v8::undefined(scope).into(),
    );
}

fn replace_released_reader_closed_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    release_reason: v8::Local<'s, v8::Value>,
) {
    let Some(promise) = rejected_promise_value(scope, release_reason) else {
        return;
    };
    suppress_promise_unhandled_rejection(scope, promise);
    set_stream_slot_value(
        scope,
        reader,
        READABLE_STREAM_READER_CLOSED_PROMISE_SLOT,
        promise,
    );
}

fn reader_lock_released_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    v8::Exception::type_error(scope, v8str(scope, READER_LOCK_RELEASED_MESSAGE))
}

pub(in crate::context_bootstrap) fn readable_stream_reader_cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !readable_stream_reader_is_branded(scope, args.this()) {
        let reason = reader_lock_released_type_error(scope);
        if let Some(promise) = rejected_promise_value(scope, reason) {
            rv.set(promise);
        }
        return;
    }
    let Some(stream) = stream_slot_object(scope, args.this(), READABLE_STREAM_READER_STREAM_SLOT)
    else {
        let reason = reader_lock_released_type_error(scope);
        if let Some(promise) = rejected_promise_value(scope, reason) {
            rv.set(promise);
        } else {
            rv.set_undefined();
        }
        return;
    };
    let Some(parsed) = webidl::parse_args::<ReadableStreamReaderCancelArgs<'s>>(scope, &args)
    else {
        return;
    };
    let reason = parsed.reason.unwrap_or_else(|| v8::undefined(scope).into());
    if let Some(promise) = cancel_readable_stream(scope, stream, reason) {
        rv.set(promise);
    } else {
        rv.set_undefined();
    }
}

pub(in crate::context_bootstrap) fn readable_stream_reader_closed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !readable_stream_reader_is_branded(scope, args.this()) {
        let message = v8str(
            scope,
            "ReadableStream reader closed called on incompatible receiver",
        );
        let reason = v8::Exception::type_error(scope, message);
        if let Some(promise) = rejected_promise_value(scope, reason) {
            rv.set(promise);
        }
        return;
    }
    if let Some(promise) = stream_slot_value(
        scope,
        args.this(),
        READABLE_STREAM_READER_CLOSED_PROMISE_SLOT,
    ) && !promise.is_null_or_undefined()
    {
        rv.set(promise);
        return;
    }
    let Some(stream) = stream_slot_object(scope, args.this(), READABLE_STREAM_READER_STREAM_SLOT)
    else {
        rv.set_undefined();
        return;
    };
    let Some((promise, pending_entry)) = readable_stream_closed_promise(scope, stream) else {
        rv.set_undefined();
        return;
    };
    set_stream_slot_value(
        scope,
        args.this(),
        READABLE_STREAM_READER_CLOSED_PROMISE_SLOT,
        promise,
    );
    if let Some(pending_entry) = pending_entry {
        set_stream_slot_value(
            scope,
            args.this(),
            READABLE_STREAM_READER_CLOSED_PROMISE_ENTRY_SLOT,
            pending_entry.into(),
        );
    }
    rv.set(promise);
}

pub(in crate::context_bootstrap) fn new_readable_stream_reader_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let relevant_context = stream.get_creation_context(scope)?;
    if relevant_context == scope.get_current_context() {
        return ReadableStreamReaderObjectDeclaration::new(stream)
            .bind(scope)
            .ok();
    }
    let stream = v8::Global::new(scope, stream);
    let reader = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let stream = v8::Local::new(target_scope, &stream);
        let reader = ReadableStreamReaderObjectDeclaration::new(stream)
            .bind(target_scope)
            .ok()?;
        v8::Global::new(target_scope, reader)
    };
    Some(v8::Local::new(scope, &reader))
}

pub(in crate::context_bootstrap) fn new_readable_stream_byob_reader_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let relevant_context = stream.get_creation_context(scope)?;
    if relevant_context == scope.get_current_context() {
        return ReadableStreamByobReaderObjectDeclaration::new(stream)
            .bind(scope)
            .ok();
    }
    let stream = v8::Global::new(scope, stream);
    let reader = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let stream = v8::Local::new(target_scope, &stream);
        let reader = ReadableStreamByobReaderObjectDeclaration::new(stream)
            .bind(target_scope)
            .ok()?;
        v8::Global::new(target_scope, reader)
    };
    Some(v8::Local::new(scope, &reader))
}

fn readable_stream_reader_is_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::ReadableStreamDefaultReader::is_instance(scope, reader)
        || web_api_interfaces::ReadableStreamBYOBReader::is_instance(scope, reader)
}
