// Copyright (c) 2011 The LevelDB Authors. All rights reserved.
// Read-only adaptations of db/recovery_test.cc, db/db_test.cc and
// db/corruption_test.cc. See ../../TESTS.md, ../../LICENSE-LevelDB and
// ../../AUTHORS-LevelDB for the exact scope and intentional differences.

use std::{fs, path::Path};

use anyhow::Result;

use super::support::*;
use crate::read_snapshot;

fn files(directory: &Path) -> Result<Entries> {
    fs::read_dir(directory)?
        .map(|entry| {
            let entry = entry?;
            Ok((
                entry.file_name().to_string_lossy().as_bytes().to_vec(),
                fs::read(entry.path())?,
            ))
        })
        .collect()
}

fn check(directory: &Path, expected: &Entries) -> Result<()> {
    let before = files(directory)?;
    assert_eq!(&read_snapshot(directory)?, expected);
    assert_eq!(
        files(directory)?,
        before,
        "recovery must not write, delete or create database files"
    );
    Ok(())
}

#[test]
fn manifest_reused() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    fs::write(
        directory.path().join("000003.log"),
        log_bytes(&[batch(1, &[(b"foo", Some(b"bar"))])]),
    )?;
    let expected = [(b"foo".to_vec(), b"bar".to_vec())].into();
    check(directory.path(), &expected)?;
    // A second read and an appended VersionEdit both preserve CURRENT and the
    // original manifest, without requiring the engine's reuse_logs option.
    let path = directory.path().join("MANIFEST-000001");
    let mut bytes = fs::read(&path)?;
    append_log(&mut bytes, &[4, 1]);
    fs::write(path, bytes)?;
    check(directory.path(), &expected)
}

#[test]
fn large_manifest_compacted_read_without_compaction() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    let path = directory.path().join("MANIFEST-000001");
    let mut bytes = fs::read(&path)?;
    bytes.resize(3 * 1048576, 0);
    fs::write(path, bytes)?;
    fs::write(
        directory.path().join("000003.log"),
        log_bytes(&[batch(1, &[(b"foo", Some(b"bar"))])]),
    )?;
    check(
        directory.path(),
        &[(b"foo".to_vec(), b"bar".to_vec())].into(),
    )
}

#[test]
fn no_log_files() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    check(directory.path(), &Entries::new())
}

#[test]
fn log_file_reuse() -> Result<()> {
    for empty_log in [true, false] {
        let directory = tempfile::tempdir()?;
        let mut edit = metadata(3, 1);
        let bytes = if empty_log {
            save_table(
                directory.path(),
                &mut edit,
                1,
                4,
                &[(internal_key(b"foo", 1, 1), b"bar".to_vec())],
            )?;
            vec![]
        } else {
            log_bytes(&[batch(1, &[(b"foo", Some(b"bar"))])])
        };
        write_manifest(directory.path(), &[edit])?;
        fs::write(directory.path().join("000003.log"), bytes)?;
        check(
            directory.path(),
            &[(b"foo".to_vec(), b"bar".to_vec())].into(),
        )?;
    }
    Ok(())
}

#[test]
fn multiple_mem_tables_recovered_in_memory() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    let mut expected = Entries::new();
    let mut records = Vec::new();
    for i in 0..1000 {
        let key = format!("{i:050}").into_bytes();
        records.push(batch(i + 1, &[(&key, Some(&key))]));
        expected.insert(key.clone(), key);
    }
    fs::write(directory.path().join("000003.log"), log_bytes(&records))?;
    check(directory.path(), &expected)
}

#[test]
fn multiple_log_files() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    let writes: &[(u64, u64, &[u8], &[u8])] = &[
        (3, 1, b"foo", b"bar"),
        (4, 1000, b"hello", b"world"),
        (5, 1001, b"hi", b"there"),
        (6, 1002, b"foo", b"bar2"),
        (2, 2000, b"hello", b"stale write"),
    ];
    for (number, sequence, key, value) in writes {
        fs::write(
            directory.path().join(format!("{number:06}.log")),
            log_bytes(&[batch(*sequence, &[(key, Some(value))])]),
        )?;
    }
    check(
        directory.path(),
        &[
            (b"foo".to_vec(), b"bar2".to_vec()),
            (b"hello".to_vec(), b"world".to_vec()),
            (b"hi".to_vec(), b"there".to_vec()),
        ]
        .into(),
    )
}

#[test]
fn manifest_missing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    fs::remove_file(directory.path().join("MANIFEST-000001"))?;
    let before = files(directory.path())?;
    assert!(read_snapshot(directory.path()).is_err());
    assert_eq!(files(directory.path())?, before);
    Ok(())
}

