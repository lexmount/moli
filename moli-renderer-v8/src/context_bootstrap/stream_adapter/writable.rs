use super::*;
use crate::text_codec::{TextCodecStore, TextDecodeError};
use crate::util::get_private_value;
use moli_streams::queue::{QueueBounds, QueueRemainderPlan};
use moli_streams::strategy::StrategySnapshot;
use moli_streams::transform::{
    AlgorithmOutcome, EnqueueErrorSource, ErrorReasonSource, FinishAlgorithm, FinishClaimPlan,
    FinishOperation, FinishResidenceState, FinishSettlementPlan, FinishSetupFailurePlan,
    ReadableErrorAction, ReadableTerminateAction, StartSettlementPlan, TransformCancelAlgorithm,
    TransformCloseAdmissionPlan, TransformEnqueueFailure, TransformFlushAlgorithm, TransformMode,
    TransformReadableSnapshot, TransformSnapshot, TransformWriteAdmissionPlan,
    TransformWriteAlgorithm, WritableCloseSettlementPlan, WriteSettlementPlan,
};
use moli_streams::writable::default_controller::{
    ContinuationPlan, PendingWriteKind, PumpState, ReadyTransition, RejectEntryPlan, SinkPumpPlan,
    TransformPumpPlan, WritableControllerSnapshot, WriteCompletion,
};
use moli_streams::writable::writer::{
    EnsurePendingPlan, EnsureRejectedPlan, InitialPromiseState, PromiseResidenceState,
    ResolvePromisePlan, plan_ensure_pending, plan_ensure_rejected, plan_resolve,
    plan_writer_promise_initialization,
};
use moli_streams::writable::{
    AbortPlan, CloseOutcome, ClosePlan, CloseSettlementPlan, CloseWithErrorPropagationPlan,
    DesiredSizePlan, ErrorPlan, FinishErroringPlan, InternalWriteEntryPlan, PendingAbortState,
    WritableKind, WritableSnapshot, WritableState, WriteAfterSizePlan, WriteRoutePlan,
};

const TRANSFORM_PENDING_WRITE_CHUNK_INDEX: u32 = 0;
const TRANSFORM_PENDING_WRITE_PROMISE_INDEX: u32 = 1;
const TRANSFORM_PENDING_WRITE_KIND_INDEX: u32 = 2;
const TRANSFORM_PENDING_WRITE_SIZE_INDEX: u32 = 3;
const TRANSFORM_PENDING_WRITE_CLOSE_KIND: &str = "close";
const TRANSFORM_PENDING_WRITE_RUNNING_KIND: &str = "running";
const TRANSFORM_PENDING_WRITE_SINK_KIND: &str = "sink";
const TRANSFORM_PENDING_WRITE_SINK_RUNNING_KIND: &str = "sink-running";
const TRANSFORM_PENDING_WRITE_SINK_CLOSE_RUNNING_KIND: &str = "sink-close-running";
const WRITABLE_QUEUE_CONTINUATION_STREAM_INDEX: u32 = 0;
const WRITABLE_PENDING_ABORT_PROMISE_INDEX: u32 = 0;
const WRITABLE_PENDING_ABORT_RESIDENCE_INDEX: u32 = 1;
const WRITABLE_PENDING_ABORT_REASON_INDEX: u32 = 2;
const WRITABLE_PENDING_ABORT_ALREADY_ERRORING_INDEX: u32 = 3;

fn writable_stream_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> WritableKind {
    if stream_slot_object(scope, stream, WRITABLE_STREAM_TARGET_READABLE_SLOT)
        .is_some_and(|readable| !readable.is_null_or_undefined())
    {
        WritableKind::Transform
    } else if stream_slot_object(scope, stream, WRITABLE_STREAM_SINK_SLOT)
        .is_some_and(|sink| !sink.is_null_or_undefined())
    {
        WritableKind::Sink
    } else {
        panic!("an initialized writable stream must retain a sink or transform readable")
    }
}

pub(in crate::context_bootstrap) fn writable_stream_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> WritableSnapshot {
    let queue_operation_in_flight = peek_pending_transform_write(scope, stream)
        .is_some_and(|entry| pending_write_kind(scope, entry).is_running());
    let state = WritableState::from_storage(
        stream_slot_bool(scope, stream, WRITABLE_STREAM_CLOSED_SLOT).unwrap_or(false),
        writable_stream_erroring(scope, stream),
        writable_stream_errored(scope, stream),
    );
    let strategy = StrategySnapshot::new(
        writable_stream_strategy_number(
            scope,
            stream,
            WRITABLE_STREAM_STRATEGY_HIGH_WATER_MARK_INDEX,
        )
        .unwrap_or(1.0),
        writable_stream_strategy_number(scope, stream, WRITABLE_STREAM_STRATEGY_TOTAL_SIZE_INDEX)
            .unwrap_or(0.0),
    );
    WritableSnapshot::new(
        state,
        writable_stream_kind(scope, stream),
        writable_stream_locked(scope, stream),
        writable_stream_close_requested(scope, stream),
        transform_stream_start_pending(scope, stream),
        pending_transform_write_count(scope, stream) as usize,
        transform_close_in_flight(scope, stream),
        queue_operation_in_flight,
        writable_stream_pending_abort_state(scope, stream),
        strategy,
    )
}

fn pending_write_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Array>,
) -> PendingWriteKind {
    match pending_transform_write_kind(scope, entry).as_deref() {
        Some(TRANSFORM_PENDING_WRITE_CLOSE_KIND) => PendingWriteKind::Close,
        Some(TRANSFORM_PENDING_WRITE_RUNNING_KIND) => PendingWriteKind::TransformRunning,
        Some(TRANSFORM_PENDING_WRITE_SINK_KIND) => PendingWriteKind::Sink,
        Some(TRANSFORM_PENDING_WRITE_SINK_RUNNING_KIND) => PendingWriteKind::SinkRunning,
        Some(TRANSFORM_PENDING_WRITE_SINK_CLOSE_RUNNING_KIND) => PendingWriteKind::SinkCloseRunning,
        _ => PendingWriteKind::Transform,
    }
}

fn pending_write_kind_name(kind: PendingWriteKind) -> Option<&'static str> {
    match kind {
        PendingWriteKind::Transform => None,
        PendingWriteKind::TransformRunning => Some(TRANSFORM_PENDING_WRITE_RUNNING_KIND),
        PendingWriteKind::Close => Some(TRANSFORM_PENDING_WRITE_CLOSE_KIND),
        PendingWriteKind::Sink => Some(TRANSFORM_PENDING_WRITE_SINK_KIND),
        PendingWriteKind::SinkRunning => Some(TRANSFORM_PENDING_WRITE_SINK_RUNNING_KIND),
        PendingWriteKind::SinkCloseRunning => Some(TRANSFORM_PENDING_WRITE_SINK_CLOSE_RUNNING_KIND),
    }
}

fn writable_controller_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> WritableControllerSnapshot {
    let queue = pending_transform_write_queue(scope, stream);
    let head = pending_transform_write_head(scope, stream) as usize;
    let storage_len = queue.map_or(0, |queue| queue.length() as usize);
    let bounds = QueueBounds::new(head, storage_len)
        .unwrap_or_else(|_| QueueBounds::new(storage_len, storage_len).unwrap());
    let head_kind = queue.and_then(|queue| {
        queue
            .get_index(scope, bounds.head() as u32)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            .map(|entry| pending_write_kind(scope, entry))
    });
    WritableControllerSnapshot::new(
        StrategySnapshot::new(
            writable_stream_strategy_number(
                scope,
                stream,
                WRITABLE_STREAM_STRATEGY_HIGH_WATER_MARK_INDEX,
            )
            .unwrap_or(1.0),
            writable_stream_strategy_number(
                scope,
                stream,
                WRITABLE_STREAM_STRATEGY_TOTAL_SIZE_INDEX,
            )
            .unwrap_or(0.0),
        ),
        bounds,
        head_kind,
        transform_stream_start_pending(scope, stream),
        writable_stream_snapshot(scope, stream).state(),
        writable_stream_close_requested(scope, stream),
        PumpState::from_stored(writable_queue_pump_state(scope, stream)),
    )
}

fn transform_readable_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    readable: v8::Local<'s, v8::Object>,
) -> TransformReadableSnapshot {
    let lifecycle = readable_stream_snapshot(scope, readable);
    TransformReadableSnapshot::new(
        lifecycle.state(),
        lifecycle.pending_read_count(),
        super::pipe::pipe_owner_state_for_source(scope, readable).is_some(),
        StrategySnapshot::new(
            stream_slot_number(scope, readable, READABLE_STREAM_HWM_SLOT).unwrap_or(1.0),
            readable_stream_queue_total_size(scope, readable),
        ),
    )
}

fn transform_stream_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
) -> TransformSnapshot {
    let writable_lifecycle = writable_stream_snapshot(scope, writable);
    let has_transformer = stream_slot_object(scope, writable, WRITABLE_STREAM_TRANSFORMER_SLOT)
        .is_some_and(|transformer| !transformer.is_null_or_undefined());
    let mode = stream_slot_string(scope, writable, WRITABLE_STREAM_MODE_SLOT);
    let has_finish_promise = stream_slot_object(scope, writable, WRITABLE_STREAM_CONTROLLER_SLOT)
        .and_then(|controller| {
            stream_slot_value(
                scope,
                controller,
                TRANSFORM_STREAM_CONTROLLER_FINISH_PROMISE_SLOT,
            )
        })
        .is_some_and(|value| value.is_promise());
    TransformSnapshot::new(
        transform_readable_snapshot(scope, readable),
        writable_lifecycle.state(),
        TransformMode::from_storage(mode.as_deref(), has_transformer),
        writable_lifecycle.start_pending(),
        writable_lifecycle.pending_write_count(),
        FinishResidenceState::from_storage(has_finish_promise),
    )
}

fn invoke_writable_stream_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    underlying_object: v8::Local<'s, v8::Object>,
    algorithm_index: u32,
    browser_method_name: &'static str,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>> {
    if let Some(algorithms) = stream_slot_array(scope, stream, WRITABLE_STREAM_ALGORITHMS_SLOT) {
        let algorithm = algorithms
            .get_index(scope, algorithm_index)
            .unwrap_or_else(|| v8::undefined(scope).into());
        return invoke_stored_stream_promise_algorithm(
            scope,
            algorithm,
            underlying_object.into(),
            arguments,
        );
    }
    call_named_method_result(scope, underlying_object, browser_method_name, arguments)
}

fn writable_stream_has_webidl_algorithms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_array(scope, stream, WRITABLE_STREAM_ALGORITHMS_SLOT).is_some()
}

pub(in crate::context_bootstrap) fn writable_stream_write_internal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    writable_stream_write_with_owner(scope, stream, chunk, None, WriteInvocation::Direct)
}

pub(in crate::context_bootstrap::stream_adapter) fn writable_stream_write_from_pipe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    writable_stream_write_with_owner(scope, stream, chunk, None, WriteInvocation::Pipe)
}

pub(in crate::context_bootstrap) fn writable_stream_writer_write_internal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
    stream: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    writable_stream_write_with_owner(scope, stream, chunk, Some(writer), WriteInvocation::Direct)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriteInvocation {
    Direct,
    Pipe,
}

fn writable_stream_write_with_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
    writer: Option<v8::Local<'s, v8::Object>>,
    invocation: WriteInvocation,
) -> Option<v8::Local<'s, v8::Value>> {
    if writable_stream_snapshot(scope, stream).plan_internal_write_entry()
        == InternalWriteEntryPlan::RejectStoredError
    {
        return writable_stream_stored_error(scope, stream)
            .and_then(|error| rejected_promise_value(scope, error));
    }
    let write_size = match writable_stream_write_size(scope, stream, chunk) {
        Ok(size) => size,
        Err(error) => {
            if let Some(readable) =
                stream_slot_object(scope, stream, WRITABLE_STREAM_TARGET_READABLE_SLOT)
                && !readable.is_null_or_undefined()
            {
                error_transform_stream_with_value(scope, stream, readable, error);
            } else {
                error_writable_stream_with_value(scope, stream, error);
            }
            return rejected_promise_value(scope, error);
        }
    };

    let owner_is_current = writer.is_none_or(|writer| {
        stream_slot_object(scope, writer, WRITABLE_STREAM_WRITER_STREAM_SLOT)
            .is_some_and(|current| current.strict_equals(stream.into()))
    });
    match writable_stream_snapshot(scope, stream).plan_write_after_size(owner_is_current) {
        WriteAfterSizePlan::Continue => {}
        WriteAfterSizePlan::RejectReleasedWriter => {
            let error = v8::Exception::type_error(
                scope,
                v8str(scope, "WritableStreamDefaultWriter lock released"),
            );
            return rejected_promise_value(scope, error);
        }
        WriteAfterSizePlan::RejectStoredError => {
            return writable_stream_stored_error(scope, stream)
                .and_then(|error| rejected_promise_value(scope, error));
        }
        WriteAfterSizePlan::RejectClosingOrClosed => {
            let error = v8::Exception::type_error(
                scope,
                v8str(scope, "Cannot write to a closing or closed WritableStream"),
            );
            return rejected_promise_value(scope, error);
        }
    }
    record_writable_stream_write_size(scope, stream, write_size);

    let readable = stream_slot_object(scope, stream, WRITABLE_STREAM_TARGET_READABLE_SLOT)
        .filter(|readable| !readable.is_null_or_undefined());
    let sink = stream_slot_object(scope, stream, WRITABLE_STREAM_SINK_SLOT)
        .filter(|sink| !sink.is_null_or_undefined());
    let writable_snapshot = writable_stream_snapshot(scope, stream);
    match writable_snapshot.plan_write_route() {
        WriteRoutePlan::Transform => {
            let readable = readable.expect("a transform writable must retain its readable side");
            if matches!(invocation, WriteInvocation::Pipe) {
                // PipeTo's read-request chunk steps already run as a
                // microtask. Queue the Transform sink write from those steps,
                // then let the Transform-owned continuation advance it. This
                // preserves the source pull-settlement ordering without
                // teaching the generic readable pipe about destination kinds.
                let (promise, pending) = new_pending_read_promise(scope)?;
                enqueue_pending_transform_write(scope, stream, chunk, pending, write_size);
                schedule_transform_writable_queue_continuation(scope, stream);
                return Some(promise.into());
            }
            match transform_stream_snapshot(scope, stream, readable).plan_write_admission() {
                TransformWriteAdmissionPlan::Queue => {
                    let (promise, pending) = new_pending_read_promise(scope)?;
                    enqueue_pending_transform_write(scope, stream, chunk, pending, write_size);
                    process_transform_writable_queue(scope, stream);
                    Some(promise.into())
                }
                TransformWriteAdmissionPlan::Run(_) => {
                    let result = perform_transform_stream_write(
                        scope,
                        stream,
                        readable,
                        chunk,
                        Some(write_size),
                    );
                    if result.is_none() {
                        finish_writable_stream_write(
                            scope,
                            stream,
                            write_size,
                            WriteCompletion::Fulfilled,
                        );
                    }
                    result
                }
            }
        }
        WriteRoutePlan::QueueSink => {
            let sink = sink?;
            let Some((promise, pending)) = new_pending_read_promise(scope) else {
                finish_writable_stream_write(scope, stream, write_size, WriteCompletion::Discarded);
                return None;
            };
            enqueue_pending_writable_sink_write(scope, stream, chunk, pending, write_size);
            process_writable_sink_write_queue(scope, stream, sink);
            Some(promise.into())
        }
    }
}

