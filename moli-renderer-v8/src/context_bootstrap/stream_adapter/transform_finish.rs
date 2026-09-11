//! TransformStream readable cancel, writable abort, and flush/close algorithms.
//!
//! A controller claims its shared finish promise before invoking user code.
//! Each reaction reads the state it needs when the algorithm's promise settles.
//! Writable queue completion remains in `writable`.

mod context;

use context::{TransformFinishContext, TransformFinishWithReason};
use moli_streams::readable::ReadableState;
use moli_streams::transform::{
    FinishAlgorithm, FinishClaimPlan, FinishOperation, FinishSetupFailurePlan,
    TransformCancelAlgorithm, TransformFlushAlgorithm,
};
use moli_streams::writable::WritableState;

use super::readable_state::{
    close_stream, error_stream, readable_stream_error, readable_stream_snapshot,
};
use super::utils::{
    StreamOwnerPublication, new_pending_read_promise, publish_required_stream_promise_reactions,
    reject_pending_read, rejected_promise_value, resolve_pending_promise,
};
use super::writable::{
    attach_transform_writable_close_settlement, error_writable_stream_with_value,
    flush_text_decoder_stream, invoke_writable_stream_algorithm, normalize_stream_algorithm_result,
    transform_stream_snapshot, with_transform_stream_relevant_realm, writable_stream_snapshot,
    writable_stream_stored_error,
};
use super::{
    TRANSFORM_STREAM_CONTROLLER_FINISH_PROMISE_SLOT,
    TRANSFORM_STREAM_CONTROLLER_FINISH_RESIDENCE_SLOT,
    WRITABLE_STREAM_ALGORITHM_TRANSFORM_CANCEL_INDEX,
    WRITABLE_STREAM_ALGORITHM_TRANSFORM_FLUSH_INDEX, WRITABLE_STREAM_ALGORITHM_TRANSFORM_INDEX,
    WRITABLE_STREAM_ALGORITHMS_SLOT, WRITABLE_STREAM_CONTROLLER_SLOT,
    WRITABLE_STREAM_TARGET_READABLE_SLOT, WRITABLE_STREAM_TRANSFORMER_SLOT, set_stream_slot_value,
    stream_slot_array, stream_slot_object, stream_slot_value,
};

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

pub(super) fn transform_stream_readable_cancel_callback<'s>(
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
    attach_transform_source_cancel_reactions(
        scope,
        cancel_promise,
        writable,
        readable,
        residence,
        reason,
    );
    rv.set(finish_promise.into());
}

fn attach_transform_source_cancel_reactions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cancel_promise: v8::Local<'s, v8::Promise>,
    writable: v8::Local<'s, v8::Object>,
    readable: v8::Local<'s, v8::Object>,
    residence: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let StreamOwnerPublication::Published(data) = (TransformFinishWithReason {
        finish: TransformFinishContext {
            writable,
            readable,
            residence,
        },
        reason,
    })
    .into_callback_data(scope) else {
        return;
    };
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
    let StreamOwnerPublication::Published(TransformFinishWithReason { finish, reason }) =
        TransformFinishWithReason::from_callback_data(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    // Observe the state when the cancel promise fulfills, including errors
    // that occurred while an asynchronous cancel callback was pending.
    if writable_stream_snapshot(scope, finish.writable).state() == WritableState::Errored {
        let error = writable_stream_stored_error(scope, finish.writable).unwrap_or(reason);
        reject_pending_read(scope, finish.residence, error);
    } else {
        error_writable_stream_with_value(scope, finish.writable, reason);
        resolve_pending_promise(scope, finish.residence, v8::undefined(scope).into());
    }
    rv.set_undefined();
}

fn transform_source_cancel_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let StreamOwnerPublication::Published(context) =
        TransformFinishWithReason::from_callback_data(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    let error = args.get(0);
    error_writable_stream_with_value(scope, context.finish.writable, error);
    reject_pending_read(scope, context.finish.residence, error);
    rv.set_undefined();
}

pub(super) fn transform_stream_sink_abort_algorithm<'s>(
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
    let StreamOwnerPublication::Published(data) = (TransformFinishWithReason {
        finish: TransformFinishContext {
            writable,
            readable,
            residence,
        },
        reason,
    })
    .into_callback_data(scope) else {
        return Some(finish_promise.into());
    };
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
    let StreamOwnerPublication::Published(TransformFinishWithReason { finish, reason }) =
        TransformFinishWithReason::from_callback_data(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    if readable_stream_snapshot(scope, finish.readable).state() == ReadableState::Errored {
        let error = readable_stream_error(scope, finish.readable).unwrap_or(reason);
        reject_pending_read(scope, finish.residence, error);
    } else {
        error_stream(scope, finish.readable, reason);
        resolve_pending_promise(scope, finish.residence, v8::undefined(scope).into());
    }
    rv.set_undefined();
}

fn transform_sink_abort_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let StreamOwnerPublication::Published(context) =
        TransformFinishWithReason::from_callback_data(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    let error = args.get(0);
    error_stream(scope, context.finish.readable, error);
    reject_pending_read(scope, context.finish.residence, error);
    rv.set_undefined();
}

pub(super) fn perform_transform_stream_close<'s>(
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
    let StreamOwnerPublication::Published(data) = (TransformFinishContext {
        writable,
        readable,
        residence,
    })
    .into_callback_data(scope) else {
        return;
    };
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
    let StreamOwnerPublication::Published(context) =
        TransformFinishContext::from_callback_data(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    match readable_stream_snapshot(scope, context.readable).state() {
        ReadableState::Errored => {
            let error = readable_stream_error(scope, context.readable)
                .unwrap_or_else(|| v8::undefined(scope).into());
            reject_pending_read(scope, context.residence, error);
        }
        ReadableState::Readable => {
            close_stream(scope, context.readable);
            resolve_pending_promise(scope, context.residence, v8::undefined(scope).into());
        }
        ReadableState::Closed => {
            resolve_pending_promise(scope, context.residence, v8::undefined(scope).into());
        }
    }
    rv.set_undefined();
}

fn transform_sink_close_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let StreamOwnerPublication::Published(context) =
        TransformFinishContext::from_callback_data(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    let error = args.get(0);
    error_stream(scope, context.readable, error);
    reject_pending_read(scope, context.residence, error);
    rv.set_undefined();
}
