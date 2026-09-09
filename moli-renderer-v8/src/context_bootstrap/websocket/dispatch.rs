use super::events::{
    dispatch_named_event, new_close_event, new_message_event, simple_event_object,
    websocket_binary_message_data, websocket_message_origin,
};
use super::helpers::{add_buffered_amount, set_websocket_ready_state, set_websocket_string_slot};
use super::stream::{dispatch_websocket_stream_event, is_websocket_stream_object};
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebSocketDispatchResult {
    Noop,
    Dispatched,
    Backpressured,
}

impl WebSocketDispatchResult {
    pub(crate) fn dispatched(self) -> bool {
        matches!(self, Self::Dispatched)
    }
}

pub(crate) fn dispatch_websocket_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    event: &WebSocketEvent,
) -> WebSocketDispatchResult {
    if is_websocket_stream_object(scope, socket) {
        return dispatch_websocket_stream_event(scope, socket, event);
    }
    match event {
        WebSocketEvent::HandshakeResponse { .. } => WebSocketDispatchResult::Noop,
        WebSocketEvent::Open {
            protocol,
            extensions,
            ..
        } => {
            set_websocket_ready_state(scope, socket, OPEN);
            set_websocket_string_slot(scope, socket, WEBSOCKET_PROTOCOL_SLOT, protocol);
            set_websocket_string_slot(scope, socket, WEBSOCKET_EXTENSIONS_SLOT, extensions);
            dispatch_named_event(scope, socket, "open").into()
        }
        WebSocketEvent::TextMessage { data, .. } => {
            let Some(data) = v8_string(scope, data).map(|value| value.into()) else {
                return WebSocketDispatchResult::Noop;
            };
            let origin = websocket_message_origin(scope, socket);
            let Some(event) = new_message_event(scope, data, &origin) else {
                return WebSocketDispatchResult::Noop;
            };
            dispatch_simple_event_target_event(
                scope,
                socket,
                WEBSOCKET_LISTENERS_SLOT,
                "message",
                event,
            )
            .into()
        }
        WebSocketEvent::BinaryMessage { data, .. } => {
            let Some(data) = websocket_binary_message_data(scope, socket, data.clone()) else {
                return WebSocketDispatchResult::Noop;
            };
            let origin = websocket_message_origin(scope, socket);
            let Some(event) = new_message_event(scope, data, &origin) else {
                return WebSocketDispatchResult::Noop;
            };
            dispatch_simple_event_target_event(
                scope,
                socket,
                WEBSOCKET_LISTENERS_SLOT,
                "message",
                event,
            )
            .into()
        }
        WebSocketEvent::SendCompleted { payload_length, .. } => {
            add_buffered_amount(scope, socket, -(*payload_length as f64));
            WebSocketDispatchResult::Noop
        }
        WebSocketEvent::Error { message, .. } => {
            set_websocket_ready_state(scope, socket, CLOSED);
            if !message.is_empty() {
                define_non_enumerable_string_property(scope, socket, "__moliLastError", message);
            }
            dispatch_named_event(scope, socket, "error").into()
        }
        WebSocketEvent::Closing { .. } => {
            set_websocket_ready_state(scope, socket, CLOSING);
            WebSocketDispatchResult::Noop
        }
        WebSocketEvent::Close {
            code,
            reason,
            was_clean,
            ..
        } => {
            set_websocket_ready_state(scope, socket, CLOSED);
            let event = new_close_event(scope, *code, reason, *was_clean)
                .unwrap_or_else(|| simple_event_object(scope, "close"));
            dispatch_simple_event_target_event(
                scope,
                socket,
                WEBSOCKET_LISTENERS_SLOT,
                "close",
                event,
            )
            .into()
        }
    }
}

impl From<bool> for WebSocketDispatchResult {
    fn from(value: bool) -> Self {
        if value { Self::Dispatched } else { Self::Noop }
    }
}
