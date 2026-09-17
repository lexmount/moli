use crate::{
    IndexedDbExternalObject, IndexedDbValue, Key, KeyPath,
    state::{DatabaseData, IndexData, ObjectStoreData, OriginState},
};

const U64_STORAGE_BYTES: u64 = std::mem::size_of::<u64>() as u64;
const I64_STORAGE_BYTES: u64 = std::mem::size_of::<i64>() as u64;
const BOOL_STORAGE_BYTES: u64 = std::mem::size_of::<bool>() as u64;

pub(crate) fn origin_usage_bytes(state: &OriginState) -> u64 {
    sum_usage(
        state
            .databases
            .iter()
            .map(|(name, database)| database_usage_bytes(name, database)),
    )
}

pub(crate) fn database_usage_bytes(name: &str, database: &DatabaseData) -> u64 {
    string_usage_bytes(name)
        .saturating_add(U64_STORAGE_BYTES)
        .saturating_add(sum_usage(
            database
                .stores
                .iter()
                .map(|(name, store)| object_store_usage_bytes(name, store)),
        ))
}

fn object_store_usage_bytes(name: &str, store: &ObjectStoreData) -> u64 {
    string_usage_bytes(name)
        .saturating_add(optional_key_path_usage_bytes(&store.key_path))
        .saturating_add(BOOL_STORAGE_BYTES)
        .saturating_add(U64_STORAGE_BYTES)
        .saturating_add(sum_usage(
            store
                .indexes
                .iter()
                .map(|(name, index)| index_usage_bytes(name, index)),
        ))
        .saturating_add(sum_usage(store.records.iter().map(|(key, value)| {
            key_usage_bytes(key).saturating_add(indexed_db_value_usage_bytes(value))
        })))
}

fn indexed_db_value_usage_bytes(value: &IndexedDbValue) -> u64 {
    value
        .wire_bytes
        .len()
        .try_into()
        .unwrap_or(u64::MAX)
        .saturating_add(sum_usage(value.external_objects.iter().map(|object| {
            match object {
                IndexedDbExternalObject::Blob { bytes, mime_type } => bytes
                    .len()
                    .try_into()
                    .unwrap_or(u64::MAX)
                    .saturating_add(string_usage_bytes(mime_type)),
                IndexedDbExternalObject::File {
                    bytes,
                    mime_type,
                    name,
                    ..
                } => bytes
                    .len()
                    .try_into()
                    .unwrap_or(u64::MAX)
                    .saturating_add(string_usage_bytes(mime_type))
                    .saturating_add(string_usage_bytes(name))
                    .saturating_add(std::mem::size_of::<f64>() as u64),
                IndexedDbExternalObject::FileSystemHandle { bucket, path, .. } => {
                    let bucket_bytes = match bucket {
                        crate::IndexedDbFileSystemHandleBucket::Default => 0,
                        crate::IndexedDbFileSystemHandleBucket::Named { .. } => U64_STORAGE_BYTES,
                    };
                    bucket_bytes.saturating_add(sum_usage(
                        path.iter().map(|component| string_usage_bytes(component)),
                    ))
                }
            }
        })))
}

fn index_usage_bytes(name: &str, index: &IndexData) -> u64 {
    string_usage_bytes(name)
        .saturating_add(key_path_usage_bytes(&index.key_path))
        .saturating_add(BOOL_STORAGE_BYTES)
        .saturating_add(BOOL_STORAGE_BYTES)
}

fn optional_key_path_usage_bytes(key_path: &Option<KeyPath>) -> u64 {
    key_path.as_ref().map(key_path_usage_bytes).unwrap_or(0)
}

fn key_path_usage_bytes(key_path: &KeyPath) -> u64 {
    match key_path {
        KeyPath::String(value) => string_usage_bytes(value),
        KeyPath::Sequence(values) => {
            sum_usage(values.iter().map(|value| string_usage_bytes(value)))
        }
    }
}

fn key_usage_bytes(key: &Key) -> u64 {
    match key {
        Key::String(value) => (value.len() as u64).saturating_mul(2),
        Key::Binary(value) => value.len() as u64,
        Key::Number(_) | Key::Date(_) => I64_STORAGE_BYTES,
        Key::Array(values) => sum_usage(values.iter().map(key_usage_bytes)),
    }
}

fn string_usage_bytes(value: &str) -> u64 {
    value.len().try_into().unwrap_or(u64::MAX)
}

pub(crate) fn sum_usage(values: impl Iterator<Item = u64>) -> u64 {
    values.fold(0, u64::saturating_add)
}
