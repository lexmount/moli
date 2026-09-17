use crate::context_bootstrap::indexed_db::stores::collection::{
    CollectionKind, collection_callback,
};

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_get_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_callback(
        scope,
        args,
        rv,
        CollectionKind::Value,
        false,
        "IDBObjectStore.getAll",
    );
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_get_all_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_callback(
        scope,
        args,
        rv,
        CollectionKind::Key,
        false,
        "IDBObjectStore.getAllKeys",
    );
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_get_all_records_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_callback(
        scope,
        args,
        rv,
        CollectionKind::Record,
        false,
        "IDBObjectStore.getAllRecords",
    );
}
