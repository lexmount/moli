use std::sync::Arc;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{Frame, VarInt, VarIntUnexpectedEnd, MAX_FRAME_SIZE};

// CloseWebTransportSession capsule type (draft-ietf-webtrans-http3-06).
const CLOSE_WEBTRANSPORT_SESSION_TYPE: u64 = 0x2843;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capsule {
    CloseWebTransportSession { code: u32, reason: String },
    Grease { num: u64 },
    Unknown { typ: VarInt, payload: Bytes },
}

impl Capsule {
    pub fn decode<B: Buf>(buf: &mut B) -> Result<Self, CapsuleError> {
        let typ = VarInt::decode(buf)?;
        let length = VarInt::decode(buf)?;

        let mut payload = buf.take(length.into_inner() as usize);

        // Check declared length first - reject immediately if too large
        if payload.limit() > MAX_FRAME_SIZE as usize {
            return Err(CapsuleError::MessageTooLong);
        }

        // Then check if all declared bytes are buffered
        if payload.remaining() < payload.limit() {
            return Err(CapsuleError::UnexpectedEnd);
        }

        let typ_val = typ.into_inner();

        if let Some(num) = is_grease(typ_val) {
            payload.advance(payload.remaining());
            return Ok(Self::Grease { num });
        }

        match typ_val {
            CLOSE_WEBTRANSPORT_SESSION_TYPE => {
                if payload.remaining() < 4 {
                    return Err(CapsuleError::UnexpectedEnd);
                }

                let error_code = payload.get_u32();

                let message_len = payload.remaining();
                if message_len > MAX_FRAME_SIZE as usize {
                    return Err(CapsuleError::MessageTooLong);
                }

                let mut message_bytes = vec![0u8; message_len];
                payload.copy_to_slice(&mut message_bytes);

                let error_message =
                    String::from_utf8(message_bytes).map_err(|_| CapsuleError::InvalidUtf8)?;

                Ok(Self::CloseWebTransportSession {
                    code: error_code,
                    reason: error_message,
                })
            }
            _ => {
                let mut payload_bytes = vec![0u8; payload.remaining()];
                payload.copy_to_slice(&mut payload_bytes);
                Ok(Self::Unknown {
                    typ,
                    payload: Bytes::from(payload_bytes),
                })
            }
        }
    }

    /// Read a capsule from a stream, consuming only the exact bytes of the capsule.
    ///
    /// Returns `Ok(None)` if the stream is cleanly closed (EOF before any bytes).
    pub async fn read<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Option<Self>, CapsuleError> {
        let typ = match VarInt::read_optional(stream).await {
            Ok(Some(v)) => v,
            Ok(None) => return Ok(None), // Clean EOF
            Err(_) => return Err(CapsuleError::UnexpectedEnd),
        };
        let length = VarInt::read(stream)
            .await
            .map_err(|_| CapsuleError::UnexpectedEnd)?;

        let length = length.into_inner();
        let typ_val = typ.into_inner();

        if length > MAX_FRAME_SIZE {
            return Err(CapsuleError::MessageTooLong);
        }

        let mut payload = stream.take(length);

        if let Some(num) = is_grease(typ_val) {
            let n = tokio::io::copy(&mut payload, &mut tokio::io::sink()).await?;
            if n < length {
                return Err(CapsuleError::UnexpectedEnd);
            }
            return Ok(Some(Self::Grease { num }));
        }

        let mut buf = Vec::with_capacity(length as usize);
        payload.read_to_end(&mut buf).await?;

        if buf.len() < length as usize {
            return Err(CapsuleError::UnexpectedEnd);
        }

        match typ_val {
            CLOSE_WEBTRANSPORT_SESSION_TYPE => {
                let mut data = buf.as_slice();
                if data.remaining() < 4 {
                    return Err(CapsuleError::UnexpectedEnd);
                }

                let error_code = data.get_u32();
                let error_message =
                    String::from_utf8(data.to_vec()).map_err(|_| CapsuleError::InvalidUtf8)?;

                Ok(Some(Self::CloseWebTransportSession {
                    code: error_code,
                    reason: error_message,
                }))
            }
            _ => Ok(Some(Self::Unknown {
                typ,
                payload: Bytes::from(buf),
            })),
        }
    }

