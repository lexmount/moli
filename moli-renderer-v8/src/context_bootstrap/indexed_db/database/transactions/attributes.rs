use crate::util::get_private_value;

pub(in crate::context_bootstrap::indexed_db) const TRANSACTION_DB_SLOT: &str =
    "__moli_idb_transaction_db";
pub(in crate::context_bootstrap::indexed_db) const TRANSACTION_MODE_SLOT: &str =
    "__moli_idb_transaction_mode";
pub(in crate::context_bootstrap::indexed_db) const TRANSACTION_ERROR_SLOT: &str =
    "__moli_idb_transaction_error";

pub(in crate::context_bootstrap::indexed_db) fn idb_transaction_db_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), TRANSACTION_DB_SLOT) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_transaction_mode_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), TRANSACTION_MODE_SLOT) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_transaction_error_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), TRANSACTION_ERROR_SLOT) {
        rv.set(value);
    }
}
