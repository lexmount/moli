use super::*;
use crate::context_bootstrap::indexed_db::indexed_db_index_is_deleted;

pub(in crate::context_bootstrap::indexed_db) fn create_index_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    IndexedDbName,
    IndexInfo,
)> {
    if indexed_db_index_is_deleted(scope, index) {
        let error = dom_exception_value(scope, "The index has been deleted.", "InvalidStateError");
        scope.throw_exception(error);
        return None;
    }
    let store = indexed_db_index_object_store(scope, index)?;
    let transaction = indexed_db_object_store_transaction(scope, store)?;
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        let error = dom_exception_value(
            scope,
            "The transaction is not active.",
            "TransactionInactiveError",
        );
        scope.throw_exception(error);
        return None;
    }
    let request = create_request_object(scope, index.into(), transaction)?;
    let store_name = indexed_db_object_store_name(scope, store)?;
    let index_info = indexed_db_index_info(scope, index)?;
    Some((request, transaction, store_name, index_info))
}
