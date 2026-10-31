use crate::context_bootstrap::indexed_db::{
    indexed_db_index_name, indexed_db_object_store_metadata, indexed_db_object_store_name,
    new_idb_name_list,
};
use crate::util::get_private_value;

pub(in crate::context_bootstrap::indexed_db) const SCHEMA_NAME_SLOT: &str =
    "moli.IndexedDb.schemaName";
pub(in crate::context_bootstrap::indexed_db) const SCHEMA_KEY_PATH_SLOT: &str =
    "moli.IndexedDb.schemaKeyPath";
pub(in crate::context_bootstrap::indexed_db) const SCHEMA_AUTO_INCREMENT_SLOT: &str =
    "moli.IndexedDb.schemaAutoIncrement";
pub(in crate::context_bootstrap::indexed_db) const SCHEMA_UNIQUE_SLOT: &str =
    "moli.IndexedDb.schemaUnique";
pub(in crate::context_bootstrap::indexed_db) const SCHEMA_MULTI_ENTRY_SLOT: &str =
    "moli.IndexedDb.schemaMultiEntry";

// Initial values are traced by their handle, including a handle whose
// schema creation failed. Cached keyPath arrays never replace native key paths.
pub(in crate::context_bootstrap::indexed_db) fn idb_schema_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("schema receiver was validated");
    let slot = args.data().to_rust_string_lossy(scope);
    if slot == SCHEMA_NAME_SLOT
        && let Some(name) = indexed_db_object_store_name(scope, receiver)
            .or_else(|| indexed_db_index_name(scope, receiver))
    {
        if let Some(value) = crate::util::v8_string_from_utf16_units(scope, name.as_utf16()) {
            rv.set(value.into());
        }
        return;
    }
    rv.set(get_private_value(scope, receiver, &slot).expect("schema attribute slot"));
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_index_names_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("schema receiver was validated");
    let metadata = indexed_db_object_store_metadata(scope, receiver);
    let names = metadata
        .as_ref()
        .map_or(&[][..], |metadata| metadata.index_names());
    rv.set(new_idb_name_list(scope, names).into());
}
