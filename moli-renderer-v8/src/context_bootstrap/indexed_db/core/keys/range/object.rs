use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::IDBKeyRange,
    require_prototype,
    data_properties,
    enumerable
)]
struct IdbKeyRangeObjectDeclaration<'scope> {
    lower: v8::Local<'scope, v8::Value>,
    upper: v8::Local<'scope, v8::Value>,
    lower_open: bool,
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
    let key_range =
        IdbKeyRangeObjectDeclaration::new(lower, upper, range.lower_open, range.upper_open)
            .bind(scope)
            .ok()?;
    register_indexed_db_wrapper(scope, key_range, IndexedDbWrapperKind::KeyRange, None);
    register_indexed_db_key_range_lifecycle(scope, key_range, true);
    Some(key_range)
}
