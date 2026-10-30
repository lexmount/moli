use super::*;
use moli_streams::transfer::{
    WritableTransferMessageErrorPlan, WritableTransferMessagePlan, WritableTransferTerminalPlan,
    WritableTransferWritePlan,
};
use protocol::{MessageKind, parts, post_to_port_handling_error};

pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    payload: &WritableStreamClonePayload,
) -> Option<()> {
    let Some(port) = ensure_message_port_wrapper_for_id(scope, payload.port_id) else {
        payload.discard_port(scope);
        return None;
    };
    let initialized = initialize_with_port(scope, stream, port);
    if initialized.is_none() {
        payload.discard_port(scope);
    }
    initialized
}

fn initialize_with_port<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    port: v8::Local<'s, v8::Object>,
) -> Option<()> {
    // The payload owner owns rollback for the whole private channel. Do not
    // publish ordinary MessagePort close semantics from a partial shell.
    let state = writable_state::create(scope, stream, port);
    let sink = v8::Object::new(scope);
    install_sink_algorithm(scope, sink, "write", write_callback, state)?;
    install_sink_algorithm(scope, sink, "close", close_callback, state)?;
    install_sink_algorithm(scope, sink, "abort", abort_callback, state)?;

    let (onmessage, onmessageerror) =
        build_transfer_port_handlers(scope, state, message_callback, messageerror_callback)?;
    initialize_writable_stream_object(scope, stream, Some(sink), 1.0, None);
    install_transfer_port_handlers(scope, port, onmessage, onmessageerror);
    Some(())
}

fn install_sink_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
    name: &'static str,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    state: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let StreamOwnerPublication::Published(function) = build_required_stream_callback(
        scope,
        v8::Function::builder(callback).data(state.into()),
        "transferred writable sink algorithm",
    ) else {
        return None;
    };
    sink.set(scope, v8str(scope, name).into(), function.into())
        .filter(|stored| *stored)
        .map(|_| ())
}

fn write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = callback_state(args.data()) else {
        return;
    };
    let chunk = args.get(0);
    match writable_state::snapshot(scope, state).plan_write() {
        WritableTransferWritePlan::PostChunk { pull_demand_after } => {
            writable_state::set_pull_demand(scope, state, pull_demand_after);
            if let Err(error) = post_from_state(scope, state, MessageKind::Chunk, chunk) {
                writable_state::finish(scope, state);
                set_rejected_result(scope, &mut rv, error);
            }
        }
        WritableTransferWritePlan::WaitForPull => {
            let Some((promise, pending)) = new_pending_read_promise(scope) else {
                panic!("transferred writable backpressure promise allocation failed")
            };
            writable_state::stage_write(scope, state, chunk, pending);
            rv.set(promise.into());
        }
        WritableTransferWritePlan::RejectInactive => {
            let error = new_dom_exception_value(
                scope,
                "Transferred WritableStream channel is closed.",
                "DataCloneError",
            );
            set_rejected_result(scope, &mut rv, error);
        }
        WritableTransferWritePlan::RejectConcurrentWrite => {
            panic!("WritableStream sink invoked a concurrent transferred write")
        }
    }
}

fn close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = callback_state(args.data()) else {
        return;
    };
    if matches!(
        writable_state::snapshot(scope, state).plan_terminal(),
        WritableTransferTerminalPlan::PostAndFinish
    ) {
        let result = post_from_state(
            scope,
            state,
            MessageKind::Close,
            v8::undefined(scope).into(),
        );
        writable_state::finish(scope, state);
        if let Err(error) = result {
            set_rejected_result(scope, &mut rv, error);
        }
    }
}

fn abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = callback_state(args.data()) else {
        return;
    };
    if matches!(
        writable_state::snapshot(scope, state).plan_terminal(),
        WritableTransferTerminalPlan::PostAndFinish
    ) {
        let result = post_from_state(scope, state, MessageKind::Error, args.get(0));
        writable_state::finish(scope, state);
        if let Err(error) = result {
            set_rejected_result(scope, &mut rv, error);
        }
    }
}

fn message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = callback_state(args.data()) else {
        rv.set_undefined();
        return;
    };
    let message = parts(scope, args.get(0));
    let kind = message.as_ref().map(|(kind, _)| *kind);
    match writable_state::snapshot(scope, state).plan_message(kind) {
        WritableTransferMessagePlan::IgnoreLateMessage => {}
        WritableTransferMessagePlan::RecordPull { pull_demand } => {
            writable_state::set_pull_demand(scope, state, pull_demand);
        }
        WritableTransferMessagePlan::PostPendingChunk => {
            let (chunk, pending) = writable_state::take_pending_write(scope, state)
                .expect("a pending transferred write must retain its chunk and promise");
            match post_from_state(scope, state, MessageKind::Chunk, chunk) {
                Ok(()) => resolve_pending_promise(scope, pending, v8::undefined(scope).into()),
                Err(error) => {
                    reject_pending_read(scope, pending, error);
                    writable_state::finish(scope, state);
                }
            }
        }
        WritableTransferMessagePlan::ErrorStreamAndFinish => {
            let reason = message
                .map(|(_, value)| value)
                .unwrap_or_else(|| v8::undefined(scope).into());
            error_local_stream_and_pending(scope, state, reason);
            writable_state::finish(scope, state);
        }
        WritableTransferMessagePlan::FailProtocol => {
            let error = new_dom_exception_value(
                scope,
                "Invalid transferred WritableStream control message.",
                "DataCloneError",
            );
            let _ = post_from_state(scope, state, MessageKind::Error, error);
            error_local_stream_and_pending(scope, state, error);
            writable_state::finish(scope, state);
        }
    }
    rv.set_undefined();
}

fn messageerror_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = callback_state(args.data()) else {
        rv.set_undefined();
        return;
    };
    if matches!(
        writable_state::snapshot(scope, state).plan_message_error(),
        WritableTransferMessageErrorPlan::ErrorStreamAndFinish
    ) {
        let error = new_dom_exception_value(
            scope,
            "Failed to deserialize transferred WritableStream control message.",
            "DataCloneError",
        );
        if let Some(port) = writable_state::port(scope, state) {
            let _ = protocol::post_to_port(scope, port, MessageKind::Error, error);
        }
        error_local_stream_and_pending(scope, state, error);
        writable_state::finish(scope, state);
    }
    rv.set_undefined();
}

fn error_local_stream_and_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    if let Some(stream) = writable_state::stream(scope, state) {
        error_writable_stream_with_value(scope, stream, error);
    }
    if let Some((_, pending)) = writable_state::take_pending_write(scope, state) {
        reject_pending_read(scope, pending, error);
    }
}

fn post_from_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    kind: MessageKind,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    let Some(port) = writable_state::port(scope, state) else {
        return Err(new_dom_exception_value(
            scope,
            "Transferred WritableStream lost its MessagePort.",
            "DataCloneError",
        ));
    };
    post_to_port_handling_error(scope, port, kind, value)
}

fn callback_state<'s>(value: v8::Local<'s, v8::Value>) -> Option<v8::Local<'s, v8::Object>> {
    v8::Local::<v8::Object>::try_from(value).ok()
}

fn set_rejected_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    error: v8::Local<'s, v8::Value>,
) {
    if let Some(promise) = rejected_promise_value(scope, error) {
        rv.set(promise);
    }
}
