use super::super::{
    budget::{Allocations, reserve_vec},
    value::{parse_idb_key_value, parse_idb_key_value_at_depth},
};
use super::*;
use moli_indexeddb::ReadBudget;

#[cfg(test)]
fn extract_index_keys_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &KeyPath,
    multi_entry: bool,
) -> Result<Vec<Key>, KeyConversionError> {
    let mut budget = u64::MAX;
    extract_index_keys_budgeted(scope, value, key_path, multi_entry, &mut budget)
}

pub(in crate::context_bootstrap::indexed_db) fn extract_index_keys_budgeted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &KeyPath,
    multi_entry: bool,
    budget: &mut dyn ReadBudget,
) -> Result<Vec<Key>, KeyConversionError> {
    v8::tc_scope!(let scope, scope);
    let mut allocations = Allocations::new(budget);
    let result = index_keys(scope, value, key_path, multi_entry, &mut allocations);
    if matches!(&result, Err(KeyConversionError::Terminated))
        || scope.is_execution_terminating()
        || !scope.can_continue()
    {
        if scope.has_caught() {
            scope.rethrow();
        }
        return Err(KeyConversionError::Terminated);
    }
    // Capacity failure is not an invalid key, even if V8 also reports a
    // conversion exception. Ordinary throwing key-path getters are omitted.
    if matches!(
        &result,
        Err(KeyConversionError::Backend(IndexedDbError::Resource(_)))
    ) {
        scope.reset();
        return result;
    }
    if scope.has_caught() {
        scope.reset();
        return Ok(Vec::new());
    }
    let keys = result?;
    let retained = keys.capacity() * std::mem::size_of::<Key>()
        + keys.iter().map(Key::heap_bytes).sum::<usize>();
    allocations.keep(retained)?;
    Ok(keys)
}

fn parsed_index_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    budget: &mut dyn ReadBudget,
    depth: usize,
) -> Result<Option<Key>, KeyConversionError> {
    match parse_idb_key_value_at_depth(scope, value, budget, depth) {
        Err(KeyConversionError::Invalid(_)) => Ok(None),
        result => result,
    }
}

fn index_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &KeyPath,
    multi_entry: bool,
    budget: &mut dyn ReadBudget,
) -> Result<Vec<Key>, KeyConversionError> {
    if let KeyPath::Sequence(paths) = key_path {
        let mut keys = Vec::new();
        for path in paths {
            let Some(value) = value_at_string_key_path(scope, value, path)? else {
                return Ok(Vec::new());
            };
            let Some(key) = parsed_index_key(scope, value, budget, 1)? else {
                return Ok(Vec::new());
            };
            reserve_vec(budget, &mut keys, 1)?;
            keys.push(key);
        }
        let mut result = Vec::new();
        reserve_vec(budget, &mut result, 1)?;
        result.push(Key::Array(keys));
        return Ok(result);
    }
    let KeyPath::String(path) = key_path else {
        return Ok(Vec::new());
    };
    let Some(current) = value_at_string_key_path(scope, value, path)? else {
        return Ok(Vec::new());
    };
    if multi_entry && let Ok(array) = v8::Local::<v8::Array>::try_from(current) {
        let mut keys = BTreeSet::new();
        for index in 0..array.length() {
            v8::tc_scope!(let scope, scope);
            let Some(value) = array.get_index(scope, index) else {
                if !scope.can_continue() {
                    if scope.has_caught() {
                        scope.rethrow();
                    }
                    return Err(KeyConversionError::Terminated);
                }
                continue;
            };
            let key = parsed_index_key(scope, value, budget, 0)?;
            if scope.is_execution_terminating() || !scope.can_continue() {
                if scope.has_caught() {
                    scope.rethrow();
                }
                return Err(KeyConversionError::Terminated);
            }
            if scope.has_caught() {
                if let Some(key) = key {
                    let bytes = key.heap_bytes();
                    drop(key);
                    budget.refund(bytes as u64);
                }
                continue;
            }
            if let Some(key) = key {
                if keys.contains(&key) {
                    let bytes = key.heap_bytes();
                    drop(key);
                    budget.refund(bytes as u64);
                } else {
                    // Allowance for an underfilled B-tree node and its edges.
                    budget.charge(
                        (12 * (std::mem::size_of::<Key>() + std::mem::size_of::<usize>()) + 64)
                            as u64,
                    )?;
                    keys.insert(key);
                }
            }
        }
        let mut result = Vec::new();
        reserve_vec(budget, &mut result, keys.len())?;
        result.extend(keys);
        return Ok(result);
    }
    let mut result = Vec::new();
    if let Some(key) = parsed_index_key(scope, current, budget, 0)? {
        reserve_vec(budget, &mut result, 1)?;
        result.push(key);
    }
    Ok(result)
}

