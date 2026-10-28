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
    let Some(target) = prepare_cursor_mutation(scope, cursor) else {
        return;
    };
    let Some(value) = serialize_js_value_for_transaction(scope, target.transaction, parsed.value)
    else {
        return;
    };
    let Some(metadata) = indexed_db_object_store_metadata(scope, target.store) else {
        return;
    };
    if let Some(key_path) = &metadata.info().key_path {
        let Some(clone) = deserialize_js_value(scope, &value) else {
            return;
        };
        let key = extract_object_store_key_from_value(scope, clone, key_path);
        if matches!(key, ObjectStoreKey::Failed) {
            return;
        }
        if !matches!(key,
            ObjectStoreKey::Key(key) if key == target.primary_key)
        {
            throw_cursor_mutation_error(
                scope,
                "The value changes the cursor's effective key.",
                "DataError",
            );
            return;
        }
    }
    let prepared = PreparedObjectStoreWrite {
        key: PreparedObjectStoreKey::Explicit(target.primary_key.clone()),
        value,
    };
    let Some(request) = target.create_request(scope, cursor) else {
        return;
    };
    schedule_transaction_operation(
        scope,
        target.transaction,
        target.store,
        request,
        &target.store_name,
        IndexedDbTransactionOperation::ObjectStoreWrite {
            prepared,
            add_only: false,
        },
    );
    // Updating storage does not replace the cursor's existing value snapshot.
    rv.set(request.into());
}
