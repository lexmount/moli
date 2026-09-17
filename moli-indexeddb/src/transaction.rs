use crate::{
    IndexedDbError, Key, TransactionMode,
    state::{ObjectStoreData, TransactionState},
};

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
pub(crate) const MAX_AUTO_INCREMENT_KEY: u64 = MAX_SAFE_INTEGER as u64;

pub(crate) fn transaction_store<'a>(
    tx: &'a TransactionState,
    store_name: &str,
) -> Result<&'a ObjectStoreData, IndexedDbError> {
    if !tx.stores.contains(store_name) {
        return Err(IndexedDbError::InvalidState(format!(
            "transaction does not cover object store `{store_name}`"
        )));
    }
    tx.working_copy.stores.get(store_name).ok_or_else(|| {
        IndexedDbError::NotFound(format!("object store `{store_name}` was not found"))
    })
}

pub(crate) fn transaction_store_mut<'a>(
    tx: &'a mut TransactionState,
    store_name: &str,
) -> Result<&'a mut ObjectStoreData, IndexedDbError> {
    if !tx.stores.contains(store_name) {
        return Err(IndexedDbError::InvalidState(format!(
            "transaction does not cover object store `{store_name}`"
        )));
    }
    tx.working_copy.stores.get_mut(store_name).ok_or_else(|| {
        IndexedDbError::NotFound(format!("object store `{store_name}` was not found"))
    })
}

pub(crate) fn ensure_writeable(tx: &TransactionState) -> Result<(), IndexedDbError> {
    match tx.mode {
        TransactionMode::ReadOnly => Err(IndexedDbError::ReadOnly(
            "transaction is readonly".to_owned(),
        )),
        TransactionMode::ReadWrite | TransactionMode::VersionChange => Ok(()),
    }
}

pub(crate) fn resolve_key(
    store: &mut ObjectStoreData,
    key: Option<Key>,
) -> Result<Key, IndexedDbError> {
    if let Some(key) = key {
        if store.auto_increment
            && let Key::Integer(value) = key
            && value > 0
        {
            let value = value as u64;
            if value > MAX_AUTO_INCREMENT_KEY {
                return Err(IndexedDbError::Constraint(
                    "auto_increment key generator exceeded the maximum safe integer value"
                        .to_owned(),
                ));
            }
            store.auto_increment_counter = store.auto_increment_counter.max(value);
        }
        return Ok(key);
    }
    if store.auto_increment {
        let key = next_generated_key(store)?;
        store.auto_increment_counter += 1;
        return Ok(key);
    }
    Err(IndexedDbError::InvalidState(
        "a key is required when auto_increment is disabled".to_owned(),
    ))
}

pub(crate) fn next_generated_key(store: &ObjectStoreData) -> Result<Key, IndexedDbError> {
    if !store.auto_increment {
        return Err(IndexedDbError::InvalidState(
            "a key is required when auto_increment is disabled".to_owned(),
        ));
    }
    if store.auto_increment_counter >= MAX_AUTO_INCREMENT_KEY {
        return Err(IndexedDbError::Constraint(
            "auto_increment key generator exceeded the maximum safe integer value".to_owned(),
        ));
    }
    Ok(Key::Integer((store.auto_increment_counter + 1) as i64))
}
