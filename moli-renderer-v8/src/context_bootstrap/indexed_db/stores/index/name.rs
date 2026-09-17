use super::*;
use crate::context_bootstrap::indexed_db::{
    idb_name_to_v8, indexed_db_index_is_deleted, indexed_db_transaction_mode,
    rename_indexed_db_index_metadata,
};
use moli_indexeddb::TransactionMode;

pub(in crate::context_bootstrap::indexed_db) fn idb_index_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(info) = indexed_db_index_info(scope, args.this()) {
        rv.set(idb_name_to_v8(scope, &info.name).into());
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_name_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    // The generated binding checks the receiver before conversion. Conversion
    // may execute script, so inspect all transaction/index state afterwards.
    let name = match webidl::convert::<webidl::DomString16>(
        scope,
        args.get(0),
        webidl::Context::member("IDBIndex", "name"),
    ) {
        Ok(name) => IndexedDbName::from_utf16(name.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if let Err(error) = rename_index(scope, args.this(), name) {
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
    }
}

fn rename_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
    name: IndexedDbName,
) -> Result<(), IndexedDbError> {
    let unavailable = || IndexedDbError::InvalidState("The index is unavailable.".to_owned());
    let store = indexed_db_index_object_store(scope, index).ok_or_else(unavailable)?;
    let transaction = indexed_db_object_store_transaction(scope, store).ok_or_else(unavailable)?;
    // https://w3c.github.io/IndexedDB/#dom-idbindex-name
    // In particular, an inactive upgrade fails before checking deletion or a
    // same-name assignment, whereas a non-upgrade always fails InvalidState.
    if indexed_db_transaction_mode(scope, transaction) != Some(TransactionMode::VersionChange) {
        return Err(IndexedDbError::InvalidState(
            "The index is not running in a version change transaction.".to_owned(),
        ));
    }
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        return Err(IndexedDbError::TransactionInactive(
            "The transaction is not active.".to_owned(),
        ));
    }
    if indexed_db_index_is_deleted(scope, index) {
        return Err(unavailable());
    }
    let old_name = indexed_db_index_info(scope, index)
        .ok_or_else(unavailable)?
        .name;
    if old_name == name {
        return Ok(());
    }
    let handle =
        transaction_handle_from_value(scope, transaction.into()).ok_or_else(unavailable)?;
    let store_name = indexed_db_object_store_name(scope, store).ok_or_else(unavailable)?;
    let context = store.get_creation_context(scope).ok_or_else(unavailable)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    with_indexed_db_manager(scope, |manager| {
        manager.rename_index(handle, &store_name, &old_name, name.clone())
    })?;
    rename_indexed_db_index_metadata(scope, store, &old_name, &name);
    Ok(())
}
