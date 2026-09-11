use super::*;

#[derive(Debug)]
pub(crate) enum EnqueueChunkError<'s> {
    ClosedOrErrored,
    Strategy(v8::Local<'s, v8::Value>),
}

pub(crate) fn enqueue_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), EnqueueChunkError<'s>> {
    if readable_stream_snapshot(scope, stream).plan_enqueue()
        == moli_streams::readable::EnqueuePlan::Reject
    {
        return Err(EnqueueChunkError::ClosedOrErrored);
    }
    let plan = super::pipe::pipe_owner_state_for_source(scope, stream).map_or(
        moli_streams::pipe::PipeIncomingChunkPlan::NotPiped,
        moli_streams::pipe::PipeOwnerState::plan_incoming_chunk,
    );
    match plan {
        moli_streams::pipe::PipeIncomingChunkPlan::NotPiped => {}
        moli_streams::pipe::PipeIncomingChunkPlan::EnqueueAndSchedule { size } => {
            // A pipe pull admitted before destination backpressure changed owns an
            // internal read request. Its chunk fulfills that request directly and
            // must not transiently reduce the source controller's desiredSize.
            // Chunks from any unrelated/in-flight source activity still use the
            // source's normal queuing strategy.
            let fulfills_pending_read = size.fulfills_pending_read();
            let queue_was_empty = readable_stream_queue_is_empty(scope, stream);
            let size = match size {
                moli_streams::pipe::PipeChunkSize::Zero => 0.0,
                moli_streams::pipe::PipeChunkSize::Strategy => {
                    readable_stream_enqueue_size(scope, stream, value)?
                }
            };
            enqueue_readable_stream_queue_value(scope, stream, value, size)
                .map_err(|_| EnqueueChunkError::ClosedOrErrored)?;
            super::pipe::schedule_pipe_drain_after_incoming_chunk(
                scope,
                stream,
                fulfills_pending_read,
                queue_was_empty,
            );
            return Ok(());
        }
    }
    if resolve_next_pending_read(scope, stream, value) {
        return Ok(());
    }
    let size = readable_stream_enqueue_size(scope, stream, value)?;
    enqueue_readable_stream_queue_value(scope, stream, value, size)
        .map_err(|_| EnqueueChunkError::ClosedOrErrored)?;
    Ok(())
}

pub(in crate::context_bootstrap) fn readable_stream_is_byte_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_bool(scope, stream, READABLE_STREAM_BYTE_STREAM_SLOT).unwrap_or(false)
}

fn readable_stream_enqueue_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Result<f64, EnqueueChunkError<'s>> {
    let size = match readable_stream_chunk_size(scope, stream, value) {
        Ok(size) => size,
        Err(error) => {
            let stream_error = readable_stream_error(scope, stream).unwrap_or(error);
            error_stream(scope, stream, stream_error);
            return Err(EnqueueChunkError::Strategy(error));
        }
    };
    let size = match size.to_number(scope) {
        Some(size) => size.value(),
        None => {
            let fallback_error = v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "The return value of a queuing strategy's size function could not be converted to a number",
                ),
            );
            let stream_error = readable_stream_error(scope, stream).unwrap_or(fallback_error);
            error_stream(scope, stream, stream_error);
            return Err(EnqueueChunkError::Strategy(fallback_error));
        }
    };
    if moli_streams::numeric::validate_queue_size(size).is_err() {
        let fallback_error = v8::Exception::range_error(
            scope,
            v8str(
                scope,
                "The return value of a queuing strategy's size function must be a finite, non-NaN, non-negative number",
            ),
        );
        let stream_error = readable_stream_error(scope, stream).unwrap_or(fallback_error);
        error_stream(scope, stream, stream_error);
        return Err(EnqueueChunkError::Strategy(fallback_error));
    };
    Ok(size)
}

pub(crate) fn close_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    if readable_stream_snapshot(scope, stream).plan_request_close()
        == moli_streams::readable::CloseRequestPlan::Ignore
    {
        return false;
    }
    set_readable_stream_close_requested(scope, stream, true);
    finish_readable_stream_close_if_requested_and_queue_empty(scope, stream);
    true
}

pub(in crate::context_bootstrap) fn finish_readable_stream_close_if_requested_and_queue_empty<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    if readable_stream_snapshot(scope, stream).plan_finish_close()
        != moli_streams::readable::FinishClosePlan::Finish
    {
        return;
    }
    if readable_stream_is_byte_stream(scope, stream)
        && let Err(error) = prepare_readable_byte_stream_close(scope, stream)
    {
        error_stream(scope, stream, error);
        return;
    }
    finish_readable_stream_close(scope, stream);
}

