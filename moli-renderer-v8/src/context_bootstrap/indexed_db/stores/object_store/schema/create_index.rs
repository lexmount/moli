use super::*;
use crate::context_bootstrap::indexed_db::{
    create_failed_index_object, schedule_schema_failure, schedule_schema_operation,
    typed_state::sql::indexed_db_request_sql_client,
};
use crate::webidl;
use moli_indexeddb::sqlite::service::Command;
use moli_indexeddb::{IndexOptionsValidationError, validate_index_options};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBObjectStore.createIndex")]
struct IdbObjectStoreCreateIndexArgs {
    #[webidl(required, converter = "raw")]
    index_name: webidl::DomString16,
    #[webidl(required, name = "keyPath", with = parse_create_index_key_path_arg)]
    key_path: KeyPath,
    #[webidl(index = 2, dictionary)]
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
    let index_name = moli_indexeddb::IndexedDbName::from_utf16(parsed.index_name.0);
    let key_path = parsed.key_path;
    let unique = parsed.options.unique;
    let multi_entry = parsed.options.multi_entry;
    let store = args.this();
    let (transaction, database, handle, store_name) =
        match object_store_versionchange_common(scope, store) {
            Ok(values) => values,
            Err(error) => {
                let error = request_error_object(scope, &error);
                scope.throw_exception(error);
                return;
            }
        };
    let Some(client) = indexed_db_request_sql_client(scope, transaction) else {
        return;
    };
    let result = if index_info_from_store_metadata(scope, store, &index_name).is_some() {
        Err(moli_indexeddb::IndexedDbError::Constraint(
            "The index already exists".into(),
        ))
    } else if !key_path.is_valid() {
        Err(moli_indexeddb::IndexedDbError::Syntax(
            "The keyPath is not valid.".into(),
        ))
    } else if let Err(error) = validate_index_options(&key_path, multi_entry) {
        Err(moli_indexeddb::IndexedDbError::InvalidAccess(
            create_index_options_error_message(error).into(),
        ))
    } else {
        let info = crate::context_bootstrap::indexed_db::IndexInfo {
            name: index_name.clone(),
            key_path: key_path.clone(),
            unique,
            multi_entry,
        };
        schedule_schema_operation(
            scope,
            transaction,
            client,
            Command::CreateIndex {
                transaction: handle,
                store: store_name.clone(),
                name: index_name,
                options: IndexOptions {
                    key_path,
                    unique,
                    multi_entry,
                },
            },
            Some((store_name.clone(), info.clone())),
        )
        .map(|()| info)
    };
    match result {
        Ok(info) => {
            if let Err(error) = set_database_index_metadata(scope, database, &store_name, &info) {
                schedule_schema_failure(scope, transaction, error);
                if let Some(index) = create_failed_index_object(scope, store, &info) {
                    rv.set(index.into());
                }
                return;
            }
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

fn create_index_options_error_message(error: IndexOptionsValidationError) -> &'static str {
    match error {
        IndexOptionsValidationError::MultiEntrySequenceKeyPath => {
            "Failed to execute 'createIndex': multiEntry cannot be used with a sequence keyPath."
        }
    }
}
