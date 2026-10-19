//! ICO directory inspection for the bounded, color-managed PNG codec path.
//! BMP icons use the image decoder's palette and AND-mask handling.

use crate::{PngDecodeError, RasterColorPrecision, png::PngOrientation};

#[derive(Debug, thiserror::Error)]
pub enum IcoDecodeError {
    #[error("invalid ICO directory")]
    InvalidDirectory,
    #[error("ICO image entry is outside the encoded buffer")]
    InvalidEntry,
    #[error("ICO directory dimensions do not match its PNG image")]
    DimensionMismatch,
    #[error(transparent)]
    Png(#[from] PngDecodeError),
}

pub(super) struct PngIcon<'a> {
    bytes: &'a [u8],
    width: u32,
    height: u32,
}

impl PngIcon<'_> {
    pub(super) fn metadata(&self) -> Result<(u32, u32, RasterColorPrecision), IcoDecodeError> {
        // Exif belongs to a standalone PNG. ICO directory frames retain their
        // encoded orientation, including when the selected frame is a PNG.
        let metadata =
            crate::png::png_metadata_with_orientation(self.bytes, PngOrientation::Ignore)?;
        // A zero directory byte denotes 256, including larger embedded PNGs.
        if metadata.0.min(256) != self.width || metadata.1.min(256) != self.height {
            return Err(IcoDecodeError::DimensionMismatch);
        }
        Ok(metadata)
    }

    pub(super) fn decode(
        &self,
        convert_profile: bool,
    ) -> Result<crate::color::DecodedColorPixels, IcoDecodeError> {
        self.metadata()?;
        Ok(crate::png::decode_png_colors_with_orientation(
            self.bytes,
            convert_profile,
            PngOrientation::Ignore,
        )?)
    }
}

fn u16_le(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes[..2].try_into().expect("ICO directory field"))
}

fn u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().expect("ICO directory field"))
}

fn dimension(value: u8) -> u32 {
    if value == 0 { 256 } else { u32::from(value) }
}

fn entry_score(entry: &[u8]) -> (u16, u32) {
    // Match the underlying ICO decoder's frame choice, including its last-entry
    // tie break, so metadata and pixel decoding always refer to the same frame.
    (
        u16_le(&entry[6..]),
        dimension(entry[0]) * dimension(entry[1]),
    )
}

pub(super) fn png_icon(bytes: &[u8]) -> Result<Option<PngIcon<'_>>, IcoDecodeError> {
    let header = bytes.get(..6).ok_or(IcoDecodeError::InvalidDirectory)?;
    if header[..4] != [0, 0, 1, 0] {
        return Err(IcoDecodeError::InvalidDirectory);
    }
    let count = usize::from(u16_le(&header[4..]));
    if count == 0 {
        return Err(IcoDecodeError::InvalidDirectory);
    }
    let directory_end = 6 + count * 16;
    let directory = bytes
        .get(6..directory_end)
        .ok_or(IcoDecodeError::InvalidDirectory)?;
    let mut selected: Option<&[u8]> = None;
    for entry in directory.chunks_exact(16) {
        if u16_le(&entry[4..]) > 256 || u16_le(&entry[6..]) > 256 {
            return Err(IcoDecodeError::InvalidDirectory);
        }
        if selected.is_none_or(|previous| entry_score(entry) >= entry_score(previous)) {
            selected = Some(entry);
        }
    }
    let selected = selected.expect("non-empty ICO directory");
    let offset =
        usize::try_from(u32_le(&selected[12..])).map_err(|_| IcoDecodeError::InvalidEntry)?;
    let length =
        usize::try_from(u32_le(&selected[8..])).map_err(|_| IcoDecodeError::InvalidEntry)?;
    let end = offset
        .checked_add(length)
        .ok_or(IcoDecodeError::InvalidEntry)?;
    if offset < directory_end {
        return Err(IcoDecodeError::InvalidEntry);
    }
    let payload = bytes.get(offset..end).ok_or(IcoDecodeError::InvalidEntry)?;
    if !payload.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(None);
    }
    Ok(Some(PngIcon {
        bytes: payload,
        width: dimension(selected[0]),
        height: dimension(selected[1]),
    }))
}

#[cfg(test)]
mod tests;
