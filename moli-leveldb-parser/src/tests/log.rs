// Copyright (c) 2011 The LevelDB Authors. All rights reserved.
// Rust ports of db/log_test.cc. See ../../LICENSE-LevelDB, ../../AUTHORS-LevelDB
// and ../../TESTS.md. Corruption cases assert Err instead of reporter/salvage.

use std::io::{self, Cursor, Read};

use anyhow::Result;

use super::support::*;
use crate::log::read_records_from;

fn read(bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut records = Vec::new();
    read_records_from(Cursor::new(bytes), |record| {
        records.push(record.to_vec());
        Ok(())
    })?;
    Ok(records)
}

fn roundtrip(records: &[Vec<u8>]) {
    assert_eq!(read(&log_bytes(records)).unwrap(), records);
}

fn error(bytes: &[u8], expected: &str) {
    let error = format!("{:#}", read(bytes).unwrap_err());
    assert!(error.contains(expected), "{error}");
}

#[test]
fn empty() {
    roundtrip(&[]);
}

#[test]
fn read_write() {
    roundtrip(&[b"foo".to_vec(), b"bar".to_vec(), vec![], b"xxxx".to_vec()]);
}

#[test]
fn many_blocks() {
    let records: Vec<_> = (0..100_000).map(|i| format!("{i}.").into_bytes()).collect();
    roundtrip(&records);
}

#[test]
fn fragmentation() {
    roundtrip(&[
        b"small".to_vec(),
        big_string(b"medium", 50_000),
        big_string(b"large", 100_000),
    ]);
}

#[test]
fn marginal_trailer() {
    roundtrip(&[
        big_string(b"foo", BLOCK_SIZE - 2 * HEADER_SIZE),
        vec![],
        b"bar".to_vec(),
    ]);
}

#[test]
fn marginal_trailer2() {
    roundtrip(&[
        big_string(b"foo", BLOCK_SIZE - 2 * HEADER_SIZE),
        b"bar".to_vec(),
    ]);
}

#[test]
fn short_trailer() {
    roundtrip(&[
        big_string(b"foo", BLOCK_SIZE - 2 * HEADER_SIZE + 4),
        vec![],
        b"bar".to_vec(),
    ]);
}

#[test]
fn aligned_eof() {
    roundtrip(&[big_string(b"foo", BLOCK_SIZE - 2 * HEADER_SIZE + 4)]);
}

#[test]
fn open_for_append() {
    let mut bytes = log_bytes(&[b"hello".to_vec()]);
    append_log(&mut bytes, b"world");
    assert_eq!(read(&bytes).unwrap(), [b"hello", b"world"]);
}

#[test]
fn random_read() {
    let mut random = Random::new(301);
    let records: Vec<_> = (0..500)
        .map(|i| big_string(format!("{i}.").as_bytes(), random.skewed(17)))
        .collect();
    roundtrip(&records);
}

#[test]
fn read_error() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("injected read error"))
        }
    }
    let result = read_records_from(Broken, |_| panic!("read error must propagate"));
    assert!(format!("{:#}", result.unwrap_err()).contains("injected read error"));
}

#[test]
fn bad_record_type() {
    let mut bytes = log_bytes(&[b"foo".to_vec()]);
    bytes[6] += 100;
    fix_log_checksum(&mut bytes, 0);
    error(&bytes, "record type");
}

#[test]
fn truncated_trailing_record_is_ignored() {
    let mut bytes = log_bytes(&[b"foo".to_vec()]);
    bytes.truncate(bytes.len() - 4);
    assert!(read(&bytes).unwrap().is_empty());
}

#[test]
fn bad_length() {
    let mut bytes = log_bytes(&[
        big_string(b"bar", BLOCK_SIZE - HEADER_SIZE),
        b"foo".to_vec(),
    ]);
    bytes[4] += 1;
    error(&bytes, "block boundary");
}

#[test]
fn bad_length_at_end_is_ignored() {
    let mut bytes = log_bytes(&[b"foo".to_vec()]);
    bytes.pop();
    assert!(read(&bytes).unwrap().is_empty());
}

#[test]
fn checksum_mismatch() {
    let mut bytes = log_bytes(&[b"foo".to_vec()]);
    bytes[0] = bytes[0].wrapping_add(10);
    error(&bytes, "checksum mismatch");
}

#[test]
fn unexpected_middle_type() {
    error(&physical_record(3, b"foo"), "without FIRST");
}

#[test]
fn unexpected_last_type() {
    error(&physical_record(4, b"foo"), "without FIRST");
}

#[test]
fn unexpected_full_type() {
    let mut bytes = log_bytes(&[b"foo".to_vec(), b"bar".to_vec()]);
    bytes[6] = 2;
    fix_log_checksum(&mut bytes, 0);
    error(&bytes, "before FULL");
}

#[test]
fn unexpected_first_type() {
    let mut bytes = log_bytes(&[b"foo".to_vec(), big_string(b"bar", 100_000)]);
    bytes[6] = 2;
    fix_log_checksum(&mut bytes, 0);
    error(&bytes, "before FIRST");
}

