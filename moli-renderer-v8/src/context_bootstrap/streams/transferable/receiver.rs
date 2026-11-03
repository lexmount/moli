use super::*;
use crate::context_bootstrap::stream_adapter::{
    enqueue_byte_chunk, finish_readable_byte_stream_source_close,
};
use moli_streams::transfer::{
    ReceiverCancelPlan, ReceiverEnqueueOutcome, ReceiverEnqueuePlan, ReceiverMessageErrorPlan,
    ReceiverMessagePlan, ReceiverPullPlan,
};
use protocol::{MessageKind, parts, post_handling_error};

pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    payload: &ReadableStreamClonePayload,
) -> Option<()> {
    initialize_mode(scope, stream, payload, false)
}

pub(super) fn initialize_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    payload: &ReadableStreamClonePayload,
    byte_stream: bool,
) -> Option<()> {
    let Some(port) = ensure_message_port_wrapper_for_id(scope, payload.port_id) else {
        payload.discard_port(scope);
        return None;
    };
    let initialized = initialize_with_port(scope, stream, port, byte_stream);
    if initialized.is_none() {
        payload.discard_port(scope);
    }
    initialized
}

pub(super) fn new_from_port<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let stream = new_readable_stream_shell_object(scope);
    initialize_with_port(scope, stream, port, false)?;
    Some(stream)
}

fn initialize_with_port<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    port: v8::Local<'s, v8::Object>,
    byte_stream: bool,
) -> Option<()> {
    // Keep construction free of local close side effects. The payload owner or
    // prepared-transfer owner must discard the complete private channel if any
    // required V8 machinery below cannot be published.
    if byte_stream {
        initialize_webidl_readable_stream_object(scope, stream, None, true, 0.0, None);
    } else {
        initialize_readable_stream_object(scope, stream, None, 0.0, None);
    }
    let state = state::create(scope, stream, port);
    let (onmessage, onmessageerror) =
        build_transfer_port_handlers(scope, state, message_callback, messageerror_callback)?;
    install_algorithms(scope, stream, port, state)?;
    install_transfer_port_handlers(scope, port, onmessage, onmessageerror);
    Some(())
}

fn install_algorithms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    port: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let controller = stream_slot_object(scope, stream, READABLE_STREAM_CONTROLLER_SLOT)?;
    let algorithms = stream_slot_array(scope, controller, STREAM_CONTROLLER_ALGORITHMS_SLOT)?;
    let StreamOwnerPublication::Published(pull) = build_required_stream_callback(
        scope,
        v8::Function::builder(pull_callback).data(state.into()),
        "transferred readable pull algorithm",
    ) else {
        return None;
    };
    let StreamOwnerPublication::Published(cancel) = build_required_stream_callback(
        scope,
        v8::Function::builder(cancel_callback).data(state.into()),
        "transferred readable cancel algorithm",
    ) else {
        return None;
    };
    let _ = algorithms.set_index(scope, READABLE_STREAM_ALGORITHM_SOURCE_INDEX, port.into());
    let _ = algorithms.set_index(scope, READABLE_STREAM_ALGORITHM_PULL_INDEX, pull.into());
    let _ = algorithms.set_index(scope, READABLE_STREAM_ALGORITHM_CANCEL_INDEX, cancel.into());
    Some(())
}

