use bytes::{Buf, BufMut};

use crate::{VarInt, VarIntUnexpectedEnd};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Frame(pub VarInt);

#[derive(Debug)]
pub(crate) enum FrameReadError {
    UnexpectedEnd,
    TooLarge(Frame),
}

impl From<VarIntUnexpectedEnd> for FrameReadError {
    fn from(_: VarIntUnexpectedEnd) -> Self {
        Self::UnexpectedEnd
    }
}

impl Frame {
    pub fn decode<B: Buf>(buf: &mut B) -> Result<Self, VarIntUnexpectedEnd> {
        let typ = VarInt::decode(buf)?;
        Ok(Frame(typ))
    }

    pub fn encode<B: BufMut>(&self, buf: &mut B) {
        self.0.encode(buf)
    }

    pub fn is_grease(&self) -> bool {
        let val = self.0.into_inner();
        if val < 0x21 {
            return false;
        }

        #[allow(unknown_lints, clippy::manual_is_multiple_of)]
        {
            (val - 0x21) % 0x1f == 0
        }
    }

    pub fn read<B: Buf>(
        buf: &mut B,
    ) -> Result<(Frame, bytes::buf::Take<&mut B>), VarIntUnexpectedEnd> {
        Self::read_bounded(buf, u64::MAX, u64::MAX).map_err(|_| VarIntUnexpectedEnd)
    }

    pub(crate) fn read_bounded<B: Buf>(
        buf: &mut B,
        headers_limit: u64,
        other_limit: u64,
    ) -> Result<(Frame, bytes::buf::Take<&mut B>), FrameReadError> {
        loop {
            let typ = Frame::decode(buf)?;
            let size = VarInt::decode(buf)?.into_inner();
            let limit = if typ == Frame::HEADERS {
                headers_limit
            } else {
                other_limit
            };
            if size > limit {
                return Err(FrameReadError::TooLarge(typ));
            }
            // A wire length cannot be narrowed until it fits the host address space.
            let size = usize::try_from(size).map_err(|_| FrameReadError::UnexpectedEnd)?;
            if buf.remaining() < size {
                return Err(FrameReadError::UnexpectedEnd);
            }
            if typ.is_grease() {
                buf.advance(size);
                continue;
            }
            return Ok((typ, buf.take(size)));
        }
    }
}

macro_rules! frames {
    {$($name:ident = $val:expr,)*} => {
        impl Frame {
            $(pub const $name: Frame = Frame(VarInt::from_u32($val));)*
        }
    }
}

/// Maximum size of a frame payload (64 KiB).
///
/// This bounds the allocation when reading frames from an async stream,
/// preventing a malicious peer from triggering huge allocations via a
/// crafted frame-length field.
pub(crate) const MAX_FRAME_SIZE: u64 = 65536;

// Sent at the start of a bidirectional stream.
frames! {
    DATA = 0x00,
    HEADERS = 0x01,
    SETTINGS = 0x04,
    WEBTRANSPORT = 0x41,
}
