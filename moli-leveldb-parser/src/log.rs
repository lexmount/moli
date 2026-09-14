use std::{fs::File, io::Read, path::Path};

use anyhow::{Context, Result, bail, ensure};

use super::{
    coding::{Decoder, masked_crc},
    layout::LogHeader,
};

const BLOCK_SIZE: usize = 32 * 1024;
const HEADER_SIZE: usize = size_of::<LogHeader>();

/// Read complete logical records, including records spanning physical blocks.
/// Like LevelDB recovery, ignore an unfinished append at EOF. A complete record
/// with a bad checksum is an error, rather than a silently incomplete import.
pub(super) fn read_records(path: &Path, visit: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
    read_records_from(File::open(path)?, visit)
}

pub(super) fn read_records_from(
    mut file: impl Read,
    mut visit: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    let mut block = Vec::with_capacity(BLOCK_SIZE);
    let mut fragment = Vec::new();
    let mut fragmented = false;
    loop {
        block.clear();
        file.by_ref()
            .take(BLOCK_SIZE as u64)
            .read_to_end(&mut block)?;
        if block.is_empty() {
            return Ok(());
        }
        let mut offset = 0;
        while offset < block.len() {
            let remaining = &block[offset..];
            if remaining.len() < HEADER_SIZE {
                ensure!(
                    block.len() < BLOCK_SIZE || remaining.iter().all(|byte| *byte == 0),
                    "nonzero LevelDB log block trailer"
                );
                break;
            }
            let header = Decoder::new(remaining).fixed::<LogHeader>()?;
            let length = usize::from(header.length.get());
            let kind = header.record_type;
            if kind == 0 && length == 0 {
                // Writers may preallocate zero-filled blocks.
                ensure!(
                    remaining.iter().all(|byte| *byte == 0),
                    "invalid LevelDB log padding"
                );
                fragment.clear();
                fragmented = false;
                break;
            }
            if HEADER_SIZE + length > remaining.len() {
                ensure!(
                    block.len() < BLOCK_SIZE,
                    "LevelDB log record crosses a block boundary"
                );
                return Ok(());
            }
            let payload = &remaining[HEADER_SIZE..HEADER_SIZE + length];
            ensure!(
                // The CRC covers the record type and payload, excluding length.
                masked_crc(
                    &remaining[std::mem::offset_of!(LogHeader, record_type)..HEADER_SIZE + length]
                ) == header.checksum.get(),
                "LevelDB log checksum mismatch"
            );
            match kind {
                1 => {
                    // Older writers could leave an empty FIRST at a block's
                    // end, then start a new record in the next block. Match
                    // LevelDB's compatibility exception only for empty data.
                    ensure!(
                        fragment.is_empty(),
                        "unfinished LevelDB log record before FULL"
                    );
                    fragmented = false;
                    visit(payload)?;
                }
                2 => {
                    ensure!(
                        fragment.is_empty(),
                        "unfinished LevelDB log record before FIRST"
                    );
                    fragment.clear();
                    fragment
                        .try_reserve(payload.len())
                        .context("LevelDB log record is too large")?;
                    fragment.extend_from_slice(payload);
                    fragmented = true;
                }
                3 | 4 => {
                    ensure!(fragmented, "LevelDB log fragment without FIRST");
                    fragment
                        .try_reserve(payload.len())
                        .context("LevelDB log record is too large")?;
                    fragment.extend_from_slice(payload);
                    if kind == 4 {
                        visit(&fragment)?;
                        fragment.clear();
                        fragmented = false;
                    }
                }
                _ => bail!("unsupported LevelDB log record type {kind}"),
            }
            offset += HEADER_SIZE + length;
        }
    }
}
