use super::*;
use crate::webidl;
use moli_webapi_declare::WebApiValue;

pub(in crate::context_bootstrap::indexed_db) fn parse_idb_key_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<KeyPath, webidl::WebIdlError> {
    if let Some(key_path) = webidl::convert_optional_sequence::<webidl::DomString>(
        scope,
        value,
        context,
        &Default::default(),
    )? {
        return Ok(KeyPath::Sequence(
            key_path.0.into_iter().map(Into::into).collect(),
        ));
    }
    webidl::convert::<webidl::DomString>(scope, value, context)
        .map(|value| KeyPath::String(value.into()))
}

pub(in crate::context_bootstrap::indexed_db) fn parse_optional_idb_key_path_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<KeyPath>, webidl::WebIdlError> {
    let context = webidl::Context::member("IDBObjectStoreParameters", name);
    match webidl::property_result(scope, object, name, context)? {
        Some(raw) if raw.is_null_or_undefined() => Ok(None),
        Some(raw) => parse_idb_key_path(scope, raw, context).map(Some),
        None => Ok(None),
    }
}

pub(in crate::context_bootstrap::indexed_db) fn key_path_to_js_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key_path: &KeyPath,
) -> Option<v8::Local<'s, v8::Value>> {
    match key_path {
        KeyPath::String(value) => v8_string(scope, value).map(Into::into),
        KeyPath::Sequence(values) => values.as_slice().to_v8_value(scope),
    }
}