fn attach_writable_sink_write_settlement_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    write_result: v8::Local<'s, v8::Value>,
) {
    let Ok(promise) = v8::Local::<v8::Promise>::try_from(write_result) else {
        return;
    };
    publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(writable_sink_write_fulfilled_callback).data(stream.into()),
        "writable sink write fulfillment",
        v8::Function::builder(writable_sink_write_rejected_callback).data(stream.into()),
        "writable sink write rejection",
        "writable sink write",
    )
    .finish_at_owner_boundary();
}

fn writable_sink_write_settlement_stream<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    v8::Local::<v8::Object>::try_from(data).ok()
}

fn writable_sink_write_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(stream) = writable_sink_write_settlement_stream(scope, args.data()) {
        finish_running_writable_sink_write(scope, stream, None);
        schedule_writable_pipe_owner_drain(scope, stream);
        process_writable_sink_write_queue_for_stream(scope, stream);
    }
    rv.set_undefined();
}

fn writable_sink_write_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(stream) = writable_sink_write_settlement_stream(scope, args.data()) {
        finish_running_writable_sink_write(scope, stream, Some(args.get(0)));
        schedule_writable_pipe_owner_drain(scope, stream);
    }
    rv.set_undefined();
}

fn process_writable_sink_write_queue_for_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let Some(sink) = stream_slot_object(scope, stream, WRITABLE_STREAM_SINK_SLOT) else {
        return;
    };
    if sink.is_null_or_undefined() {
        return;
    }
    process_writable_sink_write_queue(scope, stream, sink);
}

fn process_writable_sink_write_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    sink: v8::Local<'s, v8::Object>,
) {
    let (source, running, is_close) =
        match writable_controller_snapshot(scope, stream).plan_sink_pump() {
            SinkPumpPlan::Wait => return,
            SinkPumpPlan::FinishErroring => {
                let _ = advance_writable_stream_erroring(scope, stream);
                return;
            }
            SinkPumpPlan::StartWrite { source, running } => (source, running, false),
            SinkPumpPlan::StartClose { source, running } => (source, running, true),
        };
    let Some(entry) = peek_pending_transform_write(scope, stream) else {
        return;
    };
    if pending_write_kind(scope, entry) != source {
        return;
    }
    let pending = entry
        .get_index(scope, TRANSFORM_PENDING_WRITE_PROMISE_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    if is_close {
        set_pending_transform_write_kind(scope, entry, pending_write_kind_name(running).unwrap());
        let webidl_callback = writable_stream_has_webidl_algorithms(scope, stream);
        match invoke_writable_stream_algorithm(
            scope,
            stream,
            sink,
            WRITABLE_STREAM_ALGORITHM_SINK_CLOSE_INDEX,
            "close",
            &[],
        ) {
            Ok(result) if webidl_callback => {
                if let Some(promise) = normalize_stream_algorithm_result(scope, result) {
                    attach_queued_writable_sink_close_settlement_handlers(
                        scope,
                        stream,
                        promise.into(),
                    );
                } else {
                    finish_running_writable_sink_close(scope, stream, None);
                }
            }
            Ok(Some(result)) if result.is_promise() => {
                attach_queued_writable_sink_close_settlement_handlers(scope, stream, result);
            }
            Ok(_) => finish_running_writable_sink_close(scope, stream, None),
            Err(error) => finish_running_writable_sink_close(scope, stream, Some(error)),
        }
        return;
    }
    let chunk = entry
        .get_index(scope, TRANSFORM_PENDING_WRITE_CHUNK_INDEX)
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_pending_transform_write_kind(scope, entry, pending_write_kind_name(running).unwrap());
    let controller = stream_slot_object(scope, stream, WRITABLE_STREAM_CONTROLLER_SLOT)
        .expect("an initialized writable stream must retain its controller");
    let arguments = [chunk, controller.into()];
    let webidl_callback = writable_stream_has_webidl_algorithms(scope, stream);
    match invoke_writable_stream_algorithm(
        scope,
        stream,
        sink,
        WRITABLE_STREAM_ALGORITHM_SINK_WRITE_INDEX,
        "write",
        &arguments,
    ) {
        Ok(result) if webidl_callback => {
            if let Some(promise) = normalize_stream_algorithm_result(scope, result) {
                attach_writable_sink_write_settlement_handlers(scope, stream, promise.into());
                return;
            }
        }
        Ok(Some(result)) if result.is_promise() => {
            attach_writable_sink_write_settlement_handlers(scope, stream, result);
            return;
        }
        Ok(_) => {
            let _ = dequeue_pending_transform_write(scope, stream);
            if let Some(pending) = pending {
                resolve_pending_promise(scope, pending, v8::undefined(scope).into());
            }
            finish_pending_transform_write(scope, stream, entry, WriteCompletion::Fulfilled);
        }
        Err(error) => {
            let _ = dequeue_pending_transform_write(scope, stream);
            if let Some(pending) = pending {
                reject_pending_read(scope, pending, error);
            }
            finish_pending_transform_write(scope, stream, entry, WriteCompletion::Rejected);
            deal_with_writable_stream_rejection(scope, stream, error);
            schedule_writable_pipe_owner_drain(scope, stream);
            return;
        }
    }
    schedule_writable_pipe_owner_drain(scope, stream);
    schedule_writable_sink_write_queue_continuation(scope, stream);
}

fn attach_queued_writable_sink_close_settlement_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    close_result: v8::Local<'s, v8::Value>,
) {
    let Ok(promise) = v8::Local::<v8::Promise>::try_from(close_result) else {
        finish_running_writable_sink_close(scope, stream, None);
        return;
    };
    publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(queued_writable_sink_close_fulfilled_callback).data(stream.into()),
        "writable sink close fulfillment",
        v8::Function::builder(queued_writable_sink_close_rejected_callback).data(stream.into()),
        "writable sink close rejection",
        "writable sink close",
    )
    .finish_at_owner_boundary();
}

fn queued_writable_sink_close_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(stream) = v8::Local::<v8::Object>::try_from(args.data()) {
        finish_running_writable_sink_close(scope, stream, None);
    }
    rv.set_undefined();
}

fn queued_writable_sink_close_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(stream) = v8::Local::<v8::Object>::try_from(args.data()) {
        finish_running_writable_sink_close(scope, stream, Some(args.get(0)));
    }
    rv.set_undefined();
}

fn finish_running_writable_sink_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    error: Option<v8::Local<'s, v8::Value>>,
) {
    let Some(entry) = dequeue_pending_transform_write(scope, stream) else {
        return;
    };
    let entry_pending = entry
        .get_index(scope, TRANSFORM_PENDING_WRITE_PROMISE_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let pending = take_writable_stream_pending_close(scope, stream).or(entry_pending);
    match error {
        Some(error) => {
            if let Some(pending) = pending {
                reject_pending_read(scope, pending, error);
            }
            if let CloseSettlementPlan::Reject {
                reject_pending_abort: true,
            } = writable_stream_snapshot(scope, stream)
                .erroring()
                .plan_close_settlement(CloseOutcome::Rejected)
            {
                let record = take_writable_stream_pending_abort(scope, stream)
                    .expect("an in-flight close must retain its pending abort request");
                let residence = writable_pending_abort_residence(scope, record)
                    .expect("a pending abort request must retain its residence");
                reject_pending_read(scope, residence, error);
            }
            deal_with_writable_stream_rejection(scope, stream, error);
        }
        None => {
            if let Some(pending) = pending {
                resolve_pending_promise(scope, pending, v8::undefined(scope).into());
            }
            mark_writable_stream_closed(scope, stream);
        }
    }
}

fn schedule_writable_sink_write_queue_continuation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let next = match writable_controller_snapshot(scope, stream).plan_schedule_sink_continuation() {
        ContinuationPlan::Ignore => return,
        ContinuationPlan::Schedule(next) => next,
    };
    set_writable_queue_pump_state(scope, stream, next.stored());
    let data = v8::Array::new(scope, 1);
    let _ = data.set_index(
        scope,
        WRITABLE_QUEUE_CONTINUATION_STREAM_INDEX,
        stream.into(),
    );
    let StreamOwnerPublication::Published(callback) = build_required_stream_callback(
        scope,
        v8::Function::builder(writable_sink_write_queue_continuation_callback).data(data.into()),
        "writable sink queue continuation",
    ) else {
        return;
    };
    scope.enqueue_microtask(callback);
}

fn writable_sink_write_queue_continuation_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = v8::Local::<v8::Array>::try_from(args.data())
        .ok()
        .and_then(|data| data.get_index(scope, WRITABLE_QUEUE_CONTINUATION_STREAM_INDEX))
        .and_then(|stream| v8::Local::<v8::Object>::try_from(stream).ok());
    if let Some(stream) = stream {
        let state = PumpState::from_stored(writable_queue_pump_state(scope, stream));
        set_writable_queue_pump_state(scope, stream, state.with_sink_continuation(false).stored());
        process_writable_sink_write_queue_for_stream(scope, stream);
    }
    rv.set_undefined();
}

fn finish_running_writable_sink_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    error: Option<v8::Local<'s, v8::Value>>,
) {
    let Some(entry) = dequeue_pending_transform_write(scope, stream) else {
        return;
    };
    let pending = entry
        .get_index(scope, TRANSFORM_PENDING_WRITE_PROMISE_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    match error {
        Some(error) => {
            if let Some(pending) = pending {
                reject_pending_read(scope, pending, error);
            }
            finish_pending_transform_write(scope, stream, entry, WriteCompletion::Rejected);
            deal_with_writable_stream_rejection(scope, stream, error);
        }
        None => {
            if let Some(pending) = pending {
                resolve_pending_promise(scope, pending, v8::undefined(scope).into());
            }
            finish_pending_transform_write(scope, stream, entry, WriteCompletion::Fulfilled);
        }
    }
}

pub(in crate::context_bootstrap::stream_adapter) fn transform_stream_readable_pull_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    schedule_transform_writable_queue_continuation(scope, args.this());
    rv.set_undefined();
}

