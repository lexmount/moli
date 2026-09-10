//! V8 storage and effect adapter for the runtime-independent Streams core.
//!
//! JavaScript payload identity, wrappers, promises, callbacks, private slots,
//! ArrayBuffer operations, and queue storage live here. Stream lifecycle and
//! orchestration decisions are decoded into `moli-streams` snapshots and
//! committed from typed plans.

use super::*;
use crate::util::{get_private_value, set_private_value};

const READABLE_STREAM_PENDING_READ_RESOLVE_SLOT: &str = "__moliReadableStreamPendingResolve";
const READABLE_STREAM_PENDING_READ_REJECT_SLOT: &str = "__moliReadableStreamPendingReject";
pub(in crate::context_bootstrap) const READABLE_STREAM_ALGORITHM_SOURCE_INDEX: u32 = 0;
pub(in crate::context_bootstrap) const READABLE_STREAM_ALGORITHM_PULL_INDEX: u32 = 1;
pub(in crate::context_bootstrap) const READABLE_STREAM_ALGORITHM_CANCEL_INDEX: u32 = 2;
const READABLE_STREAM_ALGORITHM_SIZE_INDEX: u32 = 3;
const READABLE_STREAM_PULL_STATE_STARTED: u32 = 1 << 0;
const READABLE_STREAM_PULL_STATE_PULLING: u32 = 1 << 1;
const READABLE_STREAM_PULL_STATE_PULL_AGAIN: u32 = 1 << 2;
const READABLE_STREAM_PULL_STATE_CLOSE_REQUESTED: u32 = 1 << 3;
// Adapter call-stack state. This is narrower than the spec's [[pulling]]:
// it is set only while the underlying pull callback itself is executing.
const READABLE_STREAM_PULL_STATE_CALLBACK_ACTIVE: u32 = 1 << 5;
const WRITABLE_STREAM_STRATEGY_HIGH_WATER_MARK_INDEX: u32 = 0;
const WRITABLE_STREAM_STRATEGY_SIZE_ALGORITHM_INDEX: u32 = 1;
const WRITABLE_STREAM_STRATEGY_TOTAL_SIZE_INDEX: u32 = 2;
const WRITABLE_STREAM_STRATEGY_STORED_ERROR_INDEX: u32 = 3;
const WRITABLE_STREAM_STRATEGY_PENDING_WRITES_INDEX: u32 = 4;
const WRITABLE_STREAM_STRATEGY_START_PENDING_INDEX: u32 = 5;
const WRITABLE_STREAM_STRATEGY_ERRORED_INDEX: u32 = 6;
const WRITABLE_STREAM_STRATEGY_QUEUE_PUMP_STATE_INDEX: u32 = 7;
const WRITABLE_STREAM_STRATEGY_PENDING_WRITES_HEAD_INDEX: u32 = 8;
const WRITABLE_STREAM_STRATEGY_CLOSE_REQUESTED_INDEX: u32 = 9;
const WRITABLE_STREAM_STRATEGY_ERRORING_INDEX: u32 = 10;
const WRITABLE_STREAM_STRATEGY_PENDING_ABORT_INDEX: u32 = 11;
const WRITABLE_STREAM_STRATEGY_PENDING_CLOSE_INDEX: u32 = 12;
const WRITABLE_STREAM_ALGORITHM_SINK_WRITE_INDEX: u32 = 0;
const WRITABLE_STREAM_ALGORITHM_SINK_CLOSE_INDEX: u32 = 1;
const WRITABLE_STREAM_ALGORITHM_SINK_ABORT_INDEX: u32 = 2;
const WRITABLE_STREAM_ALGORITHM_TRANSFORM_INDEX: u32 = 3;
const WRITABLE_STREAM_ALGORITHM_TRANSFORM_FLUSH_INDEX: u32 = 4;
const WRITABLE_STREAM_ALGORITHM_TRANSFORM_CANCEL_INDEX: u32 = 5;
mod callbacks;
mod construction;
mod pipe;
mod pipe_owner;
mod queue_v8;
mod read_request;
mod readable;
mod readable_byte;
mod readable_state;
mod tee;
mod utils;
mod writable;