    pub fn encode<B: BufMut>(&self, buf: &mut B) {
        match self {
            Self::CloseWebTransportSession {
                code: error_code,
                reason: error_message,
            } => {
                // Encode the capsule type
                VarInt::from_u64(CLOSE_WEBTRANSPORT_SESSION_TYPE)
                    .unwrap()
                    .encode(buf);

                // Calculate and encode the length
                let length = 4 + error_message.len();
                VarInt::from_u32(length as u32).encode(buf);

                // Encode the error code (32-bit)
                buf.put_u32(*error_code);

                // Encode the error message
                buf.put_slice(error_message.as_bytes());
            }
            Self::Grease { num } => {
                // Generate grease type: 0x29 * N + 0x17
                // Check for overflow
                let grease_type = num
                    .checked_mul(0x29)
                    .and_then(|v| v.checked_add(0x17))
                    .expect("grease num value would overflow u64");

                VarInt::from_u64(grease_type).unwrap().encode(buf);

                // Grease capsules have zero-length payload
                VarInt::from_u32(0).encode(buf);
            }
            Self::Unknown { typ, payload } => {
                // Encode the capsule type
                typ.encode(buf);

                // Encode the length
                VarInt::try_from(payload.len()).unwrap().encode(buf);

                // Encode the payload
                buf.put_slice(payload);
            }
        }
    }

    pub async fn write<S: AsyncWrite + Unpin>(&self, stream: &mut S) -> Result<(), CapsuleError> {
        let mut buf = BytesMut::new();
        self.encode(&mut buf);
        stream.write_all_buf(&mut buf).await?;
        Ok(())
    }
}