pub(in crate::context_bootstrap::stream_adapter) fn transform_stream_readable_cancel_callback<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let writable = args.this();
    let reason = args.get(0);
    let Some(readable) = stream_slot_object(scope, writable, WRITABLE_STREAM_TARGET_READABLE_SLOT)
        .filter(|readable| !readable.is_null_or_undefined())
    else {
        error_writable_stream_with_value(scope, writable, reason);
        rv.set_undefined();
        return;
    };
    let Some(claim) = claim_transform_finish_residence(
        scope,
        writable,
        readable,
        FinishOperation::ReadableCancel,
    ) else {
        error_writable_stream_with_value(scope, writable, reason);
        rv.set_undefined();
        return;
    };
    let (finish_promise, residence, algorithm) = match claim {
        TransformFinishResidenceClaim::Existing(promise) => {
            rv.set(promise.into());
            return;
        }
        TransformFinishResidenceClaim::Started {
            promise,
            residence,
            algorithm,
        } => (promise, residence, algorithm),
    };

    let FinishAlgorithm::Cancel(algorithm) = algorithm else {
        unreachable!("readable cancel must claim the transform cancel algorithm")
    };
    let writable_state_before_algorithm = writable_stream_snapshot(scope, writable).state();
    if matches!(algorithm, TransformCancelAlgorithm::None) {
        clear_transform_stream_terminal_algorithms(scope, writable);
        let fulfillment_plan = transform_stream_snapshot(scope, writable, readable)
            .plan_source_cancel_fulfillment_after_algorithm(writable_state_before_algorithm);
        apply_transform_source_cancel_fulfillment(
            scope,
            writable,
            residence,
            reason,
            fulfillment_plan,
        );
        rv.set(finish_promise.into());
        return;
    }
    let cancel_result =
        invoke_transform_stream_cancel_algorithm(scope, writable, reason, algorithm);
    clear_transform_stream_terminal_algorithms(scope, writable);
    let Some(cancel_promise) = transform_algorithm_result_promise(scope, cancel_result) else {
        apply_transform_finish_setup_failure(
            scope,
            writable,
            readable,
            residence,
            FinishOperation::ReadableCancel,
            reason,
        );
        rv.set(finish_promise.into());
        return;
    };
    // Freeze the writable-side outcome at the cancel-algorithm boundary.
    // A synchronous controller error or abort from inside `cancel()` must
    // reject this finish residence, while a controller action after
    // `readable.cancel()` returns must not retroactively reject it.
    let fulfillment_plan = transform_stream_snapshot(scope, writable, readable)
        .plan_source_cancel_fulfillment_after_algorithm(writable_state_before_algorithm);
    attach_transform_source_cancel_reactions(
        scope,
        cancel_promise,
        writable,
        residence,
        reason,
        fulfillment_plan,
    );
    rv.set(finish_promise.into());
}

/// A transform controller has one terminal residence shared by writable
/// close/abort and readable cancel.
///
/// `Existing` means another terminal algorithm already owns callback
/// invocation and settlement. The caller must return the same promise without
/// invoking `flush` or `cancel` again. `Started` transfers the one-shot
/// resolver capability to this caller.
enum TransformFinishResidenceClaim<'s> {
    Existing(v8::Local<'s, v8::Promise>),
    Started {
        promise: v8::Local<'s, v8::Promise>,
        residence: v8::Local<'s, v8::Object>,
        algorithm: FinishAlgorithm,
    },
}

fn claim_transform_finish_residence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    operation: FinishOperation,
) -> Option<TransformFinishResidenceClaim<'s>> {
    let controller = stream_slot_object(scope, writable, WRITABLE_STREAM_CONTROLLER_SLOT)?;
    match transform_stream_snapshot(scope, writable, readable).plan_finish(operation) {
        FinishClaimPlan::Reuse => {
            let promise = stream_slot_value(
                scope,
                controller,
                TRANSFORM_STREAM_CONTROLLER_FINISH_PROMISE_SLOT,
            )
            .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())?;
            Some(TransformFinishResidenceClaim::Existing(promise))
        }
        FinishClaimPlan::Claim { algorithm } => {
            let (promise, residence) = new_pending_read_promise(scope)?;
            set_stream_slot_value(
                scope,
                controller,
                TRANSFORM_STREAM_CONTROLLER_FINISH_PROMISE_SLOT,
                promise.into(),
            );
            set_stream_slot_value(
                scope,
                controller,
                TRANSFORM_STREAM_CONTROLLER_FINISH_RESIDENCE_SLOT,
                residence.into(),
            );
            Some(TransformFinishResidenceClaim::Started {
                promise,
                residence,
                algorithm,
            })
        }
    }
}

fn invoke_transform_stream_cancel_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
    algorithm: TransformCancelAlgorithm,
) -> Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>> {
    if matches!(algorithm, TransformCancelAlgorithm::None) {
        return Ok(None);
    }
    let Some(transformer) = stream_slot_object(scope, writable, WRITABLE_STREAM_TRANSFORMER_SLOT)
        .filter(|transformer| !transformer.is_null_or_undefined())
    else {
        return Ok(None);
    };
    invoke_writable_stream_algorithm(
        scope,
        writable,
        transformer,
        WRITABLE_STREAM_ALGORITHM_TRANSFORM_CANCEL_INDEX,
        "cancel",
        &[reason],
    )
}

fn clear_transform_stream_terminal_algorithms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
) {
    if let Some(algorithms) = stream_slot_array(scope, writable, WRITABLE_STREAM_ALGORITHMS_SLOT) {
        for index in [
            WRITABLE_STREAM_ALGORITHM_TRANSFORM_INDEX,
            WRITABLE_STREAM_ALGORITHM_TRANSFORM_FLUSH_INDEX,
            WRITABLE_STREAM_ALGORITHM_TRANSFORM_CANCEL_INDEX,
        ] {
            let _ = algorithms.set_index(scope, index, v8::undefined(scope).into());
        }
    }
    set_stream_slot_value(
        scope,
        writable,
        WRITABLE_STREAM_TRANSFORMER_SLOT,
        v8::null(scope).into(),
    );
}

fn transform_algorithm_result_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Promise>> {
    match result {
        Ok(result) => normalize_stream_algorithm_result(scope, result),
        Err(error) => rejected_promise_value(scope, error)
            .and_then(|promise| v8::Local::<v8::Promise>::try_from(promise).ok()),
    }
}

fn apply_transform_finish_setup_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    residence: v8::Local<'s, v8::Object>,
    operation: FinishOperation,
    reason: v8::Local<'s, v8::Value>,
) {
    match transform_stream_snapshot(scope, writable, readable).plan_finish_setup_failure(operation)
    {
        FinishSetupFailurePlan::ErrorWritableWithOriginalReasonAndReject => {
            error_writable_stream_with_value(scope, writable, reason);
            reject_pending_read(scope, residence, reason);
        }
        FinishSetupFailurePlan::ErrorReadableWithOriginalReasonAndReject
        | FinishSetupFailurePlan::ErrorReadableWithUndefinedAndReject => {
            error_stream(scope, readable, reason);
            reject_pending_read(scope, residence, reason);
        }
    }
}

fn attach_transform_source_cancel_reactions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cancel_promise: v8::Local<'s, v8::Promise>,
    writable: v8::Local<'s, v8::Object>,
    residence: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
    fulfillment_plan: FinishSettlementPlan,
) {
    let data = v8::Array::new(scope, 4);
    let _ = data.set_index(scope, 0, writable.into());
    let _ = data.set_index(scope, 1, residence.into());
    let _ = data.set_index(scope, 2, reason);
    let reject_with_stored_error = matches!(
        fulfillment_plan,
        FinishSettlementPlan::RejectWithWritableStoredError
    );
    let _ = data.set_index(
        scope,
        3,
        v8::Boolean::new(scope, reject_with_stored_error).into(),
    );
    publish_required_stream_promise_reactions(
        scope,
        cancel_promise,
        v8::Function::builder(transform_source_cancel_fulfilled_callback).data(data.into()),
        "transform source cancel fulfillment",
        v8::Function::builder(transform_source_cancel_rejected_callback).data(data.into()),
        "transform source cancel rejection",
        "transform source cancel",
    )
    .finish_at_owner_boundary();
}

fn transform_source_cancel_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((writable, residence, reason, fulfillment_plan)) =
        transform_source_cancel_reaction_values(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    let Some(_readable) = stream_slot_object(scope, writable, WRITABLE_STREAM_TARGET_READABLE_SLOT)
        .filter(|readable| !readable.is_null_or_undefined())
    else {
        reject_pending_read(scope, residence, reason);
        rv.set_undefined();
        return;
    };
    apply_transform_source_cancel_fulfillment(scope, writable, residence, reason, fulfillment_plan);
    rv.set_undefined();
}

fn apply_transform_source_cancel_fulfillment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    residence: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
    fulfillment_plan: FinishSettlementPlan,
) {
    match fulfillment_plan {
        FinishSettlementPlan::RejectWithWritableStoredError => {
            let error = writable_stream_stored_error(scope, writable).unwrap_or(reason);
            reject_pending_read(scope, residence, error);
        }
        FinishSettlementPlan::ErrorWritableWithOriginalReasonAndResolve => {
            error_writable_stream_with_value(scope, writable, reason);
            resolve_pending_promise(scope, residence, v8::undefined(scope).into());
        }
        _ => unreachable!("readable cancel fulfillment produced an invalid plan"),
    }
}

fn transform_source_cancel_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((writable, residence, _, _)) =
        transform_source_cancel_reaction_values(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    let error = args.get(0);
    let Some(readable) = stream_slot_object(scope, writable, WRITABLE_STREAM_TARGET_READABLE_SLOT)
        .filter(|readable| !readable.is_null_or_undefined())
    else {
        reject_pending_read(scope, residence, error);
        rv.set_undefined();
        return;
    };
    match transform_stream_snapshot(scope, writable, readable)
        .plan_finish_settlement(FinishOperation::ReadableCancel, AlgorithmOutcome::Rejected)
    {
        FinishSettlementPlan::ErrorWritableWithCallbackErrorAndReject => {
            error_writable_stream_with_value(scope, writable, error);
            reject_pending_read(scope, residence, error);
        }
        _ => unreachable!("readable cancel rejection produced an invalid plan"),
    }
    rv.set_undefined();
}

fn transform_source_cancel_reaction_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Value>,
    FinishSettlementPlan,
)> {
    let data = v8::Local::<v8::Array>::try_from(data).ok()?;
    let writable = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let residence = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let reason = data
        .get_index(scope, 2)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let fulfillment_plan = if data
        .get_index(scope, 3)
        .is_some_and(|value| value.boolean_value(scope))
    {
        FinishSettlementPlan::RejectWithWritableStoredError
    } else {
        FinishSettlementPlan::ErrorWritableWithOriginalReasonAndResolve
    };
    Some((writable, residence, reason, fulfillment_plan))
}

fn transform_stream_sink_abort_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let readable = v8::Global::new(scope, readable);
    let reason = v8::Global::new(scope, reason);
    with_transform_stream_relevant_realm(scope, writable, |scope, writable| {
        let readable = v8::Local::new(scope, &readable);
        let reason = v8::Local::new(scope, &reason);
        transform_stream_sink_abort_algorithm_in_relevant_realm(scope, writable, readable, reason)
    })
}

fn transform_stream_sink_abort_algorithm_in_relevant_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let claim = claim_transform_finish_residence(
        scope,
        writable,
        readable,
        FinishOperation::WritableAbort,
    )?;
    let (finish_promise, residence, algorithm) = match claim {
        TransformFinishResidenceClaim::Existing(promise) => return Some(promise.into()),
        TransformFinishResidenceClaim::Started {
            promise,
            residence,
            algorithm,
        } => (promise, residence, algorithm),
    };
    let FinishAlgorithm::Cancel(algorithm) = algorithm else {
        unreachable!("writable abort must claim the transform cancel algorithm")
    };
    if matches!(algorithm, TransformCancelAlgorithm::None) {
        clear_transform_stream_terminal_algorithms(scope, writable);
        apply_transform_sink_abort_fulfillment(scope, writable, readable, residence, reason);
        return Some(finish_promise.into());
    }
    let cancel_result =
        invoke_transform_stream_cancel_algorithm(scope, writable, reason, algorithm);
    clear_transform_stream_terminal_algorithms(scope, writable);
    let Some(cancel_promise) = transform_algorithm_result_promise(scope, cancel_result) else {
        apply_transform_finish_setup_failure(
            scope,
            writable,
            readable,
            residence,
            FinishOperation::WritableAbort,
            reason,
        );
        return Some(finish_promise.into());
    };
    let data = v8::Array::new(scope, 4);
    let _ = data.set_index(scope, 0, writable.into());
    let _ = data.set_index(scope, 1, readable.into());
    let _ = data.set_index(scope, 2, residence.into());
    let _ = data.set_index(scope, 3, reason);
    if matches!(
        publish_required_stream_promise_reactions(
            scope,
            cancel_promise,
            v8::Function::builder(transform_sink_abort_fulfilled_callback).data(data.into()),
            "transform sink abort fulfillment",
            v8::Function::builder(transform_sink_abort_rejected_callback).data(data.into()),
            "transform sink abort rejection",
            "transform sink abort",
        ),
        StreamOwnerPublication::OwnerTerminating
    ) {
        return Some(finish_promise.into());
    }
    Some(finish_promise.into())
}

fn transform_sink_abort_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((readable, residence, reason)) =
        transform_sink_abort_reaction_values(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    let Some(writable) = transform_sink_abort_reaction_writable(scope, args.data()) else {
        reject_pending_read(scope, residence, reason);
        rv.set_undefined();
        return;
    };
    apply_transform_sink_abort_fulfillment(scope, writable, readable, residence, reason);
    rv.set_undefined();
}

fn apply_transform_sink_abort_fulfillment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    residence: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    match transform_stream_snapshot(scope, writable, readable)
        .plan_finish_settlement(FinishOperation::WritableAbort, AlgorithmOutcome::Fulfilled)
    {
        FinishSettlementPlan::RejectWithReadableStoredError => {
            let error = readable_stream_error(scope, readable).unwrap_or(reason);
            reject_pending_read(scope, residence, error);
        }
        FinishSettlementPlan::ErrorReadableWithOriginalReasonAndResolve => {
            error_stream(scope, readable, reason);
            resolve_pending_promise(scope, residence, v8::undefined(scope).into());
        }
        _ => unreachable!("writable abort fulfillment produced an invalid plan"),
    }
}

