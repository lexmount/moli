use super::*;
pub(super) use moli_webapi::websocket_values::{
    new_websocket_close_event as new_close_event, new_websocket_simple_event as simple_event_object,
};

pub(super) fn dispatch_named_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    event_type: &str,
) -> bool {
    let event = simple_event_object(scope, event_type);
    dispatch_simple_event_target_event(scope, socket, event_type, event)
}

pub(super) fn new_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
    origin: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let source = v8::null(scope).into();
    let ports = v8::Array::new(scope, 0);
    crate::context_bootstrap::construct_original_message_event(
        scope,
        "message",
        data,
        origin,
        &[],
        source,
        ports,
    )
}

pub(super) fn websocket_message_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
) -> String {
    super::helpers::websocket_string_slot(scope, socket, WEBSOCKET_URL_SLOT)
        .and_then(|url| url::Url::parse(&url).ok())
        .map(|url| moli_url::origin_ascii_serialization(&url))
        .unwrap_or_default()
}

pub(super) fn websocket_binary_message_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Value>> {
    if super::helpers::websocket_string_slot(scope, socket, WEBSOCKET_BINARY_TYPE_SLOT).as_deref()
        == Some("arraybuffer")
    {
        let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
        let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
        return Some(buffer.into());
    }
    blob::build_blob_object(scope, bytes, String::new()).map(|blob| blob.into())
}
