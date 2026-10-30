use super::{
    CursorDirection, INDEXED_DB_TRANSACTION_ACTIVE_SLOT, INDEXED_DB_TRANSACTION_STARTED_SLOT,
    IdbKeyRangeQuery, IndexInfo, IndexOptions, IndexedDbCursorOpenOperation,
    IndexedDbTransactionOperation, KeyPath, ObjectStoreKey, TransactionHandle, compare_idb_keys,
    create_index_object, create_key_range_object, create_request_object, dom_exception_value,
    index_info_from_store_metadata, indexed_db_index_info, indexed_db_index_object_store,
    indexed_db_object_store_database, indexed_db_object_store_name,
    indexed_db_object_store_transaction, key_in_range, object_bool_property,
    parse_cursor_direction, parse_cursor_direction_with_context, parse_idb_key, parse_idb_key_path,
    parse_key_or_range, parse_key_range_from_value, parse_optional_count,
    prepare_object_store_write, queue_transaction_request, remove_database_index_metadata,
    request_error_object, schedule_transaction_operation, set_database_index_metadata,
    submit_cursor_open_operation, transaction_handle_from_value,
};
use crate::webidl;

mod attributes;
pub(super) mod collection;
pub(in crate::context_bootstrap::indexed_db::stores) mod cursor_open_parse;
mod index;
mod key_range;
mod object_store;

pub(super) use self::attributes::*;
pub(super) use self::index::*;
pub(super) use self::key_range::*;
pub(super) use self::object_store::*;

// Allocate and admit only after key conversion has succeeded. Conversion can
// run author code, so a transaction aborted by a getter must be checked again.
fn create_operation_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
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
    let request = create_request_object(scope, source.into(), transaction)?;
    queue_transaction_request(scope, transaction, request);
    Some(request)
}
