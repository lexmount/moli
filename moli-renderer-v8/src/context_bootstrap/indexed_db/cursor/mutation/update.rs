use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBCursor.update")]
struct IdbCursorUpdateArgs<'s> {
    #[webidl(required, converter = "raw")]
    value: v8::Local<'s, v8::Value>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_update_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbCursorUpdateArgs<'s>>(scope, &args) else {
        return;
    };
    let cursor = args.this();
    let Some((store, transaction, primary_key)) = cursor_mutation_state(scope, cursor) else {
        return;
    };
    let Some((clone, bytes)) = clone_value_for_transaction(scope, transaction, parsed.value) else {
        return;
    };
    let Some(metadata) = indexed_db_object_store_metadata(scope, store) else {
        return;
    };
    if let Some(path) = &metadata.info().key_path {
        match extract_key_from_value(scope, clone, path) {
            ExtractedKey::Key(key) if key == primary_key => {}
            ExtractedKey::Error(error) => {
                error.throw(scope);
                return;
            }
            _ => {
                let error = dom_exception_value(
                    scope,
                    "The value changes the cursor's effective key.",
                    "DataError",
                );
                scope.throw_exception(error);
                return;
            }
        }
    }
    let Some(request) = create_request_object(scope, cursor.into(), transaction) else {
        return;
    };
    rv.set(request.into());
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        return;
    }
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        return;
    };
    let Some(store_name) = indexed_db_object_store_name(scope, store) else {
        return;
    };
    let prepared = PreparedObjectStoreWrite {
        key: Some(primary_key),
        value: bytes,
        injection_path: None,
    };
    // Updating storage does not replace the cursor's cached value. The shared
    // executor also applies the store's unique-index and quota constraints.
    execute_object_store_write_request(
        scope,
        store,
        request,
        handle,
        &store_name,
        &prepared,
        false,
    );
}