#[test]
fn get_level0_ordering() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut edit = metadata(3, 3);
    save_table(
        directory.path(),
        &mut edit,
        0,
        4,
        &[
            (internal_key(b"bar", 1, 1), b"b".to_vec()),
            (internal_key(b"foo", 2, 1), b"v1".to_vec()),
        ],
    )?;
    save_table(
        directory.path(),
        &mut edit,
        0,
        5,
        &[(internal_key(b"foo", 3, 1), b"v2".to_vec())],
    )?;
    write_manifest(directory.path(), &[edit])?;
    check(
        directory.path(),
        &[
            (b"bar".to_vec(), b"b".to_vec()),
            (b"foo".to_vec(), b"v2".to_vec()),
        ]
        .into(),
    )
}

#[test]
fn get_ordered_by_levels() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut edit = metadata(3, 2);
    save_table(
        directory.path(),
        &mut edit,
        2,
        4,
        &[(internal_key(b"foo", 1, 1), b"v1".to_vec())],
    )?;
    save_table(
        directory.path(),
        &mut edit,
        0,
        5,
        &[(internal_key(b"foo", 2, 1), b"v2".to_vec())],
    )?;
    write_manifest(directory.path(), &[edit])?;
    check(
        directory.path(),
        &[(b"foo".to_vec(), b"v2".to_vec())].into(),
    )
}

#[test]
fn recover() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    let path = directory.path().join("000003.log");
    let mut bytes = log_bytes(&[batch(1, &[(b"foo", Some(b"v1")), (b"baz", Some(b"v5"))])]);
    fs::write(&path, &bytes)?;
    check(
        directory.path(),
        &[
            (b"foo".to_vec(), b"v1".to_vec()),
            (b"baz".to_vec(), b"v5".to_vec()),
        ]
        .into(),
    )?;
    append_log(
        &mut bytes,
        &batch(3, &[(b"bar", Some(b"v2")), (b"foo", Some(b"v3"))]),
    );
    fs::write(&path, &bytes)?;
    let mut expected: Entries = [
        (b"foo".to_vec(), b"v3".to_vec()),
        (b"baz".to_vec(), b"v5".to_vec()),
        (b"bar".to_vec(), b"v2".to_vec()),
    ]
    .into();
    check(directory.path(), &expected)?;
    append_log(&mut bytes, &batch(5, &[(b"foo", Some(b"v4"))]));
    fs::write(path, bytes)?;
    expected.insert(b"foo".to_vec(), b"v4".to_vec());
    check(directory.path(), &expected)
}

#[test]
fn recovery_with_empty_log() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut edit = metadata(3, 2);
    save_table(
        directory.path(),
        &mut edit,
        1,
        4,
        &[
            (internal_key(b"foo", 2, 1), b"v2".to_vec()),
            (internal_key(b"foo", 1, 1), b"v1".to_vec()),
        ],
    )?;
    write_manifest(directory.path(), &[edit])?;
    let path = directory.path().join("000003.log");
    fs::write(&path, [])?;
    check(
        directory.path(),
        &[(b"foo".to_vec(), b"v2".to_vec())].into(),
    )?;
    fs::write(path, log_bytes(&[batch(3, &[(b"foo", Some(b"v3"))])]))?;
    check(
        directory.path(),
        &[(b"foo".to_vec(), b"v3".to_vec())].into(),
    )
}

#[test]
fn iter_multi_with_delete() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    fs::write(
        directory.path().join("000003.log"),
        log_bytes(&[batch(
            1,
            &[
                (b"a", Some(b"va")),
                (b"b", Some(b"vb")),
                (b"c", Some(b"vc")),
                (b"b", None),
            ],
        )]),
    )?;
    check(
        directory.path(),
        &[
            (b"a".to_vec(), b"va".to_vec()),
            (b"c".to_vec(), b"vc".to_vec()),
        ]
        .into(),
    )
}

#[test]
fn iter_multi_with_delete_and_compaction() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut edit = metadata(3, 3);
    save_table(
        directory.path(),
        &mut edit,
        1,
        4,
        &[
            (internal_key(b"a", 3, 1), b"va".to_vec()),
            (internal_key(b"b", 1, 1), b"vb".to_vec()),
            (internal_key(b"c", 2, 1), b"vc".to_vec()),
        ],
    )?;
    write_manifest(directory.path(), &[edit])?;
    fs::write(
        directory.path().join("000003.log"),
        log_bytes(&[batch(4, &[(b"b", None)])]),
    )?;
    check(
        directory.path(),
        &[
            (b"a".to_vec(), b"va".to_vec()),
            (b"c".to_vec(), b"vc".to_vec()),
        ]
        .into(),
    )
}