pub(in crate::context_bootstrap::stream_adapter) fn finish_readable_stream_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    if !readable_stream_snapshot(scope, stream)
        .state()
        .is_readable()
    {
        return;
    }
    set_readable_stream_close_requested(scope, stream, false);
    set_stream_slot_bool(scope, stream, READABLE_STREAM_CLOSED_SLOT, true);
    // A tee observes the source's actual closed transition, not merely a
    // close request: queued chunks remain readable until this point. If an
    // internal tee read owns the transition, its reaction performs the final
    // branch close after distributing the in-flight chunk.
    super::tee::close_teed_readable_stream_branches(scope, stream);
    resolve_readable_stream_closed_promises(scope, stream);
    super::pipe::schedule_pipe_to_drain(scope, stream);
    if readable_stream_is_byte_stream(scope, stream)
        && finish_readable_byte_stream_close(scope, stream)
    {
        return;
    }
    let Some(pending) = pending_read_resolvers_array(scope, stream) else {
        return;
    };
    for index in 0..pending.length() {
        let Some(entry) = pending
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        fulfill_read_request(scope, entry, v8::undefined(scope).into(), true);
    }
    set_stream_slot_value(
        scope,
        stream,
        READABLE_STREAM_PENDING_READS_SLOT,
        v8::Array::new(scope, 0).into(),
    );
}

pub(crate) fn error_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    if readable_stream_snapshot(scope, stream).plan_error()
        == moli_streams::readable::ErrorPlan::Ignore
    {
        return;
    }
    if readable_stream_is_byte_stream(scope, stream) {
        reset_byte_stream_pending_pull_intos(scope, stream);
    }
    let error_entry = v8::Array::new(scope, 1);
    let _ = error_entry.set_index(scope, 0, reason);
    set_stream_slot_value(
        scope,
        stream,
        READABLE_STREAM_ERROR_SLOT,
        error_entry.into(),
    );
    set_stream_slot_bool(scope, stream, READABLE_STREAM_CLOSED_SLOT, true);
    reset_readable_stream_queue(scope, stream);
    // Publish the source terminal state before forwarding it to composite
    // algorithms. Rejecting branch promises can enqueue reactions which must
    // never observe this source as still readable and schedule another pull.
    super::tee::error_teed_readable_stream_branches(scope, stream, reason);
    reject_readable_stream_closed_promises(scope, stream, reason);
    if let Some(pending) = pending_read_resolvers_array(scope, stream) {
        for index in 0..pending.length() {
            let Some(entry) = pending
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            else {
                continue;
            };
            if readable_stream_child_realm_rejections_are_handled(scope, stream)
                && reject_pending_read_after_timeout(scope, entry, reason)
            {
                suppress_pending_read_unhandled_rejection(scope, entry);
                continue;
            }
            error_read_request(scope, entry, reason);
        }
        set_stream_slot_value(
            scope,
            stream,
            READABLE_STREAM_PENDING_READS_SLOT,
            v8::Array::new(scope, 0).into(),
        );
    }
    // Publish the readable terminal state before PipeTo starts its shutdown
    // action.  Abort/cancel callbacks can re-enter, and must observe the
    // source as already errored while the PipeOwner owns the one settlement.
    super::pipe::source_pipe_errored(scope, stream, reason);
}

fn readable_stream_child_realm_rejections_are_handled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_bool(
        scope,
        stream,
        READABLE_STREAM_CHILD_REALM_HANDLED_REJECTION_SLOT,
    )
    .unwrap_or(false)
}

pub(in crate::context_bootstrap) fn readable_stream_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_bool(scope, stream, READABLE_STREAM_CLOSED_SLOT).unwrap_or(false)
}

pub(in crate::context_bootstrap) fn readable_stream_close_requested<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    readable_stream_pull_state_has(scope, stream, READABLE_STREAM_PULL_STATE_CLOSE_REQUESTED)
}

fn set_readable_stream_close_requested<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    requested: bool,
) {
    set_readable_stream_pull_state_bit(
        scope,
        stream,
        READABLE_STREAM_PULL_STATE_CLOSE_REQUESTED,
        requested,
    );
}

pub(in crate::context_bootstrap::stream_adapter) fn readable_stream_pull_state_has<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    bit: u32,
) -> bool {
    readable_stream_pull_state(scope, stream) & bit != 0
}

pub(in crate::context_bootstrap::stream_adapter) fn set_readable_stream_pull_state_bit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    bit: u32,
    enabled: bool,
) {
    let mut state = readable_stream_pull_state(scope, stream);
    if enabled {
        state |= bit;
    } else {
        state &= !bit;
    }
    set_stream_slot_value(
        scope,
        stream,
        READABLE_STREAM_PULL_STATE_SLOT,
        v8::Integer::new_from_unsigned(scope, state).into(),
    );
}

