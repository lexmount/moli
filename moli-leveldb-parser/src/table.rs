use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use zerocopy::{FromBytes, byteorder::little_endian::U32};

use super::{
    coding::{Decoder, masked_crc},
    layout::{BlockTrailer, TableFooter},
};

const FOOTER_SIZE: u64 = size_of::<TableFooter>() as u64;
const TRAILER_SIZE: u64 = size_of::<BlockTrailer>() as u64;
const TABLE_MAGIC: u64 = 0xdb47_7524_8b80_fb57;

pub(super) fn read_table(
    path: &Path,
    expected_size: u64,
    mut visit: impl FnMut(&[u8], &[u8]) -> Result<()>,
) -> Result<()> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    ensure!(
        size == expected_size && size >= FOOTER_SIZE,
        "invalid LevelDB table size"
    );
    file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
    let mut footer = [0; FOOTER_SIZE as usize];
    file.read_exact(&mut footer)?;
    let footer = Decoder::new(&footer).fixed::<TableFooter>()?;
    ensure!(
        footer.magic.get() == TABLE_MAGIC,
        "invalid LevelDB table magic"
    );
    let mut footer = Decoder::new(&footer.block_handles);
    // Full scans don't need the metaindex or Bloom filters.
    read_handle(&mut footer)?;
    let index_handle = read_handle(&mut footer)?;
    let index = read_block(&mut file, size - FOOTER_SIZE, index_handle)?;
    read_entries(&index, |_, handle| {
        let mut input = Decoder::new(handle);
        let handle = read_handle(&mut input)?;
        let block = read_block(&mut file, index_handle.0, handle)?;
        read_entries(&block, &mut visit)
    })
}

fn read_handle(input: &mut Decoder<'_>) -> Result<(u64, u64)> {
    Ok((input.varint()?, input.varint()?))
}

fn read_block(file: &mut File, end: u64, (offset, size): (u64, u64)) -> Result<Vec<u8>> {
    ensure!(
        offset <= end && end - offset >= TRAILER_SIZE && size <= end - offset - TRAILER_SIZE,
        "LevelDB block lies outside its table"
    );
    let size = usize::try_from(size).context("LevelDB block is too large")?;
    let length = size
        .checked_add(TRAILER_SIZE as usize)
        .context("LevelDB block length overflow")?;
    let mut block = Vec::new();
    block
        .try_reserve_exact(length)
        .context("LevelDB block is too large")?;
    block.resize(length, 0);
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut block)?;
    let trailer = Decoder::new(&block[size..]).fixed::<BlockTrailer>()?;
    let checksum_end = size + std::mem::offset_of!(BlockTrailer, checksum);
    ensure!(
        masked_crc(&block[..checksum_end]) == trailer.checksum.get(),
        "LevelDB table block checksum mismatch"
    );
    let compression = trailer.compression;
    block.truncate(size);
    match compression {
        0 => Ok(block),
        1 => {
            let length = snap::raw::decompress_len(&block)?;
            // A Snappy copy encodes at most 64 output bytes. Validate the
            // claimed size before letting the decoder allocate from its header.
            ensure!(
                length <= block.len().saturating_mul(64),
                "invalid Snappy decoded length"
            );
            let mut decoded = Vec::new();
            decoded
                .try_reserve_exact(length)
                .context("decoded LevelDB block is too large")?;
            decoded.resize(length, 0);
            let written = snap::raw::Decoder::new().decompress(&block, &mut decoded)?;
            ensure!(written == length, "invalid Snappy decoded length");
            Ok(decoded)
        }
        other => bail!("unsupported LevelDB compression type {other}"),
    }
}

/// Borrow the entries and restart offsets directly from a block, including at
/// unaligned byte offsets. Zerocopy checks the count and buffer size together.
pub(super) fn split_restarts(block: &[u8]) -> Result<(&[u8], &[U32])> {
    let (body, count) = U32::ref_from_suffix(block)
        .map_err(|_| anyhow!("truncated LevelDB block restart array"))?;
    let (entries, restarts) = <[U32]>::ref_from_suffix_with_elems(body, count.get() as usize)
        .map_err(|_| anyhow!("invalid LevelDB restart count"))?;
    // Java LevelDB can emit an empty block containing only a zero count.
    if restarts.is_empty() {
        ensure!(
            entries.is_empty(),
            "nonempty LevelDB block has no restart points"
        );
    }
    Ok((entries, restarts))
}

pub(super) fn read_entries(
    block: &[u8],
    mut visit: impl FnMut(&[u8], &[u8]) -> Result<()>,
) -> Result<()> {
    let (entries, restarts) = split_restarts(block)?;
    let entries_end = entries.len();
    let count = restarts.len();
    let mut previous = None;
    for restart in restarts {
        let offset = restart.get() as usize;
        ensure!(
            offset < entries_end || (entries_end == 0 && count == 1 && offset == 0),
            "LevelDB restart offset out of bounds"
        );
        if let Some(previous) = previous {
            ensure!(previous < offset, "unordered LevelDB restart offsets");
        } else {
            ensure!(offset == 0, "first LevelDB restart offset must be zero");
        }
        previous = Some(offset);
    }
    let mut next_restart = 0;
    let mut input = Decoder::new(entries);
    let mut key = Vec::new();
    while !input.bytes.is_empty() {
        let offset = entries_end - input.bytes.len();
        let shared = input.varint32()? as usize;
        let unshared = input.varint32()? as usize;
        let value_length = input.varint32()? as usize;
        ensure!(shared <= key.len(), "invalid LevelDB key prefix length");
        if let Some(restart) = restarts.get(next_restart) {
            let restart = restart.get() as usize;
            ensure!(restart >= offset, "LevelDB restart points inside an entry");
            if restart == offset {
                ensure!(shared == 0, "LevelDB restart entry has a shared prefix");
                next_restart += 1;
            }
        }
        let suffix = input.take(unshared)?;
        let value = input.take(value_length)?;
        key.truncate(shared);
        key.extend_from_slice(suffix);
        visit(&key, value)?;
    }
    ensure!(
        entries_end == 0 || next_restart == count,
        "LevelDB restart points inside an entry"
    );
    Ok(())
}
