use super::helpers::{
    WebSocketCloseInfo, add_buffered_amount, set_websocket_ready_state,
    throw_websocket_close_error, throw_websocket_dom_exception, websocket_id,
    websocket_ready_state,
};
use super::payload::{WebSocketSendPayload, websocket_send_payload};
use super::*;
use crate::webidl;
use moli_websocket::validate_websocket_close_request;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebSocket.close")]
struct WebSocketCloseArgs {
    #[webidl(converter = "clamped_unsigned_short")]
    code: Option<u16>,
    #[webidl(converter = "usv_string")]
    reason: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebSocketStream.close")]
struct WebSocketStreamCloseArgs {
    #[webidl(dictionary)]
    info: WebSocketCloseInfo,
}

pub(super) fn websocket_send_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let ready_state = websocket_ready_state(scope, args.this());
    if (ready_state - CONNECTING).abs() < f64::EPSILON {
        throw_websocket_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "Failed to execute 'send' on 'WebSocket': Still in CONNECTING state.",
        );
        return;
    }
    if args.length() < 1 {
        throw_type_error(
            scope,
            "Failed to execute 'send' on 'WebSocket': 1 argument required, but only 0 present.",
        );
        return;
    }
    if (ready_state - CLOSING).abs() < f64::EPSILON || (ready_state - CLOSED).abs() < f64::EPSILON {
        rv.set_undefined();
        return;
    }
    let payload = websocket_send_payload(scope, &args);
    let amount = payload.len() as f64;
    add_buffered_amount(scope, args.this(), amount);
    if let Some(socket_id) = websocket_id(scope, args.this())
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        let host = unsafe { &mut *host_ptr };
        let sent = match payload {
            WebSocketSendPayload::Text(text) => host.send_websocket_text(socket_id, text),
            WebSocketSendPayload::Binary(bytes) => host.send_websocket_binary(socket_id, bytes),
            WebSocketSendPayload::Blob(data) => host.send_websocket_blob(socket_id, data),
        };
        if !matches!(sent, Some(Ok(()))) {
            add_buffered_amount(scope, args.this(), -amount);
            set_websocket_ready_state(scope, args.this(), CLOSED);
        }
    } else if let Some(socket_id) = websocket_id(scope, args.this()) {
        let sent = match payload {
            WebSocketSendPayload::Text(text) => {
                crate::worker::send_worker_websocket_text(scope, socket_id, text)
            }
            WebSocketSendPayload::Binary(bytes) => {
                crate::worker::send_worker_websocket_binary(scope, socket_id, bytes)
            }
            WebSocketSendPayload::Blob(data) => {
                crate::worker::send_worker_websocket_blob(scope, socket_id, data)
            }
        };
        if !matches!(sent, Some(true)) {
            add_buffered_amount(scope, args.this(), -amount);
            set_websocket_ready_state(scope, args.this(), CLOSED);
        }
    }
    rv.set_undefined();
}

pub(super) fn websocket_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let ready_state = websocket_ready_state(scope, args.this());
    if (ready_state - CLOSING).abs() < f64::EPSILON || (ready_state - CLOSED).abs() < f64::EPSILON {
        rv.set_undefined();
        return;
    }
    let Some(parsed) = webidl::parse_args::<WebSocketCloseArgs>(scope, &args) else {
        return;
    };
    let reason = parsed.reason.unwrap_or_default();
    let close = match validate_websocket_close_request(parsed.code, reason) {
        Ok(close) => close,
        Err(error) => {
            throw_websocket_close_error(scope, "execute 'close' on 'WebSocket'", error);
            return;
        }
    };
    set_websocket_ready_state(scope, args.this(), CLOSING);
    if let Some(socket_id) = websocket_id(scope, args.this())
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        let host = unsafe { &mut *host_ptr };
        if !host.close_websocket(socket_id, close.code, close.reason) {
            set_websocket_ready_state(scope, args.this(), CLOSED);
        }
    } else if let Some(socket_id) = websocket_id(scope, args.this()) {
        match crate::worker::close_worker_websocket(scope, socket_id, close.code, close.reason) {
            Some(false) | None => set_websocket_ready_state(scope, args.this(), CLOSED),
            Some(true) => {}
        }
    } else {
        set_websocket_ready_state(scope, args.this(), CLOSED);
    }
    rv.set_undefined();
}

pub(super) fn websocket_stream_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<WebSocketStreamCloseArgs>(scope, &args) else {
        return;
    };
    let close = match parsed.info.normalize() {
        Ok(close) => close,
        Err(error) => {
            throw_websocket_close_error(scope, "execute 'close' on 'WebSocketStream'", error);
            return;
        }
    };
    if let Some(socket_id) = websocket_id(scope, args.this())
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        let host = unsafe { &mut *host_ptr };
        let _ = host.close_websocket(socket_id, close.code, close.reason);
    }
    rv.set_undefined();
}
