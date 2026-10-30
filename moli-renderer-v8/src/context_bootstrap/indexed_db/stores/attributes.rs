use crate::context_bootstrap::indexed_db::{
    indexed_db_index_name, indexed_db_object_store_metadata, indexed_db_object_store_name,
    new_idb_name_list, rename_indexed_db_index, rename_indexed_db_object_store,
    request_error_object,
};
use crate::util::get_private_value;
use crate::webidl;

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

// These immutable values are traced by their handle, including a handle whose
// schema creation failed. Cached keyPath arrays never replace native key paths.
pub(in crate::context_bootstrap::indexed_db) fn idb_schema_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("schema receiver was validated");
    let slot = args.data().to_rust_string_lossy(scope);
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

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("schema receiver was validated");
    if let Some(name) = indexed_db_object_store_name(scope, receiver) {
        if let Some(name) = crate::util::v8_string_from_utf16_units(scope, name.as_utf16()) {
            rv.set(name.into());
        }
    } else {
        // A handle returned before asynchronous schema admission failed still
        // exposes its immutable initial name, without reading author fields.
        rv.set(get_private_value(scope, receiver, SCHEMA_NAME_SLOT).expect("schema name slot"));
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("schema receiver was validated");
    if let Some(name) = indexed_db_index_name(scope, receiver) {
        if let Some(name) = crate::util::v8_string_from_utf16_units(scope, name.as_utf16()) {
            rv.set(name.into());
        }
    } else {
        rv.set(get_private_value(scope, receiver, SCHEMA_NAME_SLOT).expect("schema name slot"));
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_name_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("schema receiver was validated");
    let name = match webidl::convert::<webidl::DomString16>(
        scope,
        args.get(0),
        webidl::Context::member("IDBObjectStore", "name"),
    ) {
        Ok(name) => moli_indexeddb::IndexedDbName::from_utf16(name.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if let Err(error) = rename_indexed_db_object_store(scope, receiver, name) {
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_name_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("schema receiver was validated");
    let name = match webidl::convert::<webidl::DomString16>(
        scope,
        args.get(0),
        webidl::Context::member("IDBIndex", "name"),
    ) {
        Ok(name) => moli_indexeddb::IndexedDbName::from_utf16(name.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if let Err(error) = rename_indexed_db_index(scope, receiver, name) {
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
    }
}
