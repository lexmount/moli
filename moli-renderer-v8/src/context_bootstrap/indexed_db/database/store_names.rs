use crate::context_bootstrap::indexed_db::{
    indexed_db_database_store_names, indexed_db_transaction_store_names, new_idb_name_list,
};

// A fresh sorted list is a projection of native schema/scope. Schema mutations
// never read or write page properties, so own author accessors cannot intercept them.
pub(in crate::context_bootstrap::indexed_db) fn idb_database_object_store_names_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let names = indexed_db_database_store_names(scope, args.this());
    rv.set(new_idb_name_list(scope, &names).into());
}

pub(in crate::context_bootstrap::indexed_db) fn idb_transaction_object_store_names_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let names = indexed_db_transaction_store_names(scope, args.this());
    rv.set(new_idb_name_list(scope, &names).into());
}
