use super::*;
use crate::util::serialize_v8_iter_array;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(record)]
struct CursorSnapshotEntryObjectDeclaration<'scope> {
    #[webapi(data_property)]
    key: v8::Local<'scope, v8::Value>,

    #[webapi(data_property = "primaryKey")]
    primary_key: v8::Local<'scope, v8::Value>,

    #[webapi(data_property)]
    value: v8::Local<'scope, v8::Value>,
}

pub(super) fn cursor_entries_to_js_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &[CursorSnapshotEntry],
) -> Option<v8::Local<'s, v8::Array>> {
    let mut values = Vec::with_capacity(entries.len());
    for entry in entries {
        let key = key_to_js_value(scope, &entry.key);
        let primary_key = key_to_js_value(scope, &entry.primary_key);
        let value = entry
            .value
            .as_ref()
            .and_then(|bytes| deserialize_js_value(scope, bytes))
            .unwrap_or_else(|| v8::undefined(scope).into());
        values.push(CursorSnapshotEntryObjectDeclaration::new(
            key,
            primary_key,
            value,
        ));
    }
    serialize_v8_iter_array(scope, values)
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    position: usize,
) -> Option<v8::Local<'s, v8::Object>> {
    let entries = object_hidden_value(scope, cursor, INDEXED_DB_CURSOR_ENTRIES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    entries
        .get_index(scope, position as u32)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}
