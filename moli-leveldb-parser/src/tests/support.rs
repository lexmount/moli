// Copyright (c) 2011 The LevelDB Authors. All rights reserved.
// Test encoders and Random adapted from LevelDB; see ../../LICENSE-LevelDB,
// ../../AUTHORS-LevelDB and ../../TESTS.md for the pinned sources and scope.

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::Result;

pub const BLOCK_SIZE: usize = 32768;
pub const HEADER_SIZE: usize = 7;
pub type Entries = BTreeMap<Vec<u8>, Vec<u8>>;

// Fixture construction deliberately does not call the production Decoder or
// masked_crc. The CRC wrapper is checked against upstream's published vectors.
pub fn crc(bytes: &[u8]) -> u32 {
    let value = crc32c::crc32c(bytes);
    value.rotate_right(15).wrapping_add(0xa282_ead8)
}

pub fn varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8) | 128);
        value >>= 7;
    }
    output.push(value as u8);
}

pub fn slice(bytes: &[u8], output: &mut Vec<u8>) {
    varint(bytes.len() as u64, output);
    output.extend_from_slice(bytes);
}

pub fn internal_key(key: &[u8], sequence: u64, kind: u8) -> Vec<u8> {
    let mut result = key.to_vec();
    result.extend_from_slice(&((sequence << 8) | u64::from(kind)).to_le_bytes());
    result
}

pub fn batch(sequence: u64, changes: &[(&[u8], Option<&[u8]>)]) -> Vec<u8> {
    let mut result = sequence.to_le_bytes().to_vec();
    result.extend_from_slice(&(changes.len() as u32).to_le_bytes());
    for (key, value) in changes {
        result.push(u8::from(value.is_some()));
        slice(key, &mut result);
        if let Some(value) = value {
            slice(value, &mut result);
        }
    }
    result
}

pub fn physical_record(kind: u8, data: &[u8]) -> Vec<u8> {
    let length = u16::try_from(data.len()).unwrap();
    let mut checked = vec![kind];
    checked.extend_from_slice(data);
    let mut result = crc(&checked).to_le_bytes().to_vec();
    result.extend_from_slice(&length.to_le_bytes());
    result.extend_from_slice(&checked);
    result
}

pub fn append_log(output: &mut Vec<u8>, record: &[u8]) {
    let mut offset = 0;
    let mut first = true;
    loop {
        let leftover = BLOCK_SIZE - output.len() % BLOCK_SIZE;
        if leftover < HEADER_SIZE {
            output.resize(output.len() + leftover, 0);
        }
        let length =
            (record.len() - offset).min(BLOCK_SIZE - output.len() % BLOCK_SIZE - HEADER_SIZE);
        let last = offset + length == record.len();
        let kind = match (first, last) {
            (true, true) => 1,
            (true, false) => 2,
            (false, false) => 3,
            (false, true) => 4,
        };
        output.extend_from_slice(&physical_record(kind, &record[offset..offset + length]));
        offset += length;
        first = false;
        if last {
            break;
        }
    }
}

pub fn log_bytes(records: &[Vec<u8>]) -> Vec<u8> {
    let mut result = Vec::new();
    for record in records {
        append_log(&mut result, record);
    }
    result
}

pub fn fix_log_checksum(bytes: &mut [u8], offset: usize) {
    let length = u16::from_le_bytes(bytes[offset + 4..offset + 6].try_into().unwrap()) as usize;
    let checksum = crc(&bytes[offset + 6..offset + HEADER_SIZE + length]);
    bytes[offset..offset + 4].copy_from_slice(&checksum.to_le_bytes());
}

pub fn metadata(log_number: u64, sequence: u64) -> Vec<u8> {
    let mut result = vec![1];
    slice(b"leveldb.BytewiseComparator", &mut result);
    for (tag, value) in [(2, log_number), (3, 100), (4, sequence)] {
        result.push(tag);
        varint(value, &mut result);
    }
    result
}

pub fn write_manifest(directory: &Path, records: &[Vec<u8>]) -> Result<()> {
    fs::write(directory.join("CURRENT"), b"MANIFEST-000001\n")?;
    fs::write(directory.join("MANIFEST-000001"), log_bytes(records))?;
    Ok(())
}

pub fn add_table(
    edit: &mut Vec<u8>,
    level: u64,
    number: u64,
    size: u64,
    smallest: &[u8],
    largest: &[u8],
) {
    for value in [7, level, number, size] {
        varint(value, edit);
    }
    slice(smallest, edit);
    slice(largest, edit);
}

pub fn delete_table(edit: &mut Vec<u8>, level: u64, number: u64) {
    for value in [6, level, number] {
        varint(value, edit);
    }
}

