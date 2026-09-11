use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Event, prototype = "Object")]
struct WebSocketSimpleEventFallbackDeclaration {
    #[webapi(data_property, enumerable)]
    r#type: String,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebSocketMessageEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    origin: Option<v8::Local<'scope, v8::String>>,
}

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct WebSocketCloseEventInitDeclaration<'scope> {
    code: u16,
    reason: v8::Local<'scope, v8::String>,
    was_clean: bool,
}

pub(super) fn dispatch_named_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    event_type: &str,
) -> bool {
    let event = simple_event_object(scope, event_type);
    dispatch_simple_event_target_event(scope, socket, WEBSOCKET_LISTENERS_SLOT, event_type, event)
}

pub(super) fn simple_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    if let Some(ctor) = global
        .get(scope, v8str(scope, "Event").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        && let Some(event_type) = v8_string(scope, event_type)
        && let Some(event) = ctor.new_instance(scope, &[event_type.into()])
    {
        return event;
    }
    WebSocketSimpleEventFallbackDeclaration::new(event_type.to_owned())
        .bind(scope)
        .expect("WebSocket simple event fallback declaration should bind")
}

pub(super) fn new_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
    origin: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let ctor = global
        .get(scope, v8str(scope, "MessageEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let init = WebSocketMessageEventInitDeclaration::new(data, v8_string(scope, origin))
        .bind(scope)
        .ok()?;
    let event_type = v8_string(scope, "message")?;
    ctor.new_instance(scope, &[event_type.into(), init.into()])
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

pub(super) fn new_close_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    code: u16,
    reason: &str,
    was_clean: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let reason = v8_string(scope, reason)?;
    if let Some(ctor) = global
        .get(scope, v8str(scope, "CloseEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let event_type = v8_string(scope, "close")?;
        let init = WebSocketCloseEventInitDeclaration::new(code, reason, was_clean)
            .bind(scope)
            .ok()?;
        return ctor.new_instance(scope, &[event_type.into(), init.into()]);
    }
    let event = simple_event_object(scope, "close");
    WebSocketCloseEventInitDeclaration::new(code, reason, was_clean)
        .initialize(scope, event)
        .ok()?;
    Some(event)
}
