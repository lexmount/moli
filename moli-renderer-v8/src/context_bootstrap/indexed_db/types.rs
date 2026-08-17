use super::{IndexedDbError, IndexedDbValue, Key};

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

pub(super) struct PreparedObjectStoreWrite<'s> {
    pub(super) key: Option<Key>,
    pub(super) value: v8::Local<'s, v8::Value>,
}

pub(super) enum PreparedObjectStoreWriteError {
    Backend(IndexedDbError),
    DomException {
        message: &'static str,
        name: &'static str,
    },
}
