use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

use crate::{
    coding::Decoder,
    layout::{BlockTrailer, LogHeader, TableFooter, WriteBatchHeader},
    table::{read_entries, split_restarts},
};

fn at_each_alignment(bytes: &[u8], mut visit: impl FnMut(&[u8])) {
    let mut storage = vec![0xff; bytes.len() + 8];
    for offset in 0..8 {
        let input = &mut storage[offset..offset + bytes.len()];
        input.copy_from_slice(bytes);
        visit(input);
    }
}

fn borrowed<T: FromBytes + KnownLayout + Immutable + Unaligned>(input: &[u8]) -> &T {
    let mut decoder = Decoder::new(input);
    let value = decoder.fixed::<T>().unwrap();
    assert_eq!(std::ptr::from_ref(value).cast::<u8>(), input.as_ptr());
    // The header stays borrowed while the cursor advances over its payload.
    let payload = decoder.take(input.len() - size_of::<T>()).unwrap();
    assert_eq!(payload.as_ptr(), input[size_of::<T>()..].as_ptr());
    assert!(decoder.bytes.is_empty());
    value
}

#[test]
fn headers_borrow_unaligned_little_endian_fields() {
    // Literal format bytes, independent of the fixture writers and derives.
    at_each_alignment(&[1, 2, 3, 4, 5, 6, 0xfe, 0xaa], |input| {
        let header = borrowed::<LogHeader>(input);
        assert_eq!(header.checksum.get(), 0x0403_0201);
        assert_eq!(header.length.get(), 0x0605);
        // Unknown types remain representable for the reader to validate.
        assert_eq!(header.record_type, 0xfe);
    });
    at_each_alignment(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0xaa], |input| {
        let header = borrowed::<WriteBatchHeader>(input);
        assert_eq!(header.sequence.get(), 0x0807_0605_0403_0201);
        assert_eq!(header.count.get(), 0x0c0b_0a09);
    });
    at_each_alignment(&[0xfe, 1, 2, 3, 4], |input| {
        let trailer = borrowed::<BlockTrailer>(input);
        assert_eq!(trailer.compression, 0xfe);
        assert_eq!(trailer.checksum.get(), 0x0403_0201);
    });
    let mut footer = [0x10; 48];
    footer[40..].copy_from_slice(&[0x57, 0xfb, 0x80, 0x8b, 0x24, 0x75, 0x47, 0xdb]);
    at_each_alignment(&footer, |input| {
        let footer = borrowed::<TableFooter>(input);
        assert_eq!(footer.block_handles, [0x10; 40]);
        assert_eq!(footer.magic.get(), 0xdb47_7524_8b80_fb57);
    });
}

fn check_truncations<T: FromBytes + KnownLayout + Immutable + Unaligned>() {
    for length in 0..size_of::<T>() {
        let bytes = vec![0; length];
        let mut decoder = Decoder::new(&bytes);
        assert!(decoder.fixed::<T>().is_err());
        assert_eq!(decoder.bytes, bytes);
        assert_eq!(decoder.bytes.as_ptr(), bytes.as_ptr());
    }
}

#[test]
fn truncated_headers_do_not_consume_input() {
    check_truncations::<LogHeader>();
    check_truncations::<WriteBatchHeader>();
    check_truncations::<BlockTrailer>();
    check_truncations::<TableFooter>();
}

#[test]
fn restart_offsets_borrow_the_original_block_at_any_alignment() {
    // Three body bytes, two little-endian restart offsets, and their count.
    let bytes = [0xa1, 0xa2, 0xa3, 0, 0, 0, 0, 1, 2, 3, 4, 2, 0, 0, 0];
    at_each_alignment(&bytes, |input| {
        let (body, restarts) = split_restarts(input).unwrap();
        assert_eq!(body, [0xa1, 0xa2, 0xa3]);
        assert_eq!(body.as_ptr(), input.as_ptr());
        assert_eq!(restarts.len(), 2);
        assert_eq!(restarts.as_ptr().cast::<u8>(), input[3..].as_ptr());
        assert_eq!(restarts[0].get(), 0);
        assert_eq!(restarts[1].get(), 0x0403_0201);
    });
}

#[test]
fn nonempty_block_cannot_hide_entries_with_zero_restarts() {
    for body in [b"x".as_slice(), &[0; 4], &[0, 1, 1, b'k', b'v']] {
        let mut bytes = body.to_vec();
        bytes.extend_from_slice(&[0; 4]);
        assert!(read_entries(&bytes, |_, _| panic!("invalid block must be rejected")).is_err());
    }
}