fn transform_sink_abort_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((readable, residence, _)) = transform_sink_abort_reaction_values(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    let error = args.get(0);
    let Some(writable) = transform_sink_abort_reaction_writable(scope, args.data()) else {
        reject_pending_read(scope, residence, error);
        rv.set_undefined();
        return;
    };
    match transform_stream_snapshot(scope, writable, readable)
        .plan_finish_settlement(FinishOperation::WritableAbort, AlgorithmOutcome::Rejected)
    {
        FinishSettlementPlan::ErrorReadableWithCallbackErrorAndReject => {
            error_stream(scope, readable, error);
            reject_pending_read(scope, residence, error);
        }
        _ => unreachable!("writable abort rejection produced an invalid plan"),
    }
    rv.set_undefined();
}

fn transform_sink_abort_reaction_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Value>,
)> {
    let data = v8::Local::<v8::Array>::try_from(data).ok()?;
    let readable = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let residence = data
        .get_index(scope, 2)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let reason = data
        .get_index(scope, 3)
        .unwrap_or_else(|| v8::undefined(scope).into());
    Some((readable, residence, reason))
}

fn transform_sink_abort_reaction_writable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = v8::Local::<v8::Array>::try_from(data).ok()?;
    data.get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn process_transform_writable_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
) {
    process_transform_writable_queue_inner(scope, writable);
}

fn process_transform_writable_queue_inner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
) {
    let Some(readable) = stream_slot_object(scope, writable, WRITABLE_STREAM_TARGET_READABLE_SLOT)
    else {
        return;
    };
    if readable.is_null_or_undefined() {
        return;
    }
    let readable_has_capacity = transform_readable_can_accept_chunk(scope, readable);
    let (source, running, is_close) = match writable_controller_snapshot(scope, writable)
        .plan_transform_pump(readable_has_capacity)
    {
        TransformPumpPlan::Wait => return,
        TransformPumpPlan::FinishErroring => {
            let _ = advance_writable_stream_erroring(scope, writable);
            return;
        }
        TransformPumpPlan::StartWrite { source, running } => (source, running, false),
        TransformPumpPlan::StartClose { source, running } => (source, running, true),
    };
    let Some(entry) = peek_pending_transform_write(scope, writable) else {
        return;
    };
    if pending_write_kind(scope, entry) != source {
        return;
    }
    let chunk = entry
        .get_index(scope, TRANSFORM_PENDING_WRITE_CHUNK_INDEX)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let pending = entry
        .get_index(scope, TRANSFORM_PENDING_WRITE_PROMISE_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    set_pending_transform_write_kind(scope, entry, pending_write_kind_name(running).unwrap());
    let result = if is_close {
        set_transform_close_in_flight(scope, writable, true);
        perform_transform_stream_close(scope, writable, readable)
    } else {
        perform_transform_stream_write(scope, writable, readable, chunk, None)
    };
    if let Some(promise) = normalize_stream_algorithm_result(scope, result) {
        attach_pending_transform_write_continuation(scope, writable, pending, promise);
        return;
    }
    let _ = dequeue_pending_transform_write(scope, writable);
    if let Some(pending) = pending {
        resolve_pending_promise(scope, pending, v8::undefined(scope).into());
    }
    finish_pending_transform_write(scope, writable, entry, WriteCompletion::Fulfilled);
    if is_close {
        return;
    }
    if let Some(error) = writable_stream_stored_error(scope, writable) {
        reject_pending_transform_writes(scope, writable, error);
        return;
    }
    schedule_transform_writable_queue_continuation(scope, writable);
}

fn schedule_transform_writable_queue_continuation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
) {
    let next = match writable_controller_snapshot(scope, writable)
        .plan_schedule_transform_continuation()
    {
        ContinuationPlan::Ignore => return,
        ContinuationPlan::Schedule(next) => next,
    };
    set_writable_queue_pump_state(scope, writable, next.stored());
    let data = v8::Array::new(scope, 1);
    let _ = data.set_index(
        scope,
        WRITABLE_QUEUE_CONTINUATION_STREAM_INDEX,
        writable.into(),
    );
    let StreamOwnerPublication::Published(callback) = build_required_stream_callback(
        scope,
        v8::Function::builder(transform_writable_queue_continuation_callback).data(data.into()),
        "transform writable queue continuation",
    ) else {
        return;
    };
    scope.enqueue_microtask(callback);
}

fn transform_writable_queue_continuation_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = v8::Local::<v8::Array>::try_from(args.data()).ok() else {
        rv.set_undefined();
        return;
    };
    let Some(writable) = data
        .get_index(scope, WRITABLE_QUEUE_CONTINUATION_STREAM_INDEX)
        .and_then(|writable| v8::Local::<v8::Object>::try_from(writable).ok())
    else {
        rv.set_undefined();
        return;
    };
    let state = PumpState::from_stored(writable_queue_pump_state(scope, writable));
    set_writable_queue_pump_state(
        scope,
        writable,
        state.with_transform_continuation(false).stored(),
    );
    process_transform_writable_queue_inner(scope, writable);
    rv.set_undefined();
}

fn normalize_stream_algorithm_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let result = result.unwrap_or_else(|| v8::undefined(scope).into());
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(result) {
        return Some(promise);
    }
    resolved_promise_value(scope, result)
        .and_then(|promise| v8::Local::<v8::Promise>::try_from(promise).ok())
}

fn writable_queue_pump_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> u32 {
    writable_stream_strategy_number(
        scope,
        stream,
        WRITABLE_STREAM_STRATEGY_QUEUE_PUMP_STATE_INDEX,
    )
    .unwrap_or(0.0) as u32
}

fn set_writable_queue_pump_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    state: u32,
) {
    set_writable_stream_strategy_number(
        scope,
        stream,
        WRITABLE_STREAM_STRATEGY_QUEUE_PUMP_STATE_INDEX,
        state as f64,
    );
}

fn transform_close_in_flight<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    PumpState::from_stored(writable_queue_pump_state(scope, stream)).transform_close_in_flight()
}

fn set_transform_close_in_flight<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    in_flight: bool,
) {
    let state = PumpState::from_stored(writable_queue_pump_state(scope, stream));
    set_writable_queue_pump_state(
        scope,
        stream,
        state.with_transform_close_in_flight(in_flight).stored(),
    );
}

pub(in crate::context_bootstrap) fn set_transform_stream_start_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    result: Option<v8::Local<'s, v8::Value>>,
) {
    let Some(promise) = normalize_stream_algorithm_result(scope, result) else {
        return;
    };
    let data = transform_promise_reaction_data(scope, writable, readable);
    publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(transform_start_fulfilled_callback).data(data.into()),
        "transform start fulfillment",
        v8::Function::builder(transform_start_rejected_callback).data(data.into()),
        "transform start rejection",
        "transform start",
    )
    .finish_at_owner_boundary();
}

pub(in crate::context_bootstrap) fn set_writable_stream_start_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    result: Option<v8::Local<'s, v8::Value>>,
) {
    let Some(promise) = normalize_stream_algorithm_result(scope, result) else {
        return;
    };
    publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(writable_start_fulfilled_callback).data(writable.into()),
        "writable start fulfillment",
        v8::Function::builder(writable_start_rejected_callback).data(writable.into()),
        "writable start rejection",
        "writable start",
    )
    .finish_at_owner_boundary();
}

fn writable_start_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(writable) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    set_transform_stream_start_pending(scope, writable, false);
    if !advance_writable_stream_erroring(scope, writable) {
        process_writable_sink_write_queue_for_stream(scope, writable);
    }
    rv.set_undefined();
}

fn writable_start_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(writable) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    set_transform_stream_start_pending(scope, writable, false);
    deal_with_writable_stream_rejection(scope, writable, args.get(0));
    rv.set_undefined();
}

fn transform_start_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((writable, readable)) = transform_promise_reaction_streams(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    match transform_stream_snapshot(scope, writable, readable)
        .plan_start_settlement(AlgorithmOutcome::Fulfilled)
    {
        StartSettlementPlan::ClearPendingAndPump => {
            set_transform_stream_start_pending(scope, writable, false);
            if !advance_writable_stream_erroring(scope, writable) {
                process_transform_writable_queue(scope, writable);
            }
        }
        StartSettlementPlan::ClearPendingAndErrorBoth => unreachable!(),
    }
    rv.set_undefined();
}

fn transform_start_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((writable, readable)) = transform_promise_reaction_streams(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    match transform_stream_snapshot(scope, writable, readable)
        .plan_start_settlement(AlgorithmOutcome::Rejected)
    {
        StartSettlementPlan::ClearPendingAndErrorBoth => {
            set_transform_stream_start_pending(scope, writable, false);
            if !advance_writable_stream_erroring(scope, writable) {
                error_transform_stream_with_value(scope, writable, readable, args.get(0));
            }
        }
        StartSettlementPlan::ClearPendingAndPump => unreachable!(),
    }
    rv.set_undefined();
}

fn transform_stream_start_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)
        .and_then(|strategy| {
            strategy.get_index(scope, WRITABLE_STREAM_STRATEGY_START_PENDING_INDEX)
        })
        .is_some_and(|value| value.boolean_value(scope))
}

fn set_transform_stream_start_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    pending: bool,
) {
    if let Some(strategy) = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT) {
        let _ = strategy.set_index(
            scope,
            WRITABLE_STREAM_STRATEGY_START_PENDING_INDEX,
            v8::Boolean::new(scope, pending).into(),
        );
    }
}

pub(in crate::context_bootstrap) fn begin_writable_stream_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    set_transform_stream_start_pending(scope, stream, true);
}

/// Runs a browser-owned TransformStream algorithm in the transform's relevant
/// Realm. The writable side is allocated alongside the transform, so its
/// creation context owns built-in conversions, exceptions, chunks, and
/// internal promises. A page callback can still enter its own callback Realm
/// from inside this boundary.
fn with_transform_stream_relevant_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    operation: impl FnOnce(
        &mut v8::PinScope<'s, '_>,
        v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(relevant_context) = writable.get_creation_context(scope) else {
        return operation(scope, writable);
    };
    if relevant_context == scope.get_current_context() {
        return operation(scope, writable);
    }

    let writable = v8::Global::new(scope, writable);
    let result = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let writable = v8::Local::new(target_scope, &writable);
        operation(target_scope, writable).map(|value| v8::Global::new(target_scope, value))
    };
    result.map(|value| v8::Local::new(scope, &value))
}

fn perform_transform_stream_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
    write_size: Option<f64>,
) -> Option<v8::Local<'s, v8::Value>> {
    let readable = v8::Global::new(scope, readable);
    let chunk = v8::Global::new(scope, chunk);
    with_transform_stream_relevant_realm(scope, stream, |scope, stream| {
        let readable = v8::Local::new(scope, &readable);
        let chunk = v8::Local::new(scope, &chunk);
        perform_transform_stream_write_in_relevant_realm(scope, stream, readable, chunk, write_size)
    })
}

fn perform_transform_stream_write_in_relevant_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
    write_size: Option<f64>,
) -> Option<v8::Local<'s, v8::Value>> {
    let algorithm =
        transform_stream_snapshot(scope, stream, readable).plan_queued_write_algorithm();
    match algorithm {
        TransformWriteAlgorithm::TextEncoder => {
            let input = match text_encoder_stream_chunk_string(scope, chunk) {
                Ok(input) => input,
                Err(error) => {
                    return reject_transform_stream_with_value(scope, stream, readable, error);
                }
            };
            let bytes = new_uint8_array_from_bytes(scope, input.into_bytes())
                .expect("TextEncoderStream output allocation must succeed");
            if let Err(error) =
                enqueue_transform_readable_chunk(scope, stream, readable, bytes.into())
            {
                return rejected_promise_value(scope, error);
            }
            return None;
        }
        TransformWriteAlgorithm::TextDecoder => {
            if let Some(bytes) = value_buffer_source_bytes(scope, chunk) {
                return write_text_decoder_stream_chunk(scope, stream, readable, &bytes);
            }
            let error = text_decoder_stream_type_error_value(scope);
            return reject_transform_stream_with_value(scope, stream, readable, error);
        }
        TransformWriteAlgorithm::Callback => {
            let transformer = stream_slot_object(scope, stream, WRITABLE_STREAM_TRANSFORMER_SLOT)?;
            let controller = stream_slot_object(scope, stream, WRITABLE_STREAM_CONTROLLER_SLOT)
                .unwrap_or_else(|| {
                    super::super::stream_objects::new_transform_stream_controller_object(
                        scope, readable, stream,
                    )
                });
            let result = match invoke_writable_stream_algorithm(
                scope,
                stream,
                transformer,
                WRITABLE_STREAM_ALGORITHM_TRANSFORM_INDEX,
                "transform",
                &[chunk, controller.into()],
            ) {
                Ok(result) => result,
                Err(error) => {
                    return rejected_promise_value(scope, error);
                }
            };
            if let Some(result) = result {
                if write_size.is_none() {
                    return normalize_stream_algorithm_result(scope, Some(result)).map(Into::into);
                }
                return transform_write_result_promise(scope, stream, readable, result, write_size);
            }
        }
        TransformWriteAlgorithm::Identity => {}
    }
    let value = structured_clone_value(scope, chunk).unwrap_or(chunk);
    if let Err(error) = enqueue_transform_readable_chunk(scope, stream, readable, value) {
        return rejected_promise_value(scope, error);
    }
    None
}

