use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IDBIndex, require_prototype, scope_lifetime = 'scope)]
struct IdbIndexObjectDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    name: v8::Local<'scope, v8::String>,
    #[webapi(slot = SCHEMA_KEY_PATH_SLOT)]
    key_path: v8::Local<'scope, v8::Value>,
    #[webapi(slot = SCHEMA_UNIQUE_SLOT)]
    unique: bool,
    #[webapi(slot = SCHEMA_MULTI_ENTRY_SLOT)]
    multi_entry: bool,
    #[webapi(slot = INDEX_OBJECT_STORE)]
    object_store: v8::Local<'scope, v8::Object>,
}

pub(in crate::context_bootstrap::indexed_db) fn create_index_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    info: &IndexInfo,
) -> Option<v8::Local<'s, v8::Object>> {
    let index = create_failed_index_object(scope, store, info)?;
    let storage_scope = indexed_db_typed_storage_scope(scope, store);
    let owner = indexed_db_typed_execution_owner(scope, store)
        .expect("IDBIndex should inherit typed owner from object store");
    register_indexed_db_wrapper_with_owner(
        scope,
        index,
        IndexedDbWrapperKind::Index,
        owner,
        storage_scope,
    );
    register_indexed_db_index_lifecycle(scope, index, store, info)?;
    Some(index)
}

pub(in crate::context_bootstrap::indexed_db) fn create_failed_index_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    info: &IndexInfo,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = store.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let key_path_value = key_path_to_js_value(scope, &info.key_path)?;
    IdbIndexObjectDeclaration::new(
        crate::util::v8_string_from_utf16_units(scope, info.name.as_utf16())?,
        key_path_value,
        info.unique,
        info.multi_entry,
        store,
    )
    .bind(scope)
    .ok()
}