// Test-only BlockBuilder/TableBuilder equivalents. They also allow constructing
// states that normal writers cannot emit, to exercise corruption validation.
pub fn block(entries: &[(Vec<u8>, Vec<u8>)], restart_interval: usize) -> Vec<u8> {
    assert!(restart_interval > 0);
    let mut output = Vec::new();
    let mut restarts = vec![0u32];
    let mut previous: &[u8] = b"";
    for (index, (key, value)) in entries.iter().enumerate() {
        let shared = if index % restart_interval == 0 {
            if index != 0 {
                restarts.push(output.len() as u32);
            }
            0
        } else {
            previous.iter().zip(key).take_while(|(a, b)| a == b).count()
        };
        for length in [shared, key.len() - shared, value.len()] {
            varint(length as u64, &mut output);
        }
        output.extend_from_slice(&key[shared..]);
        output.extend_from_slice(value);
        previous = key;
    }
    for offset in &restarts {
        output.extend_from_slice(&offset.to_le_bytes());
    }
    output.extend_from_slice(&(restarts.len() as u32).to_le_bytes());
    output
}

fn append_block(output: &mut Vec<u8>, data: &[u8], compression: u8) -> Vec<u8> {
    let mut encoded = match compression {
        0 => data.to_vec(),
        1 => snap::raw::Encoder::new().compress_vec(data).unwrap(),
        _ => panic!("unsupported fixture compression"),
    };
    let mut handle = Vec::new();
    varint(output.len() as u64, &mut handle);
    varint(encoded.len() as u64, &mut handle);
    encoded.push(compression);
    let checksum = crc(&encoded);
    output.extend_from_slice(&encoded);
    output.extend_from_slice(&checksum.to_le_bytes());
    handle
}

pub fn table(
    entries: &[(Vec<u8>, Vec<u8>)],
    restart_interval: usize,
    entries_per_block: usize,
    compression: u8,
) -> Vec<u8> {
    let mut output = Vec::new();
    let mut index = Vec::new();
    for chunk in entries.chunks(entries_per_block) {
        let handle = append_block(&mut output, &block(chunk, restart_interval), compression);
        index.push((chunk.last().unwrap().0.clone(), handle));
    }
    let metaindex = append_block(&mut output, &block(&[], 1), 0);
    let index = append_block(&mut output, &block(&index, 1), compression);
    let mut footer = metaindex;
    footer.extend_from_slice(&index);
    footer.resize(40, 0);
    footer.extend_from_slice(&0xdb47_7524_8b80_fb57_u64.to_le_bytes());
    output.extend_from_slice(&footer);
    output
}

pub fn save_table(
    directory: &Path,
    edit: &mut Vec<u8>,
    level: u64,
    number: u64,
    entries: &[(Vec<u8>, Vec<u8>)],
) -> Result<()> {
    let bytes = table(entries, 2, 2, 0);
    fs::write(directory.join(format!("{number:06}.ldb")), &bytes)?;
    add_table(
        edit,
        level,
        number,
        bytes.len() as u64,
        &entries.first().unwrap().0,
        &entries.last().unwrap().0,
    );
    Ok(())
}

// LevelDB util/random.h's deterministic Park-Miller generator. Fixed seeds make
// the upstream stress cases reproducible without adding a rand dependency.
pub struct Random(u32);

impl Random {
    pub fn new(seed: u32) -> Self {
        let seed = seed & 0x7fff_ffff;
        Self(if seed == 0 || seed == 0x7fff_ffff {
            1
        } else {
            seed
        })
    }

    pub fn uniform(&mut self, limit: u32) -> u32 {
        self.0 = (u64::from(self.0) * 16807 % 0x7fff_ffff) as u32;
        self.0 % limit
    }

    pub fn skewed(&mut self, max_log: u32) -> usize {
        let bits = self.uniform(max_log + 1);
        self.uniform(1 << bits) as usize
    }

    pub fn key(&mut self, length: usize) -> Vec<u8> {
        const CHARS: &[u8] = &[0, 1, b'a', b'b', b'c', b'd', b'e', 0xfd, 0xfe, 0xff];
        (0..length)
            .map(|_| CHARS[self.uniform(CHARS.len() as u32) as usize])
            .collect()
    }

    pub fn string(&mut self, length: usize) -> Vec<u8> {
        (0..length).map(|_| b' ' + self.uniform(95) as u8).collect()
    }
}

pub fn big_string(partial: &[u8], length: usize) -> Vec<u8> {
    partial.iter().copied().cycle().take(length).collect()
}
