use super::*;
use crate::webidl;
use moli_indexeddb::{IndexOptionsValidationError, validate_index_options};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBObjectStore.createIndex")]
struct IdbObjectStoreCreateIndexArgs {
    #[webidl(required)]
    index_name: String,
    #[webidl(name = "keyPath", with = parse_create_index_key_path_arg)]
    key_path: KeyPath,
    #[webidl(index = 2, with = parse_create_index_options_arg)]
    options: IdbIndexParameters,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBIndexParameters")]
struct IdbIndexParameters {
    #[webidl(default = false)]
    unique: bool,
    #[webidl(default = false)]
    multi_entry: bool,
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
    if let Err(error) = validate_index_options(&key_path, multi_entry) {
        let error = dom_exception_value(
            scope,
            create_index_options_error_message(error),
            "InvalidAccessError",
        );
        scope.throw_exception(error);
        return;
    }
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
    }) {
        Ok(info) => {
            let _ = set_database_index_metadata(scope, database, &store_name, &info);
            if let Some(index) = create_index_object(scope, store, &info) {
                rv.set(index.into());
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

fn parse_create_index_key_path_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<KeyPath, webidl::WebIdlError> {
    if args.length() <= index {
        return Err(webidl::WebIdlError::custom_message(
            "Failed to execute 'createIndex': 2 arguments required, but only 1 present.",
        ));
    }
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

fn create_index_options_error_message(error: IndexOptionsValidationError) -> &'static str {
    match error {
        IndexOptionsValidationError::MultiEntrySequenceKeyPath => {
            "Failed to execute 'createIndex': multiEntry cannot be used with a sequence keyPath."
        }
    }
}
