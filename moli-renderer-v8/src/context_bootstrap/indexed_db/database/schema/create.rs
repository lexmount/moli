use super::common::version_change_transaction;
use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBDatabase.createObjectStore")]
struct IdbDatabaseCreateObjectStoreArgs {
    #[webidl(required, with = parse_store_name_arg)]
    name: IndexedDbName,
    #[webidl(index = 1, with = parse_create_object_store_options_arg)]
    options: IdbObjectStoreParameters,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBObjectStoreParameters")]
struct IdbObjectStoreParameters {
    // Dictionary conversion observes members in WebIDL lexicographic order.
    #[webidl(default = false)]
    auto_increment: bool,
    #[webidl(with = parse_optional_idb_key_path_member)]
    key_path: Option<KeyPath>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_create_object_store_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbDatabaseCreateObjectStoreArgs>(scope, &args) else {
        return;
    };
    let name = parsed.name;
    let key_path = parsed.options.key_path;
    let auto_increment = parsed.options.auto_increment;
    let database = args.this();
    let Some(transaction) = version_change_transaction(scope, database) else {
        return;
    };
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        rv.set_undefined();
        return;
    };
    let info = ObjectStoreInfo {
        name: name.clone(),
        key_path: key_path.clone(),
        auto_increment,
        index_names: Vec::new(),
    };
    match with_indexed_db_manager(scope, |manager| {
        manager.create_object_store(
            handle,
            &name,
            ObjectStoreOptions {
                key_path,
                auto_increment,
            },
        )
    }) {
        Ok(()) => {
            let _ = set_database_store_metadata(scope, database, &info, &[]);
            sync_transaction_object_store_names_from_database(scope, transaction, database);
            if let Some(store) = create_object_store_object(scope, database, transaction, &info) {
                rv.set(store.into());
            } else {
                rv.set_undefined();
            }
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}

fn parse_create_object_store_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<IdbObjectStoreParameters, webidl::WebIdlError> {
    let context = webidl::Context::argument("IDBDatabase.createObjectStore", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}

fn parse_store_name_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<IndexedDbName, webidl::WebIdlError> {
    webidl::convert::<webidl::DomString16>(
        scope,
        args.get(index),
        webidl::Context::argument("IDBDatabase.createObjectStore", (index + 1) as usize),
    )
    .map(|name| IndexedDbName::from_utf16(name.0))
}