pub(in crate::context_bootstrap) use callbacks::{
    StreamQueuingStrategy, StreamWebIdlCallbackCarrier, WebIdlReadableStreamSource,
    WebIdlTransformStreamTransformer, WebIdlWritableStreamSink, callback_carrier_value,
    invoke_stored_stream_algorithm, invoke_stored_stream_promise_algorithm,
    invoke_stored_stream_size_algorithm, invoke_stream_webidl_callback,
    parse_readable_stream_source_object, parse_stream_strategy_arg,
    parse_transform_stream_transformer_object, parse_writable_stream_sink_object,
    stored_stream_algorithm_is_webidl,
};
pub(super) use construction::{
    initialize_transform_stream_object, initialize_webidl_readable_stream_object,
    initialize_webidl_transform_stream_object, initialize_webidl_writable_stream_object,
    initialize_writable_stream_object,
};
pub(crate) use pipe::readable_stream_has_pipe_owner;
pub(in crate::context_bootstrap) use pipe::{
    install_readable_stream_pipe_to_abort_signal, new_readable_stream_pipe_owner,
    prime_readable_stream_pipe_to, register_readable_stream_pipe_owner,
};
pub(in crate::context_bootstrap) use queue_v8::{
    dequeue_readable_stream_queue_value, enqueue_readable_stream_queue_value,
    prepend_readable_stream_queue_value, readable_stream_queue_error_value,
    readable_stream_queue_exists, readable_stream_queue_is_empty, readable_stream_queue_total_size,
    reset_readable_stream_queue, take_byte_stream_bytes,
};
pub(in crate::context_bootstrap::stream_adapter) use readable::mark_readable_stream_started;
pub(in crate::context_bootstrap::stream_adapter) use readable::perform_read_from_stream;
pub(super) use readable::{
    PreparedReadableStreamRead, prepare_read_from_stream_as_promise, read_from_stream_as_promise,
    readable_stream_closed_promise,
};
pub(crate) use readable::{
    cancel_readable_stream, maybe_pull_stream, prepare_readable_stream_read_with_steps,
};
pub(crate) use readable_byte::enqueue_byte_chunk;
pub(in crate::context_bootstrap) use readable_byte::{
    enqueue_auto_allocate_pull_into, finish_byte_stream_tee_branch_close,
    finish_readable_byte_stream_close, initialize_readable_byte_stream_state,
    perform_read_into_byte_stream, prepare_readable_byte_stream_close,
    read_into_byte_stream_as_promise, readable_byte_stream_byob_request,
    readable_byte_stream_pending_byob_view, readable_stream_byob_request_respond_callback,
    readable_stream_byob_request_respond_with_new_view_callback,
    readable_stream_byob_request_view_getter, release_byte_stream_reader,
    reset_byte_stream_pending_pull_intos, respond_byte_stream_with_new_view,
};
pub(in crate::context_bootstrap) use readable_state::EnqueueChunkError;
pub(in crate::context_bootstrap) use readable_state::{
    apply_readable_stream_access_transition, disturb_readable_stream,
    finish_readable_stream_close_if_requested_and_queue_empty, lock_readable_stream,
    readable_stream_access_snapshot, readable_stream_is_byte_stream, unlock_readable_stream,
};
pub(crate) use readable_state::{
    begin_readable_stream_body_consumption, readable_stream_disturbed, readable_stream_locked,
};
pub(crate) use readable_state::{close_stream, enqueue_chunk, error_stream};
pub(super) use readable_state::{
    readable_stream_closed, readable_stream_error, reject_pending_read_requests,
    remove_pending_closed_promise, writable_stream_locked,
};
pub(super) use utils::{
    done_result, promise_then_undefined, reject_pending_read, rejected_promise_value,
    resolved_promise_value, set_resolved_promise, suppress_pending_read_unhandled_rejection,
    suppress_promise_unhandled_rejection, value_buffer_source_bytes,
};
pub(in crate::context_bootstrap) use writable::register_writable_stream_pipe_owner;
pub(in crate::context_bootstrap::stream_adapter) use writable::transform_stream_readable_cancel_callback;
pub(in crate::context_bootstrap::stream_adapter) use writable::transform_stream_readable_pull_callback;
pub(super) use writable::{
    acquire_writable_stream_writer, begin_writable_stream_start, error_transform_stream_with_value,
    error_writable_stream_with_value, release_writable_stream_writer,
    set_transform_stream_start_result, set_writable_stream_start_result,
    terminate_transform_stream, writable_stream_abort_internal, writable_stream_abort_promise,
    writable_stream_close_internal, writable_stream_has_capacity, writable_stream_snapshot,
    writable_stream_stored_error, writable_stream_write_internal,
    writable_stream_writer_closed_promise_value, writable_stream_writer_ready_promise_value,
    writable_stream_writer_write_internal,
};

