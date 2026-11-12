use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_database_upgrade_transaction, indexed_db_transaction_status,
};

pub(super) fn version_change_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let Some(transaction) = indexed_db_database_upgrade_transaction(scope, database) else {
        let error = dom_exception_value(
            scope,
            "The database is not running a version change transaction.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return None;
    };
    if !indexed_db_transaction_status(scope, transaction).is_some_and(|state| state.active) {
        let error = dom_exception_value(
            scope,
            "The version change transaction is not active.",
            "TransactionInactiveError",
        );
        scope.throw_exception(error);
        return None;
    }
    Some(transaction)
}
