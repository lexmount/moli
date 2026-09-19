use super::*;

pub(in crate::context_bootstrap::indexed_db) fn prepare_object_store_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    key_value: v8::Local<'s, v8::Value>,
) -> Option<PreparedObjectStoreWrite> {
    if indexed_db_transaction_mode(scope, transaction) == Some(TransactionMode::ReadOnly) {
        let error = dom_exception_value(scope, "The transaction is readonly.", "ReadOnlyError");
        scope.throw_exception(error);
        return None;
    }
    let metadata = indexed_db_object_store_metadata(scope, store)?;
    let info = metadata.info();
    let has_key = !key_value.is_undefined();
    if (info.key_path.is_some() && has_key)
        || (info.key_path.is_none() && !info.auto_increment && !has_key)
    {
        return invalid_write_key(scope);
    }
    let mut key = if has_key {
        Some(require_idb_key(scope, key_value)?)
    } else {
        None
    };
    let (clone, bytes) = clone_value_for_transaction(scope, transaction, value)?;
    let mut injection_path = None;
    if let Some(key_path) = &info.key_path {
        match extract_key_from_value(scope, clone, key_path) {
            ExtractedKey::Key(extracted) => key = Some(extracted),
            ExtractedKey::Invalid => return invalid_write_key(scope),
            ExtractedKey::Error(error) => {
                error.throw(scope);
                return None;
            }
            ExtractedKey::Missing => {
                let KeyPath::String(path) = key_path else {
                    return invalid_write_key(scope);
                };
                if !info.auto_increment || !can_inject_key(scope, clone, path) {
                    return invalid_write_key(scope);
                }
                injection_path = Some(path.clone());
            }
        }
    }
    Some(PreparedObjectStoreWrite {
        key,
        value: bytes,
        injection_path,
    })
}

fn invalid_write_key<T>(scope: &mut v8::PinScope<'_, '_>) -> Option<T> {
    let error = dom_exception_value(
        scope,
        "The value or key does not satisfy the object store's key requirements.",
        "DataError",
    );
    scope.throw_exception(error);
    None
}