fn transform_write_result_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    result: v8::Local<'s, v8::Value>,
    write_size: Option<f64>,
) -> Option<v8::Local<'s, v8::Value>> {
    let promise = normalize_stream_algorithm_result(scope, Some(result))?;
    let data = transform_promise_reaction_data(scope, stream, readable);
    if let Some(write_size) = write_size {
        let _ = data.set_index(scope, 2, v8::Number::new(scope, write_size).into());
    }
    match publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(transform_write_result_fulfilled_callback).data(data.into()),
        "transform write fulfillment",
        v8::Function::builder(transform_write_result_rejected_callback).data(data.into()),
        "transform write rejection",
        "transform write",
    ) {
        StreamOwnerPublication::Published(promise) => Some(promise.into()),
        StreamOwnerPublication::OwnerTerminating => None,
    }
}

fn transform_write_result_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((writable, readable)) = transform_promise_reaction_streams(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let write = transform_promise_reaction_write_size(scope, args.data());
    match transform_stream_snapshot(scope, writable, readable)
        .plan_write_settlement(AlgorithmOutcome::Fulfilled, write.is_some())
    {
        WriteSettlementPlan::Fulfill {
            finish_direct_write,
            drain_pipe,
        } => {
            if finish_direct_write && let Some((_, size)) = write {
                finish_writable_stream_write(scope, writable, size, WriteCompletion::Fulfilled);
            }
            if drain_pipe {
                schedule_writable_pipe_owner_drain(scope, writable);
            }
        }
        WriteSettlementPlan::Reject { .. } => unreachable!(),
    }
    rv.set_undefined();
}

fn transform_write_result_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((writable, readable)) = transform_promise_reaction_streams(scope, args.data()) else {
        scope.throw_exception(args.get(0));
        return;
    };
    let write = transform_promise_reaction_write_size(scope, args.data());
    match transform_stream_snapshot(scope, writable, readable)
        .plan_write_settlement(AlgorithmOutcome::Rejected, write.is_some())
    {
        WriteSettlementPlan::Reject {
            finish_direct_write,
            error,
        } => {
            if finish_direct_write && let Some((_, size)) = write {
                finish_writable_stream_write(scope, writable, size, WriteCompletion::Rejected);
            }
            let _ = apply_transform_error_plan(scope, writable, readable, args.get(0), error);
        }
        WriteSettlementPlan::Fulfill { .. } => unreachable!(),
    }
    scope.throw_exception(args.get(0));
    rv.set_undefined();
}

fn transform_readable_can_accept_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    readable: v8::Local<'s, v8::Object>,
) -> bool {
    transform_readable_snapshot(scope, readable).can_accept_chunk()
}

fn pending_transform_write_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)?
        .get_index(scope, WRITABLE_STREAM_STRATEGY_PENDING_WRITES_INDEX)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn pending_transform_write_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> u32 {
    let Some(queue) = pending_transform_write_queue(scope, stream) else {
        return 0;
    };
    queue
        .length()
        .saturating_sub(pending_transform_write_head(scope, stream))
}

fn pending_transform_write_head<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> u32 {
    writable_stream_strategy_number(
        scope,
        stream,
        WRITABLE_STREAM_STRATEGY_PENDING_WRITES_HEAD_INDEX,
    )
    .unwrap_or(0.0) as u32
}

fn set_pending_transform_write_head<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    head: u32,
) {
    set_writable_stream_strategy_number(
        scope,
        stream,
        WRITABLE_STREAM_STRATEGY_PENDING_WRITES_HEAD_INDEX,
        head as f64,
    );
}

fn enqueue_pending_transform_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
    pending: v8::Local<'s, v8::Object>,
    size: f64,
) {
    let Some(queue) = pending_transform_write_queue(scope, stream) else {
        return;
    };
    let entry = v8::Array::new(scope, 4);
    let _ = entry.set_index(scope, TRANSFORM_PENDING_WRITE_CHUNK_INDEX, chunk);
    let _ = entry.set_index(scope, TRANSFORM_PENDING_WRITE_PROMISE_INDEX, pending.into());
    let _ = entry.set_index(
        scope,
        TRANSFORM_PENDING_WRITE_SIZE_INDEX,
        v8::Number::new(scope, size).into(),
    );
    let _ = queue.set_index(scope, queue.length(), entry.into());
}

fn enqueue_pending_transform_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    pending: v8::Local<'s, v8::Object>,
) {
    let Some(queue) = pending_transform_write_queue(scope, stream) else {
        return;
    };
    let entry = v8::Array::new(scope, 4);
    let _ = entry.set_index(
        scope,
        TRANSFORM_PENDING_WRITE_CHUNK_INDEX,
        v8::undefined(scope).into(),
    );
    let _ = entry.set_index(scope, TRANSFORM_PENDING_WRITE_PROMISE_INDEX, pending.into());
    if let Some(kind) = v8_string(scope, TRANSFORM_PENDING_WRITE_CLOSE_KIND) {
        let _ = entry.set_index(scope, TRANSFORM_PENDING_WRITE_KIND_INDEX, kind.into());
    }
    let _ = queue.set_index(scope, queue.length(), entry.into());
    let strategy = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)
        .expect("an initialized writable stream must retain strategy storage");
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_PENDING_CLOSE_INDEX,
        pending.into(),
    );
}

fn enqueue_pending_writable_sink_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
    pending: v8::Local<'s, v8::Object>,
    size: f64,
) {
    let Some(queue) = pending_transform_write_queue(scope, stream) else {
        return;
    };
    let entry = v8::Array::new(scope, 4);
    let _ = entry.set_index(scope, TRANSFORM_PENDING_WRITE_CHUNK_INDEX, chunk);
    let _ = entry.set_index(scope, TRANSFORM_PENDING_WRITE_PROMISE_INDEX, pending.into());
    if let Some(kind) = v8_string(scope, TRANSFORM_PENDING_WRITE_SINK_KIND) {
        let _ = entry.set_index(scope, TRANSFORM_PENDING_WRITE_KIND_INDEX, kind.into());
    }
    let _ = entry.set_index(
        scope,
        TRANSFORM_PENDING_WRITE_SIZE_INDEX,
        v8::Number::new(scope, size).into(),
    );
    let _ = queue.set_index(scope, queue.length(), entry.into());
}

fn set_pending_transform_write_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Array>,
    kind: &str,
) {
    if let Some(kind) = v8_string(scope, kind) {
        let _ = entry.set_index(scope, TRANSFORM_PENDING_WRITE_KIND_INDEX, kind.into());
    }
}

fn peek_pending_transform_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let head = pending_transform_write_head(scope, stream);
    pending_transform_write_queue(scope, stream)?
        .get_index(scope, head)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn dequeue_pending_transform_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let plan = writable_controller_snapshot(scope, stream).plan_dequeue()?;
    let queue = pending_transform_write_queue(scope, stream)?;
    if pending_transform_write_head(scope, stream) as usize != plan.index()
        || queue.length() as usize <= plan.index()
    {
        return None;
    }
    let entry = queue
        .get_index(scope, plan.index() as u32)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    match plan.remainder() {
        QueueRemainderPlan::Reset => {
            if let Some(strategy) = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)
            {
                let empty = v8::Array::new(scope, 0);
                let _ = strategy.set_index(
                    scope,
                    WRITABLE_STREAM_STRATEGY_PENDING_WRITES_INDEX,
                    empty.into(),
                );
            }
            set_pending_transform_write_head(scope, stream, 0);
        }
        QueueRemainderPlan::AdvanceHead(next_head) => {
            let _ = queue.set_index(scope, plan.index() as u32, v8::undefined(scope).into());
            set_pending_transform_write_head(scope, stream, next_head as u32);
        }
    }
    Some(entry)
}

fn pending_transform_write_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Array>,
) -> Option<String> {
    entry
        .get_index(scope, TRANSFORM_PENDING_WRITE_KIND_INDEX)
        .and_then(|value| {
            if value.is_null_or_undefined() {
                return None;
            }
            value.to_string(scope)
        })
        .map(|value| value.to_rust_string_lossy(scope))
}

fn finish_pending_transform_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Array>,
    completion: WriteCompletion,
) {
    let size = entry
        .get_index(scope, TRANSFORM_PENDING_WRITE_SIZE_INDEX)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    finish_writable_stream_write(scope, stream, size, completion);
}

fn attach_pending_transform_write_continuation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    pending: Option<v8::Local<'s, v8::Object>>,
    promise: v8::Local<'s, v8::Promise>,
) {
    let data = pending_transform_write_continuation_data(scope, writable, pending);
    publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(pending_transform_write_fulfilled_callback).data(data.into()),
        "pending transform write fulfillment",
        v8::Function::builder(pending_transform_write_rejected_callback).data(data.into()),
        "pending transform write rejection",
        "pending transform write",
    )
    .finish_at_owner_boundary();
}

fn pending_transform_write_continuation_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    pending: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Array> {
    let data = v8::Array::new(scope, 2);
    let _ = data.set_index(scope, 0, writable.into());
    let pending_value = pending
        .map(|pending| pending.into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = data.set_index(scope, 1, pending_value);
    data
}

fn pending_transform_write_continuation_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, Option<v8::Local<'s, v8::Object>>)> {
    let data = v8::Local::<v8::Array>::try_from(data).ok()?;
    let writable = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let pending = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    Some((writable, pending))
}

fn pending_transform_write_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((writable, pending)) = pending_transform_write_continuation_parts(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    if let Some(pending) = pending {
        resolve_pending_promise(scope, pending, v8::undefined(scope).into());
    }
    if let Some(entry) = dequeue_pending_transform_write(scope, writable) {
        finish_pending_transform_write(scope, writable, entry, WriteCompletion::Fulfilled);
    }
    schedule_writable_pipe_owner_drain(scope, writable);
    process_transform_writable_queue_inner(scope, writable);
    rv.set_undefined();
}

fn pending_transform_write_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((writable, pending)) = pending_transform_write_continuation_parts(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    if let Some(entry) = dequeue_pending_transform_write(scope, writable) {
        finish_pending_transform_write(scope, writable, entry, WriteCompletion::Rejected);
    }
    let error = if let Some(readable) =
        stream_slot_object(scope, writable, WRITABLE_STREAM_TARGET_READABLE_SLOT)
        && !readable.is_null_or_undefined()
    {
        let WriteSettlementPlan::Reject { error, .. } =
            transform_stream_snapshot(scope, writable, readable)
                .plan_write_settlement(AlgorithmOutcome::Rejected, false)
        else {
            unreachable!("a rejected transform write must produce a rejection plan")
        };
        apply_transform_error_plan(scope, writable, readable, args.get(0), error)
    } else {
        deal_with_writable_stream_rejection(scope, writable, args.get(0));
        args.get(0)
    };
    // If another terminal path started erroring while the transform callback
    // was in flight, removing the running entry above releases its final
    // barrier. The stored first error remains authoritative.
    let _ = advance_writable_stream_erroring(scope, writable);
    if let Some(pending) = pending {
        reject_pending_read(scope, pending, error);
    }
    schedule_writable_pipe_owner_drain(scope, writable);
    rv.set_undefined();
}

fn reject_pending_transform_writes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(queue) = pending_transform_write_queue(scope, stream) else {
        return;
    };
    let head = pending_transform_write_head(scope, stream);
    for index in head..queue.length() {
        let Some(entry) = queue
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        else {
            continue;
        };
        finish_pending_transform_write(scope, stream, entry, WriteCompletion::Discarded);
        match WritableControllerSnapshot::plan_reject_entry(pending_write_kind(scope, entry)) {
            RejectEntryPlan::FinishWithoutRejectingPromise | RejectEntryPlan::DeferClosePromise => {
                continue;
            }
            RejectEntryPlan::FinishAndRejectPromise => {}
        }
        let Some(pending) = entry
            .get_index(scope, TRANSFORM_PENDING_WRITE_PROMISE_INDEX)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        reject_pending_read(scope, pending, reason);
    }
    if let Some(strategy) = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT) {
        let empty = v8::Array::new(scope, 0);
        let _ = strategy.set_index(
            scope,
            WRITABLE_STREAM_STRATEGY_PENDING_WRITES_INDEX,
            empty.into(),
        );
    }
    set_pending_transform_write_head(scope, stream, 0);
}

fn enqueue_transform_readable_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    match enqueue_chunk(scope, readable, chunk) {
        Ok(()) => Ok(()),
        Err(EnqueueChunkError::ClosedOrErrored) => {
            let synthesized = v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Cannot enqueue a chunk into a readable stream that is closed or errored",
                ),
            );
            let plan = transform_stream_snapshot(scope, writable, readable)
                .plan_enqueue_failure(TransformEnqueueFailure::ClosedOrErrored);
            let _ = apply_transform_error_plan(
                scope,
                writable,
                readable,
                synthesized,
                plan.propagation(),
            );
            debug_assert_eq!(
                plan.returned_error(),
                EnqueueErrorSource::SynthesizedTypeError
            );
            Err(synthesized)
        }
        Err(EnqueueChunkError::Strategy(provided)) => {
            let plan = transform_stream_snapshot(scope, writable, readable)
                .plan_enqueue_failure(TransformEnqueueFailure::Strategy);
            let propagated =
                apply_transform_error_plan(scope, writable, readable, provided, plan.propagation());
            match plan.returned_error() {
                EnqueueErrorSource::Provided => Err(provided),
                EnqueueErrorSource::ReadableStored => Err(propagated),
                EnqueueErrorSource::SynthesizedTypeError => unreachable!(),
            }
        }
    }
}

pub(in crate::context_bootstrap) fn writable_stream_has_capacity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    matches!(
        writable_stream_snapshot(scope, stream).plan_desired_size(),
        DesiredSizePlan::Value(value) if value > 0.0
    )
}