// RFC 9297 Section 5.4: Capsule types of the form 0x29 * N + 0x17
// Returns Some(N) if the value is a grease type, None otherwise
fn is_grease(val: u64) -> Option<u64> {
    if val < 0x17 {
        return None;
    }
    let num = (val - 0x17) / 0x29;
    if val == 0x29 * num + 0x17 {
        Some(num)
    } else {
        None
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum CapsuleError {
    #[error("unexpected end of buffer")]
    UnexpectedEnd,

    #[error("invalid UTF-8")]
    InvalidUtf8,

    #[error("message too long")]
    MessageTooLong,

    #[error("unknown capsule type: {0:?}")]
    UnknownType(VarInt),

    #[error("varint decode error: {0:?}")]
    VarInt(#[from] VarIntUnexpectedEnd),

    #[error("io error: {0}")]
    Io(Arc<std::io::Error>),
}

impl From<std::io::Error> for CapsuleError {
    fn from(err: std::io::Error) -> Self {
        CapsuleError::Io(Arc::new(err))
    }
}

/// Reads capsules from an HTTP/3 CONNECT stream where capsule protocol
/// data is carried inside DATA frames (RFC 9297 Section 3.2).
///
/// Handles capsules split across multiple DATA frames and multiple
/// capsules within a single DATA frame.
pub struct Http3CapsuleReader<S> {
    stream: S,
    buf: BytesMut,
}

impl<S: AsyncRead + Unpin> Http3CapsuleReader<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: BytesMut::new(),
        }
    }

    /// Read the next capsule. Returns `Ok(None)` on clean EOF.
    pub async fn read(&mut self) -> Result<Option<Capsule>, CapsuleError> {
        loop {
            if !self.buf.is_empty() {
                let mut slice = &self.buf[..];
                match Capsule::decode(&mut slice) {
                    Ok(capsule) => {
                        self.buf.advance(self.buf.len() - slice.len());
                        return Ok(Some(capsule));
                    }
                    Err(CapsuleError::UnexpectedEnd | CapsuleError::VarInt(_)) => {}
                    Err(e) => return Err(e),
                }
            }

            if !self.read_data_frame().await? {
                return if self.buf.is_empty() {
                    Ok(None)
                } else {
                    Err(CapsuleError::UnexpectedEnd)
                };
            }
        }
    }

    /// Read the next HTTP/3 DATA frame into the buffer, skipping non-DATA frames.
    /// Returns `false` on EOF.
    async fn read_data_frame(&mut self) -> Result<bool, CapsuleError> {
        loop {
            let frame_type = match VarInt::read_optional(&mut self.stream).await {
                Ok(Some(v)) => Frame(v),
                Ok(None) => return Ok(false),
                Err(_) => return Err(CapsuleError::UnexpectedEnd),
            };
            let len = VarInt::read(&mut self.stream)
                .await
                .map_err(|_| CapsuleError::UnexpectedEnd)?
                .into_inner() as usize;

            if len > MAX_FRAME_SIZE as usize {
                return Err(CapsuleError::MessageTooLong);
            }

            if frame_type != Frame::DATA {
                let mut limited = (&mut self.stream).take(len as u64);
                let n = tokio::io::copy(&mut limited, &mut tokio::io::sink()).await?;
                if (n as usize) < len {
                    return Err(CapsuleError::UnexpectedEnd);
                }
                continue;
            }

            if len > 0 {
                let start = self.buf.len();
                self.buf.resize(start + len, 0);
                self.stream.read_exact(&mut self.buf[start..]).await?;
            }
            return Ok(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_close_webtransport_session_decode() {
        // Test with spec-compliant type 0x2843 (encodes as 0x68 0x43)
        let mut data = Vec::new();
        VarInt::from_u64(0x2843).unwrap().encode(&mut data);
        VarInt::from_u32(8).encode(&mut data);
        data.extend_from_slice(b"\x00\x00\x01\xa4test");

        let mut buf = data.as_slice();
        let capsule = Capsule::decode(&mut buf).unwrap();

        match capsule {
            Capsule::CloseWebTransportSession {
                code: error_code,
                reason: error_message,
            } => {
                assert_eq!(error_code, 420);
                assert_eq!(error_message, "test");
            }
            _ => panic!("Expected CloseWebTransportSession"),
        }

        assert_eq!(buf.len(), 0); // All bytes consumed
    }

    #[test]
    fn test_close_webtransport_session_encode() {
        let capsule = Capsule::CloseWebTransportSession {
            code: 420,
            reason: "test".to_string(),
        };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        // Expected format: type(0x2843 as varint = 0x68 0x43) + length(8 as varint) + error_code(420 as u32 BE) + "test"
        assert_eq!(buf, b"\x68\x43\x08\x00\x00\x01\xa4test");
    }

    #[test]
    fn test_close_webtransport_session_roundtrip() {
        let original = Capsule::CloseWebTransportSession {
            code: 12345,
            reason: "Connection closed by application".to_string(),
        };

        let mut buf = Vec::new();
        original.encode(&mut buf);

        let mut read_buf = buf.as_slice();
        let decoded = Capsule::decode(&mut read_buf).unwrap();

        assert_eq!(original, decoded);
        assert_eq!(read_buf.len(), 0); // All bytes consumed
    }

    #[test]
    fn test_empty_error_message() {
        let capsule = Capsule::CloseWebTransportSession {
            code: 0,
            reason: String::new(),
        };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        // Type(0x2843 as varint = 0x68 0x43) + Length(4) + error_code(0)
        assert_eq!(buf, b"\x68\x43\x04\x00\x00\x00\x00");

        let mut read_buf = buf.as_slice();
        let decoded = Capsule::decode(&mut read_buf).unwrap();
        assert_eq!(capsule, decoded);
    }

    #[test]
    fn test_invalid_utf8() {
        // Create a capsule with invalid UTF-8 in the message
        let mut data = Vec::new();
        VarInt::from_u64(0x2843).unwrap().encode(&mut data); // type
        VarInt::from_u32(5).encode(&mut data); // length(5)
        data.extend_from_slice(b"\x00\x00\x00\x00"); // error_code(0)
        data.push(0xFF); // Invalid UTF-8 byte

        let mut buf = data.as_slice();
        let result = Capsule::decode(&mut buf);
        assert!(matches!(result, Err(CapsuleError::InvalidUtf8)));
    }

    #[test]
    fn test_truncated_error_code() {
        // Capsule with length indicating 3 bytes but error code needs 4
        let mut data = Vec::new();
        VarInt::from_u64(0x2843).unwrap().encode(&mut data); // type
        VarInt::from_u32(3).encode(&mut data); // length(3)
        data.extend_from_slice(b"\x00\x00\x00"); // incomplete error code

        let mut buf = data.as_slice();
        let result = Capsule::decode(&mut buf);
        assert!(matches!(result, Err(CapsuleError::UnexpectedEnd)));
    }

    #[test]
    fn test_unknown_capsule() {
        // Test handling of unknown capsule types
        let unknown_type = 0x1234u64;
        let payload_data = b"unknown payload";

        let mut data = Vec::new();
        VarInt::from_u64(unknown_type).unwrap().encode(&mut data);
        VarInt::from_u32(payload_data.len() as u32).encode(&mut data);
        data.extend_from_slice(payload_data);

        let mut buf = data.as_slice();
        let capsule = Capsule::decode(&mut buf).unwrap();

        match capsule {
            Capsule::Unknown { typ, payload } => {
                assert_eq!(typ.into_inner(), unknown_type);
                assert_eq!(payload.as_ref(), payload_data);
            }
            _ => panic!("Expected Unknown capsule"),
        }
    }

    #[test]
    fn test_unknown_capsule_roundtrip() {
        let capsule = Capsule::Unknown {
            typ: VarInt::from_u64(0x9999).unwrap(),
            payload: Bytes::from("test payload"),
        };

        let mut buf = Vec::new();
        capsule.encode(&mut buf);

        let mut read_buf = buf.as_slice();
        let decoded = Capsule::decode(&mut read_buf).unwrap();

        assert_eq!(capsule, decoded);
        assert_eq!(read_buf.len(), 0);
    }

    #[test]
    fn test_grease_capsule() {
        // Test grease formula: 0x29 * N + 0x17
        for num in [0, 1, 5, 100, 1000] {
            let capsule = Capsule::Grease { num };

            let mut buf = Vec::new();
            capsule.encode(&mut buf);

            let mut read_buf = buf.as_slice();
            let decoded = Capsule::decode(&mut read_buf).unwrap();

            assert_eq!(capsule, decoded);
            assert_eq!(read_buf.len(), 0);
        }
    }

    #[test]
    fn test_grease_values() {
        // Verify specific grease type values
        assert_eq!(is_grease(0x17), Some(0)); // N=0
        assert_eq!(is_grease(0x40), Some(1)); // N=1: 0x29 + 0x17 = 0x40
        assert_eq!(is_grease(0x69), Some(2)); // N=2: 0x29*2 + 0x17 = 0x69
        assert_eq!(is_grease(0x18), None); // Not a grease value
        assert_eq!(is_grease(0x41), None); // Not a grease value
    }

    #[test]
    #[should_panic(expected = "grease num value would overflow u64")]
    fn test_grease_overflow() {
        let capsule = Capsule::Grease { num: u64::MAX };
        let mut buf = Vec::new();
        capsule.encode(&mut buf);
    }

    #[tokio::test]
    async fn test_read_exact_consumption() {
        let capsule = Capsule::CloseWebTransportSession {
            code: 42,
            reason: "bye".to_string(),
        };
        let mut wire = Vec::new();
        capsule.encode(&mut wire);
        let trailing = b"leftover";
        wire.extend_from_slice(trailing);

        let mut cursor = std::io::Cursor::new(wire);
        let decoded = Capsule::read(&mut cursor).await.unwrap().unwrap();
        assert_eq!(capsule, decoded);

        let pos = cursor.position() as usize;
        let remaining = &cursor.into_inner()[pos..];
        assert_eq!(remaining, trailing);
    }

    #[tokio::test]
    async fn test_read_roundtrip() {
        let capsule = Capsule::CloseWebTransportSession {
            code: 100,
            reason: "test".to_string(),
        };
        let mut wire = Vec::new();
        capsule.encode(&mut wire);

        let mut cursor = std::io::Cursor::new(wire);
        let decoded = Capsule::read(&mut cursor).await.unwrap().unwrap();
        assert_eq!(capsule, decoded);
    }

    #[tokio::test]
    async fn test_read_eof_returns_none() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let result = Capsule::read(&mut cursor).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_read_rejects_too_large() {
        let mut wire = Vec::new();
        VarInt::from_u64(0x2843).unwrap().encode(&mut wire); // type
        VarInt::from_u64(MAX_FRAME_SIZE + 1)
            .unwrap()
            .encode(&mut wire); // too large

        let mut cursor = std::io::Cursor::new(wire);
        let err = Capsule::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, CapsuleError::MessageTooLong));
    }

    #[tokio::test]
    async fn test_read_truncated_payload() {
        // CloseWebTransportSession needs at least 4 bytes for error code,
        // but the stream is shorter than the declared length.
        let mut wire = Vec::new();
        VarInt::from_u64(0x2843).unwrap().encode(&mut wire);
        VarInt::from_u32(100).encode(&mut wire); // claims 100 bytes
        wire.extend_from_slice(b"short"); // only 5 bytes

        let mut cursor = std::io::Cursor::new(wire);
        let err = Capsule::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, CapsuleError::UnexpectedEnd));
    }

    #[tokio::test]
    async fn test_read_truncated_grease() {
        // GREASE capsule type (0x17 = first grease value), claims 50 bytes, only 2 present.
        let mut wire = Vec::new();
        VarInt::from_u32(0x17).encode(&mut wire);
        VarInt::from_u32(50).encode(&mut wire);
        wire.extend_from_slice(b"ab");

        let mut cursor = std::io::Cursor::new(wire);
        let err = Capsule::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, CapsuleError::UnexpectedEnd));
    }

    fn encode_capsule(c: &Capsule) -> Vec<u8> {
        let mut buf = Vec::new();
        c.encode(&mut buf);
        buf
    }

    fn wrap_in_data_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::new();
        Frame::DATA.encode(&mut frame);
        VarInt::from_u32(payload.len() as u32).encode(&mut frame);
        frame.extend_from_slice(payload);
        frame
    }

    /// Concatenate multiple DATA-framed chunks into a single wire buffer.
    fn split_into_data_frames(capsule_bytes: &[u8], splits: &[usize]) -> Vec<u8> {
        let mut wire = Vec::new();
        let mut prev = 0;
        for &s in splits {
            wire.extend_from_slice(&wrap_in_data_frame(&capsule_bytes[prev..s]));
            prev = s;
        }
        wire.extend_from_slice(&wrap_in_data_frame(&capsule_bytes[prev..]));
        wire
    }

    fn reader_from(wire: Vec<u8>) -> Http3CapsuleReader<std::io::Cursor<Vec<u8>>> {
        Http3CapsuleReader::new(std::io::Cursor::new(wire))
    }

    #[tokio::test]
    async fn test_http3_reader_single_capsule_single_frame() {
        let capsule = Capsule::CloseWebTransportSession {
            code: 42,
            reason: "bye".into(),
        };
        let mut reader = reader_from(wrap_in_data_frame(&encode_capsule(&capsule)));
        assert_eq!(reader.read().await.unwrap().unwrap(), capsule);
        assert!(reader.read().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_http3_reader_capsule_split_across_frames() {
        let capsule = Capsule::CloseWebTransportSession {
            code: 100,
            reason: "split".into(),
        };
        let bytes = encode_capsule(&capsule);
        let mut reader = reader_from(split_into_data_frames(&bytes, &[bytes.len() / 2]));
        assert_eq!(reader.read().await.unwrap().unwrap(), capsule);
    }

    #[tokio::test]
    async fn test_http3_reader_capsule_split_across_three_frames() {
        let capsule = Capsule::CloseWebTransportSession {
            code: 200,
            reason: "three".into(),
        };
        let bytes = encode_capsule(&capsule);
        let mut reader = reader_from(split_into_data_frames(&bytes, &[2, 5]));
        assert_eq!(reader.read().await.unwrap().unwrap(), capsule);
    }

    #[tokio::test]
    async fn test_http3_reader_multiple_capsules_one_frame() {
        let c1 = Capsule::Grease { num: 1 };
        let c2 = Capsule::CloseWebTransportSession {
            code: 7,
            reason: "done".into(),
        };
        let mut bytes = encode_capsule(&c1);
        bytes.extend_from_slice(&encode_capsule(&c2));

        let mut reader = reader_from(wrap_in_data_frame(&bytes));
        assert_eq!(reader.read().await.unwrap().unwrap(), c1);
        assert_eq!(reader.read().await.unwrap().unwrap(), c2);
    }

    #[tokio::test]
    async fn test_http3_reader_skips_non_data_frames() {
        let capsule = Capsule::CloseWebTransportSession {
            code: 0,
            reason: String::new(),
        };
        let mut wire = Vec::new();
        // HEADERS frame before the DATA frame.
        Frame::HEADERS.encode(&mut wire);
        VarInt::from_u32(5).encode(&mut wire);
        wire.extend_from_slice(b"hello");
        wire.extend_from_slice(&wrap_in_data_frame(&encode_capsule(&capsule)));

        let mut reader = reader_from(wire);
        assert_eq!(reader.read().await.unwrap().unwrap(), capsule);
    }

    #[tokio::test]
    async fn test_http3_reader_eof_returns_none() {
        assert!(reader_from(vec![]).read().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_http3_reader_partial_capsule_at_eof() {
        let mut partial = Vec::new();
        VarInt::from_u64(0x2843).unwrap().encode(&mut partial);
        // Missing: length + payload

        let mut reader = reader_from(wrap_in_data_frame(&partial));
        assert!(matches!(
            reader.read().await,
            Err(CapsuleError::UnexpectedEnd)
        ));
    }

    #[tokio::test]
    async fn test_http3_reader_empty_data_frame() {
        let capsule = Capsule::CloseWebTransportSession {
            code: 1,
            reason: "ok".into(),
        };
        let mut wire = wrap_in_data_frame(&[]);
        wire.extend_from_slice(&wrap_in_data_frame(&encode_capsule(&capsule)));

        let mut reader = reader_from(wire);
        assert_eq!(reader.read().await.unwrap().unwrap(), capsule);
    }
}
