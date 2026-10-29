use super::*;

pub(in crate::context_bootstrap::indexed_db) fn execute_object_store_write_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    prepared: &PreparedObjectStoreWrite,
    add_only: bool,
) {
    v8::tc_scope!(let scope, scope);
    let result = execute_prepared_write(scope, store, handle, store_name, prepared, add_only);
    if scope.has_caught() {
        if scope.can_continue()
            && let Some(error) = scope.exception()
        {
            scope.reset();
            store_request_error(scope, request, error);
        }
        return;
    }
    match result {
        Ok(key) => {
            let key = key_to_js_value(scope, &key);
            store_request_success(scope, request, key);
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}

fn execute_prepared_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    prepared: &PreparedObjectStoreWrite,
    add_only: bool,
) -> Result<Key, IndexedDbError> {
    // Inline key injection and unique-index checks share one restored snapshot.
    // Other writes can store the prepared bytes without restoring a JS value.
    let mut decoded = None;
    let mut read_value = |scope: &mut v8::PinScope<'s, '_>| {
        if let Some(value) = decoded {
            return Ok(value);
        }
        let value = deserialize_js_value(scope, &prepared.value).ok_or_else(|| {
            IndexedDbError::Serialization("Failed to read a prepared IndexedDB value.".into())
        })?;
        decoded = Some(value);
        Ok(value)
    };
    // Only storage execution can consume the generator; synchronous rejection
    // and waiting for another transaction cannot change its state.
    let key = match &prepared.key {
        PreparedObjectStoreKey::Explicit(key) => key.clone(),
        PreparedObjectStoreKey::Generate | PreparedObjectStoreKey::GenerateInline { .. } => {
            with_indexed_db_manager(scope, |manager| manager.generate_key(handle, store_name))?
        }
    };
    let value_bytes = if let PreparedObjectStoreKey::GenerateInline { key_path } = &prepared.key {
        let value = read_value(scope)?;
        inject_key_path_into_value(scope, value, key_path, &key).ok_or_else(|| {
            IndexedDbError::Serialization("Failed to inject a prepared inline key.".into())
        })?;
        serialize_js_value(scope, value).ok_or_else(|| {
            IndexedDbError::Serialization("Failed to store a prepared IndexedDB value.".into())
        })?
    } else {
        prepared.value.clone()
    };
    enforce_object_store_unique_constraints(scope, store, handle, store_name, &key, read_value)?;
    let quota = storage_bucket_quota_check_for_object_store(scope, store).transpose()?;
    with_indexed_db_manager(scope, |manager| match (add_only, quota) {
        (true, Some(quota)) => manager.add_with_quota(
            handle,
            store_name,
            Some(key),
            value_bytes,
            quota.quota_check,
        ),
        (false, Some(quota)) => manager.put_with_quota(
            handle,
            store_name,
            Some(key),
            value_bytes,
            quota.quota_check,
        ),
        (true, None) => manager.add(handle, store_name, Some(key), value_bytes),
        (false, None) => manager.put(handle, store_name, Some(key), value_bytes),
    })
}