pub(in crate::context_bootstrap) fn acquire_writable_stream_writer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
    stream: v8::Local<'s, v8::Object>,
) {
    assert!(
        writable_stream_current_writer(scope, stream).is_none(),
        "an unlocked WritableStream must not retain a current writer"
    );
    set_stream_slot_value(
        scope,
        stream,
        WRITABLE_STREAM_CURRENT_WRITER_SLOT,
        writer.into(),
    );
    set_writable_stream_locked(scope, stream, true);

    let snapshot = writable_stream_snapshot(scope, stream);
    let plan = plan_writer_promise_initialization(
        snapshot.state(),
        snapshot.close_requested(),
        snapshot.strategy().desired_size() <= 0.0,
    );
    let stored_error = matches!(plan.ready(), InitialPromiseState::RejectedStoredError)
        .then(|| writable_stream_stored_error(scope, stream))
        .flatten();
    initialize_writer_promise_residence(
        scope,
        writer,
        WRITABLE_STREAM_WRITER_READY_PROMISE_SLOT,
        plan.ready(),
        stored_error,
    );
    initialize_writer_promise_residence(
        scope,
        writer,
        WRITABLE_STREAM_WRITER_CLOSED_PROMISE_SLOT,
        plan.closed(),
        stored_error,
    );
}

pub(in crate::context_bootstrap) fn release_writable_stream_writer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
    stream: v8::Local<'s, v8::Object>,
    released_error: v8::Local<'s, v8::Value>,
) {
    assert!(
        writable_stream_current_writer(scope, stream)
            .is_some_and(|current| current.strict_equals(writer.into())),
        "a released writer must be the stream's current writer"
    );
    ensure_writer_promise_rejected(
        scope,
        writer,
        WRITABLE_STREAM_WRITER_READY_PROMISE_SLOT,
        released_error,
    );
    ensure_writer_promise_rejected(
        scope,
        writer,
        WRITABLE_STREAM_WRITER_CLOSED_PROMISE_SLOT,
        released_error,
    );
    set_stream_slot_value(
        scope,
        stream,
        WRITABLE_STREAM_CURRENT_WRITER_SLOT,
        v8::undefined(scope).into(),
    );
    set_writable_stream_locked(scope, stream, false);
    set_stream_slot_value(
        scope,
        writer,
        WRITABLE_STREAM_WRITER_STREAM_SLOT,
        v8::undefined(scope).into(),
    );
}

pub(in crate::context_bootstrap) fn writable_stream_writer_ready_promise_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    writer_promise_value(scope, writer, WRITABLE_STREAM_WRITER_READY_PROMISE_SLOT)
}

pub(in crate::context_bootstrap) fn writable_stream_writer_closed_promise_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    writer_promise_value(scope, writer, WRITABLE_STREAM_WRITER_CLOSED_PROMISE_SLOT)
}

fn writable_stream_current_writer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    stream_slot_object(scope, stream, WRITABLE_STREAM_CURRENT_WRITER_SLOT)
}

fn initialize_writer_promise_residence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
    slot: &'static str,
    state: InitialPromiseState,
    stored_error: Option<v8::Local<'s, v8::Value>>,
) {
    let (_, entry) = new_pending_read_promise(scope)
        .expect("a WritableStreamDefaultWriter promise residence must be allocated");
    set_stream_slot_value(scope, writer, slot, entry.into());
    match state {
        InitialPromiseState::Pending => {}
        InitialPromiseState::Fulfilled => {
            resolve_pending_promise(scope, entry, v8::undefined(scope).into());
        }
        InitialPromiseState::RejectedStoredError => {
            let error = stored_error.unwrap_or_else(|| v8::undefined(scope).into());
            reject_pending_read(scope, entry, error);
            suppress_pending_read_unhandled_rejection(scope, entry);
        }
    }
}

fn writer_promise_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    let entry = stream_slot_object(scope, writer, slot)?;
    get_private_value(scope, entry, READABLE_STREAM_PENDING_READ_PROMISE_SLOT)?;
    Some(entry)
}

fn writer_promise_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let entry = writer_promise_entry(scope, writer, slot)?;
    get_private_value(scope, entry, READABLE_STREAM_PENDING_READ_PROMISE_SLOT)
}

fn writer_promise_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> PromiseResidenceState {
    let Some(promise) = writer_promise_value(scope, writer, slot)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
    else {
        return PromiseResidenceState::Missing;
    };
    match promise.state() {
        v8::PromiseState::Pending => PromiseResidenceState::Pending,
        v8::PromiseState::Fulfilled => PromiseResidenceState::Fulfilled,
        v8::PromiseState::Rejected => PromiseResidenceState::Rejected,
    }
}

fn replace_writer_promise_with_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> v8::Local<'s, v8::Object> {
    let (_, entry) = new_pending_read_promise(scope)
        .expect("a required writer promise residence must be allocated");
    set_stream_slot_value(scope, writer, slot, entry.into());
    entry
}

fn ensure_writer_promise_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writer: v8::Local<'s, v8::Object>,
    slot: &'static str,
    reason: v8::Local<'s, v8::Value>,
) {
    let entry = match plan_ensure_rejected(writer_promise_state(scope, writer, slot)) {
        EnsureRejectedPlan::RejectCurrent => writer_promise_entry(scope, writer, slot)
            .expect("a pending writer promise must retain its residence"),
        EnsureRejectedPlan::ReplaceAndReject => {
            replace_writer_promise_with_pending(scope, writer, slot)
        }
    };
    reject_pending_read(scope, entry, reason);
    suppress_pending_read_unhandled_rejection(scope, entry);
}

fn ensure_current_writer_ready_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let Some(writer) = writable_stream_current_writer(scope, stream) else {
        return;
    };
    if plan_ensure_pending(writer_promise_state(
        scope,
        writer,
        WRITABLE_STREAM_WRITER_READY_PROMISE_SLOT,
    )) == EnsurePendingPlan::ReplaceWithPending
    {
        replace_writer_promise_with_pending(
            scope,
            writer,
            WRITABLE_STREAM_WRITER_READY_PROMISE_SLOT,
        );
    }
}

fn resolve_current_writer_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    slot: &'static str,
) {
    let Some(writer) = writable_stream_current_writer(scope, stream) else {
        return;
    };
    if plan_resolve(writer_promise_state(scope, writer, slot)) == ResolvePromisePlan::ResolveCurrent
    {
        let entry = writer_promise_entry(scope, writer, slot)
            .expect("a pending writer promise must retain its residence");
        resolve_pending_promise(scope, entry, v8::undefined(scope).into());
    }
}

fn resolve_current_writer_ready<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    resolve_current_writer_promise(scope, stream, WRITABLE_STREAM_WRITER_READY_PROMISE_SLOT);
}

fn resolve_current_writer_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    resolve_current_writer_promise(scope, stream, WRITABLE_STREAM_WRITER_CLOSED_PROMISE_SLOT);
}

fn reject_current_writer_ready<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    if let Some(writer) = writable_stream_current_writer(scope, stream) {
        ensure_writer_promise_rejected(
            scope,
            writer,
            WRITABLE_STREAM_WRITER_READY_PROMISE_SLOT,
            reason,
        );
    }
}

fn reject_current_writer_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    if let Some(writer) = writable_stream_current_writer(scope, stream) {
        ensure_writer_promise_rejected(
            scope,
            writer,
            WRITABLE_STREAM_WRITER_CLOSED_PROMISE_SLOT,
            reason,
        );
    }
}

fn writable_stream_write_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
) -> Result<f64, v8::Local<'s, v8::Value>> {
    let size = writable_stream_chunk_size(scope, stream, chunk)?;
    let size = match size.to_number(scope) {
        Some(size) => size.value(),
        None => {
            return Err(v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "The return value of a queuing strategy's size function could not be converted to a number",
                ),
            ));
        }
    };
    if moli_streams::numeric::validate_queue_size(size).is_err() {
        return Err(v8::Exception::range_error(
            scope,
            v8str(
                scope,
                "The return value of a queuing strategy's size function must be a finite, non-NaN, non-negative number",
            ),
        ));
    }
    Ok(size)
}

fn record_writable_stream_write_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    size: f64,
) {
    let plan = writable_controller_snapshot(scope, stream).plan_record_write(size);
    set_writable_stream_strategy_number(
        scope,
        stream,
        WRITABLE_STREAM_STRATEGY_TOTAL_SIZE_INDEX,
        plan.total().next().value(),
    );
    if plan.ready() == ReadyTransition::EnsurePending {
        ensure_current_writer_ready_pending(scope, stream);
    }
}

fn finish_writable_stream_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    size: f64,
    completion: WriteCompletion,
) {
    let plan = writable_controller_snapshot(scope, stream).plan_finish_write(size, completion);
    set_writable_stream_strategy_number(
        scope,
        stream,
        WRITABLE_STREAM_STRATEGY_TOTAL_SIZE_INDEX,
        plan.total().next().value(),
    );
    if plan.ready() == ReadyTransition::ResolvePending {
        resolve_current_writer_ready(scope, stream);
    }
}

fn writable_stream_chunk_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, v8::Local<'s, v8::Value>> {
    let Some(strategy) = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT) else {
        return Ok(v8::Number::new(scope, 1.0).into());
    };
    let Some(size_algorithm) =
        strategy.get_index(scope, WRITABLE_STREAM_STRATEGY_SIZE_ALGORITHM_INDEX)
    else {
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

fn writable_stream_strategy_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<f64> {
    stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)?
        .get_index(scope, index)?
        .number_value(scope)
}

fn set_writable_stream_strategy_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    index: u32,
    value: f64,
) {
    if let Some(strategy) = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT) {
        let value = v8::Number::new(scope, value);
        let _ = strategy.set_index(scope, index, value.into());
    }
}

pub(in crate::context_bootstrap) fn writable_stream_close_internal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let plan = writable_stream_snapshot(scope, stream).plan_close();
    match plan {
        ClosePlan::RejectAlreadyRequested => {
            let message = v8::String::new(scope, "WritableStream close is already requested")?;
            return rejected_promise_value(scope, v8::Exception::type_error(scope, message));
        }
        ClosePlan::RejectTerminal => {
            let message =
                v8::String::new(scope, "Cannot close a closed or errored WritableStream")?;
            return rejected_promise_value(scope, v8::Exception::type_error(scope, message));
        }
        ClosePlan::RequestTransform
        | ClosePlan::RequestAndQueueForErroring
        | ClosePlan::RequestAndQueueSink => {}
    }
    set_writable_stream_close_requested(scope, stream, true);
    resolve_current_writer_ready(scope, stream);
    match plan {
        ClosePlan::RequestAndQueueForErroring => {
            let (promise, pending) = new_pending_read_promise(scope)?;
            enqueue_pending_transform_close(scope, stream, pending);
            let _ = advance_writable_stream_erroring(scope, stream);
            Some(promise.into())
        }
        ClosePlan::RequestTransform => {
            let readable = stream_slot_object(scope, stream, WRITABLE_STREAM_TARGET_READABLE_SLOT)?;
            match transform_stream_snapshot(scope, stream, readable).plan_close_admission() {
                TransformCloseAdmissionPlan::Queue => {
                    let (promise, pending) = new_pending_read_promise(scope)?;
                    enqueue_pending_transform_close(scope, stream, pending);
                    process_transform_writable_queue(scope, stream);
                    Some(promise.into())
                }
                TransformCloseAdmissionPlan::Run => {
                    set_transform_close_in_flight(scope, stream, true);
                    let result = perform_transform_stream_close(scope, stream, readable);
                    if result.is_none() {
                        mark_writable_stream_closed(scope, stream);
                    }
                    result
                }
            }
        }
        ClosePlan::RequestAndQueueSink => {
            let (promise, pending) = new_pending_read_promise(scope)?;
            enqueue_pending_transform_close(scope, stream, pending);
            let sink = stream_slot_object(scope, stream, WRITABLE_STREAM_SINK_SLOT)?;
            process_writable_sink_write_queue(scope, stream, sink);
            Some(promise.into())
        }
        ClosePlan::RejectAlreadyRequested | ClosePlan::RejectTerminal => unreachable!(),
    }
}

pub(in crate::context_bootstrap::stream_adapter) fn writable_stream_close_with_error_propagation<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    match writable_stream_snapshot(scope, stream).plan_close_with_error_propagation() {
        CloseWithErrorPropagationPlan::Resolve => {
            resolved_promise_value(scope, v8::undefined(scope).into())
                .expect("pipe destination close no-op must create a resolved promise")
        }
        CloseWithErrorPropagationPlan::RejectStoredError => {
            let error = writable_stream_stored_error(scope, stream)
                .expect("an errored pipe destination must retain its stored error");
            rejected_promise_value(scope, error)
                .expect("pipe destination close rejection must create a promise")
        }
        CloseWithErrorPropagationPlan::Close => writable_stream_close_internal(scope, stream)
            .unwrap_or_else(|| {
                resolved_promise_value(scope, v8::undefined(scope).into())
                    .expect("synchronous pipe destination close must create a resolved promise")
            }),
    }
}

pub(in crate::context_bootstrap) fn writable_stream_abort_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    writable_stream_abort_internal(scope, stream, reason)
}

fn perform_transform_stream_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let readable = v8::Global::new(scope, readable);
    with_transform_stream_relevant_realm(scope, stream, |scope, stream| {
        let readable = v8::Local::new(scope, &readable);
        perform_transform_stream_close_in_relevant_realm(scope, stream, readable)
    })
}

