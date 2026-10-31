use super::*;
use crate::context_bootstrap::indexed_db::rename_indexed_db_index;

pub(in crate::context_bootstrap::indexed_db) const INDEX_NAME_FALLBACK_SLOT: &str =
    "moli.IndexedDb.indexName";

pub(in crate::context_bootstrap::indexed_db) fn idb_index_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("index receiver was validated");
    if let Some(info) = indexed_db_index_info(scope, receiver) {
        if let Some(name) = crate::util::v8_string_from_utf16_units(scope, info.name.as_utf16()) {
            rv.set(name.into());
        }
    } else if let Some(name) =
        crate::util::get_private_value(scope, receiver, INDEX_NAME_FALLBACK_SLOT)
    {
        rv.set(name);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_name_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("index receiver was validated");
    let name = match crate::webidl::convert::<crate::webidl::DomString16>(
        scope,
        args.get(0),
        crate::webidl::Context::member("IDBIndex", "name"),
    ) {
        Ok(name) => moli_indexeddb::IndexedDbName::from_utf16(name.0),
        Err(error) => {
            crate::webidl::throw_error(scope, &error);
            return;
        }
    };
    // ToString can abort, delete or rename the index; read state afterwards.
    if let Err(error) = rename_indexed_db_index(scope, receiver, name) {
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
    }
}
