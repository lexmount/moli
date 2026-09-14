use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::Result;
use rusty_leveldb::{DB, LdbIterator, Options};

use super::support::{batch, log_bytes, metadata, write_manifest};

use crate::{
    Manifest,
    coding::{Decoder, masked_crc},
    log::read_records,
    read_snapshot,
};

fn open_writer(path: &Path, compression: u8) -> Result<DB> {
    Ok(DB::open(
        path,
        Options {
            compressor: compression,
            block_size: 128,
            block_restart_interval: 2,
            ..Options::default()
        },
    )?)
}

fn engine_entries(database: &mut DB) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
    let mut iterator = database.new_iter()?;
    let mut entries = BTreeMap::new();
    while let Some((key, value)) = iterator.next() {
        entries.insert(key, value);
    }
    Ok(entries)
}

fn image(path: &Path) -> Result<BTreeMap<PathBuf, (Vec<u8>, SystemTime)>> {
    fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            Ok((
                entry.file_name().into(),
                (fs::read(entry.path())?, entry.metadata()?.modified()?),
            ))
        })
        .collect()
}

#[test]
fn matches_engine_for_tables_wal_versions_and_tombstones_without_writes() -> Result<()> {
    for compression in [0, 1] {
        let directory = tempfile::tempdir()?;
        let mut database = open_writer(directory.path(), compression)?;
        let mut expected = BTreeMap::new();
        for index in 0..200 {
            let key = format!("key-{index:04}").into_bytes();
            let value = format!("value-{index:04}-{}世界", "x".repeat(300)).into_bytes();
            database.put(&key, &value)?;
            expected.insert(key, value);
        }
        let large = vec![b'x'; 100_000];
        database.put(b"large", &large)?;
        database.compact_range(b"", b"\xff")?;
        assert!(!Manifest::read(directory.path())?.tables.is_empty());
        for index in 0..200 {
            let key = format!("key-{index:04}").into_bytes();
            if index % 3 == 0 {
                database.delete(&key)?;
                expected.remove(&key);
            } else if index % 5 == 0 {
                database.put(&key, b"new-value")?;
                expected.insert(key, b"new-value".to_vec());
            }
        }
        let large = vec![b'y'; 110_000];
        database.put(b"large", &large)?;
        expected.insert(b"large".to_vec(), large);
        database.put(b"", b"empty-key")?;
        expected.insert(Vec::new(), b"empty-key".to_vec());
        database.flush()?;
        assert_eq!(engine_entries(&mut database)?, expected);

        // The writer still holds LOCK, and its WAL has not been recovered into
        // an SST. Reading must not acquire a lock or modify any snapshot file.
        let before = image(directory.path())?;
        assert_eq!(read_snapshot(directory.path())?, expected);
        assert_eq!(image(directory.path())?, before);
    }
    Ok(())
}

#[test]
fn ignores_obsolete_tables_logs_and_manifests_after_compaction() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut database = open_writer(directory.path(), 1)?;
    database.put(b"deleted", b"must-not-resurrect")?;
    database.flush()?;
    let old_log_path = fs::read_dir(directory.path())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .find(|path| path.extension().is_some_and(|extension| extension == "log"))
        .unwrap();
    let old_log_number: u64 = old_log_path
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .parse()?;
    let old_log = fs::read(&old_log_path)?;
    database.compact_range(b"", b"\xff")?;
    let tables = Manifest::read(directory.path())?.tables;
    let old_tables = tables
        .keys()
        .map(|(_, number)| {
            let name = format!("{number:06}.ldb");
            Ok((name.clone(), fs::read(directory.path().join(name))?))
        })
        .collect::<Result<Vec<_>>>()?;
    database.delete(b"deleted")?;
    database.put(b"kept", b"current")?;
    database.compact_range(b"", b"\xff")?;
    let expected = engine_entries(&mut database)?;
    database.close()?;
    drop(database);

    let live = Manifest::read(directory.path())?;
    assert!(old_log_number < live.log_number);
    assert_ne!(old_log_number, live.previous_log_number);
    for ((_, number), _) in tables {
        assert!(
            !live
                .tables
                .keys()
                .any(|(_, live_number)| *live_number == number)
        );
    }
    for (name, bytes) in old_tables {
        fs::write(directory.path().join(name), bytes)?;
    }
    fs::write(old_log_path, old_log)?;
    fs::write(
        directory.path().join("999999.ldb"),
        b"unreferenced incomplete table",
    )?;
    fs::write(
        directory.path().join("MANIFEST-999999"),
        b"unselected manifest",
    )?;
    assert_eq!(read_snapshot(directory.path())?, expected);
    assert!(!expected.contains_key(b"deleted".as_slice()));
    Ok(())
}

