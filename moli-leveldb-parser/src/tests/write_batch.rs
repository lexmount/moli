// Copyright (c) 2011 The LevelDB Authors. All rights reserved.
// Ports of db/write_batch_test.cc; see ../../TESTS.md, ../../LICENSE-LevelDB
// and ../../AUTHORS-LevelDB. Assertions observe the latest sequence per key.

use super::support::*;
use crate::{MAX_SEQUENCE, Versions, read_batch};

#[test]
fn empty() {
    let mut versions = Versions::new();
    assert_eq!(read_batch(&batch(0, &[]), &mut versions).unwrap(), None);
    assert!(versions.is_empty());
}

#[test]
fn multiple() {
    let mut versions = Versions::new();
    let bytes = batch(
        100,
        &[
            (b"foo", Some(b"bar")),
            (b"box", None),
            (b"baz", Some(b"boo")),
        ],
    );
    assert_eq!(read_batch(&bytes, &mut versions).unwrap(), Some(102));
    assert_eq!(
        versions,
        Versions::from([
            (b"baz".to_vec(), (102, Some(b"boo".to_vec()))),
            (b"box".to_vec(), (101, None)),
            (b"foo".to_vec(), (100, Some(b"bar".to_vec()))),
        ])
    );
}

#[test]
fn corruption() {
    let mut bytes = batch(200, &[(b"foo", Some(b"bar")), (b"box", None)]);
    bytes.pop();
    assert!(read_batch(&bytes, &mut Versions::new()).is_err());
}

#[test]
fn append() {
    // Upstream appends batches with a different original sequence. On disk the
    // combined header supplies the sequence, and each appended operation adds 1.
    let mut bytes = batch(200, &[(b"a", Some(b"va"))]);
    let appended = batch(300, &[(b"b", Some(b"vb"))]);
    bytes.extend_from_slice(&appended[12..]);
    let appended = batch(300, &[(b"b", Some(b"vb")), (b"foo", None)]);
    bytes.extend_from_slice(&appended[12..]);
    bytes[8..12].copy_from_slice(&4u32.to_le_bytes());
    let mut versions = Versions::new();
    assert_eq!(read_batch(&bytes, &mut versions).unwrap(), Some(203));
    assert_eq!(
        versions,
        Versions::from([
            (b"a".to_vec(), (200, Some(b"va".to_vec()))),
            (b"b".to_vec(), (202, Some(b"vb".to_vec()))),
            (b"foo".to_vec(), (203, None)),
        ])
    );
}

#[test]
fn truncated_header() {
    for end in 0..12 {
        assert!(read_batch(&[0; 12][..end], &mut Versions::new()).is_err());
    }
}

#[test]
fn every_truncated_key_and_value_is_rejected() {
    let key = vec![b'k'; 130];
    let value = vec![b'v'; 300];
    let bytes = batch(12, &[(&key, Some(&value)), (b"deleted", None)]);
    for end in 12..bytes.len() {
        assert!(
            read_batch(&bytes[..end], &mut Versions::new()).is_err(),
            "end {end}"
        );
    }
    assert_eq!(read_batch(&bytes, &mut Versions::new()).unwrap(), Some(13));
}

#[test]
fn count_must_match_in_both_directions() {
    for count in [0u32, 2, u32::MAX] {
        let mut bytes = batch(100, &[(b"foo", Some(b"bar"))]);
        bytes[8..12].copy_from_slice(&count.to_le_bytes());
        assert!(
            read_batch(&bytes, &mut Versions::new()).is_err(),
            "count {count}"
        );
    }
}

#[test]
fn unknown_value_type() {
    for tag in [2, 7, 0xff] {
        let mut bytes = batch(100, &[(b"foo", Some(b"bar"))]);
        bytes[12] = tag;
        let error = read_batch(&bytes, &mut Versions::new()).unwrap_err();
        assert!(error.to_string().contains("value type"));
    }
}

#[test]
fn sequence_limits() {
    let changes: &[(&[u8], Option<&[u8]>)] = &[(b"a", Some(b"v")), (b"b", None)];
    assert_eq!(
        read_batch(&batch(MAX_SEQUENCE, &changes[..1]), &mut Versions::new()).unwrap(),
        Some(MAX_SEQUENCE)
    );
    assert_eq!(
        read_batch(&batch(MAX_SEQUENCE - 1, changes), &mut Versions::new()).unwrap(),
        Some(MAX_SEQUENCE)
    );
    for (sequence, changes) in [
        (MAX_SEQUENCE, changes),
        (MAX_SEQUENCE + 1, &changes[..1]),
        (u64::MAX, &changes[..0]),
    ] {
        assert!(read_batch(&batch(sequence, changes), &mut Versions::new()).is_err());
    }
}

#[test]
fn empty_key_and_value_are_distinct_from_delete() {
    let mut versions = Versions::new();
    read_batch(
        &batch(0, &[(b"", Some(b"")), (b"deleted", None)]),
        &mut versions,
    )
    .unwrap();
    assert_eq!(versions.get(b"".as_slice()), Some(&(0, Some(vec![]))));
    assert_eq!(versions.get(b"deleted".as_slice()), Some(&(1, None)));
}

#[test]
fn later_delete_cannot_be_resurrected_by_an_older_batch() {
    let mut versions = Versions::new();
    read_batch(&batch(20, &[(b"key", None)]), &mut versions).unwrap();
    read_batch(&batch(10, &[(b"key", Some(b"old"))]), &mut versions).unwrap();
    assert_eq!(versions.get(b"key".as_slice()), Some(&(20, None)));
    read_batch(&batch(30, &[(b"key", Some(b"new"))]), &mut versions).unwrap();
    assert_eq!(
        versions.get(b"key".as_slice()),
        Some(&(30, Some(b"new".to_vec())))
    );
}

#[test]
fn duplicate_sequence_requires_identical_contents() {
    let mut versions = Versions::new();
    let bytes = batch(10, &[(b"key", Some(b"value"))]);
    read_batch(&bytes, &mut versions).unwrap();
    read_batch(&bytes, &mut versions).unwrap();
    for value in [Some(b"other".as_slice()), None] {
        assert!(read_batch(&batch(10, &[(b"key", value)]), &mut versions).is_err());
    }
}
