use super::super::*;
use super::entries::{
    HEADERS_ENTRIES_SLOT, HEADERS_GUARD_SLOT, HEADERS_IMMUTABLE_SLOT, HeadersGuard,
    headers_entries, headers_entries_json, normalized_header_name_or_throw,
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Headers, prototype = "Object")]
struct HeadersStorageDeclaration {
    #[webapi(slot = HEADERS_ENTRIES_SLOT)]
    entries: String,
    #[webapi(slot = HEADERS_GUARD_SLOT)]
    guard: &'static str,
    #[webapi(slot = HEADERS_IMMUTABLE_SLOT)]
    immutable: bool,
}

pub(in crate::network_host) fn get_header_prop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let lower = normalized_header_name_or_throw(scope, name)?;
    let values = headers_entries(scope, obj)
        .into_iter()
        .filter_map(|(entry_name, value)| (entry_name == lower).then_some(value))
        .collect::<Vec<_>>();
    if values.is_empty() {
        None
    } else {
        v8_string(scope, &values.join(", ")).map(|value| value.into())
    }
}

pub(in crate::network_host) fn build_headers_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    headers: &[(String, String)],
) -> v8::Local<'s, v8::Object> {
    build_headers_object_with_state(scope, headers, HeadersGuard::None, false)
}

pub(in crate::network_host) fn build_headers_object_with_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    headers: &[(String, String)],
    guard: HeadersGuard,
    immutable: bool,
) -> v8::Local<'s, v8::Object> {
    HeadersStorageDeclaration::new(headers_entries_json(headers), guard.as_str(), immutable)
        .bind(scope)
        .expect("Headers storage declaration should bind")
}

pub(in crate::network_host) fn initialize_headers_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    headers: &[(String, String)],
) {
    HeadersStorageDeclaration::new(
        headers_entries_json(headers),
        HeadersGuard::None.as_str(),
        false,
    )
    .initialize(scope, object)
    .expect("Headers storage declaration should initialize");
}
