use super::{
    INDEX_OBJECT_STORE, OBJECT_STORE_TRANSACTION, SCHEMA_AUTO_INCREMENT_SLOT, SCHEMA_KEY_PATH_SLOT,
    SCHEMA_MULTI_ENTRY_SLOT, SCHEMA_NAME_SLOT, SCHEMA_UNIQUE_SLOT, idb_cursor_advance_callback,
    idb_cursor_continue_callback, idb_cursor_continue_primary_key_callback,
    idb_cursor_delete_callback, idb_cursor_update_callback, idb_database_close_callback,
    idb_database_create_object_store_callback, idb_database_delete_object_store_callback,
    idb_database_transaction_callback, idb_factory_cmp_callback, idb_factory_databases_callback,
    idb_factory_delete_database_callback, idb_factory_open_callback, idb_index_count_callback,
    idb_index_get_all_callback, idb_index_get_all_keys_callback,
    idb_index_get_all_records_callback, idb_index_get_callback, idb_index_get_key_callback,
    idb_index_name_setter, idb_index_open_cursor_callback, idb_index_open_key_cursor_callback,
    idb_key_range_bound_callback, idb_key_range_includes_callback,
    idb_key_range_lower_bound_callback, idb_key_range_only_callback,
    idb_key_range_upper_bound_callback, idb_object_store_add_callback,
    idb_object_store_clear_callback, idb_object_store_count_callback,
    idb_object_store_create_index_callback, idb_object_store_delete_callback,
    idb_object_store_delete_index_callback, idb_object_store_get_all_callback,
    idb_object_store_get_all_keys_callback, idb_object_store_get_all_records_callback,
    idb_object_store_get_callback, idb_object_store_get_key_callback,
    idb_object_store_index_callback, idb_object_store_index_names_getter,
    idb_object_store_name_setter, idb_object_store_open_cursor_callback,
    idb_object_store_open_key_cursor_callback, idb_object_store_put_callback,
    idb_schema_attribute_getter, idb_transaction_abort_callback, idb_transaction_commit_callback,
    idb_transaction_object_store_callback, indexed_db_runtime_factory,
    install_dom_string_list_template_bindings,
};
use anyhow::{Result, anyhow};

// Argument conversion and synchronous errors use the binding realm. Request,
// cursor and schema-handle factories enter their native owner realm themselves.

mod constructors;
mod state;

pub(in crate::context_bootstrap) use self::constructors::install_indexed_db_template_bindings;
pub(in crate::context_bootstrap) use self::state::ensure_indexed_db_runtime_state;
pub(crate) use self::state::{install_worker_indexed_db_runtime_state, window_indexed_db_getter};