fn perform_transform_stream_close_in_relevant_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let claim =
        claim_transform_finish_residence(scope, stream, readable, FinishOperation::WritableClose)?;
    let (finish_promise, residence, algorithm) = match claim {
        TransformFinishResidenceClaim::Existing(promise) => {
            return attach_transform_writable_close_settlement(scope, stream, promise);
        }
        TransformFinishResidenceClaim::Started {
            promise,
            residence,
            algorithm,
        } => (promise, residence, algorithm),
    };
    let FinishAlgorithm::Flush(algorithm) = algorithm else {
        unreachable!("writable close must claim the transform flush algorithm")
    };
    let flush_result = match algorithm {
        TransformFlushAlgorithm::TextDecoder => {
            Ok(flush_text_decoder_stream(scope, stream, readable))
        }
        TransformFlushAlgorithm::Callback => {
            let transformer = stream_slot_object(scope, stream, WRITABLE_STREAM_TRANSFORMER_SLOT)?;
            let controller = stream_slot_object(scope, stream, WRITABLE_STREAM_CONTROLLER_SLOT)
                .unwrap_or_else(|| {
                    super::super::stream_objects::new_transform_stream_controller_object(
                        scope, readable, stream,
                    )
                });
            invoke_writable_stream_algorithm(
                scope,
                stream,
                transformer,
                WRITABLE_STREAM_ALGORITHM_TRANSFORM_FLUSH_INDEX,
                "flush",
                &[controller.into()],
            )
        }
        TransformFlushAlgorithm::None => Ok(None),
    };
    clear_transform_stream_terminal_algorithms(scope, stream);
    let Some(flush_promise) = transform_algorithm_result_promise(scope, flush_result) else {
        let reason = v8::undefined(scope).into();
        apply_transform_finish_setup_failure(
            scope,
            stream,
            readable,
            residence,
            FinishOperation::WritableClose,
            reason,
        );
        return attach_transform_writable_close_settlement(scope, stream, finish_promise);
    };
    attach_transform_sink_close_reactions(scope, flush_promise, stream, readable, residence);
    attach_transform_writable_close_settlement(scope, stream, finish_promise)
}

fn attach_transform_sink_close_reactions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    flush_promise: v8::Local<'s, v8::Promise>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    residence: v8::Local<'s, v8::Object>,
) {
    let data = v8::Array::new(scope, 3);
    let _ = data.set_index(scope, 0, writable.into());
    let _ = data.set_index(scope, 1, readable.into());
    let _ = data.set_index(scope, 2, residence.into());
    publish_required_stream_promise_reactions(
        scope,
        flush_promise,
        v8::Function::builder(transform_sink_close_fulfilled_callback).data(data.into()),
        "transform sink close fulfillment",
        v8::Function::builder(transform_sink_close_rejected_callback).data(data.into()),
        "transform sink close rejection",
        "transform sink close",
    )
    .finish_at_owner_boundary();
}

fn transform_sink_close_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((writable, readable, residence)) =
        transform_sink_close_reaction_values(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    match transform_stream_snapshot(scope, writable, readable)
        .plan_finish_settlement(FinishOperation::WritableClose, AlgorithmOutcome::Fulfilled)
    {
        FinishSettlementPlan::RejectWithReadableStoredError => {
            let error = readable_stream_error(scope, readable)
                .unwrap_or_else(|| v8::undefined(scope).into());
            reject_pending_read(scope, residence, error);
        }
        FinishSettlementPlan::CloseReadableAndResolve => {
            close_stream(scope, readable);
            resolve_pending_promise(scope, residence, v8::undefined(scope).into());
        }
        FinishSettlementPlan::Resolve => {
            resolve_pending_promise(scope, residence, v8::undefined(scope).into());
        }
        _ => unreachable!("transform flush fulfillment produced an invalid plan"),
    }
    rv.set_undefined();
}

fn transform_sink_close_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((writable, readable, residence)) =
        transform_sink_close_reaction_values(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    let error = args.get(0);
    match transform_stream_snapshot(scope, writable, readable)
        .plan_finish_settlement(FinishOperation::WritableClose, AlgorithmOutcome::Rejected)
    {
        FinishSettlementPlan::ErrorReadableWithCallbackErrorAndReject => {
            error_stream(scope, readable, error);
            reject_pending_read(scope, residence, error);
        }
        _ => unreachable!("transform flush rejection produced an invalid plan"),
    }
    rv.set_undefined();
}

fn transform_sink_close_reaction_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
)> {
    let data = v8::Local::<v8::Array>::try_from(data).ok()?;
    let writable = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let readable = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let residence = data
        .get_index(scope, 2)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    Some((writable, readable, residence))
}

fn attach_transform_writable_close_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    finish_promise: v8::Local<'s, v8::Promise>,
) -> Option<v8::Local<'s, v8::Value>> {
    match publish_required_stream_promise_reactions(
        scope,
        finish_promise,
        v8::Function::builder(transform_writable_close_fulfilled_callback).data(writable.into()),
        "transform writable close fulfillment",
        v8::Function::builder(transform_writable_close_rejected_callback).data(writable.into()),
        "transform writable close rejection",
        "transform writable close",
    ) {
        StreamOwnerPublication::Published(promise) => Some(promise.into()),
        StreamOwnerPublication::OwnerTerminating => None,
    }
}

fn transform_writable_close_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(writable) = v8::Local::<v8::Object>::try_from(args.data()) {
        let plan = stream_slot_object(scope, writable, WRITABLE_STREAM_TARGET_READABLE_SLOT)
            .map(|readable| {
                transform_stream_snapshot(scope, writable, readable)
                    .plan_writable_close_settlement(AlgorithmOutcome::Fulfilled)
            })
            .unwrap_or(WritableCloseSettlementPlan::MarkClosed);
        if matches!(plan, WritableCloseSettlementPlan::MarkClosed) {
            mark_writable_stream_closed(scope, writable);
        }
    }
    rv.set_undefined();
}

fn transform_writable_close_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(writable) = v8::Local::<v8::Object>::try_from(args.data()) {
        let plan = stream_slot_object(scope, writable, WRITABLE_STREAM_TARGET_READABLE_SLOT)
            .map(|readable| {
                transform_stream_snapshot(scope, writable, readable)
                    .plan_writable_close_settlement(AlgorithmOutcome::Rejected)
            })
            .unwrap_or(WritableCloseSettlementPlan::ClearInFlightAndErrorWritable);
        if matches!(
            plan,
            WritableCloseSettlementPlan::ClearInFlightAndErrorWritable
        ) {
            set_transform_close_in_flight(scope, writable, false);
            error_writable_stream_with_value(scope, writable, args.get(0));
        }
    }
    scope.throw_exception(args.get(0));
    rv.set_undefined();
}

fn transform_promise_reaction_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    let data = v8::Array::new(scope, 2);
    let _ = data.set_index(scope, 0, writable.into());
    let _ = data.set_index(scope, 1, readable.into());
    data
}

fn transform_promise_reaction_streams<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let data = v8::Local::<v8::Array>::try_from(data).ok()?;
    let writable = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let readable = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    Some((writable, readable))
}

fn transform_promise_reaction_write_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, f64)> {
    let data = v8::Local::<v8::Array>::try_from(data).ok()?;
    let writable = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let size = data.get_index(scope, 2)?.number_value(scope)?;
    Some((writable, size))
}

fn write_text_decoder_stream_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    bytes: &[u8],
) -> Option<v8::Local<'s, v8::Value>> {
    match decode_text_decoder_stream(scope, stream, bytes, true) {
        Ok(text) => {
            if !text.is_empty()
                && let Some(value) = v8_string(scope, &text)
            {
                let _ = enqueue_chunk(scope, readable, value.into());
            }
            None
        }
        Err(error) => reject_text_decoder_stream(scope, stream, readable, error),
    }
}

fn flush_text_decoder_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    match decode_text_decoder_stream(scope, stream, &[], false) {
        Ok(text) => {
            if !text.is_empty()
                && let Some(value) = v8_string(scope, &text)
            {
                let _ = enqueue_chunk(scope, readable, value.into());
            }
            None
        }
        Err(error) => reject_text_decoder_stream(scope, stream, readable, error),
    }
}

fn text_encoder_stream_chunk_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    chunk: v8::Local<'s, v8::Value>,
) -> Result<String, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    match chunk.to_string(&scope) {
        Some(value) => Ok(value.to_rust_string_lossy(&scope)),
        None if scope.has_caught() => Err(scope.exception().unwrap_or_else(|| {
            let message = v8str(&scope, "Failed to convert chunk");
            v8::Exception::type_error(&scope, message)
        })),
        None => {
            let message = v8str(&scope, "Failed to convert chunk");
            Err(v8::Exception::type_error(&scope, message))
        }
    }
}

fn decode_text_decoder_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    bytes: &[u8],
    stream_decode: bool,
) -> Result<String, TextDecodeError> {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return Ok(String::new());
    };
    let Some(decoder_id) = text_decoder_stream_decoder_id(scope, stream) else {
        return Ok(String::new());
    };
    unsafe { &mut *host_ptr }
        .text_codecs_mut()
        .decode(decoder_id, bytes, stream_decode)
}

fn text_decoder_stream_decoder_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    TextCodecStore::decoder_id_from_object(scope, stream)
}

fn reject_text_decoder_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    error: TextDecodeError,
) -> Option<v8::Local<'s, v8::Value>> {
    let error = text_decode_error_value(scope, error);
    reject_transform_stream_with_value(scope, stream, readable, error)
}

fn reject_transform_stream_with_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    error_transform_stream_with_value(scope, stream, readable, error);
    rejected_promise_value(scope, error)
}

pub(in crate::context_bootstrap) fn error_transform_stream_with_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    let plan = transform_stream_snapshot(scope, stream, readable).plan_error();
    let _ = apply_transform_error_plan(scope, stream, readable, error, plan);
}

fn apply_transform_error_plan<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    provided: v8::Local<'s, v8::Value>,
    plan: moli_streams::transform::TransformErrorPlan,
) -> v8::Local<'s, v8::Value> {
    let error = match plan.reason() {
        ErrorReasonSource::Provided => provided,
        ErrorReasonSource::ReadableStored => {
            readable_stream_error(scope, readable).unwrap_or(provided)
        }
    };
    if matches!(plan.readable(), ReadableErrorAction::Error) {
        error_stream(scope, readable, error);
    }
    error_writable_stream_with_value(scope, writable, error);
    error
}

pub(in crate::context_bootstrap) fn terminate_transform_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
) {
    let plan = transform_stream_snapshot(scope, stream, readable).plan_terminate();
    if matches!(plan.readable(), ReadableTerminateAction::Close) {
        let _ = close_stream(scope, readable);
    }
    let error = v8::Exception::type_error(
        scope,
        v8str(scope, "The transform stream has been terminated"),
    );
    error_writable_stream_with_value(scope, stream, error);
}

pub(in crate::context_bootstrap) fn error_writable_stream_with_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    match writable_stream_snapshot(scope, stream).plan_error() {
        ErrorPlan::Ignore => {}
        ErrorPlan::Start { finish_immediately } => {
            set_writable_stream_erroring(scope, stream, true);
            if let Some(strategy) = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)
            {
                let _ =
                    strategy.set_index(scope, WRITABLE_STREAM_STRATEGY_STORED_ERROR_INDEX, error);
            }
            reject_current_writer_ready(scope, stream, error);
            if finish_immediately {
                let _ = advance_writable_stream_erroring(scope, stream);
            }
        }
    }
}

fn deal_with_writable_stream_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    match writable_stream_snapshot(scope, stream).state() {
        WritableState::Writable => error_writable_stream_with_value(scope, stream, error),
        WritableState::Erroring => {
            let _ = advance_writable_stream_erroring(scope, stream);
        }
        WritableState::Closed | WritableState::Errored => {}
    }
}

/// Advances the two-phase writable error transition after a start or in-flight
/// operation barrier may have cleared. Returns true when the stream was already
/// in the error lifecycle, so callers must not continue normal queue pumping.
fn advance_writable_stream_erroring<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    match writable_stream_snapshot(scope, stream).plan_finish_erroring() {
        FinishErroringPlan::Ignore => false,
        FinishErroringPlan::Wait => true,
        plan @ (FinishErroringPlan::FinishWithoutAbort
        | FinishErroringPlan::FinishAndRejectAbort
        | FinishErroringPlan::FinishAndRunAbort) => {
            finish_writable_stream_erroring(scope, stream, plan);
            true
        }
    }
}

fn finish_writable_stream_erroring<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    plan: FinishErroringPlan,
) {
    let error =
        writable_stream_stored_error(scope, stream).unwrap_or_else(|| v8::undefined(scope).into());
    set_writable_stream_erroring(scope, stream, false);
    set_writable_stream_errored(scope, stream, true);
    reject_pending_transform_writes(scope, stream, error);

    match plan {
        FinishErroringPlan::FinishWithoutAbort => {
            project_writable_stream_terminal_error(scope, stream, error);
        }
        FinishErroringPlan::FinishAndRejectAbort => {
            let record = take_writable_stream_pending_abort(scope, stream)
                .expect("an already-erroring abort plan must retain its request");
            let residence = writable_pending_abort_residence(scope, record)
                .expect("a pending abort request must retain its residence");
            reject_pending_read(scope, residence, error);
            project_writable_stream_terminal_error(scope, stream, error);
        }
        FinishErroringPlan::FinishAndRunAbort => {
            let record = take_writable_stream_pending_abort(scope, stream)
                .expect("an abort-initiated error plan must retain its request");
            let residence = writable_pending_abort_residence(scope, record)
                .expect("a pending abort request must retain its residence");
            let reason = writable_pending_abort_reason(scope, record);
            run_writable_stream_abort_algorithm(scope, stream, residence, reason);
        }
        FinishErroringPlan::Ignore | FinishErroringPlan::Wait => {
            unreachable!("only a finish plan may complete writable erroring")
        }
    }
}

