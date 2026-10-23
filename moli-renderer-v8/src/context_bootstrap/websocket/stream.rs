use super::helpers::{
    set_websocket_value_slot, websocket_dom_exception_value, websocket_id, websocket_object_slot,
    websocket_value_slot,
};
use super::payload::{WebSocketSendPayload, websocket_stream_send_payload_from_value};
use super::*;
use crate::context_bootstrap::constructors::{
    new_websocket_error_value, websocket_error_close_info,
};
use crate::context_bootstrap::stream_adapter::{
    new_lazy_readable_stream_object, new_writable_stream_object, readable_stream_queue_total_size,
};
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain)]
struct WebSocketStreamOpenInfoDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    readable: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    writable: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    protocol: Option<v8::Local<'scope, v8::String>>,
    #[webapi(data_property, enumerable)]
    extensions: Option<v8::Local<'scope, v8::String>>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct WebSocketStreamWritableSinkDeclaration<'scope> {
    #[webapi(slot = WEBSOCKET_ID_SLOT)]
    socket_id: f64,
    #[webapi(slot = WEBSOCKET_STREAM_SINK_CLOSED_PROMISE_SLOT)]
    closed: Option<v8::Local<'scope, v8::Value>>,
    #[webapi(slot = WEBSOCKET_STREAM_PENDING_WRITES_SLOT, init = "array")]
    pending_writes: (),
    #[webapi(method, callback = websocket_stream_sink_write_callback, length = 1)]
    write: (),
    #[webapi(method, callback = websocket_stream_sink_close_callback, length = 0)]
    close: (),
    #[webapi(method, callback = websocket_stream_sink_abort_callback, length = 1)]
    abort: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct WebSocketStreamReadableSourceDeclaration {
    #[webapi(slot = WEBSOCKET_ID_SLOT)]
    socket_id: f64,
    #[webapi(method, callback = websocket_stream_source_pull_callback, length = 0)]
    pull: (),
    #[webapi(method, callback = websocket_stream_source_cancel_callback, length = 1)]
    cancel: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct WebSocketStreamPendingWriteDeclaration<'scope> {
    #[webapi(slot = WEBSOCKET_STREAM_PROMISE_RESOLVER_SLOT)]
    resolver: v8::Local<'scope, v8::PromiseResolver>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct WebSocketStreamCloseInfoDeclaration<'scope> {
    close_code: f64,
    reason: Option<v8::Local<'scope, v8::String>>,
}

pub(super) fn dispatch_websocket_stream_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    event: &WebSocketEvent,
) -> WebSocketDispatchResult {
    match event {
        WebSocketEvent::HandshakeResponse { .. } => WebSocketDispatchResult::Noop,
        WebSocketEvent::Open {
            socket_id,
            protocol,
            extensions,
            ..
        } => {
            let source = new_websocket_stream_readable_source(scope, *socket_id);
            let readable = new_lazy_readable_stream_object(scope, source, 1.0, None);
            let sink = new_websocket_stream_writable_sink(scope, stream, *socket_id);
            let writable = new_writable_stream_object(scope, Some(sink), 1.0, None);
            set_websocket_value_slot(
                scope,
                stream,
                WEBSOCKET_STREAM_READABLE_SLOT,
                readable.into(),
            );
            set_websocket_value_slot(
                scope,
                stream,
                WEBSOCKET_STREAM_WRITABLE_SLOT,
                writable.into(),
            );
            let open_info = WebSocketStreamOpenInfoDeclaration::new(
                readable,
                writable,
                v8_string(scope, protocol),
                v8_string(scope, extensions),
            )
            .bind(scope)
            .expect("WebSocketStream open info declaration should bind");
            resolve_websocket_stream_promise(
                scope,
                stream,
                WEBSOCKET_STREAM_OPENED_RESOLVER_SLOT,
                open_info.into(),
            );
            WebSocketDispatchResult::Dispatched
        }
        WebSocketEvent::TextMessage { data, .. } => {
            let Some(readable) =
                websocket_object_slot(scope, stream, WEBSOCKET_STREAM_READABLE_SLOT)
            else {
                return WebSocketDispatchResult::Noop;
            };
            if !websocket_stream_readable_can_accept_message(scope, readable) {
                return WebSocketDispatchResult::Backpressured;
            };
            let Some(value) = v8_string(scope, data) else {
                return WebSocketDispatchResult::Noop;
            };
            let _ = enqueue_chunk(scope, readable, value.into());
            WebSocketDispatchResult::Dispatched
        }
        WebSocketEvent::BinaryMessage { data, .. } => {
            let Some(readable) =
                websocket_object_slot(scope, stream, WEBSOCKET_STREAM_READABLE_SLOT)
            else {
                return WebSocketDispatchResult::Noop;
            };
            if !websocket_stream_readable_can_accept_message(scope, readable) {
                return WebSocketDispatchResult::Backpressured;
            };
            let Some(value) = new_uint8_array_from_bytes(scope, data.clone()) else {
                return WebSocketDispatchResult::Noop;
            };
            let _ = enqueue_chunk(scope, readable, value.into());
            WebSocketDispatchResult::Dispatched
        }
        WebSocketEvent::SendCompleted { .. } => {
            if resolve_next_websocket_stream_pending_write(scope, stream) {
                WebSocketDispatchResult::Dispatched
            } else {
                WebSocketDispatchResult::Noop
            }
        }
        WebSocketEvent::Error { message, .. } => {
            let error = new_websocket_error_object(scope, message, Some(1006), "");
            set_websocket_value_slot(scope, stream, WEBSOCKET_STREAM_ERROR_SLOT, error);
            if let Some(readable) =
                websocket_object_slot(scope, stream, WEBSOCKET_STREAM_READABLE_SLOT)
            {
                error_stream(scope, readable, error);
            }
            if let Some(writable) =
                websocket_object_slot(scope, stream, WEBSOCKET_STREAM_WRITABLE_SLOT)
            {
                reject_all_websocket_stream_pending_writes(scope, writable, error);
                error_writable_stream_with_value(scope, writable, error);
            }
            reject_websocket_stream_promise(
                scope,
                stream,
                WEBSOCKET_STREAM_OPENED_RESOLVER_SLOT,
                error,
            );
            reject_websocket_stream_promise(
                scope,
                stream,
                WEBSOCKET_STREAM_CLOSED_RESOLVER_SLOT,
                error,
            );
            WebSocketDispatchResult::Dispatched
        }
        WebSocketEvent::Closing { .. } => WebSocketDispatchResult::Noop,
        WebSocketEvent::Close {
            code,
            reason,
            was_clean,
            ..
        } => {
            if *was_clean {
                let mut had_interrupted_write = false;
                if let Some(readable) =
                    websocket_object_slot(scope, stream, WEBSOCKET_STREAM_READABLE_SLOT)
                {
                    close_stream(scope, readable);
                }
                if let Some(writable) =
                    websocket_object_slot(scope, stream, WEBSOCKET_STREAM_WRITABLE_SLOT)
                {
                    had_interrupted_write = websocket_stream_has_interrupted_write(scope, writable);
                    let error = websocket_dom_exception_value(
                        scope,
                        "InvalidStateError",
                        11,
                        "The WebSocketStream writable is closed.",
                    );
                    reject_all_websocket_stream_pending_writes(scope, writable, error);
                    error_writable_stream_with_value(scope, writable, error);
                }
                if had_interrupted_write {
                    // WPT treats remote close that interrupts a queued write as a
                    // WebSocketStream close error, while the interrupted write and
                    // later writes reject with InvalidStateError from the writable.
                    let error = new_websocket_error_object(scope, "", Some(*code), reason);
                    reject_websocket_stream_promise(
                        scope,
                        stream,
                        WEBSOCKET_STREAM_CLOSED_RESOLVER_SLOT,
                        error,
                    );
                } else {
                    let close_info = new_websocket_stream_close_info(scope, *code, reason);
                    resolve_websocket_stream_promise(
                        scope,
                        stream,
                        WEBSOCKET_STREAM_CLOSED_RESOLVER_SLOT,
                        close_info.into(),
                    );
                }
            } else {
                let error = websocket_value_slot(scope, stream, WEBSOCKET_STREAM_ERROR_SLOT)
                    .filter(|value| !value.is_null_or_undefined())
                    .unwrap_or_else(|| {
                        let error = new_websocket_error_object(scope, "", Some(*code), reason);
                        set_websocket_value_slot(scope, stream, WEBSOCKET_STREAM_ERROR_SLOT, error);
                        error
                    });
                if let Some(readable) =
                    websocket_object_slot(scope, stream, WEBSOCKET_STREAM_READABLE_SLOT)
                {
                    error_stream(scope, readable, error);
                }
                if let Some(writable) =
                    websocket_object_slot(scope, stream, WEBSOCKET_STREAM_WRITABLE_SLOT)
                {
                    reject_all_websocket_stream_pending_writes(scope, writable, error);
                    error_writable_stream_with_value(scope, writable, error);
                }
                reject_websocket_stream_promise(
                    scope,
                    stream,
                    WEBSOCKET_STREAM_CLOSED_RESOLVER_SLOT,
                    error,
                );
            }
            WebSocketDispatchResult::Dispatched
        }
    }
}

fn websocket_stream_readable_can_accept_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    readable: v8::Local<'s, v8::Object>,
) -> bool {
    if readable_stream_pending_read_count(scope, readable) > 0 {
        return true;
    }
    let high_water_mark = stream_slot_number(scope, readable, READABLE_STREAM_HWM_SLOT)
        .unwrap_or(1.0)
        .max(0.0);
    moli_streams::strategy::StrategySnapshot::new(
        high_water_mark,
        readable_stream_queue_total_size(scope, readable),
    )
    .has_capacity()
}

