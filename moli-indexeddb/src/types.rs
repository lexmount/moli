use serde::{Deserialize, Serialize};

use crate::{IndexedDbName, Key, KeyPath};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ObjectStoreOptions {
    pub key_path: Option<KeyPath>,
    pub auto_increment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexOptions {
    pub key_path: KeyPath,
    pub unique: bool,
    pub multi_entry: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexInfo {
    pub name: IndexedDbName,
    pub key_path: KeyPath,
    pub unique: bool,
    pub multi_entry: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub enum TransactionMode {
    ReadOnly,
    ReadWrite,
    VersionChange,
}

/// Effective storage policy after the renderer resolves the bucket default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransactionDurability {
    #[default]
    Relaxed,
    Strict,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TransactionCommitOptions {
    pub durability: TransactionDurability,
    pub quota: Option<IndexedDbQuotaCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenDisposition {
    Existing,
    UpgradeNeeded { old_version: u64, new_version: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenOptions {
    pub origin: String,
    pub name: String,
    pub version: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenResult {
    pub database: DatabaseHandle,
    pub disposition: OpenDisposition,
    pub upgrade_transaction: Option<TransactionHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseInfo {
    pub name: String,
    pub version: u64,
    pub object_store_names: Vec<IndexedDbName>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseNameAndVersion {
    pub name: String,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectStoreInfo {
    pub name: IndexedDbName,
    pub key_path: Option<KeyPath>,
    pub auto_increment: bool,
    pub index_names: Vec<IndexedDbName>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedDbQuotaCheck {
    pub quota: u64,
    pub non_indexed_db_usage: u64,
}

/// A renderer-neutral structured-clone value stored in an IndexedDB record.
///
/// Like Chromium's `IDBValue`, the V8 wire bytes and external Blob/File
/// objects are kept as separate typed payloads. The storage backend owns both
/// parts so record replacement, deletion, quota accounting, and persistence
/// have one lifecycle boundary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexedDbValue {
    pub(crate) wire_bytes: Vec<u8>,
    pub(crate) external_objects: Vec<IndexedDbExternalObject>,
}

impl IndexedDbValue {
    pub fn new(wire_bytes: Vec<u8>, external_objects: Vec<IndexedDbExternalObject>) -> Self {
        Self {
            wire_bytes,
            external_objects,
        }
    }

    pub fn wire_bytes(&self) -> &[u8] {
        &self.wire_bytes
    }

    pub fn external_objects(&self) -> &[IndexedDbExternalObject] {
        &self.external_objects
    }

    pub fn into_parts(self) -> (Vec<u8>, Vec<IndexedDbExternalObject>) {
        (self.wire_bytes, self.external_objects)
    }
}

impl From<Vec<u8>> for IndexedDbValue {
    fn from(wire_bytes: Vec<u8>) -> Self {
        Self::new(wire_bytes, Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IndexedDbExternalObject {
    Blob {
        bytes: Vec<u8>,
        mime_type: String,
    },
    File {
        bytes: Vec<u8>,
        mime_type: String,
        name: String,
        last_modified: f64,
    },
    /// Durable sandboxed File System Access handle metadata.
    ///
    /// The owning storage key is deliberately omitted. Deserialization must
    /// bind this relative locator to the IndexedDB reader's storage scope and
    /// re-authorize the persistent bucket ID there.
    FileSystemHandle {
        kind: IndexedDbFileSystemHandleKind,
        bucket: IndexedDbFileSystemHandleBucket,
        path: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexedDbFileSystemHandleKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum IndexedDbFileSystemHandleBucket {
    Default,
    Named { bucket_id: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestOutcome {
    Value(Option<IndexedDbValue>),
    Values(Vec<IndexedDbValue>),
    Key(Option<Key>),
    Keys(Vec<Key>),
    Count(u64),
    Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DatabaseHandle(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionHandle(u64);

impl DatabaseHandle {
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn into_raw(self) -> u64 {
        self.0
    }
}

impl TransactionHandle {
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn into_raw(self) -> u64 {
        self.0
    }
}
