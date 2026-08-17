use super::{
    CursorDirection, CursorSnapshotEntry, DatabaseHandle, DatabaseInfo,
    INDEXED_DB_CURSOR_ENTRIES_SLOT, INDEXED_DB_CURSOR_KEY_ONLY_SLOT,
    INDEXED_DB_CURSOR_POSITION_SLOT, INDEXED_DB_DATABASE_CLOSED_SLOT,
    INDEXED_DB_DATABASE_HANDLE_SLOT, INDEXED_DB_DATABASE_KEY_SLOT, INDEXED_DB_EVENT_LISTENERS_SLOT,
    INDEXED_DB_KEY_RANGE_MARKER_SLOT, INDEXED_DB_REQUEST_ERROR_SLOT,
    INDEXED_DB_REQUEST_READY_STATE_SLOT, INDEXED_DB_REQUEST_RESULT_SLOT,
    INDEXED_DB_TRANSACTION_DB_KEY_SLOT, INDEXED_DB_TRANSACTION_FINISHED_SLOT,
    INDEXED_DB_TRANSACTION_HANDLE_SLOT, IdbKeyRangeQuery, IndexEntry, IndexInfo, IndexedDbError,
    IndexedDbExecutionOwner, IndexedDbExternalObject, IndexedDbManager,
    IndexedDbObjectStoreMetadata, IndexedDbRuntimeArray, IndexedDbStorageScope, IndexedDbValue,
    IndexedDbWrapperKind, Key, KeyPath, ObjectStoreInfo, PreparedObjectStoreWrite,
    PreparedObjectStoreWriteError, TransactionHandle, TransactionMode,
    context_host_ptr_from_global_bridge, dispatch_version_change_event,
    global_constructor_prototype, indexed_db_database_store_metadata,
    indexed_db_object_store_metadata, indexed_db_runtime_array, indexed_db_typed_execution_owner,
    indexed_db_typed_storage_scope, new_null_prototype_object, object_bool_property,
    object_hidden_value, object_number_property, object_property_as_object, object_string_property,
    push_unique_object_to_indexed_db_runtime_array, register_indexed_db_cursor_lifecycle,
    register_indexed_db_database_lifecycle, register_indexed_db_index_lifecycle,
    register_indexed_db_key_range_lifecycle, register_indexed_db_object_store_lifecycle,
    register_indexed_db_request_lifecycle, register_indexed_db_transaction_lifecycle,
    register_indexed_db_wrapper, register_indexed_db_wrapper_with_owner,
    remove_indexed_db_database_index_metadata, remove_indexed_db_database_store_metadata,
    replace_indexed_db_database_metadata, replace_indexed_db_runtime_array,
    set_indexed_db_database_index_metadata, set_indexed_db_database_store_metadata,
    set_indexed_db_internal_object_property, set_indexed_db_object_store_metadata,
    set_indexed_db_request_surface_value, set_indexed_db_slot_value, v8_string, v8str,
};
use std::collections::BTreeSet;
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

mod clone;
mod cursors;
mod env;
mod keys;
mod objects;
mod registry;

pub(super) use self::clone::*;
pub(super) use self::cursors::*;
pub(in crate::context_bootstrap) use self::env::indexed_db_usage_bytes_for_storage_key;
pub(crate) use self::env::set_indexed_db_manager_for_context;
pub(super) use self::env::*;
#[cfg(test)]
pub(crate) use self::env::{
    indexed_db_manager_context_slot_present_for_test,
    indexed_db_manager_isolate_slot_present_for_test,
};
pub(super) use self::keys::*;
pub(super) use self::objects::*;
pub(super) use self::registry::*;