fn readable_stream_pending_read_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    readable: v8::Local<'s, v8::Object>,
) -> u32 {
    stream_slot_array(scope, readable, READABLE_STREAM_PENDING_READS_SLOT)
        .map(|pending| pending.length())
        .unwrap_or(0)
}

pub(super) fn reject_websocket_stream_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let error = websocket_dom_exception_value(
        scope,
        "AbortError",
        20,
        "The WebSocketStream connection was aborted.",
    );
    reject_websocket_stream_promise(scope, stream, WEBSOCKET_STREAM_OPENED_RESOLVER_SLOT, error);
    reject_websocket_stream_promise(scope, stream, WEBSOCKET_STREAM_CLOSED_RESOLVER_SLOT, error);
}

fn new_websocket_stream_writable_sink<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    socket_id: u64,
) -> v8::Local<'s, v8::Object> {
    WebSocketStreamWritableSinkDeclaration::new(
        socket_id as f64,
        websocket_value_slot(scope, stream, WEBSOCKET_STREAM_CLOSED_SLOT),
    )
    .bind(scope)
    .expect("WebSocketStream writable sink declaration should bind")
}

fn new_websocket_stream_readable_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket_id: u64,
) -> v8::Local<'s, v8::Object> {
    WebSocketStreamReadableSourceDeclaration::new(socket_id as f64)
        .bind(scope)
        .expect("WebSocketStream readable source declaration should bind")
}

