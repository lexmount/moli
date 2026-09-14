use anyhow::{Result, anyhow, bail, ensure};
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

pub(super) struct Decoder<'a> {
    pub bytes: &'a [u8],
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    pub fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        ensure!(length <= self.bytes.len(), "truncated LevelDB field");
        let (value, rest) = self.bytes.split_at(length);
        self.bytes = rest;
        Ok(value)
    }

    pub fn fixed<T: FromBytes + KnownLayout + Immutable + Unaligned>(&mut self) -> Result<&'a T> {
        let (value, rest) =
            T::ref_from_prefix(self.bytes).map_err(|_| anyhow!("truncated LevelDB field"))?;
        self.bytes = rest;
        Ok(value)
    }

    pub fn varint(&mut self) -> Result<u64> {
        let mut value = 0;
        for shift in (0..70).step_by(7) {
            let byte = self.take(1)?[0];
            ensure!(shift != 63 || byte <= 1, "LevelDB varint overflow");
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Ok(value);
            }
        }
        bail!("invalid LevelDB varint")
    }

    pub fn varint32(&mut self) -> Result<u32> {
        let mut value = 0;
        for shift in (0..35).step_by(7) {
            let byte = self.take(1)?[0];
            ensure!(shift != 28 || byte <= 0x0f, "LevelDB varint32 overflow");
            value |= u32::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Ok(value);
            }
        }
        bail!("invalid LevelDB varint32")
    }

    pub fn slice(&mut self) -> Result<&'a [u8]> {
        let length = self.varint32()? as usize;
        self.take(length)
    }
}

pub(super) fn masked_crc(bytes: &[u8]) -> u32 {
    crc32c::crc32c(bytes)
        .rotate_right(15)
        .wrapping_add(0xa282_ead8)
}
