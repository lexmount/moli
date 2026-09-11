//! Adapter-owned implementations of the Streams Standard read-request structs.
//!
//! A public reader owns a promise-backed request. Composite algorithms such as
//! `ReadableStreamTee` instead own internal chunk/close/error steps. Keeping
//! both forms behind this boundary is important: routing an internal chunk
//! through a JavaScript promise would perform thenable assimilation on the
//! `{ value, done }` result and make stream internals observable to author
//! modifications of `Object.prototype.then`.

use super::utils::require_internal_stream_value;
use super::*;
use moli_webapi_declare::WebApiObject;

const READ_REQUEST_CHUNK_STEPS_SLOT: &str = "__moliStreamReadRequestChunkSteps";
const READ_REQUEST_CLOSE_STEPS_SLOT: &str = "__moliStreamReadRequestCloseSteps";
const READ_REQUEST_ERROR_STEPS_SLOT: &str = "__moliStreamReadRequestErrorSteps";

#[derive(WebApiObject)]
#[webapi(record)]
struct InternalReadRequestDeclaration<'scope> {
    #[webapi(slot = READ_REQUEST_CHUNK_STEPS_SLOT)]
    chunk_steps: v8::Local<'scope, v8::Function>,
    #[webapi(slot = READ_REQUEST_CLOSE_STEPS_SLOT)]
    close_steps: v8::Local<'scope, v8::Function>,
    #[webapi(slot = READ_REQUEST_ERROR_STEPS_SLOT)]
    error_steps: v8::Local<'scope, v8::Function>,
}

pub(super) fn new_internal_read_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    chunk_steps: v8::Local<'s, v8::Function>,
    close_steps: v8::Local<'s, v8::Function>,
    error_steps: v8::Local<'s, v8::Function>,
) -> v8::Local<'s, v8::Object> {
    InternalReadRequestDeclaration::new(chunk_steps, close_steps, error_steps)
        .bind(scope)
        .expect("internal Streams read request should bind")
}

/// Performs a read request's chunk or close steps.
///
/// `value` is passed to close steps as well so the same representation can
/// implement both a default read request (where it is `undefined`) and a BYOB
/// read-into request (where it can be the terminal zero-length view).
pub(super) fn fulfill_read_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    done: bool,
) {
    let step_slot = if done {
        READ_REQUEST_CLOSE_STEPS_SLOT
    } else {
        READ_REQUEST_CHUNK_STEPS_SLOT
    };
    if let Some(step) = get_private_value(scope, request, step_slot)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let undefined = v8::undefined(scope);
        require_internal_stream_value(
            step.call(scope, undefined.into(), &[value]),
            "read-request step invocation",
            if done { "close steps" } else { "chunk steps" },
        );
        return;
    }

    let result = iter_result(scope, value, done);
    resolve_pending_promise(scope, request, result.into());
}

/// Performs a read request's error steps without assuming that it is backed by
/// a JavaScript promise.
pub(super) fn error_read_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    if let Some(step) = get_private_value(scope, request, READ_REQUEST_ERROR_STEPS_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let undefined = v8::undefined(scope);
        require_internal_stream_value(
            step.call(scope, undefined.into(), &[reason]),
            "read-request step invocation",
            "error steps",
        );
        return;
    }

    reject_pending_read(scope, request, reason);
}
