use super::*;
pub(super) use moli_streams::transfer::TransferMessageKind as MessageKind;

pub(super) enum MessageError<'s> {
    Exception(v8::Local<'s, v8::Value>),
    State(&'static str),
}

impl<'s> MessageError<'s> {
    pub(super) fn into_value(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        match self {
            Self::Exception(error) => error,
            Self::State(message) => state::type_error(scope, message),
        }
    }
}

pub(super) fn parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Value>,
) -> Option<(MessageKind, v8::Local<'s, v8::Value>)> {
    let event = v8::Local::<v8::Object>::try_from(event).ok()?;
    let data = crate::context_bootstrap::event_backing(scope, event)
        .get(scope, v8str(scope, "data").into())?;
    let envelope = v8::Local::<v8::Array>::try_from(data).ok()?;
    let kind = MessageKind::try_from(envelope.get_index(scope, 0)?.uint32_value(scope)?).ok()?;
    let value = envelope
        .get_index(scope, 1)
        .unwrap_or_else(|| v8::undefined(scope).into());
    Some((kind, value))
}

pub(super) fn post<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    kind: MessageKind,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), MessageError<'s>> {
    let port = state::port(scope, state).ok_or(MessageError::State("missing transfer port"))?;
    post_to_port(scope, port, kind, value)
}

pub(super) fn post_to_port<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    kind: MessageKind,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), MessageError<'s>> {
    // Blink only treats a caught exception from MessagePort::postMessage as
    // PackAndPostMessageHandlingError. V8 termination is not catchable, so a
    // worker context disappearing while a transfer callback is packing a
    // control message must not be converted into an Error message for the
    // peer. The worker task boundary owns the actual teardown.
    if scope.is_execution_terminating() || crate::worker::worker_termination_requested(scope) {
        return Ok(());
    }
    let port_id = message_port_id_from_object(scope, port)
        .ok_or(MessageError::State("missing transfer port id"))?;
    let envelope = v8::Array::new(scope, 2);
    let _ = envelope.set_index(
        scope,
        0,
        v8::Integer::new_from_unsigned(scope, kind.wire_code()).into(),
    );
    let _ = envelope.set_index(scope, 1, value);
    let payload: V8StructuredClonePayload = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        match crate::structured_clone::serialize_for_wire_for_runtime(&mut scope, envelope.into()) {
            Some(payload) => payload,
            None if scope.has_terminated()
                || scope.is_execution_terminating()
                || crate::worker::worker_termination_requested(&mut scope) =>
            {
                return Ok(());
            }
            None if scope.has_caught() => {
                let error = scope
                    .exception()
                    .unwrap_or_else(|| v8::undefined(&scope).into());
                scope.reset();
                return Err(MessageError::Exception(error));
            }
            None => {
                return Err(MessageError::State("failed to serialize transfer message"));
            }
        }
    };
    if scope.is_execution_terminating() || crate::worker::worker_termination_requested(scope) {
        return Ok(());
    }
    let registry = current_message_port_registry(scope)
        .ok_or(MessageError::State("missing message port registry"))?;
    // A terminal message may already have disentangled the peer while older
    // messages are still queued for delivery. A late pull/cancel in that
    // window is a benign race, not a protocol failure.
    if let Some(peer_id) = registry.enqueue_message_to_message_port(port_id, payload) {
        schedule_message_port_delivery(scope, peer_id);
    }
    Ok(())
}

/// Post one cross-realm stream message and, if packing fails, notify the peer
/// with the clone error before returning that same error to the algorithm.
///
/// This is the Streams Standard's `PackAndPostMessageHandlingError`: the
/// initiating pull/cancel promise rejects while the opposite side is also
/// terminated with the serialization failure.
pub(super) fn post_handling_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    kind: MessageKind,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    match post(scope, state, kind, value) {
        Ok(()) => Ok(()),
        Err(error) => {
            let error = error.into_value(scope);
            let _ = post(scope, state, MessageKind::Error, error);
            Err(error)
        }
    }
}

pub(super) fn post_to_port_handling_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port: v8::Local<'s, v8::Object>,
    kind: MessageKind,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    match post_to_port(scope, port, kind, value) {
        Ok(()) => Ok(()),
        Err(error) => {
            let error = error.into_value(scope);
            let _ = post_to_port(scope, port, MessageKind::Error, error);
            Err(error)
        }
    }
}
