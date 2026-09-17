use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_object_store_is_deleted, indexed_db_transaction_mode,
};
use moli_indexeddb::TransactionMode;

pub(in crate::context_bootstrap::indexed_db) fn object_store_versionchange_common<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Result<
    (
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Object>,
        TransactionHandle,
        String,
    ),
    IndexedDbError,
> {
    let unavailable =
        || IndexedDbError::InvalidState("The object store is unavailable.".to_owned());
    let transaction = indexed_db_object_store_transaction(scope, store).ok_or_else(unavailable)?;
    if indexed_db_transaction_mode(scope, transaction) != Some(TransactionMode::VersionChange) {
        return Err(IndexedDbError::InvalidState(
            "The object store is not running in a version change transaction.".to_owned(),
        ));
    }
    if indexed_db_object_store_is_deleted(scope, store) {
        return Err(IndexedDbError::InvalidState(
            "The object store was deleted.".to_owned(),
        ));
    }
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        return Err(IndexedDbError::TransactionInactive(
            "The transaction is not active.".to_owned(),
        ));
    }
    let database = indexed_db_object_store_database(scope, store).ok_or_else(unavailable)?;
    let handle =
        transaction_handle_from_value(scope, transaction.into()).ok_or_else(unavailable)?;
    let store_name = indexed_db_object_store_name(scope, store).ok_or_else(unavailable)?;
    Ok((transaction, database, handle, store_name))
}
