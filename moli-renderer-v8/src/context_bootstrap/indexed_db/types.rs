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
    pub(super) key: Option<Key>,
    pub(super) value: IndexedDbValue,
    // Present only when an auto-generated key must be injected at execution.
    pub(super) injection_path: Option<String>,
}
