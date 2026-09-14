use std::{collections::BTreeMap, fs, io::ErrorKind, path::Path};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

// Keys use UTF-16 units so JSON text and explicit UTF-16 keys merge identically.
// Values keep owned UTF-8 strings when possible, including Firefox's SQL values.
pub(crate) type LocalStorage = BTreeMap<String, BTreeMap<Vec<u16>, DomString>>;

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum DomString {
    Text(String),
    Utf16 { utf16: Vec<u16> },
}

impl From<String> for DomString {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<Vec<u16>> for DomString {
    fn from(units: Vec<u16>) -> Self {
        match String::from_utf16(&units) {
            Ok(text) => Self::Text(text),
            Err(_) => Self::Utf16 { utf16: units },
        }
    }
}

impl DomString {
    fn into_units(self) -> Vec<u16> {
        match self {
            Self::Text(text) => text.encode_utf16().collect(),
            Self::Utf16 { utf16 } => utf16,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StorageFile {
    version: u8,
    origins: BTreeMap<String, StorageArea>,
}

#[derive(Serialize, Deserialize)]
struct StorageArea {
    entries: Vec<StorageEntry>,
}

#[derive(Serialize, Deserialize)]
struct StorageEntry {
    key: DomString,
    value: DomString,
}

pub(crate) fn merge(target: &mut LocalStorage, source: LocalStorage) {
    for (origin, entries) in source {
        if !entries.is_empty() {
            target.entry(origin).or_default().extend(entries);
        }
    }
}

/// Prepare the merged file without writing it, so other inputs can be checked
/// before any destination data is replaced. Later sources win on matching keys.
pub(crate) fn prepare(destination: &Path, imported: LocalStorage) -> Result<Option<Vec<u8>>> {
    if imported.values().all(BTreeMap::is_empty) {
        return Ok(None);
    }
    let mut origins = load(destination)?;
    merge(&mut origins, imported);
    let file = StorageFile {
        version: 1,
        origins: origins
            .into_iter()
            .map(|(origin, entries)| {
                let entries = entries
                    .into_iter()
                    .map(|(key, value)| StorageEntry {
                        key: key.into(),
                        value,
                    })
                    .collect();
                (origin, StorageArea { entries })
            })
            .collect(),
    };
    serde_json::to_vec_pretty(&file)
        .context("failed to serialize imported localStorage")
        .map(Some)
}

fn load(path: &Path) -> Result<LocalStorage> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(LocalStorage::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read Moli localStorage `{}`", path.display()));
        }
    };
    if bytes.is_empty() {
        return Ok(LocalStorage::new());
    }
    let file: StorageFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse Moli localStorage `{}`", path.display()))?;
    ensure!(
        file.version == 1,
        "unsupported Moli localStorage version {}",
        file.version
    );
    Ok(file
        .origins
        .into_iter()
        .map(|(origin, area)| {
            let entries = area
                .entries
                .into_iter()
                .map(|entry| (entry.key.into_units(), entry.value))
                .collect();
            (origin, entries)
        })
        .collect())
}
