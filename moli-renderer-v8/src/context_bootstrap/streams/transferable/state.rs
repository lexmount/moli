use super::*;
use moli_streams::transfer::{TransferFinishPlan, TransferSnapshot};

const STREAM_SLOT: &str = "__moliReadableStreamTransferStream";
const PORT_SLOT: &str = "__moliReadableStreamTransferPort";
const STAGED_CHUNK_SLOT: &str = "__moliReadableStreamTransferStagedChunk";
const READ_IN_FLIGHT_SLOT: &str = "__moliReadableStreamTransferReadInFlight";
const PULL_DEMAND_SLOT: &str = "__moliReadableStreamTransferPullDemand";
const ACTIVE_SLOT: &str = "__moliReadableStreamTransferActive";

/// Create the state for one side of one transfer channel.
///
/// This state must not live on the `ReadableStream` itself. A transferred
/// stream can be transferred again, in which case it is simultaneously the
/// receiver for the previous channel and the sender for the next one. Each
/// role needs its own port and lifecycle state.
pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    port: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let initial = TransferSnapshot::initial();
    let state = v8::Object::new(scope);
    set_stream_slot_value(scope, state, STREAM_SLOT, stream.into());
    set_stream_slot_value(scope, state, PORT_SLOT, port.into());
    set_stream_slot_value(scope, state, STAGED_CHUNK_SLOT, v8::null(scope).into());
    set_stream_slot_bool(scope, state, READ_IN_FLIGHT_SLOT, initial.read_in_flight());
    set_stream_slot_value(
        scope,
        state,
        PULL_DEMAND_SLOT,
        v8::Integer::new_from_unsigned(scope, initial.pull_demand()).into(),
    );
    set_stream_slot_bool(scope, state, ACTIVE_SLOT, initial.active());
    state
}

pub(super) fn stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    stream_slot_object(scope, state, STREAM_SLOT)
}

pub(super) fn port<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    stream_slot_object(scope, state, PORT_SLOT)
}

pub(super) fn snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> TransferSnapshot {
    TransferSnapshot::new(
        stream_slot_bool(scope, state, ACTIVE_SLOT).unwrap_or(false),
        stream_slot_bool(scope, state, READ_IN_FLIGHT_SLOT).unwrap_or(false),
        stream_slot_number(scope, state, PULL_DEMAND_SLOT)
            .map(|value| value as u32)
            .unwrap_or(0),
        staged_chunk(scope, state).is_some(),
    )
}

pub(super) fn set_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    active: bool,
) {
    set_stream_slot_bool(scope, state, ACTIVE_SLOT, active);
}

pub(super) fn set_read_in_flight<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    in_flight: bool,
) {
    set_stream_slot_bool(scope, state, READ_IN_FLIGHT_SLOT, in_flight);
}

pub(super) fn stage_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let staged = v8::Array::new(scope, 1);
    let _ = staged.set_index(scope, 0, value);
    set_stream_slot_value(scope, state, STAGED_CHUNK_SLOT, staged.into());
}

pub(super) fn staged_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    stream_slot_array(scope, state, STAGED_CHUNK_SLOT)?.get_index(scope, 0)
}

pub(super) fn take_staged_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let value = staged_chunk(scope, state)?;
    set_stream_slot_value(scope, state, STAGED_CHUNK_SLOT, v8::null(scope).into());
    Some(value)
}

pub(super) fn set_pull_demand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    demand: u32,
) {
    set_stream_slot_value(
        scope,
        state,
        PULL_DEMAND_SLOT,
        v8::Integer::new_from_unsigned(scope, demand).into(),
    );
}

pub(super) fn finish<'s>(scope: &mut v8::PinScope<'s, '_>, state: v8::Local<'s, v8::Object>) {
    if matches!(
        snapshot(scope, state).plan_finish(),
        TransferFinishPlan::DeactivateAndClosePort
    ) {
        set_active(scope, state, false);
        close_port(scope, state);
        request_body::remove_abort(scope, state);
    }
}

pub(super) fn close_port<'s>(scope: &mut v8::PinScope<'s, '_>, state: v8::Local<'s, v8::Object>) {
    let Some(port) = port(scope, state) else {
        return;
    };
    if let Some(port_id) = message_port_id_from_object(scope, port)
        && let Some(registry) = current_message_port_registry(scope)
    {
        registry.close_message_port(port_id);
    }
    close_message_port_object(scope, port);
}

pub(super) fn type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    let message = v8::String::new(scope, message).expect("transfer TypeError message");
    v8::Exception::type_error(scope, message)
}
