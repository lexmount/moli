//! Stream commands use the existing connection owned by their current realm.

use super::payload::WebSocketSendPayload;
use super::*;

pub(super) fn send_websocket_stream_payload(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
    payload: WebSocketSendPayload,
) -> Option<bool> {
    if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host };
        match payload {
            WebSocketSendPayload::Text(text) => host.send_websocket_text(socket_id, text),
            WebSocketSendPayload::Binary(bytes) => host.send_websocket_binary(socket_id, bytes),
            WebSocketSendPayload::Blob(data) => host.send_websocket_blob(socket_id, data),
        }
        .map(|result| result.is_ok())
    } else {
        match payload {
            WebSocketSendPayload::Text(text) => {
                crate::worker::send_worker_websocket_text(scope, socket_id, text)
            }
            WebSocketSendPayload::Binary(bytes) => {
                crate::worker::send_worker_websocket_binary(scope, socket_id, bytes)
            }
            WebSocketSendPayload::Blob(data) => {
                crate::worker::send_worker_websocket_blob(scope, socket_id, data)
            }
        }
    }
}

pub(super) fn close_websocket_stream_transport(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
    code: Option<u16>,
    reason: String,
) {
    if let Some(stream) = websocket_stream_transport_wrapper(scope, socket_id) {
        super::helpers::set_websocket_value_slot(
            scope,
            stream,
            WEBSOCKET_STREAM_CLOSE_REQUESTED_SLOT,
            v8::Boolean::new(scope, true).into(),
        );
    }
    if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host };
        let _ = host.close_websocket(socket_id, code, reason);
        host.signal_websocket_stream_pull(socket_id);
    } else {
        let _ = crate::worker::close_worker_websocket(scope, socket_id, code, reason);
        // A cancelled readable may have left an owned frame parked. Resume the
        // exact socket so it can discard that frame and observe its Close task.
        crate::worker::signal_worker_websocket_stream_pull(scope, socket_id);
    }
}

pub(super) fn websocket_stream_transport_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket_id: u64,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host }
            .websocket_dispatch_target(scope, socket_id)
            .map(|(_, _, _, wrapper)| wrapper)
    } else {
        crate::worker::worker_websocket_wrapper(scope, socket_id)
    }
}
