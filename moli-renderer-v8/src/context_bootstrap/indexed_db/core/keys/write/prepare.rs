use super::*;

pub(in crate::context_bootstrap::indexed_db) fn prepare_object_store_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    explicit_key: Option<Key>,
) -> Option<PreparedObjectStoreWrite> {
    let metadata = indexed_db_object_store_metadata(scope, store)?;
    let info = metadata.info();
    let serialized = serialize_js_value_for_transaction(scope, transaction, value)?;
    let key = match &info.key_path {
        Some(key_path) => {
            if explicit_key.is_some() {
                return invalid_write(
                    scope,
                    "The store uses in-line keys and does not accept a key argument.",
                );
            }
            let clone = deserialize_js_value(scope, &serialized)?;
            match extract_object_store_key_from_value(scope, clone, key_path) {
                ExtractedObjectStoreKey::Key(key) => PreparedObjectStoreKey::Explicit(key),
                ExtractedObjectStoreKey::Invalid => {
                    return invalid_write(
                        scope,
                        "The value at the object store keyPath is not a valid key.",
                    );
                }
                ExtractedObjectStoreKey::Missing => {
                    if !info.auto_increment {
                        return invalid_write(
                            scope,
                            "The value does not contain the object store keyPath.",
                        );
                    }
                    let KeyPath::String(key_path) = key_path else {
                        return invalid_write(
                            scope,
                            "A compound keyPath cannot accept generated keys.",
                        );
                    };
                    if !can_inject_key_path_into_value(scope, clone, key_path) {
                        return invalid_write(
                            scope,
                            "The value cannot accept a generated inline key.",
                        );
                    }
                    PreparedObjectStoreKey::GenerateInline {
                        key_path: key_path.clone(),
                    }
                }
            }
        }
        None => match explicit_key {
            Some(key) => PreparedObjectStoreKey::Explicit(key),
            None if info.auto_increment => PreparedObjectStoreKey::Generate,
            None => {
                return invalid_write(scope, "A key is required for stores without autoIncrement.");
            }
        },
    };
    Some(PreparedObjectStoreWrite {
        key,
        value: serialized,
    })
}

fn invalid_write(
    scope: &mut v8::PinScope<'_, '_>,
    message: &str,
) -> Option<PreparedObjectStoreWrite> {
    let error = dom_exception_value(scope, message, "DataError");
    scope.throw_exception(error);
    None
}