fn project_writable_stream_terminal_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    if let Some(close) = take_writable_stream_pending_close(scope, stream) {
        reject_pending_read(scope, close, error);
    }
    reject_current_writer_closed(scope, stream, error);
    reject_writable_stream_pipe_owner(scope, stream, error);
}

fn mark_writable_stream_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let CloseSettlementPlan::Close {
        clear_stored_error,
        resolve_pending_abort,
    } = writable_stream_snapshot(scope, stream)
        .erroring()
        .plan_close_settlement(CloseOutcome::Fulfilled)
    else {
        unreachable!("a fulfilled in-flight close must produce a close settlement")
    };
    set_transform_close_in_flight(scope, stream, false);
    set_writable_stream_erroring(scope, stream, false);
    set_writable_stream_errored(scope, stream, false);
    if resolve_pending_abort {
        let record = take_writable_stream_pending_abort(scope, stream)
            .expect("an in-flight close must retain its pending abort request");
        let residence = writable_pending_abort_residence(scope, record)
            .expect("a pending abort request must retain its residence");
        resolve_pending_promise(scope, residence, v8::undefined(scope).into());
    }
    if let Some(strategy) = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)
        && clear_stored_error
    {
        let _ = strategy.set_index(
            scope,
            WRITABLE_STREAM_STRATEGY_STORED_ERROR_INDEX,
            v8::undefined(scope).into(),
        );
    }
    set_stream_slot_bool(scope, stream, WRITABLE_STREAM_CLOSED_SLOT, true);
    resolve_current_writer_closed(scope, stream);
}

pub(in crate::context_bootstrap) fn writable_stream_stored_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    if writable_stream_erroring(scope, stream) || writable_stream_errored(scope, stream) {
        return stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)
            .and_then(|strategy| {
                strategy.get_index(scope, WRITABLE_STREAM_STRATEGY_STORED_ERROR_INDEX)
            })
            .or_else(|| Some(v8::undefined(scope).into()));
    }
    None
}

fn writable_stream_erroring<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)
        .and_then(|strategy| strategy.get_index(scope, WRITABLE_STREAM_STRATEGY_ERRORING_INDEX))
        .is_some_and(|value| value.boolean_value(scope))
}

pub(in crate::context_bootstrap) fn writable_stream_errored<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)
        .and_then(|strategy| strategy.get_index(scope, WRITABLE_STREAM_STRATEGY_ERRORED_INDEX))
        .is_some_and(|value| value.boolean_value(scope))
}

pub(in crate::context_bootstrap) fn writable_stream_close_requested<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)
        .and_then(|strategy| {
            strategy.get_index(scope, WRITABLE_STREAM_STRATEGY_CLOSE_REQUESTED_INDEX)
        })
        .is_some_and(|value| value.boolean_value(scope))
}

fn set_writable_stream_close_requested<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: bool,
) {
    if let Some(strategy) = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT) {
        let _ = strategy.set_index(
            scope,
            WRITABLE_STREAM_STRATEGY_CLOSE_REQUESTED_INDEX,
            v8::Boolean::new(scope, value).into(),
        );
    }
}

fn set_writable_stream_errored<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: bool,
) {
    if let Some(strategy) = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT) {
        let _ = strategy.set_index(
            scope,
            WRITABLE_STREAM_STRATEGY_ERRORED_INDEX,
            v8::Boolean::new(scope, value).into(),
        );
    }
}

fn set_writable_stream_erroring<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: bool,
) {
    if let Some(strategy) = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT) {
        let _ = strategy.set_index(
            scope,
            WRITABLE_STREAM_STRATEGY_ERRORING_INDEX,
            v8::Boolean::new(scope, value).into(),
        );
    }
}

fn text_decode_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: TextDecodeError,
) -> v8::Local<'s, v8::Value> {
    v8::Exception::type_error(scope, v8str(scope, error.message()))
}

fn text_decoder_stream_type_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    v8::Exception::type_error(
        scope,
        v8str(scope, "TextDecoderStream chunk must be a BufferSource"),
    )
}

pub(in crate::context_bootstrap) fn writable_stream_abort_internal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    if matches!(
        writable_stream_snapshot(scope, stream).state(),
        WritableState::Closed | WritableState::Errored
    ) {
        return resolved_promise_value(scope, v8::undefined(scope).into());
    }
    abort_writable_stream_controller_signal(scope, stream, reason);
    match writable_stream_snapshot(scope, stream).plan_abort() {
        AbortPlan::Resolve => resolved_promise_value(scope, v8::undefined(scope).into()),
        AbortPlan::ReusePending => {
            writable_stream_pending_abort_promise(scope, stream).map(Into::into)
        }
        AbortPlan::CreatePending {
            was_already_erroring,
            start_erroring,
        } => {
            let (promise, residence) = new_pending_read_promise(scope)?;
            let stored_reason = if was_already_erroring {
                v8::undefined(scope).into()
            } else {
                reason
            };
            store_writable_stream_pending_abort(
                scope,
                stream,
                promise,
                residence,
                stored_reason,
                was_already_erroring,
            );
            if start_erroring {
                error_writable_stream_with_value(scope, stream, reason);
            } else {
                let _ = advance_writable_stream_erroring(scope, stream);
            }
            Some(promise.into())
        }
    }
}

fn run_writable_stream_abort_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    residence: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let result = match writable_stream_snapshot(scope, stream).kind() {
        WritableKind::Transform => {
            let Some(readable) =
                stream_slot_object(scope, stream, WRITABLE_STREAM_TARGET_READABLE_SLOT)
            else {
                panic!("a transform writable must retain its readable side during abort");
            };
            transform_stream_sink_abort_algorithm(scope, stream, readable, reason)
                .or_else(|| resolved_promise_value(scope, v8::undefined(scope).into()))
        }
        WritableKind::Sink => {
            let sink = stream_slot_object(scope, stream, WRITABLE_STREAM_SINK_SLOT)
                .expect("a sink writable must retain its underlying sink during abort");
            let webidl_callback = writable_stream_has_webidl_algorithms(scope, stream);
            let abort_result = invoke_writable_stream_algorithm(
                scope,
                stream,
                sink,
                WRITABLE_STREAM_ALGORITHM_SINK_ABORT_INDEX,
                "abort",
                &[reason],
            );
            match abort_result {
                Ok(result) if webidl_callback => {
                    normalize_stream_algorithm_result(scope, result).map(Into::into)
                }
                Ok(Some(result)) if result.is_promise() => Some(result),
                Ok(_) => resolved_promise_value(scope, v8::undefined(scope).into()),
                Err(error) => rejected_promise_value(scope, error),
            }
        }
    }
    .expect("a required writable abort algorithm must produce a promise");

    let promise = v8::Local::<v8::Promise>::try_from(result)
        .expect("a normalized writable abort result must be a promise");
    let data = v8::Array::new(scope, 2);
    let _ = data.set_index(scope, 0, stream.into());
    let _ = data.set_index(scope, 1, residence.into());
    publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(writable_abort_fulfilled_callback).data(data.into()),
        "writable abort fulfillment",
        v8::Function::builder(writable_abort_rejected_callback).data(data.into()),
        "writable abort rejection",
        "writable abort",
    )
    .finish_at_owner_boundary();
}

fn writable_abort_reaction_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let data = v8::Local::<v8::Array>::try_from(value).ok()?;
    let stream = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let residence = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    Some((stream, residence))
}

fn writable_abort_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let (stream, residence) = writable_abort_reaction_parts(scope, args.data())
        .expect("writable abort fulfillment must retain stream and residence");
    resolve_pending_promise(scope, residence, v8::undefined(scope).into());
    let error =
        writable_stream_stored_error(scope, stream).unwrap_or_else(|| v8::undefined(scope).into());
    project_writable_stream_terminal_error(scope, stream, error);
    rv.set_undefined();
}

fn writable_abort_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let (stream, residence) = writable_abort_reaction_parts(scope, args.data())
        .expect("writable abort rejection must retain stream and residence");
    reject_pending_read(scope, residence, args.get(0));
    let error =
        writable_stream_stored_error(scope, stream).unwrap_or_else(|| v8::undefined(scope).into());
    project_writable_stream_terminal_error(scope, stream, error);
    rv.set_undefined();
}

fn writable_stream_pending_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)?
        .get_index(scope, WRITABLE_STREAM_STRATEGY_PENDING_CLOSE_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn take_writable_stream_pending_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let residence = writable_stream_pending_close(scope, stream)?;
    let strategy = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)?;
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_PENDING_CLOSE_INDEX,
        v8::undefined(scope).into(),
    );
    Some(residence)
}

fn writable_stream_pending_abort_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)?
        .get_index(scope, WRITABLE_STREAM_STRATEGY_PENDING_ABORT_INDEX)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn writable_stream_pending_abort_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> PendingAbortState {
    let Some(record) = writable_stream_pending_abort_record(scope, stream) else {
        return PendingAbortState::None;
    };
    if record
        .get_index(scope, WRITABLE_PENDING_ABORT_ALREADY_ERRORING_INDEX)
        .is_some_and(|value| value.boolean_value(scope))
    {
        PendingAbortState::AlreadyErroring
    } else {
        PendingAbortState::InitiatedErroring
    }
}

fn writable_stream_pending_abort_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    writable_stream_pending_abort_record(scope, stream)?
        .get_index(scope, WRITABLE_PENDING_ABORT_PROMISE_INDEX)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
}

fn writable_pending_abort_residence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Object>> {
    record
        .get_index(scope, WRITABLE_PENDING_ABORT_RESIDENCE_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn writable_pending_abort_reason<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Value> {
    record
        .get_index(scope, WRITABLE_PENDING_ABORT_REASON_INDEX)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn store_writable_stream_pending_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    promise: v8::Local<'s, v8::Promise>,
    residence: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
    was_already_erroring: bool,
) {
    let record = v8::Array::new(scope, 4);
    let _ = record.set_index(scope, WRITABLE_PENDING_ABORT_PROMISE_INDEX, promise.into());
    let _ = record.set_index(
        scope,
        WRITABLE_PENDING_ABORT_RESIDENCE_INDEX,
        residence.into(),
    );
    let _ = record.set_index(scope, WRITABLE_PENDING_ABORT_REASON_INDEX, reason);
    let _ = record.set_index(
        scope,
        WRITABLE_PENDING_ABORT_ALREADY_ERRORING_INDEX,
        v8::Boolean::new(scope, was_already_erroring).into(),
    );
    let strategy = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)
        .expect("an initialized writable stream must retain strategy storage");
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_PENDING_ABORT_INDEX,
        record.into(),
    );
}

fn take_writable_stream_pending_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let record = writable_stream_pending_abort_record(scope, stream)?;
    let strategy = stream_slot_array(scope, stream, WRITABLE_STREAM_STRATEGY_SLOT)?;
    let _ = strategy.set_index(
        scope,
        WRITABLE_STREAM_STRATEGY_PENDING_ABORT_INDEX,
        v8::undefined(scope).into(),
    );
    Some(record)
}

fn abort_writable_stream_controller_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(controller) = stream_slot_object(scope, stream, WRITABLE_STREAM_CONTROLLER_SLOT)
    else {
        return;
    };
    let Some(signal) = stream_slot_object(scope, controller, STREAM_CONTROLLER_SIGNAL_SLOT) else {
        return;
    };
    let signal_reason = if reason.is_undefined() {
        crate::context_bootstrap::new_dom_exception_value(
            scope,
            "This operation was aborted",
            "AbortError",
        )
    } else {
        reason
    };
    if let Some(signal) = crate::abort_signal_route::ResolvedAbortSignal::resolve(scope, signal) {
        signal.abort(scope, signal_reason);
    }
}

pub(in crate::context_bootstrap) fn register_writable_stream_pipe_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) {
    set_required_stream_slot_value(
        scope,
        stream,
        WRITABLE_STREAM_PIPE_OWNER_SLOT,
        owner.into(),
        "writable pipe owner registration",
    );
}

fn writable_stream_pipe_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    stream_slot_object(scope, stream, WRITABLE_STREAM_PIPE_OWNER_SLOT)
        .filter(|value| !value.is_null_or_undefined())
}

fn schedule_writable_pipe_owner_drain<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = writable_stream_pipe_owner(scope, stream) else {
        return;
    };
    super::pipe::schedule_pipe_owner_drain(scope, owner);
}

pub(in crate::context_bootstrap::stream_adapter) fn clear_writable_stream_pipe_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) {
    if writable_stream_pipe_owner(scope, stream)
        .is_some_and(|current| current.strict_equals(owner.into()))
    {
        set_required_stream_slot_value(
            scope,
            stream,
            WRITABLE_STREAM_PIPE_OWNER_SLOT,
            v8::null(scope).into(),
            "writable pipe owner cleanup",
        );
    }
}

fn reject_writable_stream_pipe_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(owner) = writable_stream_pipe_owner(scope, stream) else {
        return;
    };
    super::pipe::destination_pipe_errored(scope, owner, reason);
}
