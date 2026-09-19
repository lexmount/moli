use super::*;

pub(in crate::context_bootstrap::indexed_db) fn validate_existing_index_entries(
    scope: &mut v8::PinScope<'_, '_>,
    handle: TransactionHandle,
    store_name: &IndexedDbName,
    index: &IndexInfo,
) -> Result<(), IndexedDbError> {
    if !index.unique {
        return Ok(());
    }
    let entries = scan_index_entries(scope, handle, store_name, index, None)?;
    // The scan sorts by index key and deduplicates each record's multiEntry
    // keys. Equal keys in adjacent records therefore violate uniqueness.
    if entries
        .windows(2)
        .any(|pair| pair[0].index_key == pair[1].index_key)
    {
        return Err(IndexedDbError::Constraint(format!(
            "unique index `{}` already contains key",
            index.name
        )));
    }
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn enforce_object_store_unique_constraints<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &IndexedDbName,
    primary_key: &Key,
    value: v8::Local<'s, v8::Value>,
) -> std::result::Result<(), IndexedDbError> {
    let indexes = indexed_db_object_store_metadata(scope, store)
        .map(|metadata| metadata.indexes_in_name_order())
        .unwrap_or_default();
    for index in indexes {
        if !index.unique {
            continue;
        }

        let candidate_keys =
            extract_index_keys_from_value(scope, value, &index.key_path, index.multi_entry);
        if candidate_keys.is_empty() {
            continue;
        }

        let seen: BTreeSet<_> = candidate_keys.into_iter().collect();

        let existing = scan_index_entries(scope, handle, store_name, &index, None)?;
        if existing
            .into_iter()
            .any(|entry| entry.primary_key != *primary_key && seen.contains(&entry.index_key))
        {
            return Err(IndexedDbError::Constraint(format!(
                "unique index `{}` already contains key",
                index.name
            )));
        }
    }
    Ok(())
}