fn readable_stream_pull_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> u32 {
    stream_slot_number(scope, stream, READABLE_STREAM_PULL_STATE_SLOT)
        .map(|value| value as u32)
        .unwrap_or(READABLE_STREAM_PULL_STATE_STARTED)
}

pub(in crate::context_bootstrap) fn readable_stream_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let value = stream_slot_value(scope, stream, READABLE_STREAM_ERROR_SLOT)?;
    if let Ok(entry) = v8::Local::<v8::Array>::try_from(value) {
        return entry
            .get_index(scope, 0)
            .or_else(|| Some(v8::undefined(scope).into()));
    }
    Some(value).filter(|value| !value.is_null_or_undefined())
}

pub(in crate::context_bootstrap::stream_adapter) fn readable_stream_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> moli_streams::readable::ReadableSnapshot {
    let state = moli_streams::readable::ReadableState::from_storage(
        readable_stream_closed(scope, stream),
        readable_stream_error(scope, stream).is_some(),
    );
    let pending_read_count = stream_slot_array(scope, stream, READABLE_STREAM_PENDING_READS_SLOT)
        .map(|pending| pending.length() as usize)
        .unwrap_or(0);
    moli_streams::readable::ReadableSnapshot::new(
        state,
        readable_stream_close_requested(scope, stream),
        readable_stream_queue_is_empty(scope, stream),
        pending_read_count,
    )
}

pub(in crate::context_bootstrap::stream_adapter) fn enqueue_pending_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    let Some(pending) = pending_read_resolvers_array(scope, stream) else {
        return;
    };
    let _ = pending.set_index(scope, pending.length(), entry.into());
}

pub(in crate::context_bootstrap) fn reject_pending_read_requests<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(pending) = pending_read_resolvers_array(scope, stream) else {
        return;
    };
    for index in 0..pending.length() {
        let Some(entry) = pending
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        error_read_request(scope, entry, reason);
    }
    set_stream_slot_value(
        scope,
        stream,
        READABLE_STREAM_PENDING_READS_SLOT,
        v8::Array::new(scope, 0).into(),
    );
}

pub(in crate::context_bootstrap::stream_adapter) fn enqueue_pending_closed_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    let Some(pending) = pending_closed_resolvers_array(scope, stream) else {
        return;
    };
    let _ = pending.set_index(scope, pending.length(), entry.into());
}

pub(in crate::context_bootstrap) fn remove_pending_closed_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    let Some(pending) = pending_closed_resolvers_array(scope, stream) else {
        return;
    };
    let next = v8::Array::new(scope, 0);
    for index in 0..pending.length() {
        let Some(candidate) = pending.get_index(scope, index) else {
            continue;
        };
        if candidate.strict_equals(entry.into()) {
            continue;
        }
        let _ = next.set_index(scope, next.length(), candidate);
    }
    set_stream_slot_value(
        scope,
        stream,
        READABLE_STREAM_PENDING_CLOSED_PROMISES_SLOT,
        next.into(),
    );
}

fn resolve_next_pending_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    let Some(entry) = dequeue_first_pending_read(scope, stream) else {
        return false;
    };
    fulfill_read_request(scope, entry, value, false);
    true
}

pub(in crate::context_bootstrap::stream_adapter) fn dequeue_first_pending_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let pending = pending_read_resolvers_array(scope, stream)?;
    if pending.length() == 0 {
        return None;
    }
    let entry = pending
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let next = v8::Array::new(scope, 0);
    for index in 1..pending.length() {
        if let Some(entry) = pending.get_index(scope, index) {
            let _ = next.set_index(scope, next.length(), entry);
        }
    }
    set_stream_slot_value(
        scope,
        stream,
        READABLE_STREAM_PENDING_READS_SLOT,
        next.into(),
    );
    Some(entry)
}

fn readable_stream_chunk_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, v8::Local<'s, v8::Value>> {
    let Some(size_algorithm) = readable_stream_controller_algorithm_value(
        scope,
        stream,
        READABLE_STREAM_ALGORITHM_SIZE_INDEX,
    ) else {
        return Ok(v8::Number::new(scope, 1.0).into());
    };
    if size_algorithm.is_undefined() {
        return Ok(v8::Number::new(scope, 1.0).into());
    }
    match invoke_stored_stream_size_algorithm(scope, size_algorithm, chunk) {
        Ok(Some(result)) => Ok(result),
        Ok(None) => Ok(v8::undefined(scope).into()),
        Err(error) => Err(error),
    }
}

