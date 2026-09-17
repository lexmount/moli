use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
};

use moli_crypto::sha256_hex;
use serde::{Deserialize, Serialize};

use crate::{
    IndexedDbError, IndexedDbExternalObject, IndexedDbName, IndexedDbValue, Key, KeyPath,
    TransactionDurability,
    state::{DatabaseData, IndexData, IndexedDbManager, ObjectStoreData, OriginState},
};

mod atomic;

pub(crate) fn prepare_storage_directory(path: &Path) -> std::io::Result<()> {
    atomic::prepare_directory(path)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentOrigin {
    #[serde(default)]
    origin: Option<String>,
    databases: BTreeMap<String, PersistentDatabase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentDatabase {
    version: u64,
    stores: BTreeMap<String, PersistentObjectStore>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    stores_utf16: Vec<PersistentNamedObjectStore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentNamedObjectStore {
    name: Vec<u16>,
    #[serde(flatten)]
    store: PersistentObjectStore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentObjectStore {
    key_path: Option<KeyPath>,
    auto_increment: bool,
    auto_increment_counter: u64,
    #[serde(default)]
    indexes: BTreeMap<String, PersistentIndex>,
    // JSON strings cannot represent unpaired UTF-16 surrogates. Keep the
    // existing map format for Unicode names and store only other names here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    indexes_utf16: Vec<PersistentNamedIndex>,
    records: Vec<PersistentRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentIndex {
    key_path: KeyPath,
    unique: bool,
    multi_entry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentNamedIndex {
    name: Vec<u16>,
    #[serde(flatten)]
    index: PersistentIndex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentRecord {
    key: Key,
    value: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    external_objects: Vec<IndexedDbExternalObject>,
}

pub(crate) enum IndexedDbPersistenceBackend {
    InMemory,
    JsonFiles { storage_root: PathBuf },
}

impl IndexedDbManager {
    pub(crate) fn ensure_origin_loaded(&mut self, origin: &str) -> Result<(), IndexedDbError> {
        if self.origins.contains_key(origin) {
            return Ok(());
        }
        let state = if let Some(bytes) = self.read_persisted_origin(origin)? {
            let persisted: PersistentOrigin = serde_json::from_slice(&bytes).map_err(|err| {
                IndexedDbError::Corruption(format!("failed to decode origin state: {err}"))
            })?;
            origin_state_from_persistent(persisted)
        } else {
            OriginState {
                databases: BTreeMap::new(),
            }
        };
        self.origins.insert(origin.to_owned(), state);
        Ok(())
    }

    pub(crate) fn persist_origin(&self, origin: &str) -> Result<(), IndexedDbError> {
        self.persist_origin_with_durability(origin, TransactionDurability::Relaxed)
    }

    pub(crate) fn persist_origin_with_durability(
        &self,
        origin: &str,
        durability: TransactionDurability,
    ) -> Result<(), IndexedDbError> {
        if matches!(self.backend, IndexedDbPersistenceBackend::InMemory) {
            return Ok(());
        }
        let Some(state) = self.origins.get(origin) else {
            return Ok(());
        };
        let persisted = persistent_origin_from_state(origin, state);
        let bytes = serde_json::to_vec_pretty(&persisted).map_err(|err| {
            IndexedDbError::Serialization(format!("failed to encode origin state: {err}"))
        })?;
        self.write_persisted_origin(origin, &bytes, durability)
    }

    pub(crate) fn read_persisted_origin(
        &self,
        origin: &str,
    ) -> Result<Option<Vec<u8>>, IndexedDbError> {
        match &self.backend {
            IndexedDbPersistenceBackend::InMemory => Ok(None),
            IndexedDbPersistenceBackend::JsonFiles { storage_root } => {
                let path = origin_path(storage_root, origin);
                if !path.exists() {
                    return Ok(None);
                }
                fs::read(&path).map(Some).map_err(|err| {
                    IndexedDbError::Io(format!(
                        "failed to read origin state `{}`: {err}",
                        path.display()
                    ))
                })
            }
        }
    }

    pub(crate) fn write_persisted_origin(
        &self,
        origin: &str,
        bytes: &[u8],
        durability: TransactionDurability,
    ) -> Result<(), IndexedDbError> {
        match &self.backend {
            IndexedDbPersistenceBackend::InMemory => Ok(()),
            IndexedDbPersistenceBackend::JsonFiles { storage_root } => {
                let path = origin_path(storage_root, origin);
                if let Some(parent) = path.parent() {
                    prepare_storage_directory(parent).map_err(|err| {
                        IndexedDbError::Io(format!(
                            "failed to create parent directory `{}`: {err}",
                            parent.display()
                        ))
                    })?;
                }
                atomic::replace(&path, bytes, durability).map_err(|err| {
                    IndexedDbError::Io(format!(
                        "failed to write origin state `{}`: {err}",
                        path.display()
                    ))
                })
            }
        }
    }

    pub(crate) fn remove_persisted_origin(&self, origin: &str) -> Result<(), IndexedDbError> {
        match &self.backend {
            IndexedDbPersistenceBackend::InMemory => Ok(()),
            IndexedDbPersistenceBackend::JsonFiles { storage_root } => {
                let path = origin_path(storage_root, origin);
                match fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(IndexedDbError::Io(format!(
                        "failed to remove origin state `{}`: {error}",
                        path.display()
                    ))),
                }
            }
        }
    }

    pub(crate) fn persisted_origins_with_prefix(
        &self,
        origin_prefix: &str,
    ) -> Result<Vec<(String, OriginState)>, IndexedDbError> {
        match &self.backend {
            IndexedDbPersistenceBackend::InMemory => Ok(Vec::new()),
            IndexedDbPersistenceBackend::JsonFiles { storage_root } => {
                let entries = fs::read_dir(storage_root).map_err(|err| {
                    IndexedDbError::Io(format!(
                        "failed to read IndexedDB storage root `{}`: {err}",
                        storage_root.display()
                    ))
                })?;
                let mut origins = Vec::new();
                for entry in entries {
                    let entry = entry.map_err(|err| {
                        IndexedDbError::Io(format!(
                            "failed to read IndexedDB storage root entry `{}`: {err}",
                            storage_root.display()
                        ))
                    })?;
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    let Some(sanitized_origin) = file_name.strip_suffix(".json") else {
                        continue;
                    };
                    let bytes = fs::read(&path).map_err(|err| {
                        IndexedDbError::Io(format!(
                            "failed to read origin state `{}`: {err}",
                            path.display()
                        ))
                    })?;
                    let persisted: PersistentOrigin =
                        serde_json::from_slice(&bytes).map_err(|err| {
                            IndexedDbError::Corruption(format!(
                                "failed to decode origin state: {err}"
                            ))
                        })?;
                    let origin = persisted
                        .origin
                        .clone()
                        .or_else(|| unsanitize_origin(sanitized_origin));
                    let Some(origin) = origin else {
                        continue;
                    };
                    if !origin.starts_with(origin_prefix) {
                        continue;
                    }
                    origins.push((origin, origin_state_from_persistent(persisted)));
                }
                Ok(origins)
            }
        }
    }

    pub(crate) fn remove_persisted_origins_with_prefix(
        &self,
        origin_prefix: &str,
    ) -> Result<(), IndexedDbError> {
        match &self.backend {
            IndexedDbPersistenceBackend::InMemory => Ok(()),
            IndexedDbPersistenceBackend::JsonFiles { storage_root } => {
                let sanitized_prefix = sanitize_origin(origin_prefix);
                let entries = fs::read_dir(storage_root).map_err(|err| {
                    IndexedDbError::Io(format!(
                        "failed to read IndexedDB storage root `{}`: {err}",
                        storage_root.display()
                    ))
                })?;
                for entry in entries {
                    let entry = entry.map_err(|err| {
                        IndexedDbError::Io(format!(
                            "failed to read IndexedDB storage root entry `{}`: {err}",
                            storage_root.display()
                        ))
                    })?;
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    let Some(sanitized_origin) = file_name.strip_suffix(".json") else {
                        continue;
                    };
                    let remove = if sanitized_origin.starts_with(&sanitized_prefix) {
                        true
                    } else {
                        let bytes = fs::read(&path).map_err(|err| {
                            IndexedDbError::Io(format!(
                                "failed to read origin state `{}`: {err}",
                                path.display()
                            ))
                        })?;
                        let persisted: PersistentOrigin =
                            serde_json::from_slice(&bytes).map_err(|err| {
                                IndexedDbError::Corruption(format!(
                                    "failed to decode origin state: {err}"
                                ))
                            })?;
                        persisted
                            .origin
                            .as_deref()
                            .is_some_and(|origin| origin.starts_with(origin_prefix))
                    };
                    if !remove {
                        continue;
                    }
                    fs::remove_file(&path).map_err(|error| {
                        IndexedDbError::Io(format!(
                            "failed to remove origin state `{}`: {error}",
                            path.display()
                        ))
                    })?;
                }
                Ok(())
            }
        }
    }
}

pub(crate) fn sanitize_origin(origin: &str) -> String {
    let mut encoded = String::with_capacity(origin.len() * 2);
    for byte in origin.as_bytes() {
        let _ = fmt::Write::write_fmt(&mut encoded, format_args!("{byte:02x}"));
    }
    encoded
}

pub(crate) fn origin_file_stem(origin: &str) -> String {
    let encoded = sanitize_origin(origin);
    if encoded.len() <= 180 {
        return encoded;
    }
    format!("h-{}", sha256_hex(origin.as_bytes()))
}

fn unsanitize_origin(encoded: &str) -> Option<String> {
    if !encoded.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    let mut chars = encoded.as_bytes().chunks_exact(2);
    for pair in &mut chars {
        let pair = std::str::from_utf8(pair).ok()?;
        bytes.push(u8::from_str_radix(pair, 16).ok()?);
    }
    String::from_utf8(bytes).ok()
}

pub(crate) fn origin_path(storage_root: &Path, origin: &str) -> PathBuf {
    storage_root.join(format!("{}.json", origin_file_stem(origin)))
}

fn origin_state_from_persistent(persistent: PersistentOrigin) -> OriginState {
    let mut databases = BTreeMap::new();
    for (name, database) in persistent.databases {
        let mut stores = BTreeMap::new();
        for (store_name, store) in database
            .stores
            .into_iter()
            .map(|(name, store)| (IndexedDbName::from(name), store))
            .chain(
                database
                    .stores_utf16
                    .into_iter()
                    .map(|named| (IndexedDbName::from_utf16(named.name), named.store)),
            )
        {
            let records = store
                .records
                .into_iter()
                .map(|record| {
                    (
                        record.key,
                        IndexedDbValue::new(record.value, record.external_objects),
                    )
                })
                .collect();
            stores.insert(
                store_name,
                ObjectStoreData {
                    key_path: store.key_path,
                    auto_increment: store.auto_increment,
                    auto_increment_counter: store.auto_increment_counter,
                    indexes: store
                        .indexes
                        .into_iter()
                        .map(|(name, index)| (IndexedDbName::from(name), index))
                        .chain(
                            store
                                .indexes_utf16
                                .into_iter()
                                .map(|named| (IndexedDbName::from_utf16(named.name), named.index)),
                        )
                        .map(|(name, index)| {
                            (
                                name,
                                IndexData {
                                    key_path: index.key_path,
                                    unique: index.unique,
                                    multi_entry: index.multi_entry,
                                },
                            )
                        })
                        .collect(),
                    records,
                },
            );
        }
        databases.insert(
            name,
            DatabaseData {
                version: database.version,
                stores,
            },
        );
    }
    OriginState { databases }
}

fn persistent_origin_from_state(origin: &str, state: &OriginState) -> PersistentOrigin {
    let mut databases = BTreeMap::new();
    for (name, database) in &state.databases {
        let mut stores = BTreeMap::new();
        let mut stores_utf16 = Vec::new();
        for (store_name, store) in &database.stores {
            let mut indexes = BTreeMap::new();
            let mut indexes_utf16 = Vec::new();
            for (name, index) in &store.indexes {
                let index = PersistentIndex {
                    key_path: index.key_path.clone(),
                    unique: index.unique,
                    multi_entry: index.multi_entry,
                };
                match String::from_utf16(name.as_utf16()) {
                    Ok(name) => {
                        indexes.insert(name, index);
                    }
                    Err(_) => indexes_utf16.push(PersistentNamedIndex {
                        name: name.as_utf16().to_vec(),
                        index,
                    }),
                }
            }
            let store = PersistentObjectStore {
                key_path: store.key_path.clone(),
                auto_increment: store.auto_increment,
                auto_increment_counter: store.auto_increment_counter,
                indexes,
                indexes_utf16,
                records: store
                    .records
                    .iter()
                    .map(|(key, value)| PersistentRecord {
                        key: key.clone(),
                        value: value.wire_bytes.clone(),
                        external_objects: value.external_objects.clone(),
                    })
                    .collect(),
            };
            match String::from_utf16(store_name.as_utf16()) {
                Ok(name) => {
                    stores.insert(name, store);
                }
                Err(_) => stores_utf16.push(PersistentNamedObjectStore {
                    name: store_name.as_utf16().to_vec(),
                    store,
                }),
            }
        }
        databases.insert(
            name.clone(),
            PersistentDatabase {
                version: database.version,
                stores,
                stores_utf16,
            },
        );
    }
    PersistentOrigin {
        origin: Some(origin.to_owned()),
        databases,
    }
}
