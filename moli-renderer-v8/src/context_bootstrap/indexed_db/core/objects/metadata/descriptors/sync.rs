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
    let context = store.get_creation_context(scope)?;
    if context == scope.get_current_context() {
        return sync_store_surface_in_current_context(scope, store, metadata);
    }
    let target_scope = &mut v8::ContextScope::new(scope, context);
    sync_store_surface_in_current_context(target_scope, store, metadata)
}

fn sync_store_surface_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    metadata: IndexedDbObjectStoreMetadata,
) -> Option<()> {
    set_indexed_db_object_store_metadata(scope, store, metadata.clone())?;
    sync_store_surface_fields(scope, store, metadata.info(), metadata.index_names())
}

// Also used for an already-aborted schema handle: all observable fields live in
// V8, without retaining a redundant copy of the failed definition.
pub(in crate::context_bootstrap::indexed_db) fn sync_store_surface_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    info: &ObjectStoreInfo,
    names: &[String],
) -> Option<()> {
    let key_path_value = match &info.key_path {
        Some(value) => key_path_to_js_value(scope, value)?,
        None => v8::null(scope).into(),
    };
    let index_names = new_idb_name_list(scope, names);
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
