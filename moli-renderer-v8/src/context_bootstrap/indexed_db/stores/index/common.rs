use super::*;

pub(in crate::context_bootstrap::indexed_db) fn index_operation_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, String, IndexInfo)> {
    let store = indexed_db_index_object_store(scope, index)?;
    let transaction = indexed_db_object_store_transaction(scope, store)?;
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        return None;
    }
    let store_name = indexed_db_object_store_name(scope, store)?;
    let index_info = indexed_db_index_info(scope, index)?;
    Some((transaction, store_name, index_info))
}
