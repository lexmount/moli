use std::{
    collections::HashMap,
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use bytes::{Buf, BufMut, BytesMut};

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{Frame, StreamUni, VarInt, VarIntUnexpectedEnd, MAX_FRAME_SIZE};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Setting(pub VarInt);

impl Setting {
    pub fn decode<B: Buf>(buf: &mut B) -> Result<Self, VarIntUnexpectedEnd> {
        Ok(Setting(VarInt::decode(buf)?))
    }

    pub fn encode<B: BufMut>(&self, buf: &mut B) {
        self.0.encode(buf)
    }

    // Reference : https://datatracker.ietf.org/doc/html/rfc9114#section-7.2.4.1
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
}

impl Debug for Setting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Setting::QPACK_MAX_TABLE_CAPACITY => write!(f, "QPACK_MAX_TABLE_CAPACITY"),
            Setting::MAX_FIELD_SECTION_SIZE => write!(f, "MAX_FIELD_SECTION_SIZE"),
            Setting::QPACK_BLOCKED_STREAMS => write!(f, "QPACK_BLOCKED_STREAMS"),
            Setting::ENABLE_CONNECT_PROTOCOL => write!(f, "ENABLE_CONNECT_PROTOCOL"),
            Setting::ENABLE_DATAGRAM => write!(f, "ENABLE_DATAGRAM"),
            Setting::ENABLE_DATAGRAM_DEPRECATED => write!(f, "ENABLE_DATAGRAM_DEPRECATED"),
            Setting::WEBTRANSPORT_ENABLE_DEPRECATED => write!(f, "WEBTRANSPORT_ENABLE_DEPRECATED"),
            Setting::WEBTRANSPORT_MAX_SESSIONS_DEPRECATED => {
                write!(f, "WEBTRANSPORT_MAX_SESSIONS_DEPRECATED")
            }
            Setting::WEBTRANSPORT_MAX_SESSIONS => write!(f, "WEBTRANSPORT_MAX_SESSIONS"),
            x if x.is_grease() => write!(f, "GREASE SETTING [{:x?}]", x.0.into_inner()),
            x => write!(f, "UNKNOWN_SETTING [{:x?}]", x.0.into_inner()),
        }
    }
}

macro_rules! settings {
    {$($name:ident = $val:expr,)*} => {
        impl Setting {
            $(pub const $name: Setting = Setting(VarInt::from_u32($val));)*
        }
    }
}

settings! {
    // These are for HTTP/3 and we can ignore them
    QPACK_MAX_TABLE_CAPACITY = 0x1, // default is 0, which disables QPACK dynamic table
    MAX_FIELD_SECTION_SIZE = 0x6,
    QPACK_BLOCKED_STREAMS = 0x7,

    // Both of these are required for WebTransport
    ENABLE_CONNECT_PROTOCOL = 0x8,
    ENABLE_DATAGRAM = 0x33,
    ENABLE_DATAGRAM_DEPRECATED = 0xFFD277, // still used by Chrome

    // Removed in draft 06
    WEBTRANSPORT_ENABLE_DEPRECATED = 0x2b603742,
    WEBTRANSPORT_MAX_SESSIONS_DEPRECATED = 0x2b603743,

    // New way to enable WebTransport
    WEBTRANSPORT_MAX_SESSIONS = 0xc671706a,
}

#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum SettingsError {
    #[error("unexpected end of input")]
    UnexpectedEnd,

    #[error("unexpected stream type {0:?}")]
    UnexpectedStreamType(StreamUni),

    #[error("unexpected frame {0:?}")]
    UnexpectedFrame(Frame),

    #[error("invalid size")]
    InvalidSize,

    #[error("frame too large")]
    FrameTooLarge,

    #[error("io error: {0}")]
    Io(Arc<std::io::Error>),
}

impl From<std::io::Error> for SettingsError {
    fn from(err: std::io::Error) -> Self {
        SettingsError::Io(Arc::new(err))
    }
}

// A map of settings to values.
#[derive(Default, Debug)]
pub struct Settings(HashMap<Setting, VarInt>);

