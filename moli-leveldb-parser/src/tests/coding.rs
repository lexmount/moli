// Copyright (c) 2011 The LevelDB Authors. All rights reserved.
// Ports of util/coding_test.cc and util/crc32c_test.cc; see ../../TESTS.md,
// ../../LICENSE-LevelDB and ../../AUTHORS-LevelDB.

use zerocopy::byteorder::little_endian::{U32, U64};

use super::support::{slice, varint};
use crate::coding::{Decoder, masked_crc};

#[test]
fn fixed32() {
    let bytes: Vec<_> = (0..100_000u32).flat_map(u32::to_le_bytes).collect();
    let mut input = Decoder::new(&bytes);
    for expected in 0..100_000 {
        assert_eq!(input.fixed::<U32>().unwrap().get(), expected);
    }
    assert!(input.bytes.is_empty());
}

#[test]
fn fixed64() {
    let values: Vec<_> = (0..64)
        .flat_map(|power| {
            let v = 1u64 << power;
            [v - 1, v, v + 1]
        })
        .collect();
    let bytes: Vec<_> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut input = Decoder::new(&bytes);
    for expected in values {
        assert_eq!(input.fixed::<U64>().unwrap().get(), expected);
    }
    assert!(input.bytes.is_empty());
}

#[test]
fn encoding_output() {
    assert_eq!(
        Decoder::new(&[1, 2, 3, 4]).fixed::<U32>().unwrap().get(),
        0x0403_0201
    );
    assert_eq!(
        Decoder::new(&[1, 2, 3, 4, 5, 6, 7, 8])
            .fixed::<U64>()
            .unwrap()
            .get(),
        0x0807_0605_0403_0201
    );
}

#[test]
fn varint32() {
    let values: Vec<_> = (0..32 * 32u32).map(|i| (i / 32) << (i % 32)).collect();
    let mut bytes = Vec::new();
    for value in &values {
        varint(u64::from(*value), &mut bytes);
    }
    let mut input = Decoder::new(&bytes);
    for value in values {
        let before = input.bytes.len();
        assert_eq!(input.varint32().unwrap(), value);
        assert_eq!(
            before - input.bytes.len(),
            ((32 - value.leading_zeros()).max(1).div_ceil(7)) as usize
        );
    }
    assert!(input.bytes.is_empty());
}

#[test]
fn varint64() {
    let mut values = vec![0, 100, u64::MAX, u64::MAX - 1];
    values.extend((0..64).flat_map(|k| {
        let v = 1u64 << k;
        [v, v - 1, v + 1]
    }));
    let mut bytes = Vec::new();
    for value in &values {
        varint(*value, &mut bytes);
    }
    let mut input = Decoder::new(&bytes);
    for value in values {
        let before = input.bytes.len();
        assert_eq!(input.varint().unwrap(), value);
        assert_eq!(
            before - input.bytes.len(),
            ((64 - value.leading_zeros()).max(1).div_ceil(7)) as usize
        );
    }
    assert!(input.bytes.is_empty());
}

#[test]
fn varint32_overflow() {
    assert!(
        Decoder::new(&[0x81, 0x82, 0x83, 0x84, 0x85, 0x11])
            .varint32()
            .is_err()
    );
}

#[test]
fn varint32_truncation() {
    let value = (1u32 << 31) + 100;
    let mut bytes = Vec::new();
    varint(u64::from(value), &mut bytes);
    for end in 0..bytes.len() {
        assert!(Decoder::new(&bytes[..end]).varint32().is_err());
    }
    assert_eq!(Decoder::new(&bytes).varint32().unwrap(), value);
}

#[test]
fn varint64_overflow() {
    assert!(
        Decoder::new(&[
            0x81, 0x82, 0x83, 0x84, 0x85, 0x81, 0x82, 0x83, 0x84, 0x85, 0x11
        ])
        .varint()
        .is_err()
    );
}

#[test]
fn varint64_truncation() {
    let value = (1u64 << 63) + 100;
    let mut bytes = Vec::new();
    varint(value, &mut bytes);
    for end in 0..bytes.len() {
        assert!(Decoder::new(&bytes[..end]).varint().is_err());
    }
    assert_eq!(Decoder::new(&bytes).varint().unwrap(), value);
}

#[test]
fn strings() {
    let values = [vec![], b"foo".to_vec(), b"bar".to_vec(), vec![b'x'; 200]];
    let mut bytes = Vec::new();
    for value in &values {
        slice(value, &mut bytes);
    }
    let mut input = Decoder::new(&bytes);
    for value in values {
        assert_eq!(input.slice().unwrap(), value);
    }
    assert!(input.bytes.is_empty());
}

#[test]
fn crc_standard_results() {
    let vectors = [
        (vec![0; 32], 0x8a91_36aa_u32),
        (vec![0xff; 32], 0x62a8_ab43),
        ((0..32).collect(), 0x46dd_794e),
        ((0..32).rev().collect(), 0x113f_db5c),
        (
            vec![
                0x01, 0xc0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x14, 0, 0, 0, 0, 0, 0x04, 0,
                0, 0, 0, 0x14, 0, 0, 0, 0x18, 0x28, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0, 0, 0, 0, 0,
            ],
            0xd996_3a56,
        ),
    ];
    for (bytes, expected) in vectors {
        // Check the actual parser wrapper, including its masking step.
        assert_eq!(unmask(masked_crc(&bytes)), expected);
    }
}

fn unmask(value: u32) -> u32 {
    value.wrapping_sub(0xa282_ead8).rotate_left(15)
}

#[test]
fn crc_values() {
    assert_ne!(masked_crc(b"a"), masked_crc(b"foo"));
}

#[test]
fn crc_mask() {
    let crc = crc32c::crc32c(b"foo");
    assert_ne!(crc, masked_crc(b"foo"));
    assert_eq!(crc, unmask(masked_crc(b"foo")));
}

#[test]
fn fixed_fields_cannot_read_past_end() {
    for length in 0..8 {
        assert!(Decoder::new(&[0; 8][..length]).fixed::<U64>().is_err());
    }
}

#[test]
fn length_prefixed_fields_are_bounded() {
    for bytes in [
        vec![0xff, 0xff, 0xff, 0xff, 0x0f],
        vec![3, b'a', b'b'],
        vec![0x80],
    ] {
        assert!(Decoder::new(&bytes).slice().is_err());
    }
}

#[test]
fn varint32_overlong_zero_is_rejected() {
    assert!(
        Decoder::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0])
            .varint32()
            .is_err()
    );
}

#[test]
fn varint_high_bits_cannot_wrap() {
    assert!(
        Decoder::new(&[0xff, 0xff, 0xff, 0xff, 0x10])
            .varint32()
            .is_err()
    );
    assert!(
        Decoder::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2])
            .varint()
            .is_err()
    );
}