#[test]
fn sequence_number_recovery() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut edit = metadata(3, 5);
    let entries: Vec<_> = (1..=5)
        .rev()
        .map(|i| (internal_key(b"foo", i, 1), format!("v{i}").into_bytes()))
        .collect();
    save_table(directory.path(), &mut edit, 1, 4, &entries)?;
    write_manifest(directory.path(), &[edit])?;
    check(
        directory.path(),
        &[(b"foo".to_vec(), b"v5".to_vec())].into(),
    )?;
    fs::write(
        directory.path().join("000003.log"),
        log_bytes(&[batch(6, &[(b"foo", Some(b"v6"))])]),
    )?;
    check(
        directory.path(),
        &[(b"foo".to_vec(), b"v6".to_vec())].into(),
    )
}

#[test]
fn corrupted_recovery_is_an_error_without_salvage() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    let records: Vec<_> = (1..=100)
        .map(|i| {
            batch(
                i,
                &[(format!("{i:016}").as_bytes(), Some(&vec![b'x'; 1000]))],
            )
        })
        .collect();
    let mut bytes = log_bytes(&records);
    bytes[19] ^= 1;
    bytes[BLOCK_SIZE + 1000] ^= 1;
    fs::write(directory.path().join("000003.log"), bytes)?;
    let before = files(directory.path())?;
    assert!(read_snapshot(directory.path()).is_err());
    assert_eq!(files(directory.path())?, before);
    Ok(())
}

#[test]
fn sst_versions_newer_than_the_recovered_sequence_are_invisible() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut edit = metadata(3, 1);
    save_table(
        directory.path(),
        &mut edit,
        1,
        4,
        &[
            (internal_key(b"foo", 5, 1), b"future".to_vec()),
            (internal_key(b"foo", 1, 1), b"current".to_vec()),
        ],
    )?;
    write_manifest(directory.path(), &[edit])?;
    check(
        directory.path(),
        &[(b"foo".to_vec(), b"current".to_vec())].into(),
    )?;
    fs::write(
        directory.path().join("000003.log"),
        log_bytes(&[batch(5, &[(b"other", Some(b"value"))])]),
    )?;
    check(
        directory.path(),
        &[
            (b"foo".to_vec(), b"future".to_vec()),
            (b"other".to_vec(), b"value".to_vec()),
        ]
        .into(),
    )
}

#[test]
fn table_tombstone_hides_an_older_wal_value() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut edit = metadata(3, 5);
    save_table(
        directory.path(),
        &mut edit,
        1,
        4,
        &[(internal_key(b"foo", 5, 0), vec![])],
    )?;
    write_manifest(directory.path(), &[edit])?;
    fs::write(
        directory.path().join("000003.log"),
        log_bytes(&[batch(1, &[(b"foo", Some(b"old"))])]),
    )?;
    check(directory.path(), &Entries::new())
}

#[test]
fn empty_batch_cannot_advance_the_snapshot_sequence() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut edit = metadata(3, 1);
    save_table(
        directory.path(),
        &mut edit,
        1,
        4,
        &[
            (internal_key(b"foo", 5, 1), b"future".to_vec()),
            (internal_key(b"foo", 1, 1), b"current".to_vec()),
        ],
    )?;
    write_manifest(directory.path(), &[edit])?;
    fs::write(
        directory.path().join("000003.log"),
        log_bytes(&[batch(100, &[])]),
    )?;
    check(
        directory.path(),
        &[(b"foo".to_vec(), b"current".to_vec())].into(),
    )
}

#[test]
fn malformed_batch_does_not_return_a_partial_snapshot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    let mut malformed = batch(2, &[(b"foo", Some(b"updated")), (b"bar", None)]);
    malformed.pop();
    fs::write(
        directory.path().join("000003.log"),
        log_bytes(&[batch(1, &[(b"foo", Some(b"old"))]), malformed]),
    )?;
    assert!(read_snapshot(directory.path()).is_err());
    Ok(())
}

#[test]
fn duplicate_log_number_is_rejected() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    for name in ["000003.log", "3.log"] {
        fs::write(directory.path().join(name), [])?;
    }
    assert!(
        read_snapshot(directory.path())
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    Ok(())
}

#[test]
fn native_chromium_tables_and_fragmented_wal() -> Result<()> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chromium/recovery");
    let mut expected = Entries::new();
    for index in 0..200 {
        if index % 7 == 0 {
            continue;
        }
        let key = format!("key-{index:04}").into_bytes();
        let value = if index % 5 == 0 {
            b"new".to_vec()
        } else {
            format!("value-{index}{}", "x".repeat(300)).into_bytes()
        };
        expected.insert(key, value);
    }
    expected.insert(vec![], vec![]);
    expected.insert(vec![0, 0xff, b'k'], vec![0xff, 0]);
    expected.insert(b"large".to_vec(), vec![b'y'; 120_000]);
    assert_eq!(expected.len(), 174);
    check(&directory, &expected)
}
