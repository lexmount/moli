//! Fixed on-disk layouts shared by the log, batch and table readers.
//!
//! Little-endian numeric wrappers have alignment 1, so these views work at
//! arbitrary byte offsets without packed fields or unsafe code. All bit
//! patterns are representable; readers validate tags, lengths and checksums.

use zerocopy::{
    FromBytes, Immutable, KnownLayout, Unaligned,
    byteorder::little_endian::{U16, U32, U64},
};

/// Physical record header used by both WAL and MANIFEST files.
#[derive(FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
pub(super) struct LogHeader {
    pub checksum: U32,
    pub length: U16,
    pub record_type: u8,
}

#[derive(FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
pub(super) struct WriteBatchHeader {
    pub sequence: U64,
    pub count: U32,
}

#[derive(FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
pub(super) struct BlockTrailer {
    pub compression: u8,
    pub checksum: U32,
}

#[derive(FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
pub(super) struct TableFooter {
    /// Two varint-encoded block handles followed by padding to 40 bytes.
    pub block_handles: [u8; 40],
    pub magic: U64,
}

// Keep the structs tied to the file format even if fields are edited later.
const _: () = {
    assert!(size_of::<LogHeader>() == 7);
    assert!(size_of::<WriteBatchHeader>() == 12);
    assert!(size_of::<BlockTrailer>() == 5);
    assert!(size_of::<TableFooter>() == 48);
};
