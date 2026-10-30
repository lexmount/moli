use super::{IndexedDbValue, Key};

pub(super) use moli_indexeddb::{
    KeyRange as IdbKeyRangeQuery, RecordSnapshot as CursorSnapshotEntry,
};

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
    // An exception or termination has already been propagated to V8.
    Failed,
}