pub(in crate::context_bootstrap) use construction::{
    initialize_readable_stream_object, initialize_transform_stream_endpoints,
    new_lazy_readable_byte_stream_object, new_lazy_readable_stream_object,
    new_readable_byte_stream_object, new_readable_stream_object, new_readable_stream_shell_object,
    new_transform_stream_shell_object, new_writable_stream_object,
    new_writable_stream_shell_object,
};
pub(in crate::context_bootstrap::stream_adapter) use read_request::{
    error_read_request, fulfill_read_request, new_internal_read_request,
};
pub(in crate::context_bootstrap::stream_adapter) use readable_state::enqueue_pending_closed_promise;
pub(in crate::context_bootstrap::stream_adapter) use readable_state::{
    enqueue_pending_read, finish_readable_stream_close,
    readable_stream_controller_algorithm_object, readable_stream_controller_algorithm_value,
    readable_stream_pull_state_has, readable_stream_snapshot, set_readable_stream_pull_state_bit,
};
pub(in crate::context_bootstrap) use tee::{
    TeeStartError, tee_readable_stream, tee_readable_stream_with_cloned_branch,
};
pub(crate) use utils::require_internal_stream_value;
pub(in crate::context_bootstrap) use utils::{
    StreamOwnerPublication, build_required_stream_callback, call_function_result,
    call_named_method, call_named_method_result, iter_result,
    publish_required_stream_promise_reactions,
};
pub(in crate::context_bootstrap) use utils::{new_pending_read_promise, resolve_pending_promise};
pub(in crate::context_bootstrap::stream_adapter) use utils::{
    reject_pending_read_after_timeout, resolve_callable_property_result,
};

pub(in crate::context_bootstrap) fn stream_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, stream, slot)
}

pub(in crate::context_bootstrap) fn stream_slot_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    stream_slot_value(scope, stream, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn stream_slot_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Array>> {
    stream_slot_value(scope, stream, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn stream_slot_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<bool> {
    get_private_value(scope, stream, slot).map(|value| value.boolean_value(scope))
}

pub(in crate::context_bootstrap) fn stream_slot_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    get_private_value(scope, stream, slot).and_then(|value| value.number_value(scope))
}

pub(in crate::context_bootstrap) fn stream_slot_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    let value = get_private_value(scope, stream, slot)?;
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(in crate::context_bootstrap) fn set_stream_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, stream, slot, value);
}

/// Publish a private slot which is required to keep an internal Streams owner
/// reachable or its state machine coherent. Unlike the general-purpose slot
/// helper, this boundary must not turn a V8 publication failure into a
/// partially registered operation.
pub(in crate::context_bootstrap) fn set_required_stream_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: v8::Local<'s, v8::Value>,
    role: &'static str,
) {
    let key = crate::util::private_key(scope, slot)
        .unwrap_or_else(|| panic!("required Streams private key for `{role}` must publish"));
    let published = object
        .set_private(scope, key, value)
        .unwrap_or_else(|| panic!("required Streams private slot for `{role}` must publish"));
    assert!(
        published,
        "required Streams private slot for `{role}` must be accepted"
    );
}

pub(in crate::context_bootstrap) fn set_stream_slot_bool(
    scope: &mut v8::PinScope<'_, '_>,
    stream: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: bool,
) {
    let stored = v8::Boolean::new(scope, value);
    set_private_value(scope, stream, slot, stored.into());
}

pub(in crate::context_bootstrap) fn set_writable_stream_locked(
    scope: &mut v8::PinScope<'_, '_>,
    stream: v8::Local<'_, v8::Object>,
    locked: bool,
) {
    set_stream_slot_bool(scope, stream, WRITABLE_STREAM_LOCKED_SLOT, locked);
}