impl Settings {
    pub fn decode<B: Buf>(buf: &mut B) -> Result<Self, SettingsError> {
        let typ = StreamUni::decode(buf).map_err(|_| SettingsError::UnexpectedEnd)?;
        if typ != StreamUni::CONTROL {
            return Err(SettingsError::UnexpectedStreamType(typ));
        }

        let (typ, mut data) =
            Frame::read_bounded(buf, MAX_FRAME_SIZE, MAX_FRAME_SIZE).map_err(|err| match err {
                crate::frame::FrameReadError::UnexpectedEnd => SettingsError::UnexpectedEnd,
                crate::frame::FrameReadError::TooLarge(_) => SettingsError::FrameTooLarge,
            })?;
        if typ != Frame::SETTINGS {
            return Err(SettingsError::UnexpectedFrame(typ));
        }

        let mut settings = Settings::default();
        while data.has_remaining() {
            // These return a different error because retrying won't help.
            let id = Setting::decode(&mut data).map_err(|_| SettingsError::InvalidSize)?;
            let value = VarInt::decode(&mut data).map_err(|_| SettingsError::InvalidSize)?;
            // Only add if it is not grease
            if !id.is_grease() {
                settings.0.insert(id, value);
            }
        }

        Ok(settings)
    }

    /// Read settings from a stream, consuming only the exact bytes of the stream type + frame.
    pub async fn read<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Self, SettingsError> {
        let typ = StreamUni(
            VarInt::read(stream)
                .await
                .map_err(|_| SettingsError::UnexpectedEnd)?,
        );
        if typ != StreamUni::CONTROL {
            return Err(SettingsError::UnexpectedStreamType(typ));
        }

        loop {
            let frame_typ = Frame(
                VarInt::read(stream)
                    .await
                    .map_err(|_| SettingsError::UnexpectedEnd)?,
            );
            let size = VarInt::read(stream)
                .await
                .map_err(|_| SettingsError::UnexpectedEnd)?;

            let size = size.into_inner();
            if size > MAX_FRAME_SIZE {
                return Err(SettingsError::FrameTooLarge);
            }

            let mut payload = stream.take(size);

            if frame_typ.is_grease() {
                let n = tokio::io::copy(&mut payload, &mut tokio::io::sink()).await?;
                if n < size {
                    return Err(SettingsError::UnexpectedEnd);
                }
                continue;
            }

            let mut buf = Vec::with_capacity(size as usize);
            payload.read_to_end(&mut buf).await?;

            if buf.len() < size as usize {
                return Err(SettingsError::UnexpectedEnd);
            }

            if frame_typ != Frame::SETTINGS {
                return Err(SettingsError::UnexpectedFrame(frame_typ));
            }

            let mut data = buf.as_slice();
            let mut settings = Settings::default();
            while data.has_remaining() {
                let id = Setting::decode(&mut data).map_err(|_| SettingsError::InvalidSize)?;
                let value = VarInt::decode(&mut data).map_err(|_| SettingsError::InvalidSize)?;
                if !id.is_grease() {
                    settings.0.insert(id, value);
                }
            }

            return Ok(settings);
        }
    }

    pub fn encode<B: BufMut>(&self, buf: &mut B) {
        StreamUni::CONTROL.encode(buf);
        Frame::SETTINGS.encode(buf);

        // Encode to a temporary buffer so we can learn the length.
        // TODO avoid doing this, just use a fixed size varint.
        let mut tmp = Vec::new();
        for (id, value) in &self.0 {
            id.encode(&mut tmp);
            value.encode(&mut tmp);
        }

        VarInt::from_u32(tmp.len() as u32).encode(buf);
        buf.put_slice(&tmp);
    }

    pub async fn write<S: AsyncWrite + Unpin>(&self, stream: &mut S) -> Result<(), SettingsError> {
        // TODO avoid allocating to the heap
        let mut buf = BytesMut::new();
        self.encode(&mut buf);
        stream.write_all_buf(&mut buf).await?;
        Ok(())
    }

    pub fn enable_webtransport(&mut self, max_sessions: u32) {
        let max = VarInt::from_u32(max_sessions);

        self.insert(
            Setting::MAX_FIELD_SECTION_SIZE,
            VarInt::from_u32(crate::MAX_FIELD_SECTION_SIZE as u32),
        );
        self.insert(Setting::ENABLE_CONNECT_PROTOCOL, VarInt::from_u32(1));
        self.insert(Setting::ENABLE_DATAGRAM, VarInt::from_u32(1));
        self.insert(Setting::ENABLE_DATAGRAM_DEPRECATED, VarInt::from_u32(1));
        self.insert(Setting::WEBTRANSPORT_MAX_SESSIONS, max);

        // TODO remove when 07 is in the wild
        self.insert(Setting::WEBTRANSPORT_MAX_SESSIONS_DEPRECATED, max);
        self.insert(Setting::WEBTRANSPORT_ENABLE_DEPRECATED, VarInt::from_u32(1));
    }