fn websocket_stream_source_pull_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(socket_id) = websocket_id(scope, args.this()) {
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            let host = unsafe { &*host_ptr };
            host.signal_websocket_stream_pull(socket_id);
        } else {
            crate::worker::signal_worker_websocket_stream_pull(scope, socket_id);
        }
    }
    rv.set_undefined();
}

fn websocket_stream_sink_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let payload = match websocket_stream_send_payload_from_value(scope, args.get(0)) {
        Ok(payload) => payload,
        Err(error) => {
            let _ = resolver.reject(scope, error);
            rv.set(promise.into());
            return;
        }
    };
    let admission = if let Some(socket_id) = websocket_id(scope, args.this())
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        let host = unsafe { &*host_ptr };
        match payload {
            WebSocketSendPayload::Text(text) => host.send_websocket_text(socket_id, text),
            WebSocketSendPayload::Binary(bytes) => host.send_websocket_binary(socket_id, bytes),
        }
    } else {
        None
    };
    if admission.is_some() {
        // The transport may already have queued its terminal event when command
        // admission fails. Keep the write pending until that owner's Close or
        // Error task settles it together with the stream, instead of exposing
        // the transport's state before the realm has observed closure.
        enqueue_websocket_stream_pending_write(scope, args.this(), resolver);
    } else {
        let error = websocket_dom_exception_value(
            scope,
            "InvalidStateError",
            11,
            "The WebSocketStream writable is closed.",
        );
        let _ = resolver.reject(scope, error);
    }
    rv.set(promise.into());
}

fn enqueue_websocket_stream_pending_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) {
    let Some(pending) = websocket_stream_pending_writes(scope, sink) else {
        return;
    };
    let entry = WebSocketStreamPendingWriteDeclaration::new(resolver)
        .bind(scope)
        .expect("WebSocketStream pending write declaration should bind");
    let _ = pending.set_index(scope, pending.length(), entry.into());
}

fn resolve_next_websocket_stream_pending_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    let Some((sink, pending)) = websocket_stream_pending_writes_for_stream(scope, stream) else {
        return false;
    };
    if pending.length() == 0 {
        return false;
    }
    let Some(entry) = pending
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    replace_websocket_stream_pending_writes_from_index(scope, sink, pending, 1);
    if let Some(resolver) =
        websocket_stream_resolver(scope, entry, WEBSOCKET_STREAM_PROMISE_RESOLVER_SLOT)
    {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return true;
    }
    false
}

