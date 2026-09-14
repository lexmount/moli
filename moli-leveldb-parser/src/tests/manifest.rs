// Copyright (c) 2011 The LevelDB Authors. All rights reserved.
// VersionEditTest.EncodeDecode plus parser regressions for db/version_edit.cc.
// See ../../TESTS.md, ../../LICENSE-LevelDB and ../../AUTHORS-LevelDB.

use std::fs;

use anyhow::Result;

use super::support::*;
use crate::{MAX_SEQUENCE, manifest::Manifest};

fn read(edits: &[Vec<u8>]) -> Result<Manifest> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), edits)?;
    Manifest::read(directory.path())
}

#[test]
fn encode_decode() -> Result<()> {
    const BIG: u64 = 1 << 50;
    let mut records = vec![metadata(BIG + 100, BIG + 1000)];
    let mut edit = Vec::new();
    for i in 0..4 {
        let parsed = read(&records)?;
        assert_eq!(parsed.tables.len(), i as usize);
        add_table(
            &mut edit,
            3,
            BIG + 300 + i,
            BIG + 400 + i,
            &internal_key(b"foo", BIG + 500 + i, 1),
            &internal_key(b"zoo", BIG + 600 + i, 0),
        );
        delete_table(&mut edit, 4, BIG + 700 + i);
        edit.push(5);
        varint(i, &mut edit);
        slice(&internal_key(b"x", BIG + 900 + i, 1), &mut edit);
        records.truncate(1);
        records.push(edit.clone());
    }
    let parsed = read(&records)?;
    assert_eq!(parsed.log_number, BIG + 100);
    assert_eq!(parsed.last_sequence, BIG + 1000);
    for i in 0..4 {
        assert_eq!(
            parsed.tables.get(&(3, BIG + 300 + i)),
            Some(&(BIG + 400 + i))
        );
    }
    Ok(())
}

#[test]
fn comparator_check() {
    let mut edit = vec![1];
    slice(b"foo", &mut edit);
    assert!(format!("{:#}", read(&[metadata(3, 0), edit]).err().unwrap()).contains("comparator"));
}

#[test]
fn required_metadata_can_span_records() -> Result<()> {
    let parsed = read(&[vec![2, 3], vec![], vec![3, 4], vec![4, 5]])?;
    assert_eq!((parsed.log_number, parsed.last_sequence), (3, 5));
    Ok(())
}

#[test]
fn missing_required_metadata() {
    for mask in 0..7 {
        let fields = [vec![2, 3], vec![3, 4], vec![4, 5]];
        let edits: Vec<_> = fields
            .into_iter()
            .enumerate()
            .filter_map(|(i, field)| ((mask & (1 << i)) != 0).then_some(field))
            .collect();
        assert!(read(&edits).is_err(), "mask {mask}");
    }
}

#[test]
fn later_metadata_and_previous_log_override_earlier_values() -> Result<()> {
    let parsed = read(&[metadata(3, 5), vec![2, 8, 9, 3, 4, 12]])?;
    assert_eq!(
        (
            parsed.log_number,
            parsed.previous_log_number,
            parsed.last_sequence
        ),
        (8, 3, 12)
    );
    Ok(())
}

#[test]
fn unknown_tags() {
    for tag in [0, 8, 10, 100, u32::MAX as u64, u64::MAX] {
        let mut edit = Vec::new();
        varint(tag, &mut edit);
        assert!(read(&[metadata(3, 0), edit]).is_err(), "tag {tag}");
    }
}

#[test]
fn truncated_version_edit_fields() {
    let mut fields = Vec::new();
    let mut comparator = vec![1];
    slice(b"leveldb.BytewiseComparator", &mut comparator);
    fields.push(comparator);
    for tag in [2, 3, 4, 9] {
        let mut field = vec![tag];
        varint(1 << 50, &mut field);
        fields.push(field);
    }
    let mut compaction = vec![5, 1];
    slice(&internal_key(b"x", 100, 1), &mut compaction);
    fields.push(compaction);
    let mut deleted = Vec::new();
    delete_table(&mut deleted, 1, 1 << 50);
    fields.push(deleted);
    let mut added = Vec::new();
    add_table(
        &mut added,
        1,
        1 << 50,
        4096,
        &internal_key(b"a", 1, 1),
        &internal_key(b"z", 9, 1),
    );
    fields.push(added);
    for field in fields {
        for end in 1..field.len() {
            assert!(
                read(&[metadata(3, 0), field[..end].to_vec()]).is_err(),
                "tag {}, end {end}",
                field[0]
            );
        }
        assert!(read(&[metadata(3, 0), field]).is_ok());
    }
}

