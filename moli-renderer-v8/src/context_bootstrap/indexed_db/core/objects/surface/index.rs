use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IDBIndex, require_prototype)]
struct IdbIndexObjectDeclaration {}

pub(in crate::context_bootstrap::indexed_db) fn create_index_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    info: &IndexInfo,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(index) =
        crate::context_bootstrap::indexed_db::cached_indexed_db_index(scope, store, &info.name)
    {
        return Some(index);
    }
    let key_path_value = key_path_to_js_value(scope, &info.key_path)?;
    let index = IdbIndexObjectDeclaration::new().bind(scope).ok()?;
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
    register_indexed_db_index_lifecycle(scope, index, store, info.clone(), key_path_value);
    Some(index)
}
