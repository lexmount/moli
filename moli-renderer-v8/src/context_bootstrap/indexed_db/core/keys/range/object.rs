use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IDBKeyRange, require_prototype)]
struct IdbKeyRangeObjectDeclaration<'scope> {
    #[webapi(slot = LOWER)]
    lower: v8::Local<'scope, v8::Value>,
    #[webapi(slot = UPPER)]
    upper: v8::Local<'scope, v8::Value>,
    #[webapi(slot = LOWER_OPEN)]
    lower_open: bool,
    #[webapi(slot = UPPER_OPEN)]
    upper_open: bool,
}

pub(in crate::context_bootstrap::indexed_db) fn create_key_range_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: &IdbKeyRangeQuery,
) -> Option<v8::Local<'s, v8::Object>> {
    let lower = range
        .lower
        .as_ref()
        .map(|key| key_to_js_value(scope, key))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let upper = range
        .upper
        .as_ref()
        .map(|key| key_to_js_value(scope, key))
        .unwrap_or_else(|| v8::undefined(scope).into());
    // The range owns private snapshots that are never exposed to author code.
    // V8 traces them with the range, without a runtime table of strong roots.
    IdbKeyRangeObjectDeclaration::new(lower, upper, range.lower_open, range.upper_open)
        .bind(scope)
        .ok()
}
