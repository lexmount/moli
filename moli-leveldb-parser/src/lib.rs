//! Read the current contents of a stable LevelDB snapshot without opening
//! a database engine. Only CURRENT, its MANIFEST, live tables and recoverable WAL
//! files are read. Recovery happens in memory; no locks, logs, tables, manifests
//! or compactions are created, including when the reader is dropped.
//!
//! Formats: https://github.com/google/leveldb/tree/main/doc
//! Manifest tags: https://github.com/google/leveldb/blob/main/db/version_edit.cc

mod coding;
mod layout;
mod log;
mod manifest;
mod table;

use std::{
    collections::{BTreeMap, btree_map::Entry},
    fs,
    path::Path,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use zerocopy::{FromBytes, byteorder::little_endian::U64};

use coding::Decoder;
use layout::WriteBatchHeader;
use manifest::Manifest;

const MAX_SEQUENCE: u64 = (1 << 56) - 1;
type Versions = BTreeMap<Vec<u8>, (u64, Option<Vec<u8>>)>;

/// Read the latest key/value pairs from an existing, stable LevelDB directory.
///
/// The caller must keep the directory unchanged for the duration of this call
/// (for example, by copying a closed database). This function acquires no locks
/// and never modifies the directory. It supports the bytewise comparator and
/// uncompressed or Snappy-compressed tables, including legacy `.sst` filenames.
/// Deleted keys and obsolete files are excluded; complete WAL records are
/// replayed in memory. An unfinished append at EOF is ignored.
///
/// # Errors
///
/// Returns an error for I/O failures, missing live tables, corrupt complete
/// records, or unsupported formats. No partial result is returned on failure.
pub fn read_snapshot(directory: &Path) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
    let manifest = Manifest::read(directory)?;
    let mut versions = Versions::new();
    let mut logs = BTreeMap::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str().and_then(|name| name.strip_suffix(".log")) else {
            continue;
        };
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let number = name.parse::<u64>().context("invalid LevelDB log number")?;
        if number >= manifest.log_number || number == manifest.previous_log_number {
            ensure!(
                logs.insert(number, entry.path()).is_none(),
                "duplicate LevelDB log number"
            );
        }
    }
    let mut last_sequence = manifest.last_sequence;
    for path in logs.values() {
        log::read_records(path, |record| {
            if let Some(sequence) = read_batch(record, &mut versions)? {
                last_sequence = last_sequence.max(sequence);
            }
            Ok(())
        })
        .with_context(|| format!("failed to read LevelDB WAL `{}`", path.display()))?;
    }
    // WAL recovery establishes the latest visible sequence, just as a normal
    // database open does. SST versions newer than that snapshot are invisible.
    for ((_, number), size) in manifest.tables {
        let mut path = directory.join(format!("{number:06}.ldb"));
        if !path.try_exists()? {
            path = directory.join(format!("{number:06}.sst"));
        }
        table::read_table(&path, size, |key, value| {
            let (key, tag) =
                U64::ref_from_suffix(key).map_err(|_| anyhow!("truncated LevelDB internal key"))?;
            let tag = tag.get();
            let value = match tag & 0xff {
                0 => None,
                1 => Some(value),
                other => bail!("unsupported LevelDB value type {other}"),
            };
            if tag >> 8 <= last_sequence {
                apply(&mut versions, key, tag >> 8, value)?;
            }
            Ok(())
        })
        .with_context(|| format!("failed to read LevelDB table `{}`", path.display()))?;
    }
    Ok(versions
        .into_iter()
        .filter_map(|(key, (_, value))| value.map(|value| (key, value)))
        .collect())
}

fn read_batch(record: &[u8], versions: &mut Versions) -> Result<Option<u64>> {
    let mut input = Decoder::new(record);
    let header = input.fixed::<WriteBatchHeader>()?;
    let sequence = header.sequence.get();
    let count = header.count.get();
    ensure!(
        sequence <= MAX_SEQUENCE && u64::from(count.saturating_sub(1)) <= MAX_SEQUENCE - sequence,
        "LevelDB batch sequence overflow"
    );
    for index in 0..count {
        let kind = input.take(1)?[0];
        let key = input.slice()?;
        let value = match kind {
            0 => None,
            1 => Some(input.slice()?),
            other => bail!("unsupported LevelDB batch value type {other}"),
        };
        apply(versions, key, sequence + u64::from(index), value)?;
    }
    ensure!(
        input.bytes.is_empty(),
        "LevelDB batch record count mismatch"
    );
    Ok((count > 0).then_some(sequence + u64::from(count.saturating_sub(1))))
}

fn apply(versions: &mut Versions, key: &[u8], sequence: u64, value: Option<&[u8]>) -> Result<()> {
    match versions.entry(key.to_vec()) {
        Entry::Vacant(entry) => {
            entry.insert((sequence, value.map(<[u8]>::to_vec)));
        }
        Entry::Occupied(mut entry) => {
            let (previous_sequence, previous_value) = entry.get();
            if *previous_sequence == sequence {
                ensure!(
                    previous_value.as_deref() == value,
                    "conflicting LevelDB values at the same sequence"
                );
            } else if *previous_sequence < sequence {
                entry.insert((sequence, value.map(<[u8]>::to_vec)));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