    // Returns the maximum number of sessions supported.
    pub fn supports_webtransport(&self) -> u64 {
        // Sent by Chrome 114.0.5735.198 (July 19, 2023)
        // Setting(1): 65536,              // qpack_max_table_capacity
        // Setting(6): 16384,              // max_field_section_size
        // Setting(7): 100,                // qpack_blocked_streams
        // Setting(51): 1,                 // enable_datagram
        // Setting(16765559): 1            // enable_datagram_deprecated
        // Setting(727725890): 1,          // webtransport_max_sessions_deprecated
        // Setting(4445614305): 454654587, // grease

        // NOTE: The presence of ENABLE_WEBTRANSPORT implies ENABLE_CONNECT is supported.

        let datagram = self
            .get(&Setting::ENABLE_DATAGRAM)
            .or(self.get(&Setting::ENABLE_DATAGRAM_DEPRECATED))
            .map(|v| v.into_inner());

        if datagram != Some(1) {
            return 0;
        }

        // The deprecated (before draft-07) way of enabling WebTransport was to send two parameters.
        // Both would send ENABLE=1 and the server would send MAX_SESSIONS=N to limit the sessions.
        // Now both just send MAX_SESSIONS, and a non-zero value means WebTransport is enabled.

        if let Some(max) = self.get(&Setting::WEBTRANSPORT_MAX_SESSIONS) {
            return max.into_inner();
        }

        let enabled = self
            .get(&Setting::WEBTRANSPORT_ENABLE_DEPRECATED)
            .map(|v| v.into_inner());
        if enabled != Some(1) {
            return 0;
        }

        // Only the server is allowed to set this one, so if it's None we assume it's 1.
        self.get(&Setting::WEBTRANSPORT_MAX_SESSIONS_DEPRECATED)
            .map(|v| v.into_inner())
            .unwrap_or(1)
    }
}