fn message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(state) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(stream) = state::stream(scope, state) else {
        rv.set_undefined();
        return;
    };
    let Some((kind, value)) = parts(scope, args.get(0)) else {
        if matches!(
            state::snapshot(scope, state).plan_receiver_message(None),
            ReceiverMessagePlan::FailProtocol
        ) {
            fail_protocol(scope, state, "Invalid transferred stream chunk message.");
        }
        rv.set_undefined();
        return;
    };
    match state::snapshot(scope, state).plan_receiver_message(Some(kind)) {
        ReceiverMessagePlan::IgnoreLateMessage => {}
        ReceiverMessagePlan::EnqueueChunk => {
            if readable_stream_is_byte_stream(scope, stream)
                && v8::Local::<v8::Uint8Array>::try_from(value)
                    .is_ok_and(|chunk| chunk.byte_length() == 0)
            {
                // Empty upload chunks are not EOF and cannot be enqueued by
                // byte controllers. Retain the read and request another chunk.
                let undefined = v8::undefined(scope).into();
                if let Err(error) = post_handling_error(scope, state, MessageKind::Pull, undefined)
                {
                    error_stream(scope, stream, error);
                    state::finish(scope, state);
                }
                rv.set_undefined();
                return;
            }
            let result = if readable_stream_is_byte_stream(scope, stream) {
                enqueue_byte_chunk(scope, stream, value)
            } else {
                enqueue_chunk(scope, stream, value)
            };
            let outcome = match result {
                Ok(()) => ReceiverEnqueueOutcome::Enqueued,
                Err(EnqueueChunkError::ClosedOrErrored) => ReceiverEnqueueOutcome::StreamTerminal,
                Err(EnqueueChunkError::Strategy(error)) => {
                    error_stream(scope, stream, error);
                    ReceiverEnqueueOutcome::StrategyError
                }
            };
            if matches!(outcome.plan(), ReceiverEnqueuePlan::Finish) {
                state::finish(scope, state);
            }
        }
        ReceiverMessagePlan::CloseStreamAndFinish => {
            let _ = close_stream(scope, stream);
            if let Err(error) = finish_readable_byte_stream_source_close(scope, stream) {
                error_stream(scope, stream, error);
            }
            state::finish(scope, state);
        }
        ReceiverMessagePlan::ErrorStreamAndFinish => {
            error_stream(scope, stream, value);
            state::finish(scope, state);
        }
        ReceiverMessagePlan::FailProtocol => {
            fail_protocol(
                scope,
                state,
                "Unexpected pull message on transferred stream receiver.",
            );
        }
    }
    rv.set_undefined();
}

fn fail_protocol<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    message: &str,
) {
    if let Some(stream) = state::stream(scope, state) {
        let error = new_dom_exception_value(scope, message, "DataCloneError");
        error_stream(scope, stream, error);
    }
    state::finish(scope, state);
}

fn messageerror_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(state) = v8::Local::<v8::Object>::try_from(args.data())
        && matches!(
            state::snapshot(scope, state).plan_receiver_message_error(),
            ReceiverMessageErrorPlan::ErrorStreamAndFinish
        )
        && let Some(stream) = state::stream(scope, state)
    {
        let error = new_dom_exception_value(
            scope,
            "Failed to deserialize transferred stream chunk.",
            "DataCloneError",
        );
        let _ = protocol::post(scope, state, MessageKind::Error, error);
        error_stream(scope, stream, error);
        state::finish(scope, state);
    }
    rv.set_undefined();
}

fn pull_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(state) = v8::Local::<v8::Object>::try_from(args.data())
        && matches!(
            state::snapshot(scope, state).plan_receiver_pull(),
            ReceiverPullPlan::PostPull
        )
    {
        let undefined = v8::undefined(scope).into();
        if let Err(error) = post_handling_error(scope, state, MessageKind::Pull, undefined) {
            state::finish(scope, state);
            if let Some(promise) = rejected_promise_value(scope, error) {
                rv.set(promise);
                return;
            }
            if let Some(stream) = state::stream(scope, state) {
                error_stream(scope, stream, error);
            }
        }
    }
    rv.set_undefined();
}

fn cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(state) = v8::Local::<v8::Object>::try_from(args.data())
        && matches!(
            state::snapshot(scope, state).plan_receiver_cancel(),
            ReceiverCancelPlan::PostErrorAndFinish
        )
    {
        let reason = args.get(0);
        let result = post_handling_error(scope, state, MessageKind::Error, reason);
        state::finish(scope, state);
        if let Err(error) = result
            && let Some(promise) = rejected_promise_value(scope, error)
        {
            rv.set(promise);
            return;
        }
    }
    rv.set_undefined();
}
