use super::*;

pub(in crate::context_bootstrap::indexed_db) enum ExtractedKey {
    Key(Key),
    Missing,
    Invalid,
}

pub(in crate::context_bootstrap::indexed_db) fn extract_key_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &KeyPath,
) -> ExtractedKey {
    match key_path {
        KeyPath::Sequence(paths) => {
            let mut keys = Vec::with_capacity(paths.len());
            for path in paths {
                match extract_string_key(scope, value, path) {
                    ExtractedKey::Key(key) => keys.push(key),
                    result => return result,
                }
            }
            ExtractedKey::Key(Key::Array(keys))
        }
        KeyPath::String(path) => extract_string_key(scope, value, path),
    }
}

fn extract_string_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    path: &str,
) -> ExtractedKey {
    let Some(value) = value_at_string_key_path(scope, value, path) else {
        return ExtractedKey::Missing;
    };
    match parse_idb_key(scope, value) {
        Ok(Some(key)) => ExtractedKey::Key(key),
        _ => ExtractedKey::Invalid,
    }
}

pub(in crate::context_bootstrap::indexed_db) fn extract_index_keys_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &KeyPath,
    multi_entry: bool,
) -> Vec<Key> {
    if multi_entry
        && let KeyPath::String(path) = key_path
        && let Some(value) = value_at_string_key_path(scope, value, path)
        && let Ok(array) = v8::Local::<v8::Array>::try_from(value)
    {
        let mut keys = Vec::new();
        let mut seen = BTreeSet::new();
        for index in 0..array.length() {
            let property =
                v8::String::new(scope, &index.to_string()).expect("array index allocation");
            if array.has_own_property(scope, property.into()) != Some(true) {
                continue;
            }
            let Some(entry) = array.get_index(scope, index) else {
                continue;
            };
            if let Ok(Some(key)) = parse_idb_key(scope, entry)
                && seen.insert(key.clone())
            {
                keys.push(key);
            }
        }
        return keys;
    }
    match extract_key_from_value(scope, value, key_path) {
        ExtractedKey::Key(key) => vec![key],
        ExtractedKey::Missing | ExtractedKey::Invalid => Vec::new(),
    }
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
        if segment == "length"
            && let Ok(string) = v8::Local::<v8::String>::try_from(current)
        {
            current = v8::Number::new(scope, string.length() as f64).into();
            continue;
        }
        let object = v8::Local::<v8::Object>::try_from(current).ok()?;
        if let Some(special) = special_key_path_property(scope, object, segment) {
            current = special;
            continue;
        }
        let property = v8_string(scope, segment)?;
        if object.has_own_property(scope, property.into()) != Some(true) {
            return None;
        }
        current = object.get(scope, property.into())?;
        if current.is_undefined() {
            return None;
        }
    }
    Some(current)
}

fn special_key_path_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    segment: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    // Read the platform data, not mutable Blob/File prototype accessors.
    if matches!(segment, "size" | "type" | "name" | "lastModified")
        && let Some(file) = crate::context_bootstrap::selected_file_from_object(scope, object)
    {
        return Some(match segment {
            "size" => v8::Number::new(scope, file.bytes.len() as f64).into(),
            "type" => v8_string(scope, &file.mime_type)?.into(),
            "name" => v8_string(scope, &file.name)?.into(),
            "lastModified" => v8::Number::new(scope, file.last_modified).into(),
            _ => unreachable!(),
        });
    }
    match segment {
        "size" => crate::blob::blob_bytes_from_object(scope, object)
            .map(|bytes| v8::Number::new(scope, bytes.len() as f64).into()),
        "type" => crate::blob::blob_mime_type_from_object(scope, object)
            .and_then(|mime| v8_string(scope, &mime))
            .map(Into::into),
        _ => None,
    }
}
