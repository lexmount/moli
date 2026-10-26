// Based on Quinn: https://github.com/quinn-rs/quinn/tree/main/quinn-proto/src
// Licensed under Apache-2.0 OR MIT

use std::{convert::TryInto, fmt, io::Cursor};

use bytes::{Buf, BufMut};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// An integer less than 2^62
///
/// Values of this type are suitable for encoding as QUIC variable-length integer.
// It would be neat if we could express to Rust that the top two bits are available for use as enum
// discriminants
#[derive(Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VarInt(pub(crate) u64);

impl VarInt {
    /// The largest representable value
    pub const MAX: Self = Self((1 << 62) - 1);
    /// The largest encoded value length
    pub const MAX_SIZE: usize = 8;

    /// Construct a `VarInt` infallibly
    pub const fn from_u32(x: u32) -> Self {
        Self(x as u64)
    }

    /// Succeeds if `x` < 2^62
    pub fn from_u64(x: u64) -> Result<Self, VarIntBoundsExceeded> {
        if x < 2u64.pow(62) {
            Ok(Self(x))
        } else {
            Err(VarIntBoundsExceeded)
        }
    }

    /// Create a VarInt without ensuring it's in range
    ///
    /// # Safety
    ///
    /// `x` must be less than 2^62.
    pub const unsafe fn from_u64_unchecked(x: u64) -> Self {
        Self(x)
    }

    /// Extract the integer value
    pub const fn into_inner(self) -> u64 {
        self.0
    }

    /// Compute the number of bytes needed to encode this value
    pub fn size(self) -> usize {
        let x = self.0;
        if x < 2u64.pow(6) {
            1
        } else if x < 2u64.pow(14) {
            2
        } else if x < 2u64.pow(30) {
            4
        } else if x < 2u64.pow(62) {
            8
        } else {
            unreachable!("malformed VarInt");
        }
    }
}

impl From<VarInt> for u64 {
    fn from(x: VarInt) -> Self {
        x.0
    }
}

impl From<u8> for VarInt {
    fn from(x: u8) -> Self {
        Self(x.into())
    }
}

impl From<u16> for VarInt {
    fn from(x: u16) -> Self {
        Self(x.into())
    }
}

impl From<u32> for VarInt {
    fn from(x: u32) -> Self {
        Self(x.into())
    }
}

impl std::convert::TryFrom<u64> for VarInt {
    type Error = VarIntBoundsExceeded;
    /// Succeeds iff `x` < 2^62
    fn try_from(x: u64) -> Result<Self, VarIntBoundsExceeded> {
        Self::from_u64(x)
    }
}

impl std::convert::TryFrom<u128> for VarInt {
    type Error = VarIntBoundsExceeded;
    /// Succeeds iff `x` < 2^62
    fn try_from(x: u128) -> Result<Self, VarIntBoundsExceeded> {
        Self::from_u64(x.try_into().map_err(|_| VarIntBoundsExceeded)?)
    }
}

impl std::convert::TryFrom<usize> for VarInt {
    type Error = VarIntBoundsExceeded;
    /// Succeeds iff `x` < 2^62
    fn try_from(x: usize) -> Result<Self, VarIntBoundsExceeded> {
        Self::try_from(x as u64)
    }
}

impl fmt::Debug for VarInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for VarInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl VarInt {
    pub fn decode<B: Buf>(r: &mut B) -> Result<Self, VarIntUnexpectedEnd> {
        if !r.has_remaining() {
            return Err(VarIntUnexpectedEnd);
        }
        let mut buf = [0; 8];
        buf[0] = r.get_u8();
        let tag = buf[0] >> 6;
        buf[0] &= 0b0011_1111;
        let x = match tag {
            0b00 => u64::from(buf[0]),
            0b01 => {
                if r.remaining() < 1 {
                    return Err(VarIntUnexpectedEnd);
                }
                r.copy_to_slice(&mut buf[1..2]);
                u64::from(u16::from_be_bytes(buf[..2].try_into().unwrap()))
            }
            0b10 => {
                if r.remaining() < 3 {
                    return Err(VarIntUnexpectedEnd);
                }
                r.copy_to_slice(&mut buf[1..4]);
                u64::from(u32::from_be_bytes(buf[..4].try_into().unwrap()))
            }
            0b11 => {
                if r.remaining() < 7 {
                    return Err(VarIntUnexpectedEnd);
                }
                r.copy_to_slice(&mut buf[1..8]);
                u64::from_be_bytes(buf)
            }
            _ => unreachable!(),
        };
        Ok(Self(x))
    }

    /// Read a varint from the stream.
    pub async fn read<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Self, VarIntUnexpectedEnd> {
        Self::read_optional(stream)
            .await?
            .ok_or(VarIntUnexpectedEnd)
    }

    /// Read a varint from the stream, returning `Ok(None)` on clean EOF.
    ///
    /// Returns `Ok(None)` if EOF is encountered before any byte is read,
    /// `Err(VarIntUnexpectedEnd)` if EOF occurs mid-varint, and
    /// `Ok(Some(varint))` on success.
    pub async fn read_optional<S: AsyncRead + Unpin>(
        stream: &mut S,
    ) -> Result<Option<Self>, VarIntUnexpectedEnd> {
        // 8 bytes is the max size of a varint
        let mut buf = [0; 8];

        // Read the first byte because it includes the length.
        // EOF here means clean end-of-stream.
        match stream.read_exact(&mut buf[0..1]).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(_) => return Err(VarIntUnexpectedEnd),
        }

        // 0b00 = 1, 0b01 = 2, 0b10 = 4, 0b11 = 8
        let size = 1 << (buf[0] >> 6);
        stream
            .read_exact(&mut buf[1..size])
            .await
            .map_err(|_| VarIntUnexpectedEnd)?;

        // Use a cursor to read the varint on the stack.
        let mut cursor = Cursor::new(&buf[..size]);
        let v = VarInt::decode(&mut cursor).unwrap();

        Ok(Some(v))
    }

    pub fn encode<B: BufMut>(&self, w: &mut B) {
        let x = self.0;
        if x < 2u64.pow(6) {
            w.put_u8(x as u8);
        } else if x < 2u64.pow(14) {
            w.put_u16((0b01 << 14) | x as u16);
        } else if x < 2u64.pow(30) {
            w.put_u32((0b10 << 30) | x as u32);
        } else if x < 2u64.pow(62) {
            w.put_u64((0b11 << 62) | x);
        } else {
            unreachable!("malformed VarInt")
        }
    }

    pub async fn write<S: AsyncWrite + Unpin>(
        &self,
        stream: &mut S,
    ) -> Result<(), VarIntUnexpectedEnd> {
        // Super jaink but keeps everything on the stack.
        let mut buf = [0u8; 8];
        let mut cursor: &mut [u8] = &mut buf;
        self.encode(&mut cursor);
        let size = 8 - cursor.len();

        let mut cursor = &buf[..size];
        stream
            .write_all_buf(&mut cursor)
            .await
            .map_err(|_| VarIntUnexpectedEnd)?;

        Ok(())
    }
}

/// Error returned when constructing a `VarInt` from a value >= 2^62
#[derive(Debug, Copy, Clone, Eq, PartialEq, Error)]
#[error("value too large for varint encoding")]
pub struct VarIntBoundsExceeded;

#[derive(Error, Debug, Copy, Clone, Eq, PartialEq)]
#[error("unexpected end of buffer")]
pub struct VarIntUnexpectedEnd;