pub(in crate::context_bootstrap::stream_adapter) fn readable_stream_controller_algorithm_value<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    algorithm_index: u32,
) -> Option<v8::Local<'s, v8::Value>> {
    let controller = stream_slot_object(scope, stream, READABLE_STREAM_CONTROLLER_SLOT)?;
    let algorithms = stream_slot_array(scope, controller, STREAM_CONTROLLER_ALGORITHMS_SLOT)?;
    algorithms.get_index(scope, algorithm_index)
}

pub(in crate::context_bootstrap::stream_adapter) fn readable_stream_controller_algorithm_object<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    algorithm_index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    readable_stream_controller_algorithm_value(scope, stream, algorithm_index)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn pending_read_resolvers_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    stream_slot_array(scope, stream, READABLE_STREAM_PENDING_READS_SLOT)
}

fn pending_closed_resolvers_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    stream_slot_array(scope, stream, READABLE_STREAM_PENDING_CLOSED_PROMISES_SLOT)
}

fn resolve_readable_stream_closed_promises<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let Some(pending) = pending_closed_resolvers_array(scope, stream) else {
        return;
    };
    for index in 0..pending.length() {
        let Some(entry) = pending
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        resolve_pending_promise(scope, entry, v8::undefined(scope).into());
    }
    set_stream_slot_value(
        scope,
        stream,
        READABLE_STREAM_PENDING_CLOSED_PROMISES_SLOT,
        v8::Array::new(scope, 0).into(),
    );
}

fn reject_readable_stream_closed_promises<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(pending) = pending_closed_resolvers_array(scope, stream) else {
        return;
    };
    for index in 0..pending.length() {
        let Some(entry) = pending
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        reject_pending_read(scope, entry, reason);
    }
    set_stream_slot_value(
        scope,
        stream,
        READABLE_STREAM_PENDING_CLOSED_PROMISES_SLOT,
        v8::Array::new(scope, 0).into(),
    );
}

pub(crate) fn readable_stream_locked<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    readable_stream_access_snapshot(scope, stream).locked()
}

pub(crate) fn readable_stream_disturbed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    readable_stream_access_snapshot(scope, stream).disturbed()
}

// Fetch can consume buffered/native body storage without running a JS reader.
// Commit the same access state as acquiring a reader and starting its read.
pub(crate) fn begin_readable_stream_body_consumption<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    if !lock_readable_stream(scope, stream) {
        return false;
    }
    disturb_readable_stream(scope, stream);
    true
}

pub(in crate::context_bootstrap) fn readable_stream_access_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> moli_streams::readable::ReadableAccessSnapshot {
    moli_streams::readable::ReadableAccessSnapshot::new(
        stream_slot_bool(scope, stream, READABLE_STREAM_LOCKED_SLOT).unwrap_or(false),
        stream_slot_bool(scope, stream, READABLE_STREAM_DISTURBED_SLOT).unwrap_or(false),
    )
}

/// Apply an immediate core access plan. Callers may perform internal V8
/// allocation first, but must not run author JavaScript between decoding the
/// transition source and this storage commit.
pub(in crate::context_bootstrap) fn apply_readable_stream_access_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    transition: moli_streams::readable::ReadableAccessTransition,
) {
    let source = transition.source();
    let next = transition.next();
    if source.locked() != next.locked() {
        set_stream_slot_bool(scope, stream, READABLE_STREAM_LOCKED_SLOT, next.locked());
    }
    if source.disturbed() != next.disturbed() {
        set_stream_slot_bool(
            scope,
            stream,
            READABLE_STREAM_DISTURBED_SLOT,
            next.disturbed(),
        );
    }
}

pub(in crate::context_bootstrap) fn lock_readable_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    match readable_stream_access_snapshot(scope, stream).plan_lock() {
        moli_streams::readable::ReadableLockPlan::Lock(transition) => {
            apply_readable_stream_access_transition(scope, stream, transition);
            true
        }
        moli_streams::readable::ReadableLockPlan::RejectLocked => false,
    }
}

pub(in crate::context_bootstrap) fn unlock_readable_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let transition = readable_stream_access_snapshot(scope, stream).plan_unlock();
    apply_readable_stream_access_transition(scope, stream, transition);
}

pub(in crate::context_bootstrap) fn disturb_readable_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let transition = readable_stream_access_snapshot(scope, stream).plan_disturb();
    apply_readable_stream_access_transition(scope, stream, transition);
}

pub(in crate::context_bootstrap) fn writable_stream_locked<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    moli_streams::writable::WritableAccessSnapshot::new(
        stream_slot_bool(scope, stream, WRITABLE_STREAM_LOCKED_SLOT).unwrap_or(false),
    )
    .locked()
}