#[test]
fn invalid_levels() {
    for level in [7, u32::MAX as u64, u64::MAX] {
        for tag in [5, 6, 7] {
            let mut edit = vec![tag];
            varint(level, &mut edit);
            edit.extend_from_slice(&[0; 32]);
            assert!(
                format!("{:#}", read(&[metadata(3, 0), edit]).err().unwrap()).contains("LevelDB")
            );
        }
    }
}

#[test]
fn truncated_internal_keys() {
    for length in 0..8 {
        let mut compaction = vec![5, 1];
        slice(&[0; 8][..length], &mut compaction);
        assert!(read(&[metadata(3, 0), compaction]).is_err());
        for short_smallest in [true, false] {
            let good = internal_key(b"key", 1, 1);
            let short = &[0; 8][..length];
            let (smallest, largest) = if short_smallest {
                (short, good.as_slice())
            } else {
                (good.as_slice(), short)
            };
            let mut edit = Vec::new();
            add_table(&mut edit, 1, 4, 4096, smallest, largest);
            assert!(read(&[metadata(3, 0), edit]).is_err());
        }
    }
}

#[test]
fn deleted_files_and_level_moves() -> Result<()> {
    let key = internal_key(b"key", 1, 1);
    let mut first = metadata(3, 1);
    add_table(&mut first, 0, 4, 100, &key, &key);
    add_table(&mut first, 1, 5, 200, &key, &key);
    let mut edit = Vec::new();
    add_table(&mut edit, 2, 4, 100, &key, &key);
    delete_table(&mut edit, 0, 4);
    delete_table(&mut edit, 1, 5);
    let parsed = read(&[first, edit])?;
    assert_eq!(parsed.tables, [((2, 4), 100)].into());
    Ok(())
}

#[test]
fn one_edit_applies_deletions_before_additions() -> Result<()> {
    let key = internal_key(b"key", 1, 1);
    let mut edit = metadata(3, 1);
    add_table(&mut edit, 1, 4, 100, &key, &key);
    delete_table(&mut edit, 1, 4);
    assert_eq!(read(&[edit])?.tables, [((1, 4), 100)].into());
    Ok(())
}

#[test]
fn sequence_overflow() {
    assert!(read(&[metadata(3, MAX_SEQUENCE)]).is_ok());
    assert!(read(&[metadata(3, MAX_SEQUENCE + 1)]).is_err());
}

#[test]
fn current_must_name_one_manifest() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 0)])?;
    for name in [
        "",
        "\n",
        "MANIFEST-000001",
        "MANIFEST-000001\n\n",
        "../MANIFEST-000001\n",
        "MANIFEST-../000001\n",
        "MANIFEST-000001/child\n",
        "MANIFEST-abc\n",
        "/MANIFEST-000001\n",
    ] {
        fs::write(directory.path().join("CURRENT"), name)?;
        assert!(Manifest::read(directory.path()).is_err(), "{name:?}");
    }
    Ok(())
}

#[test]
fn current_selects_the_manifest_not_its_largest_number() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 5)])?;
    fs::write(
        directory.path().join("MANIFEST-999999"),
        log_bytes(&[metadata(4, 100)]),
    )?;
    assert_eq!(Manifest::read(directory.path())?.last_sequence, 5);
    Ok(())
}

#[test]
fn incomplete_last_edit_is_ignored() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let first = metadata(3, 5);
    let mut next = vec![4];
    varint(1 << 50, &mut next);
    let bytes = log_bytes(&[first, next]);
    write_manifest(directory.path(), &[])?;
    fs::write(
        directory.path().join("MANIFEST-000001"),
        &bytes[..bytes.len() - 1],
    )?;
    assert_eq!(Manifest::read(directory.path())?.last_sequence, 5);
    Ok(())
}

#[test]
fn complete_edit_checksum_failure_is_not_ignored() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_manifest(directory.path(), &[metadata(3, 5)])?;
    let path = directory.path().join("MANIFEST-000001");
    let mut bytes = fs::read(&path)?;
    bytes[0] ^= 1;
    fs::write(path, bytes)?;
    assert!(format!("{:#}", Manifest::read(directory.path()).err().unwrap()).contains("checksum"));
    Ok(())
}
