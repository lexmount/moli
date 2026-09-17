use super::{
    idb_cursor_advance_callback, idb_cursor_continue_callback,
    idb_cursor_continue_primary_key_callback, idb_cursor_delete_callback,
    idb_cursor_update_callback, idb_database_close_callback,
    idb_database_create_object_store_callback, idb_database_delete_object_store_callback,
    idb_database_transaction_callback, idb_factory_cmp_callback, idb_factory_databases_callback,
    idb_factory_delete_database_callback, idb_factory_open_callback,
    idb_index_count_callback as idb_index_count_callback_in_current_context,
    idb_index_get_all_callback, idb_index_get_all_keys_callback,
    idb_index_get_all_records_callback,
    idb_index_get_callback as idb_index_get_callback_in_current_context,
    idb_index_get_key_callback as idb_index_get_key_callback_in_current_context,
    idb_index_name_getter, idb_index_name_setter,
    idb_index_open_cursor_callback as idb_index_open_cursor_callback_in_current_context,
    idb_index_open_key_cursor_callback as idb_index_open_key_cursor_callback_in_current_context,
    idb_key_range_bound_callback, idb_key_range_includes_callback,
    idb_key_range_lower_bound_callback, idb_key_range_only_callback,
    idb_key_range_upper_bound_callback,
    idb_object_store_add_callback as idb_object_store_add_callback_in_current_context,
    idb_object_store_clear_callback as idb_object_store_clear_callback_in_current_context,
    idb_object_store_count_callback as idb_object_store_count_callback_in_current_context,
    idb_object_store_create_index_callback,
    idb_object_store_delete_callback as idb_object_store_delete_callback_in_current_context,
    idb_object_store_delete_index_callback, idb_object_store_get_all_callback,
    idb_object_store_get_all_keys_callback, idb_object_store_get_all_records_callback,
    idb_object_store_get_callback as idb_object_store_get_callback_in_current_context,
    idb_object_store_get_key_callback as idb_object_store_get_key_callback_in_current_context,
    idb_object_store_index_callback,
    idb_object_store_open_cursor_callback as idb_object_store_open_cursor_callback_in_current_context,
    idb_object_store_open_key_cursor_callback as idb_object_store_open_key_cursor_callback_in_current_context,
    idb_object_store_put_callback as idb_object_store_put_callback_in_current_context,
    idb_transaction_abort_callback, idb_transaction_commit_callback,
    idb_transaction_object_store_callback, indexed_db_runtime_factory,
    install_dom_string_list_template_bindings, v8str,
};
use anyhow::{Result, anyhow};

macro_rules! indexed_db_receiver_realm_callback {
    ($callback:ident, $callback_in_current_context:ident) => {
        fn $callback<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            rv: v8::ReturnValue<'s, v8::Value>,
        ) {
            let receiver = args.this();
            let Some(relevant_context) = receiver.get_creation_context(scope) else {
                return;
            };
            if relevant_context == scope.get_current_context() {
                $callback_in_current_context(scope, args, rv);
                return;
            }
            let scope = &mut v8::ContextScope::new(scope, relevant_context);
            $callback_in_current_context(scope, args, rv);
        }
    };
}

indexed_db_receiver_realm_callback!(
    idb_index_count_callback,
    idb_index_count_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_index_get_callback,
    idb_index_get_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_index_get_key_callback,
    idb_index_get_key_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_index_open_cursor_callback,
    idb_index_open_cursor_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_index_open_key_cursor_callback,
    idb_index_open_key_cursor_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_object_store_add_callback,
    idb_object_store_add_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_object_store_clear_callback,
    idb_object_store_clear_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_object_store_count_callback,
    idb_object_store_count_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_object_store_delete_callback,
    idb_object_store_delete_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_object_store_get_callback,
    idb_object_store_get_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_object_store_get_key_callback,
    idb_object_store_get_key_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_object_store_open_cursor_callback,
    idb_object_store_open_cursor_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_object_store_open_key_cursor_callback,
    idb_object_store_open_key_cursor_callback_in_current_context
);
indexed_db_receiver_realm_callback!(
    idb_object_store_put_callback,
    idb_object_store_put_callback_in_current_context
);

mod constructors;
mod helpers;
mod state;

pub(in crate::context_bootstrap) use self::constructors::install_indexed_db_template_bindings;
use self::helpers::*;
pub(in crate::context_bootstrap) use self::state::ensure_indexed_db_runtime_state;
pub(crate) use self::state::{install_worker_indexed_db_runtime_state, window_indexed_db_getter};
