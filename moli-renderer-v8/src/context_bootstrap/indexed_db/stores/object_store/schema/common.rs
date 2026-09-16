use super::*;
use crate::context_bootstrap::indexed_db::indexed_db_object_store_is_deleted;

pub(in crate::context_bootstrap::indexed_db) fn object_store_versionchange_common<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    TransactionHandle,
    String,
)> {
    if indexed_db_object_store_is_deleted(scope, store) {
        return None;
    }
    let transaction = indexed_db_object_store_transaction(scope, store)?;
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
        || object_string_property(scope, transaction, "mode").as_deref() != Some("versionchange")
    {
        return None;
    }
    let database = indexed_db_object_store_database(scope, store)?;
    let handle = transaction_handle_from_value(scope, transaction.into())?;
    let store_name = indexed_db_object_store_name(scope, store)?;
    Some((transaction, database, handle, store_name))
}
