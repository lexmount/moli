use super::*;
use moli_streams::transfer::{
    SenderDrainPlan, SenderFailurePlan, SenderMessagePlan, SenderReadFulfillmentPlan,
    SenderReadReactionPlan, SenderReadRejectionPlan, SenderReadResultSnapshot, SenderStartReadPlan,
};
use protocol::{MessageKind, parts, post};

pub(super) fn start_read<'s>(scope: &mut v8::PinScope<'s, '_>, state: v8::Local<'s, v8::Object>) {
    if matches!(
        state::snapshot(scope, state).plan_sender_start_read(),
        SenderStartReadPlan::Ignore
    ) {
        return;
    }
    let Some(stream) = state::stream(scope, state) else {
        return;
    };
    let Some(promise) = read_from_stream_as_promise(scope, stream) else {
        return;
    };
    if matches!(
        publish_required_stream_promise_reactions(
            scope,
            promise,
            v8::Function::builder(read_fulfilled_callback).data(state.into()),
            "transferred readable sender fulfillment",
            v8::Function::builder(read_rejected_callback).data(state.into()),
            "transferred readable sender rejection",
            "transferred readable sender read",
        ),
        StreamOwnerPublication::OwnerTerminating
    ) {
        return;
    }
    // `then2` only queues reactions; publishing the in-flight bit after the
    // attachment cannot race the callbacks and avoids a false owner when V8
    // abandons publication for worker teardown.
    state::set_read_in_flight(scope, state, true);
}

fn read_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(state) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    state::set_read_in_flight(scope, state, false);
    if matches!(
        state::snapshot(scope, state).plan_sender_read_reaction(),
        SenderReadReactionPlan::Ignore
    ) {
        rv.set_undefined();
        return;
    }
    let Ok(result) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        let plan = state::snapshot(scope, state)
            .plan_sender_read_fulfilled(SenderReadResultSnapshot::new(false, false));
        apply_read_fulfillment(scope, state, plan, None);
        rv.set_undefined();
        return;
    };
    let done = result
        .get(scope, v8str(scope, "done").into())
        .is_some_and(|value| value.boolean_value(scope));
    if done {
        let plan = state::snapshot(scope, state)
            .plan_sender_read_fulfilled(SenderReadResultSnapshot::new(true, true));
        apply_read_fulfillment(scope, state, plan, None);
        rv.set_undefined();
        return;
    }
    let value = result
        .get(scope, v8str(scope, "value").into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    let plan = state::snapshot(scope, state)
        .plan_sender_read_fulfilled(SenderReadResultSnapshot::new(true, false));
    apply_read_fulfillment(scope, state, plan, Some(value));
    rv.set_undefined();
}

fn apply_read_fulfillment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    plan: SenderReadFulfillmentPlan,
    value: Option<v8::Local<'s, v8::Value>>,
) {
    match plan {
        SenderReadFulfillmentPlan::Ignore => {}
        SenderReadFulfillmentPlan::FailInvalidResult => {
            let error = state::type_error(scope, "Invalid stream read result");
            fail(scope, state, error);
        }
        SenderReadFulfillmentPlan::PostCloseAndFinish => {
            let undefined = v8::undefined(scope).into();
            let _ = post(scope, state, MessageKind::Close, undefined);
            state::finish(scope, state);
        }
        SenderReadFulfillmentPlan::StageChunk => {
            let Some(value) = value else {
                unreachable!("staging a transfer chunk requires a read value")
            };
            state::stage_chunk(scope, state, value);
        }
        SenderReadFulfillmentPlan::PostChunk { pull_demand_after } => {
            let Some(value) = value else {
                unreachable!("posting a transfer chunk requires a read value")
            };
            state::set_pull_demand(scope, state, pull_demand_after);
            match post(scope, state, MessageKind::Chunk, value) {
                Ok(()) => start_read(scope, state),
                Err(error) => {
                    let error = error.into_value(scope);
                    fail(scope, state, error);
                }
            }
        }
    }
}

fn read_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(state) = v8::Local::<v8::Object>::try_from(args.data()) {
        state::set_read_in_flight(scope, state, false);
        if matches!(
            state::snapshot(scope, state).plan_sender_read_rejected(),
            SenderReadRejectionPlan::PostErrorAndFinish
        ) {
            let _ = post(scope, state, MessageKind::Error, args.get(0));
            state::finish(scope, state);
        }
    }
    rv.set_undefined();
}

pub(super) fn message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(state) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some((kind, value)) = parts(scope, args.get(0)) else {
        if matches!(
            state::snapshot(scope, state).plan_sender_message(None),
            SenderMessagePlan::FailProtocol
        ) {
            let error = new_dom_exception_value(
                scope,
                "Invalid transferred stream control message.",
                "DataCloneError",
            );
            fail(scope, state, error);
        }
        rv.set_undefined();
        return;
    };
    match state::snapshot(scope, state).plan_sender_message(Some(kind)) {
        SenderMessagePlan::IgnoreLateMessage => {}
        SenderMessagePlan::RecordPull { pull_demand, drain } => {
            state::set_pull_demand(scope, state, pull_demand);
            apply_drain(scope, state, drain);
        }
        SenderMessagePlan::CancelSourceAndFinish => {
            state::set_active(scope, state, false);
            close_port_and_cancel_source(scope, state, value);
        }
        SenderMessagePlan::FailProtocol => {
            let error = new_dom_exception_value(
                scope,
                "Unexpected data message on transferred stream sender.",
                "DataCloneError",
            );
            fail(scope, state, error);
        }
    }
    rv.set_undefined();
}

pub(super) fn messageerror_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(state) = v8::Local::<v8::Object>::try_from(args.data()) {
        let error = new_dom_exception_value(
            scope,
            "Failed to deserialize transferred stream control message.",
            "DataCloneError",
        );
        fail(scope, state, error);
    }
    rv.set_undefined();
}

fn apply_drain<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    plan: SenderDrainPlan,
) {
    match plan {
        SenderDrainPlan::Ignore => {}
        SenderDrainPlan::StartRead => start_read(scope, state),
        SenderDrainPlan::PostStagedChunk { pull_demand_after } => {
            let Some(chunk) = state::take_staged_chunk(scope, state) else {
                start_read(scope, state);
                return;
            };
            state::set_pull_demand(scope, state, pull_demand_after);
            match post(scope, state, MessageKind::Chunk, chunk) {
                Ok(()) => start_read(scope, state),
                Err(error) => {
                    let error = error.into_value(scope);
                    fail(scope, state, error);
                }
            }
        }
    }
}

fn fail<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    match state::snapshot(scope, state).plan_sender_failure() {
        SenderFailurePlan::Ignore => {}
        SenderFailurePlan::PostErrorCancelSourceAndFinish => {
            state::set_active(scope, state, false);
            let _ = post(scope, state, MessageKind::Error, error);
            close_port_and_cancel_source(scope, state, error);
        }
    }
}

fn close_port_and_cancel_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    // Both terminal paths deactivate before reaching this helper. Release the
    // channel before cancellation can reenter or terminate the owning realm.
    state::close_port(scope, state);
    if let Some(stream) = state::stream(scope, state)
        && let Some(promise) = cancel_readable_stream(scope, stream, reason)
    {
        suppress_promise_unhandled_rejection(scope, promise);
    }
}
