use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBObjectStore.deleteIndex")]
struct IdbObjectStoreDeleteIndexArgs {
    #[webidl(required)]
    index_name: String,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_delete_index_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbObjectStoreDeleteIndexArgs>(scope, &args) else {
        return;
    };
    let index_name = parsed.index_name;
    let store = args.this();
    let Some((_transaction, database, handle, store_name)) =
        object_store_versionchange_common(scope, store)
    else {
        let error = dom_exception_value(
            scope,
            "The object store is not running in a version change transaction.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return;
    };
    match with_indexed_db_manager(scope, |manager| {
        manager.delete_index(handle, &store_name, &index_name)
    }) {
        Ok(()) => {
            let _ = remove_database_index_metadata(scope, database, &store_name, &index_name);
            rv.set_undefined();
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}
