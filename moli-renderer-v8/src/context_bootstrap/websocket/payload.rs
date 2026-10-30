use super::*;

pub(super) enum WebSocketSendPayload {
    Text(String),
    Binary(Vec<u8>),
    Blob(moli_file_api::BlobData),
}

impl WebSocketSendPayload {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Binary(bytes) => bytes.len(),
            Self::Blob(data) => data.len(),
        }
    }
}

pub(super) fn websocket_send_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> WebSocketSendPayload {
    websocket_send_payload_from_value(scope, args.get(0))
}

pub(super) fn websocket_send_payload_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> WebSocketSendPayload {
    if !value.is_null_or_undefined() {
        if let Some(bytes) = buffer_source_bytes_from_value(scope, value) {
            return WebSocketSendPayload::Binary(bytes);
        }
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && let Some(data) = blob::blob_data_from_object(scope, object)
        {
            return blob_payload(data);
        }
    }
    WebSocketSendPayload::Text(
        value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default(),
    )
}

pub(super) fn websocket_stream_send_payload_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<WebSocketSendPayload, v8::Local<'s, v8::Value>> {
    if !value.is_null_or_undefined() {
        if let Some(bytes) = stream_buffer_source_bytes_from_value(scope, value)? {
            return Ok(WebSocketSendPayload::Binary(bytes));
        }
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && let Some(data) = blob::blob_data_from_object(scope, object)
        {
            return Ok(blob_payload(data));
        }
    }
    value_to_string_for_stream_payload(scope, value).map(WebSocketSendPayload::Text)
}

fn blob_payload(data: moli_file_api::BlobData) -> WebSocketSendPayload {
    match data.as_bytes() {
        Some(bytes) => WebSocketSendPayload::Binary(bytes.to_vec()),
        None => WebSocketSendPayload::Blob(data),
    }
}

fn buffer_source_bytes_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<Vec<u8>> {
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        let view = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())?;
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        return Some(bytes);
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        return Some(bytes);
    }
    None
}

fn stream_buffer_source_bytes_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<Vec<u8>>, v8::Local<'s, v8::Value>> {
    if value.is_shared_array_buffer() {
        return Err(websocket_stream_write_type_error(
            scope,
            "WebSocketStream does not support SharedArrayBuffer payloads.",
        ));
    }
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        reject_unsupported_stream_backing_store(scope, &buffer.get_backing_store())?;
        let view = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())
            .ok_or_else(|| websocket_stream_write_type_error(scope, "Invalid ArrayBuffer."))?;
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        return Ok(Some(bytes));
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let backing_store = view.get_backing_store().ok_or_else(|| {
            websocket_stream_write_type_error(scope, "Invalid ArrayBufferView payload.")
        })?;
        reject_unsupported_stream_backing_store(scope, &backing_store)?;
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        return Ok(Some(bytes));
    }
    Ok(None)
}

fn reject_unsupported_stream_backing_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing_store: &v8::BackingStore,
) -> Result<(), v8::Local<'s, v8::Value>> {
    if backing_store.is_shared() {
        return Err(websocket_stream_write_type_error(
            scope,
            "WebSocketStream does not support SharedArrayBuffer-backed payloads.",
        ));
    }
    if backing_store.is_resizable_by_user_javascript() {
        return Err(websocket_stream_write_type_error(
            scope,
            "WebSocketStream does not support resizable ArrayBuffer payloads.",
        ));
    }
    Ok(())
}

fn value_to_string_for_stream_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<String, v8::Local<'s, v8::Value>> {
    {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let try_scope = try_catch.init();
        if let Some(text) = value.to_string(&try_scope) {
            return Ok(text.to_rust_string_lossy(&try_scope));
        }
    }
    // WPT expects WebSocketStream write() to reject unstringifiable chunks with
    // TypeError; do not expose embedder-specific V8 conversion exceptions here.
    Err(websocket_stream_write_type_error(
        scope,
        "Failed to stringify WebSocketStream write payload.",
    ))
}

fn websocket_stream_write_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    let message =
        v8_string(scope, message).unwrap_or_else(|| v8str(scope, "WebSocketStream write failed."));
    v8::Exception::type_error(scope, message)
}
