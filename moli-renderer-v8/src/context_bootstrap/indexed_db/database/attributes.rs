use crate::util::{get_private_value, set_private_value};

pub(in crate::context_bootstrap::indexed_db) const DATABASE_NAME_SLOT: &str =
    "__moli_idb_database_name";
pub(in crate::context_bootstrap::indexed_db) const DATABASE_VERSION_SLOT: &str =
    "__moli_idb_database_version";

pub(in crate::context_bootstrap::indexed_db) fn idb_database_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), DATABASE_NAME_SLOT) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_version_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), DATABASE_VERSION_SLOT) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn set_indexed_db_database_version(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
    version: u64,
) {
    // A connection retains its version after close. Upgrade rollback changes
    // this native value without redefining any author-owned property.
    let version = v8::Number::new(scope, version as f64);
    set_private_value(scope, database, DATABASE_VERSION_SLOT, version.into());
}
