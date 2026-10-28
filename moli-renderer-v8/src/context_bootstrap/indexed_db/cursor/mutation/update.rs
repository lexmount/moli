use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBCursor.update")]
struct IdbCursorUpdateArgs<'s> {
    #[webidl(required, converter = "raw")]
    value: v8::Local<'s, v8::Value>,
}

fn cursor_update_preserves_primary_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    primary_key: &Key,
) -> bool {
    let Some(metadata) = indexed_db_object_store_metadata(scope, store) else {
        return false;
    };
    let Some(key_path) = &metadata.info().key_path else {
        return true;
    };
    let Some(derived_key) = extract_index_keys_from_value(scope, value, key_path, false)
        .into_iter()
        .next()
    else {
        return false;
    };
    &derived_key == primary_key
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
    let Some((store, _, primary_key)) = cursor_mutation_state(scope, cursor) else {
        return;
    };
    let Some(value_bytes) = serialize_js_value(scope, parsed.value) else {
        return;
    };
    if !cursor_update_preserves_primary_key(scope, store, parsed.value, &primary_key) {
        let error = dom_exception_value(
            scope,
            "Failed to execute 'update': the value changes the effective key.",
            "DataError",
        );
        scope.throw_exception(error);
        return;
    }
    let Some((request, handle, store_name)) = create_cursor_request(scope, cursor) else {
        return;
    };
    let quota_check = match storage_bucket_quota_check_for_object_store(scope, store) {
        Some(Ok(quota)) => Some(quota),
        Some(Err(error)) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
            rv.set(request.into());
            return;
        }
        None => None,
    };
    match with_indexed_db_manager(scope, |manager| {
        if let Some(quota) = quota_check {
            manager.put_with_quota(
                handle,
                &store_name,
                Some(primary_key.clone()),
                value_bytes,
                quota.quota_check,
            )
        } else {
            manager.put(handle, &store_name, Some(primary_key.clone()), value_bytes)
        }
    }) {
        Ok(key) => {
            let key = key_to_js_value(scope, &key);
            store_request_success(scope, request, key);
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
    rv.set(request.into());
}