#[test]
fn reads_legacy_sst_extension_and_rejects_corrupt_or_missing_live_tables() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut database = open_writer(directory.path(), 1)?;
    database.put(b"key", b"value")?;
    database.compact_range(b"", b"\xff")?;
    database.close()?;
    drop(database);
    let manifest = Manifest::read(directory.path())?;
    let (_, number) = manifest.tables.keys().next().unwrap();
    let original = directory.path().join(format!("{number:06}.ldb"));
    let legacy = original.with_extension("sst");
    fs::rename(&original, &legacy)?;
    assert_eq!(
        read_snapshot(directory.path())?.get(b"key".as_slice()),
        Some(&b"value".to_vec())
    );
    let mut bytes = fs::read(&legacy)?;
    bytes[0] ^= 1;
    fs::write(&legacy, &bytes)?;
    let error = read_snapshot(directory.path()).unwrap_err();
    assert!(format!("{error:#}").contains("checksum"));
    fs::remove_file(legacy)?;
    assert!(read_snapshot(directory.path()).is_err());
    Ok(())
}

#[test]
fn rejects_malformed_table_blocks_even_with_valid_checksums() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut database = open_writer(directory.path(), 0)?;
    database.put(b"key", b"value")?;
    database.compact_range(b"", b"\xff")?;
    database.close()?;
    drop(database);
    let manifest = Manifest::read(directory.path())?;
    let (_, number) = manifest.tables.keys().next().unwrap();
    let path = directory.path().join(format!("{number:06}.ldb"));
    let original = fs::read(&path)?;

    let mut footer = Decoder::new(&original[original.len() - 48..original.len() - 8]);
    footer.varint()?;
    footer.varint()?;
    let index_offset = footer.varint()? as usize;
    let index_size = footer.varint()? as usize;
    assert_eq!(original[index_offset + index_size], 0);
    let mut index = Decoder::new(&original[index_offset..index_offset + index_size]);
    assert_eq!(index.varint32()?, 0);
    let key_length = index.varint32()? as usize;
    let value_length = index.varint32()? as usize;
    index.take(key_length)?;
    let mut handle = Decoder::new(index.take(value_length)?);
    let offset = handle.varint()? as usize;
    let size = handle.varint()? as usize;

    for case in 0..7 {
        let mut bytes = original.clone();
        match case {
            0 => bytes[offset] = 1,       // First entry cannot share a key prefix.
            1 => bytes[offset + 1] = 127, // Key extends outside the block.
            2 => bytes[offset + 2] = 127, // Value extends outside the block.
            3 => bytes[offset + size - 4..offset + size].fill(0xff), // Restart count overflow.
            4 => bytes[offset + size - 8] = 1, // First restart must start at zero.
            5 => bytes[offset + size] = 42, // Unknown compression type.
            6 => {
                bytes[offset..offset + 5].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
                bytes[offset + size] = 1; // Snappy header claims a 4 GiB block.
            }
            _ => unreachable!(),
        }
        let checksum = masked_crc(&bytes[offset..offset + size + 1]);
        bytes[offset + size + 1..offset + size + 5].copy_from_slice(&checksum.to_le_bytes());
        fs::write(&path, bytes)?;
        assert!(read_snapshot(directory.path()).is_err(), "case {case}");
    }
    fs::write(path, original)?;
    assert_eq!(
        read_snapshot(directory.path())?.get(b"key".as_slice()),
        Some(&b"value".to_vec())
    );
    Ok(())
}

fn empty_snapshot(path: &Path) -> Result<PathBuf> {
    write_manifest(path, &[metadata(3, 0)])?;
    Ok(path.join("000003.log"))
}

fn put_batch(sequence: u64, key: &[u8], value: &[u8]) -> Vec<u8> {
    batch(sequence, &[(key, Some(value))])
}

