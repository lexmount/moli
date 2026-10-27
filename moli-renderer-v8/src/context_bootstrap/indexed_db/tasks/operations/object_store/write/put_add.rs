use super::*;

pub(in crate::context_bootstrap::indexed_db) fn execute_object_store_write_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    value: v8::Local<'s, v8::Value>,
    key_value: v8::Local<'s, v8::Value>,
    add_only: bool,
) {
    let explicit_key = match parse_idb_key(scope, key_value) {
        Ok(key) => key,
        Err(error) => {
            let error = error.into_value(scope);
            store_request_error(scope, request, error);
            return;
        }
    };
    let prepared =
        match prepare_object_store_write(scope, store, handle, store_name, value, explicit_key) {
            Ok(prepared) => prepared,
            Err(PreparedObjectStoreWriteError::DomException { message, name }) => {
                let error = dom_exception_value(scope, message, name);
                store_request_error(scope, request, error);
                return;
            }
            Err(PreparedObjectStoreWriteError::Backend(error)) => {
                let error = request_error_object(scope, &error);
                store_request_error(scope, request, error);
                return;
            }
        };
    let Some(value_bytes) = serialize_js_value(scope, prepared.value) else {
        return;
    };
    let Some(primary_key) = prepared.key.clone() else {
        let error = dom_exception_value(
            scope,
            "Failed to execute the operation: a key is required for stores without autoIncrement.",
            "DataError",
        );
        store_request_error(scope, request, error);
        return;
    };
    if let Err(error) = enforce_object_store_unique_constraints(
        scope,
        store,
        handle,
        store_name,
        &primary_key,
        prepared.value,
    ) {
        let error = request_error_object(scope, &error);
        store_request_error(scope, request, error);
        return;
    }
    let quota_check = match storage_bucket_quota_check_for_object_store(scope, store) {
        Some(Ok(quota)) => Some(quota),
        Some(Err(error)) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
            return;
        }
        None => None,
    };
    let result = if add_only {
        with_indexed_db_manager(scope, |manager| {
            if let Some(quota) = quota_check {
                manager.add_with_quota(
                    handle,
                    store_name,
                    prepared.key.clone(),
                    value_bytes,
                    quota.quota_check,
                )
            } else {
                manager.add(handle, store_name, prepared.key.clone(), value_bytes)
            }
        })
    } else {
        with_indexed_db_manager(scope, |manager| {
            if let Some(quota) = quota_check {
                manager.put_with_quota(
                    handle,
                    store_name,
                    prepared.key.clone(),
                    value_bytes,
                    quota.quota_check,
                )
            } else {
                manager.put(handle, store_name, prepared.key.clone(), value_bytes)
            }
        })
    };
    match result {
        Ok(key) => {
            let js_key = key_to_js_value(scope, &key);
            store_request_success(scope, request, js_key);
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}
