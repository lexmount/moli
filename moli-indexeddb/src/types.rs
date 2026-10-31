use serde::{Deserialize, Serialize};

use crate::{Key, KeyPath};

/// Renderer-neutral bounds for storage and index queries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyRange {
    pub lower: Option<Key>,
    pub upper: Option<Key>,
    pub lower_open: bool,
    pub upper_open: bool,
}

impl KeyRange {
    pub fn validate(&self) -> Result<(), crate::IndexedDbError> {
        for key in self.lower.iter().chain(self.upper.iter()) {
            key.validate_depth()
                .map_err(|message| crate::IndexedDbError::Data(message.to_owned()))?;
        }
        if let (Some(lower), Some(upper)) = (&self.lower, &self.upper)
            && (lower > upper || (lower == upper && (self.lower_open || self.upper_open)))
        {
            return Err(crate::IndexedDbError::Data(
                "invalid IndexedDB key range".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn contains(&self, key: &Key) -> bool {
        let above_lower = self
            .lower
            .as_ref()
            .is_none_or(|lower| key > lower || (key == lower && !self.lower_open));
        let below_upper = self
            .upper
            .as_ref()
            .is_none_or(|upper| key < upper || (key == upper && !self.upper_open));
        above_lower && below_upper
    }
}

/// One materialized result, optionally projected to keys only.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordSnapshot {
    pub key: Key,
    pub primary_key: Key,
    pub value: Option<IndexedDbValue>,
}

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
    pub name: crate::IndexedDbName,
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

/// Resolved durability policy for a native transaction. The frontend retains
/// the Web API hint and resolves its bucket default before admitting the work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransactionDurability {
    #[default]
    Relaxed,
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectStoreInfo {
    pub name: crate::IndexedDbName,
    pub key_path: Option<KeyPath>,
    pub auto_increment: bool,
    pub index_names: Vec<crate::IndexedDbName>,
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
#[derive(Debug, Clone, Default, PartialEq)]
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

    pub fn has_deferred_blob_bytes(&self) -> bool {
        self.external_objects.iter().any(|object| match object {
            IndexedDbExternalObject::Blob { bytes, .. }
            | IndexedDbExternalObject::File { bytes, .. } => bytes.as_bytes().is_none(),
            _ => false,
        })
    }

    /// Prepare immutable Blob attachments before enqueueing a SQL write. This
    /// may read another database, so it must not run inside a SQLite job.
    pub fn prepare_blob_bytes(
        mut self,
        complete: impl FnOnce(Self, Result<(), crate::IndexedDbError>) + Send + 'static,
    ) {
        let mut parts = Vec::new();
        let mut selected = Vec::new();
        for (index, object) in self.external_objects.iter().enumerate() {
            if let IndexedDbExternalObject::Blob { bytes, .. }
            | IndexedDbExternalObject::File { bytes, .. } = object
                && bytes.as_bytes().is_none()
            {
                selected.push((index, bytes.len()));
                parts.push(bytes.clone());
            }
        }
        if parts.is_empty() {
            complete(self, Ok(()));
            return;
        }
        let data = match moli_file_api::BlobData::concat(parts) {
            Ok(data) => data,
            Err(error) => {
                complete(self, Err(crate::IndexedDbError::Io(error.to_string())));
                return;
            }
        };
        data.read_all(Box::new(move |result| {
            let bytes: moli_file_api::BlobData = match result {
                Ok(bytes) => bytes.into(),
                Err(error) => {
                    complete(self, Err(crate::IndexedDbError::Io(error.to_string())));
                    return;
                }
            };
            let mut offset = 0;
            for (index, length) in selected {
                let data = bytes.slice(offset..offset + length);
                match &mut self.external_objects[index] {
                    IndexedDbExternalObject::Blob { bytes, .. }
                    | IndexedDbExternalObject::File { bytes, .. } => *bytes = data,
                    _ => unreachable!("selected a Blob attachment"),
                }
                offset += length;
            }
            complete(self, Ok(()));
        }));
    }
}

impl From<Vec<u8>> for IndexedDbValue {
    fn from(wire_bytes: Vec<u8>) -> Self {
        Self::new(wire_bytes, Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndexedDbExternalObject {
    Blob {
        bytes: moli_file_api::BlobData,
        mime_type: String,
    },
    File {
        bytes: moli_file_api::BlobData,
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