#[test]
fn replays_new_and_previous_wals_and_uses_sequence_order() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let log = empty_snapshot(directory.path())?;
    // Include the legacy previous WAL as well as a newer, unregistered WAL.
    let manifest = directory.path().join("MANIFEST-000001");
    let mut bytes = fs::read(&manifest)?;
    bytes.extend_from_slice(&log_bytes(&[vec![9, 2]]));
    fs::write(&manifest, bytes)?;
    fs::write(
        directory.path().join("000002.log"),
        log_bytes(&[put_batch(1, b"previous", b"value")]),
    )?;
    fs::write(&log, log_bytes(&[put_batch(8, b"key", b"newer")]))?;
    fs::write(
        directory.path().join("000004.log"),
        log_bytes(&[put_batch(3, b"key", b"older")]),
    )?;
    let values = read_snapshot(directory.path())?;
    assert_eq!(values.get(b"key".as_slice()), Some(&b"newer".to_vec()));
    assert_eq!(values.get(b"previous".as_slice()), Some(&b"value".to_vec()));
    Ok(())
}

#[test]
fn ignores_only_incomplete_wal_tail_and_rejects_corrupt_complete_records() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let log = empty_snapshot(directory.path())?;
    let complete = put_batch(1, b"committed", b"value");
    let unfinished = put_batch(2, b"unfinished", &vec![b'x'; 100_000]);
    let valid = log_bytes(&[complete, unfinished]);
    for missing in [1, 1000, 40_000] {
        fs::write(&log, &valid[..valid.len() - missing])?;
        let values = read_snapshot(directory.path())?;
        assert_eq!(values.len(), 1);
        assert!(values.contains_key(b"committed".as_slice()));
    }
    let mut corrupt = valid;
    corrupt[8] ^= 1;
    fs::write(&log, corrupt)?;
    assert!(format!("{:#}", read_snapshot(directory.path()).unwrap_err()).contains("checksum"));
    Ok(())
}

#[test]
fn handles_log_block_padding_and_empty_first_fragments() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("log");
    for first_length in [32754, 32755, 32760, 32761] {
        let records = vec![
            vec![b'a'; first_length],
            vec![b'b'; 70_000],
            b"end".to_vec(),
        ];
        fs::write(&path, log_bytes(&records))?;
        let mut decoded = Vec::new();
        read_records(&path, |record| {
            decoded.push(record.to_vec());
            Ok(())
        })?;
        assert_eq!(decoded, records);
    }
    Ok(())
}

#[test]
fn rejects_malformed_batches_and_current_paths_without_panicking() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let log = empty_snapshot(directory.path())?;
    let valid = put_batch(1, b"key", b"value");
    let mut wrong_count = valid.clone();
    wrong_count[8] = 2;
    let mut unknown_type = valid.clone();
    unknown_type[12] = 2;
    let mut length_overflow = valid.clone();
    length_overflow.splice(13..14, [0xff; 10]);
    let mut extra_bytes = valid.clone();
    extra_bytes.push(0);
    for malformed in [
        valid[..11].to_vec(),
        wrong_count,
        unknown_type,
        length_overflow,
        extra_bytes,
        put_batch(u64::MAX, b"key", b"value"),
    ] {
        fs::write(&log, log_bytes(&[malformed]))?;
        assert!(read_snapshot(directory.path()).is_err());
    }
    fs::write(&log, [])?;
    for current in [
        "../MANIFEST-000001\n",
        "MANIFEST-../000001\n",
        "MANIFEST-000001",
        "\n",
    ] {
        fs::write(directory.path().join("CURRENT"), current)?;
        assert!(read_snapshot(directory.path()).is_err());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn reads_a_directory_with_no_write_permissions() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    struct RestorePermissions(Vec<(PathBuf, fs::Permissions)>);
    impl Drop for RestorePermissions {
        fn drop(&mut self) {
            for (path, permissions) in &self.0 {
                fs::set_permissions(path, permissions.clone()).unwrap();
            }
        }
    }

    let directory = tempfile::tempdir()?;
    let log = empty_snapshot(directory.path())?;
    fs::write(log, log_bytes(&[put_batch(1, b"key", b"value")]))?;
    let before = image(directory.path())?;
    let mut permissions = RestorePermissions(vec![(
        directory.path().to_owned(),
        fs::metadata(directory.path())?.permissions(),
    )]);
    for entry in fs::read_dir(directory.path())? {
        let entry = entry?;
        permissions
            .0
            .push((entry.path(), entry.metadata()?.permissions()));
        fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o444))?;
    }
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o555))?;
    assert_eq!(
        read_snapshot(directory.path())?.get(b"key".as_slice()),
        Some(&b"value".to_vec())
    );
    assert_eq!(image(directory.path())?, before);
    Ok(())
}