fn value_at_string_key_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, IndexedDbError> {
    let mut current = value;
    for segment in key_path.split('.').filter(|_| !key_path.is_empty()) {
        if current.is_string() && segment == "length" {
            let text = v8::Local::<v8::String>::try_from(current).unwrap();
            current = v8::Number::new(scope, text.length() as f64).into();
            continue;
        }
        let Ok(object) = v8::Local::<v8::Object>::try_from(current) else {
            return Ok(None);
        };
        if let Some(value) = host_attribute(scope, object, segment) {
            current = value?;
            continue;
        }
        let property = v8_string(scope, segment).ok_or_else(|| {
            IndexedDbError::Resource("Unable to allocate an IndexedDB key path".into())
        })?;
        match object.has_own_property(scope, property.into()) {
            Some(true) => {}
            Some(false) => return Ok(None),
            None => {
                return Err(IndexedDbError::Serialization(
                    "Unable to inspect an IndexedDB key path".into(),
                ));
            }
        }
        let Some(next) = object.get(scope, property.into()) else {
            return Err(IndexedDbError::Serialization(
                "Unable to read an IndexedDB key path".into(),
            ));
        };
        if next.is_undefined() {
            return Ok(None);
        }
        current = next;
    }
    Ok(Some(current))
}

fn host_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<Result<v8::Local<'s, v8::Value>, IndexedDbError>> {
    let kind = moli_webapi_declare::web_api_object_type(scope, object)?.name();
    let value = match (kind, name) {
        ("Blob" | "File", "size") => crate::blob::blob_size_from_object(scope, object)
            .map(|size| v8::Number::new(scope, size as f64).into()),
        ("Blob" | "File", "type") => crate::blob::blob_mime_type_from_object(scope, object)
            .and_then(|value| v8_string(scope, &value).map(Into::into)),
        ("File", "name") => {
            crate::context_bootstrap::file_api::file_name_from_object(scope, object)
                .and_then(|value| v8_string(scope, &value).map(Into::into))
        }
        ("File", "lastModified") => {
            crate::context_bootstrap::file_api::file_last_modified_from_object(scope, object)
                .map(|value| v8::Number::new(scope, value).into())
        }
        _ => return None,
    };
    Some(value.ok_or_else(|| {
        IndexedDbError::Serialization("Unable to read IndexedDB Blob/File metadata".into())
    }))
}

pub(in crate::context_bootstrap::indexed_db) fn extract_object_store_key_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &KeyPath,
) -> ObjectStoreKey {
    v8::tc_scope!(let scope, scope);
    let result = object_store_key(scope, value, key_path);
    if scope.is_execution_terminating() || scope.has_caught() {
        if scope.has_caught() {
            scope.rethrow();
        }
        return ObjectStoreKey::Failed;
    }
    match result {
        Ok(key) => key,
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
            scope.rethrow();
            ObjectStoreKey::Failed
        }
    }
}

fn object_store_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &KeyPath,
) -> Result<ObjectStoreKey, IndexedDbError> {
    if let KeyPath::Sequence(paths) = key_path {
        let mut keys = Vec::with_capacity(paths.len());
        for path in paths {
            match object_store_key(scope, value, &KeyPath::String(path.clone()))? {
                ObjectStoreKey::Key(key) => keys.push(key),
                other => return Ok(other),
            }
        }
        let key = Key::Array(keys);
        return Ok(if key.validate_depth().is_ok() {
            ObjectStoreKey::Key(key)
        } else {
            ObjectStoreKey::Invalid
        });
    }
    let KeyPath::String(path) = key_path else {
        return Ok(ObjectStoreKey::Missing);
    };
    let Some(value) = value_at_string_key_path(scope, value, path)? else {
        return Ok(ObjectStoreKey::Missing);
    };
    Ok(match parse_idb_key_value(scope, value) {
        Ok(Some(key)) => ObjectStoreKey::Key(key),
        Ok(None) => ObjectStoreKey::Missing,
        Err(KeyConversionError::Invalid(_)) => ObjectStoreKey::Invalid,
        Err(KeyConversionError::Backend(error)) => return Err(error),
        Err(_) => ObjectStoreKey::Failed,
    })
}

#[cfg(test)]
mod tests;
