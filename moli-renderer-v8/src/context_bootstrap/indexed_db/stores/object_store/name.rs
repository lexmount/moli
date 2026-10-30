use super::*;
use crate::context_bootstrap::indexed_db::rename_indexed_db_object_store;

pub(in crate::context_bootstrap::indexed_db) const STORE_NAME_FALLBACK_SLOT: &str =
    "moli.IndexedDb.storeName";

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("store receiver was validated");
    if let Some(name) = indexed_db_object_store_name(scope, receiver) {
        if let Some(name) = crate::util::v8_string_from_utf16_units(scope, name.as_utf16()) {
            rv.set(name.into());
        }
    } else if let Some(name) =
        crate::util::get_private_value(scope, receiver, STORE_NAME_FALLBACK_SLOT)
    {
        rv.set(name);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_name_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("store receiver was validated");
    let name = match crate::webidl::convert::<crate::webidl::DomString16>(
        scope,
        args.get(0),
        crate::webidl::Context::member("IDBObjectStore", "name"),
    ) {
        Ok(name) => moli_indexeddb::IndexedDbName::from_utf16(name.0),
        Err(error) => {
            crate::webidl::throw_error(scope, &error);
            return;
        }
    };
    if let Err(error) = rename_indexed_db_object_store(scope, receiver, name) {
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
    }
}