#[test]
fn missing_last_is_ignored() {
    let mut bytes = log_bytes(&[big_string(b"bar", BLOCK_SIZE)]);
    bytes.truncate(bytes.len() - 14);
    assert!(read(&bytes).unwrap().is_empty());
}

#[test]
fn partial_last_is_ignored() {
    let mut bytes = log_bytes(&[big_string(b"bar", BLOCK_SIZE)]);
    bytes.pop();
    assert!(read(&bytes).unwrap().is_empty());
}

#[test]
fn error_joins_records() {
    let mut bytes = log_bytes(&[
        big_string(b"foo", BLOCK_SIZE),
        big_string(b"bar", BLOCK_SIZE),
        b"correct".to_vec(),
    ]);
    bytes[BLOCK_SIZE..2 * BLOCK_SIZE].fill(b'x');
    let mut seen = Vec::new();
    assert!(
        read_records_from(Cursor::new(bytes), |record| {
            seen.push(record.to_vec());
            Ok(())
        })
        .is_err()
    );
    assert!(
        seen.is_empty(),
        "fragments from different records must never be joined"
    );
}

// Additional regressions for the compatibility behavior documented in the
// pinned db/log_reader.cc and for the snapshot reader's strict error policy.
#[test]
fn legacy_empty_first_before_full_or_first() {
    for record in [b"full".to_vec(), big_string(b"fragmented", 100_000)] {
        let first = big_string(b"foo", BLOCK_SIZE - 2 * HEADER_SIZE);
        let mut bytes = log_bytes(std::slice::from_ref(&first));
        bytes.extend_from_slice(&physical_record(2, b""));
        append_log(&mut bytes, &record);
        assert_eq!(read(&bytes).unwrap(), [first, record]);
    }
}

#[test]
fn all_fragment_checksums_are_verified() {
    let original = log_bytes(&[big_string(b"payload", 3 * BLOCK_SIZE)]);
    for offset in [0, BLOCK_SIZE, 2 * BLOCK_SIZE, 3 * BLOCK_SIZE] {
        let mut bytes = original.clone();
        bytes[offset + HEADER_SIZE] ^= 1;
        error(&bytes, "checksum mismatch");
    }
}

#[test]
fn truncated_tail_never_emits_a_partial_logical_record() {
    let prefix = log_bytes(&[b"committed".to_vec()]);
    let mut bytes = prefix.clone();
    append_log(&mut bytes, &big_string(b"pending", 2 * BLOCK_SIZE));
    let mut cuts: Vec<_> = (prefix.len()..prefix.len() + HEADER_SIZE + 2).collect();
    for boundary in [BLOCK_SIZE, 2 * BLOCK_SIZE, bytes.len()] {
        cuts.extend(boundary - HEADER_SIZE..boundary + HEADER_SIZE);
    }
    for cut in cuts.into_iter().filter(|cut| *cut < bytes.len()) {
        assert_eq!(read(&bytes[..cut]).unwrap(), [b"committed"], "cut {cut}");
    }
}

#[test]
fn zero_filled_preallocation() {
    let mut bytes = log_bytes(&[b"foo".to_vec()]);
    bytes.resize(3 * BLOCK_SIZE, 0);
    append_log(&mut bytes, b"bar");
    assert_eq!(read(&bytes).unwrap(), [b"foo", b"bar"]);
}

#[test]
fn padding_cannot_join_fragments() {
    let mut bytes = physical_record(2, b"first");
    bytes.resize(BLOCK_SIZE, 0);
    bytes.extend_from_slice(&physical_record(4, b"unrelated"));
    error(&bytes, "without FIRST");
}

#[test]
fn nonzero_padding_is_corruption() {
    let mut bytes = vec![0; BLOCK_SIZE];
    bytes[HEADER_SIZE] = 1;
    error(&bytes, "padding");
    let mut bytes = log_bytes(&[big_string(b"foo", BLOCK_SIZE - HEADER_SIZE - 1)]);
    bytes.push(1);
    error(&bytes, "trailer");
}

#[test]
fn short_reads_and_interrupted_io() {
    struct Chunked {
        inner: Cursor<Vec<u8>>,
        calls: usize,
    }
    impl Read for Chunked {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.calls += 1;
            if self.calls.is_multiple_of(7) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let length = buffer.len().min(13);
            self.inner.read(&mut buffer[..length])
        }
    }
    let records = [big_string(b"foo", 2 * BLOCK_SIZE), b"tail".to_vec()];
    let source = Chunked {
        inner: Cursor::new(log_bytes(&records)),
        calls: 0,
    };
    let mut seen = Vec::new();
    read_records_from(source, |record| {
        seen.push(record.to_vec());
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, records);
}

#[test]
fn visitor_error_stops_the_read() {
    let bytes = log_bytes(&[b"foo".to_vec(), b"bar".to_vec()]);
    let mut calls = 0;
    let result = read_records_from(Cursor::new(bytes), |_| {
        calls += 1;
        anyhow::bail!("visitor error")
    });
    assert!(format!("{:#}", result.unwrap_err()).contains("visitor error"));
    assert_eq!(calls, 1);
}
