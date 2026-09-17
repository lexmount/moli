use super::*;

pub(super) fn version_change_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let Some(transaction) = object_property_as_object(
        scope,
        database,
        INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
    ) else {
        let error = dom_exception_value(
            scope,
            "The database is not running a version change transaction.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return None;
    };
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        let error = dom_exception_value(
            scope,
            "The upgrade transaction is not active.",
            "TransactionInactiveError",
        );
        scope.throw_exception(error);
        return None;
    }
    Some(transaction)
}
