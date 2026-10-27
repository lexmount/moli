use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_index_is_deleted, object_store_active_transaction,
};

pub(in crate::context_bootstrap::indexed_db) fn index_operation_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, String, IndexInfo)> {
    if indexed_db_index_is_deleted(scope, index) {
        let error = dom_exception_value(scope, "The index has been deleted.", "InvalidStateError");
        scope.throw_exception(error);
        return None;
    }
    let store = indexed_db_index_object_store(scope, index)?;
    let transaction = object_store_active_transaction(scope, store)?;
    let store_name = indexed_db_object_store_name(scope, store)?;
    let index_info = indexed_db_index_info(scope, index)?;
    Some((transaction, store_name, index_info))
}
