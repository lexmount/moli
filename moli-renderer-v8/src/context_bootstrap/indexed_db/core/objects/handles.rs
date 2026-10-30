use super::*;

pub(in crate::context_bootstrap::indexed_db) fn database_handle_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<DatabaseHandle> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    crate::context_bootstrap::indexed_db::indexed_db_native_database_handle(scope, object)
}

pub(in crate::context_bootstrap::indexed_db) fn transaction_handle_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<TransactionHandle> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    crate::context_bootstrap::indexed_db::indexed_db_native_transaction_handle(scope, object)
}
