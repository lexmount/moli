use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::IDBObjectStore,
    scope_lifetime = 'scope,
    data_properties,
    enumerable
)]
struct IdbObjectStoreSurfaceDeclaration<'scope, 'value> {
    name: &'value str,
    key_path: v8::Local<'scope, v8::Value>,
    auto_increment: bool,
    index_names: v8::Local<'scope, v8::Object>,
}

pub(in crate::context_bootstrap::indexed_db) fn sync_store_surface_from_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    metadata: IndexedDbObjectStoreMetadata,
) -> Option<()> {
    let info = metadata.info().clone();
    let key_path_value = match &info.key_path {
        Some(value) => key_path_to_js_value(scope, value)?,
        None => v8::null(scope).into(),
    };
    let index_names = new_idb_dom_string_list(scope, &info.index_names);
    set_indexed_db_object_store_metadata(scope, store, metadata)?;
    IdbObjectStoreSurfaceDeclaration::new(
        &info.name,
        key_path_value,
        info.auto_increment,
        index_names,
    )
    .initialize(scope, store)
    .ok()?;
    Some(())
}
