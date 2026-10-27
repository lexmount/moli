use super::*;

pub(in crate::context_bootstrap::indexed_db) fn create_store_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let transaction = indexed_db_object_store_transaction(scope, source)?;
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        return None;
    }
    let request = create_request_object(scope, source.into(), transaction)?;
    queue_transaction_request(scope, transaction, request);
    Some((request, transaction))
}

pub(in crate::context_bootstrap::indexed_db) fn object_store_operation_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, String)> {
    let transaction = indexed_db_object_store_transaction(scope, store)?;
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        return None;
    }
    let name = indexed_db_object_store_name(scope, store)?;
    Some((transaction, name))
}