fn reject_all_websocket_stream_pending_writes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(sink) = stream_slot_object(scope, writable, WRITABLE_STREAM_SINK_SLOT) else {
        return;
    };
    let Some(pending) = websocket_stream_pending_writes(scope, sink) else {
        return;
    };
    for index in 0..pending.length() {
        let Some(entry) = pending
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if let Some(resolver) =
            websocket_stream_resolver(scope, entry, WEBSOCKET_STREAM_PROMISE_RESOLVER_SLOT)
        {
            let _ = resolver.reject(scope, reason);
        }
    }
    set_private_value(
        scope,
        sink,
        WEBSOCKET_STREAM_PENDING_WRITES_SLOT,
        v8::Array::new(scope, 0).into(),
    );
}

fn websocket_stream_pending_writes_for_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Array>)> {
    let writable = websocket_object_slot(scope, stream, WEBSOCKET_STREAM_WRITABLE_SLOT)?;
    let sink = stream_slot_object(scope, writable, WRITABLE_STREAM_SINK_SLOT)?;
    let pending = websocket_stream_pending_writes(scope, sink)?;
    Some((sink, pending))
}

fn websocket_stream_has_interrupted_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    writable: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(sink) = stream_slot_object(scope, writable, WRITABLE_STREAM_SINK_SLOT) else {
        return false;
    };
    websocket_stream_pending_writes(scope, sink).is_some_and(|pending| pending.length() > 0)
}

fn websocket_stream_pending_writes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, sink, WEBSOCKET_STREAM_PENDING_WRITES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn replace_websocket_stream_pending_writes_from_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
    pending: v8::Local<'s, v8::Array>,
    start: u32,
) {
    let next = v8::Array::new(scope, 0);
    for index in start..pending.length() {
        if let Some(entry) = pending.get_index(scope, index) {
            let _ = next.set_index(scope, next.length(), entry);
        }
    }
    set_private_value(
        scope,
        sink,
        WEBSOCKET_STREAM_PENDING_WRITES_SLOT,
        next.into(),
    );
}

fn websocket_stream_sink_abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    close_websocket_stream_with_reason(scope, args.this(), args.get(0));
    rv.set_undefined();
}

fn websocket_stream_sink_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(socket_id) = websocket_id(scope, args.this())
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        let host = unsafe { &mut *host_ptr };
        let _ = host.close_websocket(socket_id, None, String::new());
    }
    if let Some(close_promise) = websocket_stream_sink_close_promise(scope, args.this()) {
        rv.set(close_promise);
        return;
    }
    rv.set_undefined();
}

fn websocket_stream_sink_close_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sink: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let promise = get_private_value(scope, sink, WEBSOCKET_STREAM_SINK_CLOSED_PROMISE_SLOT)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())?;
    let on_fulfilled = v8::Function::builder(websocket_stream_return_undefined_callback)
        .length(0)
        .build(scope)?;
    promise.then(scope, on_fulfilled).map(Into::into)
}

fn websocket_stream_return_undefined_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_undefined();
}

fn websocket_stream_source_cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    close_websocket_stream_with_reason(scope, args.this(), args.get(0));
    rv.set_undefined();
}

fn close_websocket_stream_with_reason<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let (close_code, reason) =
        websocket_error_close_info(scope, reason).unwrap_or((None, String::new()));
    if let Some(socket_id) = websocket_id(scope, target)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        let host = unsafe { &mut *host_ptr };
        let _ = host.close_websocket(socket_id, close_code, reason);
    }
}

pub(super) fn is_websocket_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::WebSocketStream::is_instance(scope, object)
}

fn resolve_websocket_stream_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
    value: v8::Local<'s, v8::Value>,
) {
    if let Some(resolver) = websocket_stream_resolver(scope, stream, slot_name) {
        let _ = resolver.resolve(scope, value);
    }
}

fn reject_websocket_stream_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
    value: v8::Local<'s, v8::Value>,
) {
    if let Some(resolver) = websocket_stream_resolver(scope, stream, slot_name) {
        let _ = resolver.reject(scope, value);
    }
}

fn websocket_stream_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let object = get_private_value(scope, target, slot_name)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    // These private slots are populated only with PromiseResolver::new results
    // by the stream constructor and native pending-write declarations.
    Some(unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
}

fn new_websocket_stream_close_info<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    close_code: u16,
    reason: &str,
) -> v8::Local<'s, v8::Object> {
    WebSocketStreamCloseInfoDeclaration::new(close_code as f64, v8_string(scope, reason))
        .bind(scope)
        .expect("WebSocketStream close info declaration should bind")
}

fn new_websocket_error_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    close_code: Option<u16>,
    reason: &str,
) -> v8::Local<'s, v8::Value> {
    new_websocket_error_value(scope, message, close_code, reason)
}
