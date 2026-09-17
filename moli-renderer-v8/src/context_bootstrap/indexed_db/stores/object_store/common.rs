use super::*;
use crate::context_bootstrap::indexed_db::indexed_db_object_store_is_deleted;

pub(in crate::context_bootstrap::indexed_db) fn object_store_active_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if indexed_db_object_store_is_deleted(scope, source) {
        let error = dom_exception_value(
            scope,
            "The object store has been deleted.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return None;
    }
    let transaction = indexed_db_object_store_transaction(scope, source)?;
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
    Some(transaction)
}

pub(in crate::context_bootstrap::indexed_db) fn create_store_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let transaction = object_store_active_transaction(scope, source)?;
    let request = create_request_object(scope, source.into(), transaction)?;
    Some((request, transaction))
}

pub(in crate::context_bootstrap::indexed_db) fn object_store_operation_common<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    IndexedDbName,
)> {
    let (request, transaction) = create_store_request(scope, store)?;
    let name = indexed_db_object_store_name(scope, store)?;
    Some((request, transaction, name))
}
