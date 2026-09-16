use super::*;
use crate::context_bootstrap::indexed_db::sync_indexed_db_store_handles;

pub(in crate::context_bootstrap::indexed_db) fn sync_transaction_object_store_names_from_database<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
) {
    let store_names = object_property_as_object(scope, database, "objectStoreNames")
        .map(|value| dom_string_list_values(scope, value))
        .unwrap_or_default();
    let object_store_names = new_idb_name_list(scope, &store_names);
    let _ = transaction.set(
        scope,
        v8str(scope, "objectStoreNames").into(),
        object_store_names.into(),
    );
}

pub(in crate::context_bootstrap::indexed_db) fn set_database_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    info: &ObjectStoreInfo,
    indexes: &[IndexInfo],
) -> Option<()> {
    let typed_metadata = IndexedDbObjectStoreMetadata::new(info.clone(), indexes.iter().cloned());
    set_indexed_db_database_store_metadata(scope, database, typed_metadata)?;
    let mut store_names = object_property_as_object(scope, database, "objectStoreNames")
        .map(|value| dom_string_list_values(scope, value))
        .unwrap_or_default();
    if !store_names.iter().any(|name| name == &info.name) {
        store_names.push(info.name.clone());
    }
    let object_store_names = new_idb_name_list(scope, &store_names);
    let _ = database.set(
        scope,
        v8str(scope, "objectStoreNames").into(),
        object_store_names.into(),
    );
    Some(())
}

pub(in crate::context_bootstrap::indexed_db) fn remove_database_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &str,
) -> Option<()> {
    remove_indexed_db_database_store_metadata(scope, database, store_name)?;
    sync_indexed_db_store_handles(scope, database, store_name);
    let mut store_names = object_property_as_object(scope, database, "objectStoreNames")
        .map(|value| dom_string_list_values(scope, value))
        .unwrap_or_default();
    store_names.retain(|name| name != store_name);
    let object_store_names = new_idb_name_list(scope, &store_names);
    let _ = database.set(
        scope,
        v8str(scope, "objectStoreNames").into(),
        object_store_names.into(),
    );
    Some(())
}
