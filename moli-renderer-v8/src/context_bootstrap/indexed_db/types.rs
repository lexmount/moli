use super::{IndexedDbValue, Key};

#[derive(Debug, Clone)]
pub(super) struct IdbKeyRangeQuery {
    pub(super) lower: Option<Key>,
    pub(super) upper: Option<Key>,
    pub(super) lower_open: bool,
    pub(super) upper_open: bool,
}

#[derive(Debug, Clone)]
pub(super) struct IndexEntry {
    pub(super) index_key: Key,
    pub(super) primary_key: Key,
    pub(super) value: IndexedDbValue,
}

#[derive(Debug, Clone)]
pub(super) struct CursorSnapshotEntry {
    pub(super) key: Key,
    pub(super) primary_key: Key,
    pub(super) value: Option<IndexedDbValue>,
}

pub(super) struct PreparedObjectStoreWrite {
    pub(super) key: PreparedObjectStoreKey,
    pub(super) value: IndexedDbValue,
}

pub(super) enum PreparedObjectStoreKey {
    Explicit(Key),
    Generate,
    GenerateInline { key_path: String },
}

/// Key conversion and key-path extraction distinguish invalid data from a
/// JavaScript exception. Invalid data is checked after value serialization.
pub(super) enum ObjectStoreKey {
    Key(Key),
    Missing,
    Invalid,
}
