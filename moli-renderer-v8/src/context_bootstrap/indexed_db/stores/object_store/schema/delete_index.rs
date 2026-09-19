use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBObjectStore.deleteIndex")]
struct IdbObjectStoreDeleteIndexArgs {
    #[webidl(required, with = parse_index_name_arg)]
    index_name: IndexedDbName,
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
    let (_transaction, database, handle, store_name) =
        match object_store_versionchange_common(scope, store) {
            Ok(state) => state,
            Err(error) => {
                let error = request_error_object(scope, &error);
                scope.throw_exception(error);
                return;
            }
        };
    let Some(context) = store.get_creation_context(scope) else {
        return;
    };
    let result = {
        let scope = &mut v8::ContextScope::new(scope, context);
        with_indexed_db_manager(scope, |manager| {
            manager.delete_index(handle, &store_name, &index_name)
        })
        .map(|()| {
            let _ = remove_database_index_metadata(scope, database, &store_name, &index_name);
        })
    };
    match result {
        Ok(()) => rv.set_undefined(),
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}

fn parse_index_name_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<IndexedDbName, webidl::WebIdlError> {
    webidl::convert::<webidl::DomString16>(
        scope,
        args.get(index),
        webidl::Context::argument("IDBObjectStore.deleteIndex", (index + 1) as usize),
    )
    .map(|name| IndexedDbName::from_utf16(name.0))
}