impl Deref for Settings {
    type Target = HashMap<Setting, VarInt>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Settings {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode_settings(settings: &Settings) -> Vec<u8> {
        let mut buf = BytesMut::new();
        settings.encode(&mut buf);
        buf.to_vec()
    }

    #[test]
    fn webtransport_advertises_enforced_field_section_size() {
        let mut settings = Settings::default();
        settings.enable_webtransport(4);
        let decoded = Settings::decode(&mut encode_settings(&settings).as_slice()).unwrap();
        assert_eq!(
            decoded
                .get(&Setting::MAX_FIELD_SECTION_SIZE)
                .map(|size| size.into_inner()),
            Some(crate::MAX_FIELD_SECTION_SIZE)
        );
    }

    #[test]
    fn sync_settings_reject_declared_size_before_payload_arrives() {
        for size in [MAX_FRAME_SIZE + 1, (1_u64 << 32) + 2] {
            let mut wire = Vec::new();
            StreamUni::CONTROL.encode(&mut wire);
            Frame::SETTINGS.encode(&mut wire);
            VarInt::try_from(size).unwrap().encode(&mut wire);
            assert!(matches!(
                Settings::decode(&mut wire.as_slice()),
                Err(SettingsError::FrameTooLarge)
            ));
        }
    }

    #[tokio::test]
    async fn settings_skip_large_and_repeated_grease_without_recursion() {
        let mut settings = Settings::default();
        settings.enable_webtransport(1);
        let encoded = encode_settings(&settings);
        let mut frame = encoded.as_slice();
        StreamUni::decode(&mut frame).unwrap();
        let mut wire = Vec::new();
        StreamUni::CONTROL.encode(&mut wire);
        Frame(VarInt::from_u32(0x21)).encode(&mut wire);
        VarInt::from_u32(MAX_FRAME_SIZE as u32).encode(&mut wire);
        wire.resize(wire.len() + MAX_FRAME_SIZE as usize, 0);
        for _ in 0..10_000 {
            Frame(VarInt::from_u32(0x21)).encode(&mut wire);
            VarInt::from_u32(0).encode(&mut wire);
        }
        wire.extend_from_slice(frame);
        assert_eq!(
            Settings::decode(&mut wire.as_slice())
                .unwrap()
                .supports_webtransport(),
            1
        );
        assert_eq!(
            Settings::read(&mut Cursor::new(wire))
                .await
                .unwrap()
                .supports_webtransport(),
            1
        );
    }

    #[tokio::test]
    async fn read_exact_consumption() {
        let mut settings = Settings::default();
        settings.enable_webtransport(1);

        let mut wire = encode_settings(&settings);
        let trailing = b"trailing";
        wire.extend_from_slice(trailing);

        let mut cursor = Cursor::new(wire);
        let decoded = Settings::read(&mut cursor).await.unwrap();
        assert_eq!(decoded.supports_webtransport(), 1);

        let pos = cursor.position() as usize;
        let remaining = &cursor.into_inner()[pos..];
        assert_eq!(remaining, trailing);
    }

    #[tokio::test]
    async fn read_roundtrip() {
        let mut settings = Settings::default();
        settings.enable_webtransport(4);

        let wire = encode_settings(&settings);
        let mut cursor = Cursor::new(wire);
        let decoded = Settings::read(&mut cursor).await.unwrap();
        assert_eq!(decoded.supports_webtransport(), 4);
    }

    #[tokio::test]
    async fn read_empty_stream() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let err = Settings::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, SettingsError::UnexpectedEnd));
    }

    #[tokio::test]
    async fn read_wrong_stream_type() {
        let mut wire = Vec::new();
        StreamUni::PUSH.encode(&mut wire); // Wrong stream type

        let mut cursor = Cursor::new(wire);
        let err = Settings::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, SettingsError::UnexpectedStreamType(_)));
    }

    #[tokio::test]
    async fn read_wrong_frame_type() {
        let mut wire = Vec::new();
        StreamUni::CONTROL.encode(&mut wire);
        Frame::HEADERS.encode(&mut wire); // Wrong frame type
        VarInt::from_u32(0).encode(&mut wire);

        let mut cursor = Cursor::new(wire);
        let err = Settings::read(&mut cursor).await.unwrap_err();
        assert!(
            matches!(err, SettingsError::UnexpectedFrame(f) if f == Frame::HEADERS),
            "expected UnexpectedFrame(HEADERS), got {err:?}"
        );
    }

    #[tokio::test]
    async fn read_rejects_frame_too_large() {
        let mut wire = Vec::new();
        StreamUni::CONTROL.encode(&mut wire);
        Frame::SETTINGS.encode(&mut wire);
        VarInt::from_u32(128 * 1024).encode(&mut wire); // > 64 KiB

        let mut cursor = Cursor::new(wire);
        let err = Settings::read(&mut cursor).await.unwrap_err();
        assert!(
            matches!(err, SettingsError::FrameTooLarge),
            "expected FrameTooLarge, got {err:?}"
        );
    }

    #[tokio::test]
    async fn read_skips_grease_frame() {
        let mut settings = Settings::default();
        settings.enable_webtransport(1);

        // Build: CONTROL stream type + GREASE frame + SETTINGS frame
        let mut wire = Vec::new();
        StreamUni::CONTROL.encode(&mut wire);

        // Insert a GREASE frame (type 0x21 is the first GREASE value)
        VarInt::from_u32(0x21).encode(&mut wire);
        VarInt::from_u32(4).encode(&mut wire);
        wire.extend_from_slice(b"junk");

        // Now the real SETTINGS frame (without the stream type prefix, since we already wrote it)
        Frame::SETTINGS.encode(&mut wire);
        let mut tmp = Vec::new();
        for (id, value) in settings.iter() {
            id.encode(&mut tmp);
            value.encode(&mut tmp);
        }
        VarInt::from_u32(tmp.len() as u32).encode(&mut wire);
        wire.extend_from_slice(&tmp);

        let mut cursor = Cursor::new(wire);
        let decoded = Settings::read(&mut cursor).await.unwrap();
        assert_eq!(decoded.supports_webtransport(), 1);
    }

    #[tokio::test]
    async fn read_truncated_payload() {
        let mut wire = Vec::new();
        StreamUni::CONTROL.encode(&mut wire);
        Frame::SETTINGS.encode(&mut wire);
        VarInt::from_u32(100).encode(&mut wire); // claims 100 bytes
        wire.extend_from_slice(b"short"); // only 5 bytes

        let mut cursor = Cursor::new(wire);
        let err = Settings::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, SettingsError::UnexpectedEnd));
    }

    #[tokio::test]
    async fn read_truncated_grease() {
        let mut wire = Vec::new();
        StreamUni::CONTROL.encode(&mut wire);
        VarInt::from_u32(0x21).encode(&mut wire); // GREASE frame type
        VarInt::from_u32(50).encode(&mut wire); // claims 50 bytes
        wire.extend_from_slice(b"ab"); // only 2 bytes

        let mut cursor = Cursor::new(wire);
        let err = Settings::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, SettingsError::UnexpectedEnd));
    }
}
