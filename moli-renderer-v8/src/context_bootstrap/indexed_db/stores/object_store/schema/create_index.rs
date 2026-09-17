use super::*;
use crate::context_bootstrap::indexed_db::{
    enqueue_transaction_operation_error, validate_existing_index_entries,
};
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBObjectStore.createIndex")]
struct IdbObjectStoreCreateIndexArgs {
    #[webidl(required, with = parse_index_name_arg)]
    index_name: IndexedDbName,
    #[webidl(required, name = "keyPath", with = parse_create_index_key_path_arg)]
    key_path: KeyPath,
    #[webidl(index = 2, with = parse_create_index_options_arg)]
    options: IdbIndexParameters,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBIndexParameters")]
struct IdbIndexParameters {
    // Dictionary conversion observes members in WebIDL lexicographic order.
    #[webidl(default = false)]
    multi_entry: bool,
    #[webidl(default = false)]
    unique: bool,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_create_index_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbObjectStoreCreateIndexArgs>(scope, &args) else {
        return;
    };
    let index_name = parsed.index_name;
    let key_path = parsed.key_path;
    let unique = parsed.options.unique;
    let multi_entry = parsed.options.multi_entry;
    let store = args.this();
    let (transaction, database, handle, store_name) =
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
    // Conversion and synchronous exceptions belong to the called method's
    // realm. Only the mutation and new index wrapper use the store's realm.
    let result = {
        let scope = &mut v8::ContextScope::new(scope, context);
        with_indexed_db_manager(scope, |manager| {
            manager.create_index(
                handle,
                &store_name,
                &index_name,
                IndexOptions {
                    key_path,
                    unique,
                    multi_entry,
                },
            )
        })
        .map(|info| {
            let _ = set_database_index_metadata(scope, database, &store_name, &info);
            // Existing operations execute eagerly, but their results are
            // delivered in the database task queue. Capture this creation's
            // constraint result now so later writes/deletes cannot change it,
            // and deliver its failure in the same operation order.
            if let Err(error) = validate_existing_index_entries(scope, handle, &store_name, &info) {
                enqueue_transaction_operation_error(scope, transaction, error);
            }
            create_index_object(scope, store, &info)
        })
    };
    match result {
        Ok(Some(index)) => rv.set(index.into()),
        Ok(None) => rv.set_undefined(),
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}

fn parse_create_index_key_path_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<KeyPath, webidl::WebIdlError> {
    parse_idb_key_path(
        scope,
        args.get(index),
        webidl::Context::argument("IDBObjectStore.createIndex", (index + 1) as usize),
    )
}

fn parse_create_index_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<IdbIndexParameters, webidl::WebIdlError> {
    let context = webidl::Context::argument("IDBObjectStore.createIndex", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}

fn parse_index_name_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<IndexedDbName, webidl::WebIdlError> {
    webidl::convert::<webidl::DomString16>(
        scope,
        args.get(index),
        webidl::Context::argument("IDBObjectStore.createIndex", (index + 1) as usize),
    )
    .map(|name| IndexedDbName::from_utf16(name.0))
}
