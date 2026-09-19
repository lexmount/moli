use super::*;

pub(in crate::context_bootstrap::indexed_db) fn execute_object_store_write_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &IndexedDbName,
    prepared: &PreparedObjectStoreWrite,
    add_only: bool,
) {
    let primary_key = match prepared.key.clone().map(Ok).unwrap_or_else(|| {
        with_indexed_db_manager(scope, |manager| {
            manager.next_generated_key(handle, store_name)
        })
    }) {
        Ok(key) => key,
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
            return;
        }
    };
    let Some(value) = deserialize_js_value(scope, &prepared.value) else {
        return;
    };
    let value_bytes = if let Some(path) = &prepared.injection_path {
        if inject_key_path_into_value(scope, value, path, &primary_key).is_none() {
            return;
        }
        let Some(bytes) = serialize_js_value(scope, value) else {
            return;
        };
        bytes
    } else {
        prepared.value.clone()
    };
    if let Err(error) = enforce_object_store_unique_constraints(
        scope,
        store,
        handle,
        store_name,
        &primary_key,
        value,
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
                    Some(primary_key.clone()),
                    value_bytes,
                    quota.quota_check,
                )
            } else {
                manager.add(handle, store_name, Some(primary_key.clone()), value_bytes)
            }
        })
    } else {
        with_indexed_db_manager(scope, |manager| {
            if let Some(quota) = quota_check {
                manager.put_with_quota(
                    handle,
                    store_name,
                    Some(primary_key.clone()),
                    value_bytes,
                    quota.quota_check,
                )
            } else {
                manager.put(handle, store_name, Some(primary_key.clone()), value_bytes)
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
