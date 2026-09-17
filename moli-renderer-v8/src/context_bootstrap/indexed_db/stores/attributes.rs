use crate::context_bootstrap::indexed_db::{
    indexed_db_index_info, indexed_db_index_key_path, indexed_db_index_object_store,
    indexed_db_object_store_key_path, indexed_db_object_store_metadata,
    indexed_db_object_store_transaction, new_idb_name_list,
};

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_key_path_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = indexed_db_object_store_key_path(scope, args.this()) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_auto_increment_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(metadata) = indexed_db_object_store_metadata(scope, args.this()) {
        rv.set_bool(metadata.info().auto_increment);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_index_names_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(metadata) = indexed_db_object_store_metadata(scope, args.this()) {
        rv.set(new_idb_name_list(scope, &metadata.info().index_names).into());
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_transaction_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(transaction) = indexed_db_object_store_transaction(scope, args.this()) {
        rv.set(transaction.into());
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_key_path_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = indexed_db_index_key_path(scope, args.this()) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_multi_entry_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(info) = indexed_db_index_info(scope, args.this()) {
        rv.set_bool(info.multi_entry);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_unique_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(info) = indexed_db_index_info(scope, args.this()) {
        rv.set_bool(info.unique);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_object_store_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(store) = indexed_db_index_object_store(scope, args.this()) {
        rv.set(store.into());
    }
}
