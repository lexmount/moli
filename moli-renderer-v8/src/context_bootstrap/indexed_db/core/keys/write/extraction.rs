use super::*;

pub(in crate::context_bootstrap::indexed_db) fn extract_index_keys_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &KeyPath,
    multi_entry: bool,
) -> Vec<Key> {
    if let KeyPath::Sequence(paths) = key_path {
        let mut keys = Vec::with_capacity(paths.len());
        for path in paths {
            let Some(key) = extract_key_from_string_key_path(scope, value, path) else {
                return Vec::new();
            };
            keys.push(key);
        }
        return vec![Key::Array(keys)];
    }
    let KeyPath::String(key_path) = key_path else {
        return Vec::new();
    };
    extract_keys_from_string_key_path(scope, value, key_path, multi_entry)
}

fn extract_keys_from_string_key_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &str,
    multi_entry: bool,
) -> Vec<Key> {
    let Some(current) = value_at_string_key_path(scope, value, key_path) else {
        return Vec::new();
    };
    if multi_entry && let Ok(array) = v8::Local::<v8::Array>::try_from(current) {
        let mut keys = Vec::new();
        for index in 0..array.length() {
            let Some(entry) = array.get_index(scope, index) else {
                continue;
            };
            if let Ok(Some(key)) = parse_idb_key(scope, entry) {
                keys.push(key);
            }
        }
        return keys;
    }

    parse_idb_key(scope, current)
        .ok()
        .flatten()
        .into_iter()
        .collect()
}

fn extract_key_from_string_key_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &str,
) -> Option<Key> {
    value_at_string_key_path(scope, value, key_path)
        .and_then(|value| parse_idb_key(scope, value).ok().flatten())
}

fn value_at_string_key_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    if key_path.is_empty() {
        return Some(value);
    }
    let mut current = value;
    for segment in key_path.split('.') {
        if current.is_string() && segment == "length" {
            let text = v8::Local::<v8::String>::try_from(current).ok()?;
            current = v8::Number::new(scope, text.length() as f64).into();
            continue;
        }
        let Ok(object) = v8::Local::<v8::Object>::try_from(current) else {
            return None;
        };
        let property = v8_string(scope, segment).unwrap_or_else(|| v8::String::empty(scope));
        // Ordinary key paths only inspect the clone's own data properties.
        // The specification also exposes these intrinsic Blob/File attributes.
        let host_attribute = matches!(
            (
                moli_webapi_declare::web_api_object_type(scope, object).map(|kind| kind.name()),
                segment
            ),
            (Some("Blob" | "File"), "size" | "type") | (Some("File"), "name" | "lastModified")
        );
        if !host_attribute && object.has_own_property(scope, property.into()) != Some(true) {
            return None;
        }
        let next = object.get(scope, property.into())?;
        if next.is_undefined() {
            return None;
        }
        current = next;
    }
    Some(current)
}

pub(in crate::context_bootstrap::indexed_db) fn extract_object_store_key_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &KeyPath,
) -> ObjectStoreKey {
    if let KeyPath::Sequence(paths) = key_path {
        let mut keys = Vec::with_capacity(paths.len());
        for path in paths {
            match extract_object_store_key_from_value(scope, value, &KeyPath::String(path.clone()))
            {
                ObjectStoreKey::Key(key) => keys.push(key),
                other => return other,
            }
        }
        return ObjectStoreKey::Key(Key::Array(keys));
    }
    let KeyPath::String(path) = key_path else {
        return ObjectStoreKey::Missing;
    };
    let Some(value) = value_at_string_key_path(scope, value, path) else {
        return ObjectStoreKey::Missing;
    };
    match parse_idb_key(scope, value) {
        Ok(Some(key)) => ObjectStoreKey::Key(key),
        Ok(None) => ObjectStoreKey::Missing,
        Err(_) => ObjectStoreKey::Invalid,
    }
}
