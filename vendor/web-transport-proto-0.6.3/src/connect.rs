use std::{str::FromStr, sync::Arc};

use bytes::{Buf, BufMut, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use url::Url;

use super::{qpack, Frame, VarInt, MAX_FRAME_SIZE};

use thiserror::Error;

// RFC 9114 section 4.2.2: each decoded field costs name + value + 32 bytes.
pub(crate) const MAX_FIELD_SECTION_SIZE: u64 = 16 * 1024;

// Errors that can occur during the connect request.
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum ConnectError {
    #[error("unexpected end of input")]
    UnexpectedEnd,

    #[error("qpack error")]
    QpackError(#[from] qpack::DecodeError),

    #[error("unexpected frame {0:?}")]
    UnexpectedFrame(Frame),

    #[error("invalid method")]
    InvalidMethod,

    #[error("invalid url")]
    InvalidUrl(#[from] url::ParseError),

    #[error("invalid status")]
    InvalidStatus,

    #[error("expected 200, got: {0:?}")]
    WrongStatus(Option<http::StatusCode>),

    #[error("expected connect, got: {0:?}")]
    WrongMethod(Option<http::method::Method>),

    #[error("expected https, got: {0:?}")]
    WrongScheme(Option<String>),

    #[error("expected authority header")]
    WrongAuthority,

    #[error("expected webtransport, got: {0:?}")]
    WrongProtocol(Option<String>),

    #[error("expected path header")]
    WrongPath,

    #[error("invalid protocol header")]
    InvalidProtocol,

    #[error("structured field error: {0}")]
    StructuredFieldError(Arc<sfv::Error>),

    #[error("frame too large")]
    FrameTooLarge,

    #[error("non-200 status: {0:?}")]
    ErrorStatus(http::StatusCode),

    #[error("io error: {0}")]
    Io(Arc<std::io::Error>),

    #[error("invalid http header value")]
    InvalidHttpHeaderValue,

    #[error("invalid http header name")]
    InvalidHttpHeaderName,

    /// The encoded HEADERS frame or its decoded field section exceeds the CONNECT limit.
    #[error("field section too large")]
    FieldSectionTooLarge,
}

impl From<qpack::FieldSectionError> for ConnectError {
    fn from(err: qpack::FieldSectionError) -> Self {
        match err {
            qpack::FieldSectionError::Decode(err) => Self::QpackError(err),
            qpack::FieldSectionError::TooLarge => Self::FieldSectionTooLarge,
        }
    }
}

impl From<std::io::Error> for ConnectError {
    fn from(err: std::io::Error) -> Self {
        ConnectError::Io(Arc::new(err))
    }
}

impl From<sfv::Error> for ConnectError {
    fn from(err: sfv::Error) -> Self {
        ConnectError::StructuredFieldError(Arc::new(err))
    }
}

/// A CONNECT request to initiate a WebTransport session.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ConnectRequest {
    /// The URL to connect to.
    pub url: Url,

    /// The subprotocols requested (if any).
    pub protocols: Vec<String>,

    /// The raw HTTP/3 headers from the request.
    pub headers: http::HeaderMap,
}

impl ConnectRequest {
    pub fn new(url: impl Into<Url>) -> Self {
        Self {
            url: url.into(),
            protocols: Vec::new(),
            headers: http::HeaderMap::new(),
        }
    }

    pub fn with_protocol(mut self, protocol: impl Into<String>) -> Self {
        self.protocols.push(protocol.into());
        self
    }

    pub fn with_protocols(mut self, protocols: impl IntoIterator<Item = String>) -> Self {
        self.protocols.extend(protocols);
        self
    }

    pub fn with_header(mut self, name: http::HeaderName, value: http::HeaderValue) -> Self {
        self.headers.append(name, value);
        self
    }

    pub fn with_headers(mut self, headers: http::HeaderMap) -> Self {
        self.headers.extend(headers);
        self
    }

    pub fn decode<B: Buf>(buf: &mut B) -> Result<Self, ConnectError> {
        let mut data = decode_headers_frame(buf)?;
        Self::decode_headers(&mut data)
    }

    fn decode_headers<B: Buf>(data: &mut B) -> Result<Self, ConnectError> {
        let headers = qpack::Headers::decode(data)?;

        let scheme = match headers.get(":scheme") {
            Some("https") => "https",
            Some(scheme) => Err(ConnectError::WrongScheme(Some(scheme.to_string())))?,
            None => return Err(ConnectError::WrongScheme(None)),
        };

        let authority = headers
            .get(":authority")
            .ok_or(ConnectError::WrongAuthority)?;

        let path_and_query = headers.get(":path").ok_or(ConnectError::WrongPath)?;

        let method = headers.get(":method");
        match method
            .map(|method| method.try_into().map_err(|_| ConnectError::InvalidMethod))
            .transpose()?
        {
            Some(http::Method::CONNECT) => (),
            o => return Err(ConnectError::WrongMethod(o)),
        };

        let protocol = headers.get(":protocol");
        if protocol != Some("webtransport") {
            return Err(ConnectError::WrongProtocol(protocol.map(|s| s.to_string())));
        }

        let protocols = headers
            .get(protocol_negotiation::AVAILABLE_NAME)
            .map(protocol_negotiation::decode_list)
            .transpose()
            .map_err(|_| ConnectError::InvalidProtocol)?
            .unwrap_or_default();

        let url = Url::parse(&format!("{scheme}://{authority}{path_and_query}"))?;

        // Save all headers, excluding pseudo-headers and protocol negotiation headers
        // (protocol negotiation is handled via the `protocols` field).
        let mut raw_headers = http::HeaderMap::new();
        for (item_header_name, item_header_value) in headers.fields.iter() {
            if item_header_name.starts_with(':') {
                continue;
            }
            if item_header_name == protocol_negotiation::AVAILABLE_NAME {
                continue;
            }
            let header_name = http::HeaderName::from_bytes(item_header_name.as_bytes())
                .map_err(|_| ConnectError::InvalidHttpHeaderName)?;
            let header_value = http::HeaderValue::from_str(item_header_value)
                .map_err(|_| ConnectError::InvalidHttpHeaderValue)?;
            raw_headers.append(header_name, header_value);
        }

        Ok(Self {
            url,
            protocols,
            headers: raw_headers,
        })
    }

    /// Read a CONNECT request from a stream, consuming only the exact bytes of the frame.
    pub async fn read<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Self, ConnectError> {
        let buf = read_headers_frame(stream).await?;
        Self::decode_headers(&mut buf.as_slice())
    }

    pub fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ConnectError> {
        let mut headers = qpack::Headers::default();
        for (item_header_name, item_header_value) in self.headers.iter() {
            // Skip protocol negotiation headers; they are derived from `self.protocols`.
            if item_header_name == protocol_negotiation::AVAILABLE_NAME {
                continue;
            }
            // http::HeaderValue can contain arbitrary bytes (not just UTF-8).
            // The to_str() method fails when the header value contains invalid UTF-8 bytes
            let item_header_value_str = item_header_value
                .to_str()
                .map_err(|_| ConnectError::InvalidHttpHeaderValue)?;
            headers.set(item_header_name.as_str(), item_header_value_str);
        }
        headers.set(":method", "CONNECT");
        headers.set(":scheme", self.url.scheme());
        headers.set(":authority", self.url.authority());
        let path_and_query = match self.url.query() {
            Some(query) => format!("{}?{}", self.url.path(), query),
            None => self.url.path().to_string(),
        };
        headers.set(":path", &path_and_query);
        headers.set(":protocol", "webtransport");

        if !self.protocols.is_empty() {
            let encoded = protocol_negotiation::encode_list(&self.protocols)?;
            headers.set(protocol_negotiation::AVAILABLE_NAME, &encoded);
        }

        encode_headers_frame(&headers, buf)
    }

    pub async fn write<S: AsyncWrite + Unpin>(&self, stream: &mut S) -> Result<(), ConnectError> {
        let mut buf = BytesMut::new();
        self.encode(&mut buf)?;
        stream.write_all_buf(&mut buf).await?;
        Ok(())
    }
}

impl From<Url> for ConnectRequest {
    fn from(url: Url) -> Self {
        Self {
            url,
            protocols: Vec::new(),
            headers: http::HeaderMap::new(),
        }
    }
}

/// A CONNECT response to accept or reject a WebTransport session.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ConnectResponse {
    /// The status code of the response.
    pub status: http::status::StatusCode,

    /// The subprotocol selected by the server, if any
    pub protocol: Option<String>,
}

impl ConnectResponse {
    pub const OK: Self = Self {
        status: http::StatusCode::OK,
        protocol: None,
    };

    pub fn new(status: http::StatusCode) -> Self {
        Self {
            status,
            protocol: None,
        }
    }

    pub fn with_protocol(mut self, protocol: impl Into<String>) -> Self {
        self.protocol = Some(protocol.into());
        self
    }

    pub fn decode<B: Buf>(buf: &mut B) -> Result<Self, ConnectError> {
        let mut data = decode_headers_frame(buf)?;
        Self::decode_headers(&mut data)
    }

    fn decode_headers<B: Buf>(data: &mut B) -> Result<Self, ConnectError> {
        let headers = qpack::Headers::decode(data)?;

        let status = match headers
            .get(":status")
            .map(|status| {
                http::StatusCode::from_str(status).map_err(|_| ConnectError::InvalidStatus)
            })
            .transpose()?
        {
            Some(status) if status.is_success() => status,
            o => return Err(ConnectError::WrongStatus(o)),
        };

        let protocol = headers
            .get(protocol_negotiation::SELECTED_NAME)
            // Protocol negotiation is optional in the legacy HTTP/3 profile.
            // An invalid Structured Field does not invalidate the response.
            .and_then(|value| protocol_negotiation::decode_item(value).ok());

        Ok(Self { status, protocol })
    }

    /// Read a CONNECT response from a stream, consuming only the exact bytes of the frame.
    pub async fn read<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Self, ConnectError> {
        let buf = read_headers_frame(stream).await?;
        Self::decode_headers(&mut buf.as_slice())
    }

    pub fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ConnectError> {
        let mut headers = qpack::Headers::default();
        headers.set(":status", self.status.as_str());
        headers.set("sec-webtransport-http3-draft", "draft02");

        if let Some(protocol) = self.protocol.as_ref() {
            let encoded = protocol_negotiation::encode_item(protocol)?;
            headers.set(protocol_negotiation::SELECTED_NAME, &encoded);
        }

        encode_headers_frame(&headers, buf)
    }

    pub async fn write<S: AsyncWrite + Unpin>(&self, stream: &mut S) -> Result<(), ConnectError> {
        let mut buf = BytesMut::new();
        self.encode(&mut buf)?;
        stream.write_all_buf(&mut buf).await?;
        Ok(())
    }
}

impl Default for ConnectResponse {
    fn default() -> Self {
        Self::OK
    }
}

impl From<http::StatusCode> for ConnectResponse {
    fn from(status: http::StatusCode) -> Self {
        Self {
            status,
            protocol: None,
        }
    }
}

fn check_frame_size(typ: Frame, size: u64) -> Result<(), ConnectError> {
    if typ == Frame::HEADERS && size > MAX_FIELD_SECTION_SIZE {
        return Err(ConnectError::FieldSectionTooLarge);
    }
    if size > MAX_FRAME_SIZE {
        return Err(ConnectError::FrameTooLarge);
    }
    Ok(())
}

fn encode_headers_frame<B: BufMut>(
    headers: &qpack::Headers,
    buf: &mut B,
) -> Result<(), ConnectError> {
    // Use a temporary buffer so we can compute the size and write nothing on error.
    let mut tmp = Vec::new();
    headers.encode(&mut tmp)?;
    let size = VarInt::from_u32(tmp.len() as u32);

    Frame::HEADERS.encode(buf);
    size.encode(buf);
    buf.put_slice(&tmp);

    Ok(())
}

fn decode_headers_frame<B: Buf>(buf: &mut B) -> Result<bytes::buf::Take<&mut B>, ConnectError> {
    let (typ, data) = Frame::read_bounded(buf, MAX_FIELD_SECTION_SIZE, MAX_FRAME_SIZE).map_err(
        |err| match err {
            crate::frame::FrameReadError::UnexpectedEnd => ConnectError::UnexpectedEnd,
            crate::frame::FrameReadError::TooLarge(Frame::HEADERS) => {
                ConnectError::FieldSectionTooLarge
            }
            crate::frame::FrameReadError::TooLarge(_) => ConnectError::FrameTooLarge,
        },
    )?;
    if typ != Frame::HEADERS {
        return Err(ConnectError::UnexpectedFrame(typ));
    }
    Ok(data)
}

/// Read the next HEADERS frame from the stream, skipping any GREASE frames.
///
/// Returns the raw payload bytes of the HEADERS frame.
async fn read_headers_frame<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>, ConnectError> {
    loop {
        let typ = Frame(
            VarInt::read(stream)
                .await
                .map_err(|_| ConnectError::UnexpectedEnd)?,
        );
        let size = VarInt::read(stream)
            .await
            .map_err(|_| ConnectError::UnexpectedEnd)?;

        let size = size.into_inner();
        check_frame_size(typ, size)?;

        let mut payload = stream.take(size);

        if typ.is_grease() {
            let n = tokio::io::copy(&mut payload, &mut tokio::io::sink()).await?;
            if n < size {
                return Err(ConnectError::UnexpectedEnd);
            }
            continue;
        }

        let mut buf = Vec::with_capacity(size as usize);
        payload.read_to_end(&mut buf).await?;

        if buf.len() < size as usize {
            return Err(ConnectError::UnexpectedEnd);
        }

        if typ != Frame::HEADERS {
            return Err(ConnectError::UnexpectedFrame(typ));
        }

        return Ok(buf);
    }
}

mod protocol_negotiation {
    //! WebTransport sub-protocol negotiation using RFC 8941 Structured Fields,
    //!
    //! according to [draft 14](https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-14.html#section-3.3)

    use sfv::{Item, ItemSerializer, List, ListEntry, ListSerializer, Parser, StringRef};

    use crate::ConnectError;

    /// The header name for the available protocols, sent within the WebTransport Connect request.
    pub const AVAILABLE_NAME: &str = "wt-available-protocols";
    /// The header name for the selected protocol, sent within the WebTransport Connect response.
    pub const SELECTED_NAME: &str = "wt-protocol";

    /// Encode a list of protocol strings as an RFC 8941 Structured Field List.
    pub fn encode_list(protocols: &[String]) -> Result<String, ConnectError> {
        let mut serializer = ListSerializer::new();
        for protocol in protocols {
            let s = StringRef::from_str(protocol)?;
            let _ = serializer.bare_item(s);
        }
        serializer.finish().ok_or(ConnectError::InvalidProtocol)
    }

    /// Decode an RFC 8941 Structured Field List of strings.
    pub fn decode_list(value: &str) -> Result<Vec<String>, ConnectError> {
        let list = Parser::new(value).parse::<List>()?;

        list.iter()
            .map(|entry| match entry {
                ListEntry::Item(item) => Ok(item
                    .bare_item
                    .as_string()
                    .ok_or(ConnectError::InvalidProtocol)?
                    .as_str()
                    .to_string()),
                _ => Err(ConnectError::InvalidProtocol),
            })
            .collect()
    }

    /// Encode a single string as an RFC 8941 Structured Field Item.
    pub fn encode_item(protocol: &str) -> Result<String, ConnectError> {
        let s = StringRef::from_str(protocol)?;
        Ok(ItemSerializer::new().bare_item(s).finish())
    }

    /// Decode an RFC 8941 Structured Field Item (single string).
    pub fn decode_item(value: &str) -> Result<String, ConnectError> {
        let item = Parser::new(value).parse::<Item>()?;
        Ok(item
            .bare_item
            .as_string()
            .ok_or(ConnectError::InvalidProtocol)?
            .as_str()
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Build a framed CONNECT request on the wire.
    fn encode_request(url: &str) -> Vec<u8> {
        let req = ConnectRequest::new(Url::parse(url).unwrap());
        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();
        buf
    }

    /// Build a framed CONNECT response on the wire.
    fn encode_response() -> Vec<u8> {
        let resp = ConnectResponse::OK;
        let mut buf = Vec::new();
        resp.encode(&mut buf).unwrap();
        buf
    }

    /// Encode a GREASE frame: type (0x21 = first GREASE value) + length + payload.
    fn encode_grease_frame(payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        VarInt::from_u32(0x21).encode(&mut buf); // GREASE frame type
        VarInt::from_u32(payload.len() as u32).encode(&mut buf);
        buf.extend_from_slice(payload);
        buf
    }

    fn reframe(payload: &[u8]) -> Vec<u8> {
        let mut wire = Vec::new();
        Frame::HEADERS.encode(&mut wire);
        VarInt::from_u32(payload.len() as u32).encode(&mut wire);
        wire.extend_from_slice(payload);
        wire
    }

    fn payload(mut wire: &[u8]) -> Vec<u8> {
        let (_, mut data) = Frame::read(&mut wire).unwrap();
        data.copy_to_bytes(data.remaining()).to_vec()
    }

    fn append_literal(payload: &mut Vec<u8>, name: &[u8], value: &[u8]) {
        qpack::encode_prefix(payload, 3, 0b00100, name.len());
        payload.extend_from_slice(name);
        qpack::encode_prefix(payload, 7, 0, value.len());
        payload.extend_from_slice(value);
    }

    fn assert_section_too_large<T: std::fmt::Debug>(result: Result<T, ConnectError>) {
        assert_eq!(
            result
                .expect_err("oversized section was accepted")
                .to_string(),
            "field section too large"
        );
    }

    #[test]
    fn connect_rejects_literal_field_accordion() {
        let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
        for base in [encode_request("https://example.com/"), encode_response()] {
            let mut headers = payload(&base);
            for i in 0..4_000 {
                append_literal(&mut headers, &[alphabet[i / 64], alphabet[i % 64]], b"");
            }
            assert!(headers.len() < 16 * 1024);
            let wire = reframe(&headers);
            assert_section_too_large(ConnectRequest::decode(&mut wire.as_slice()));
            assert_section_too_large(ConnectResponse::decode(&mut wire.as_slice()));
        }
    }

    #[test]
    fn response_charges_repeated_static_fields() {
        let mut headers = payload(&encode_response());
        // The indexed `:status: 200` line is one byte but costs 42 decoded bytes.
        let status = headers[2];
        assert_eq!(status, 0b1100_0000 | 25);
        headers.resize(headers.len() + 500, status);
        assert!(headers.len() < MAX_FIELD_SECTION_SIZE as usize);
        assert_section_too_large(ConnectResponse::decode(&mut reframe(&headers).as_slice()));
    }

    #[tokio::test]
    async fn request_encode_refuses_field_section_over_limit() {
        // The pseudo-headers for https://example.com/ cost 234 bytes; `x-pad` costs 37 plus its value.
        let fill = MAX_FIELD_SECTION_SIZE as usize - 234 - 37;
        let request = |len: usize| {
            ConnectRequest::new(Url::parse("https://example.com/").unwrap()).with_header(
                http::HeaderName::from_static("x-pad"),
                http::HeaderValue::from_str(&"a".repeat(len)).unwrap(),
            )
        };

        let mut wire = Vec::new();
        request(fill).encode(&mut wire).unwrap();
        let decoded = ConnectRequest::decode(&mut wire.as_slice()).unwrap();
        assert_eq!(decoded.headers["x-pad"].len(), fill);

        let mut wire = Vec::new();
        assert_section_too_large(request(fill + 1).encode(&mut wire));
        assert_section_too_large(request(fill + 1).write(&mut wire).await);
        assert!(
            wire.is_empty(),
            "a refused request wrote {} bytes",
            wire.len()
        );
    }

    #[tokio::test]
    async fn response_encode_refuses_field_section_over_limit() {
        let response =
            ConnectResponse::OK.with_protocol("a".repeat(MAX_FIELD_SECTION_SIZE as usize));
        let mut wire = Vec::new();
        assert_section_too_large(response.encode(&mut wire));
        assert_section_too_large(response.write(&mut wire).await);
        assert!(
            wire.is_empty(),
            "a refused response wrote {} bytes",
            wire.len()
        );
    }

    // RFC 7541 Appendix B gives 'a' the five-bit code 00011.
    fn huffman_a(count: usize) -> Vec<u8> {
        let bits = count * 5;
        let mut encoded = vec![0; bits.div_ceil(8)];
        for i in 0..count {
            for bit in [3, 4] {
                let position = i * 5 + bit;
                encoded[position / 8] |= 1 << (7 - position % 8);
            }
        }
        for position in bits..encoded.len() * 8 {
            encoded[position / 8] |= 1 << (7 - position % 8);
        }
        encoded
    }

    #[test]
    fn connect_rejects_huffman_expansion() {
        for base in [encode_request("https://example.com/"), encode_response()] {
            let mut headers = payload(&base);
            qpack::encode_prefix(&mut headers, 3, 0b00100, 1);
            headers.push(b'x');
            let encoded = huffman_a(17_000);
            qpack::encode_prefix(&mut headers, 7, 1, encoded.len());
            headers.extend_from_slice(&encoded);
            assert!(headers.len() < 16 * 1024);
            let wire = reframe(&headers);
            assert_section_too_large(ConnectRequest::decode(&mut wire.as_slice()));
            assert_section_too_large(ConnectResponse::decode(&mut wire.as_slice()));
        }
    }

    #[tokio::test]
    async fn connect_rejects_declared_size_before_payload_arrives() {
        let mut wire = Vec::new();
        Frame::HEADERS.encode(&mut wire);
        VarInt::from_u32(16 * 1024 + 1).encode(&mut wire);
        assert_section_too_large(ConnectRequest::decode(&mut wire.as_slice()));
        assert_section_too_large(ConnectResponse::decode(&mut wire.as_slice()));
        assert_section_too_large(ConnectRequest::read(&mut Cursor::new(wire.clone())).await);
        assert_section_too_large(ConnectResponse::read(&mut Cursor::new(wire)).await);
    }

    #[test]
    fn response_charges_duplicate_literal_names() {
        let mut headers = payload(&encode_response());
        for _ in 0..500 {
            append_literal(&mut headers, b"x", b"");
        }
        assert_section_too_large(ConnectResponse::decode(&mut reframe(&headers).as_slice()));
    }

    #[test]
    fn connect_rejects_huffman_name_expansion() {
        let mut headers = payload(&encode_request("https://example.com/"));
        let name = huffman_a(17_000);
        qpack::encode_prefix(&mut headers, 3, 0b00101, name.len());
        headers.extend_from_slice(&name);
        qpack::encode_prefix(&mut headers, 7, 0, 0);
        assert!(headers.len() < MAX_FIELD_SECTION_SIZE as usize);
        let wire = reframe(&headers);
        assert_section_too_large(ConnectRequest::decode(&mut wire.as_slice()));
        assert_section_too_large(ConnectResponse::decode(&mut wire.as_slice()));
    }

    #[tokio::test]
    async fn grease_keeps_its_frame_budget_and_skips_iteratively() {
        let mut grease = encode_grease_frame(&vec![0; 20 * 1024]);
        for _ in 0..10_000 {
            grease.extend_from_slice(&encode_grease_frame(b""));
        }
        for base in [encode_request("https://example.com/"), encode_response()] {
            let mut wire = grease.clone();
            wire.extend_from_slice(&base);
            if base == encode_response() {
                assert_eq!(
                    ConnectResponse::decode(&mut wire.as_slice())
                        .unwrap()
                        .status,
                    http::StatusCode::OK
                );
                assert_eq!(
                    ConnectResponse::read(&mut Cursor::new(wire))
                        .await
                        .unwrap()
                        .status,
                    http::StatusCode::OK
                );
            } else {
                assert_eq!(
                    ConnectRequest::decode(&mut wire.as_slice())
                        .unwrap()
                        .url
                        .as_str(),
                    "https://example.com/"
                );
                assert_eq!(
                    ConnectRequest::read(&mut Cursor::new(wire))
                        .await
                        .unwrap()
                        .url
                        .as_str(),
                    "https://example.com/"
                );
            }
        }
        let wire = encode_grease_frame(&vec![0; MAX_FRAME_SIZE as usize + 1]);
        assert!(matches!(
            ConnectRequest::decode(&mut wire.as_slice()),
            Err(ConnectError::FrameTooLarge)
        ));
        assert!(matches!(
            ConnectResponse::read(&mut Cursor::new(wire)).await,
            Err(ConnectError::FrameTooLarge)
        ));
    }

    #[test]
    fn declared_size_is_checked_without_pointer_width_truncation() {
        let mut wire = Vec::new();
        Frame::HEADERS.encode(&mut wire);
        VarInt::try_from((1_u64 << 32) + 2)
            .unwrap()
            .encode(&mut wire);
        wire.extend_from_slice(&[0, 0]);
        assert_section_too_large(ConnectRequest::decode(&mut wire.as_slice()));
        assert_section_too_large(ConnectResponse::decode(&mut wire.as_slice()));
    }

    // ---- ConnectRequest::read tests ----

    #[tokio::test]
    async fn request_read_exact_consumption() {
        // Verify read() consumes only the frame bytes, not trailing data.
        let mut wire = encode_request("https://example.com/path");
        let trailing = b"trailing data";
        wire.extend_from_slice(trailing);

        let mut cursor = Cursor::new(wire);
        let req = ConnectRequest::read(&mut cursor).await.unwrap();
        assert_eq!(req.url.as_str(), "https://example.com/path");

        // The cursor should be positioned right after the frame, leaving trailing data.
        let pos = cursor.position() as usize;
        let remaining = &cursor.into_inner()[pos..];
        assert_eq!(remaining, trailing);
    }

    #[tokio::test]
    async fn request_read_roundtrip() {
        let wire = encode_request("https://example.com/foo?bar=1");
        let mut cursor = Cursor::new(wire);
        let req = ConnectRequest::read(&mut cursor).await.unwrap();
        assert_eq!(req.url.as_str(), "https://example.com/foo?bar=1");
    }

    #[tokio::test]
    async fn request_read_skips_grease() {
        // Prepend a GREASE frame before the real HEADERS frame.
        let mut wire = encode_grease_frame(b"junk");
        wire.extend_from_slice(&encode_request("https://example.com/"));

        let mut cursor = Cursor::new(wire);
        let req = ConnectRequest::read(&mut cursor).await.unwrap();
        assert_eq!(req.url.as_str(), "https://example.com/");
    }

    #[tokio::test]
    async fn request_read_rejects_frame_too_large() {
        // Craft a frame header claiming a huge payload.
        let mut wire = Vec::new();
        Frame::HEADERS.encode(&mut wire);
        // Refuse the HEADERS length before reading any payload.
        VarInt::from_u32(128 * 1024).encode(&mut wire);

        let mut cursor = Cursor::new(wire);
        let err = ConnectRequest::read(&mut cursor).await.unwrap_err();
        assert!(
            matches!(err, ConnectError::FieldSectionTooLarge),
            "expected FieldSectionTooLarge, got {err:?}"
        );
    }

    #[tokio::test]
    async fn request_read_rejects_wrong_frame_type() {
        let mut wire = Vec::new();
        Frame::DATA.encode(&mut wire);
        VarInt::from_u32(0).encode(&mut wire);

        let mut cursor = Cursor::new(wire);
        let err = ConnectRequest::read(&mut cursor).await.unwrap_err();
        assert!(
            matches!(err, ConnectError::UnexpectedFrame(f) if f == Frame::DATA),
            "expected UnexpectedFrame(DATA), got {err:?}"
        );
    }

    #[tokio::test]
    async fn request_read_empty_stream() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let err = ConnectRequest::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, ConnectError::UnexpectedEnd));
    }

    // ---- ConnectResponse::read tests ----

    #[tokio::test]
    async fn response_read_exact_consumption() {
        let mut wire = encode_response();
        let trailing = b"extra bytes";
        wire.extend_from_slice(trailing);

        let mut cursor = Cursor::new(wire);
        let resp = ConnectResponse::read(&mut cursor).await.unwrap();
        assert_eq!(resp.status, http::StatusCode::OK);

        let pos = cursor.position() as usize;
        let remaining = &cursor.into_inner()[pos..];
        assert_eq!(remaining, trailing);
    }

    #[tokio::test]
    async fn response_read_roundtrip() {
        let wire = encode_response();
        let mut cursor = Cursor::new(wire);
        let resp = ConnectResponse::read(&mut cursor).await.unwrap();
        assert_eq!(resp.status, http::StatusCode::OK);
    }

    #[tokio::test]
    async fn response_read_skips_grease() {
        let mut wire = encode_grease_frame(b"grease");
        wire.extend_from_slice(&encode_response());

        let mut cursor = Cursor::new(wire);
        let resp = ConnectResponse::read(&mut cursor).await.unwrap();
        assert_eq!(resp.status, http::StatusCode::OK);
    }

    #[tokio::test]
    async fn response_read_rejects_frame_too_large() {
        let mut wire = Vec::new();
        Frame::HEADERS.encode(&mut wire);
        VarInt::from_u32(128 * 1024).encode(&mut wire);

        let mut cursor = Cursor::new(wire);
        let err = ConnectResponse::read(&mut cursor).await.unwrap_err();
        assert!(
            matches!(err, ConnectError::FieldSectionTooLarge),
            "expected FieldSectionTooLarge, got {err:?}"
        );
    }

    #[tokio::test]
    async fn response_read_rejects_wrong_frame_type() {
        let mut wire = Vec::new();
        Frame::SETTINGS.encode(&mut wire);
        VarInt::from_u32(0).encode(&mut wire);

        let mut cursor = Cursor::new(wire);
        let err = ConnectResponse::read(&mut cursor).await.unwrap_err();
        assert!(
            matches!(err, ConnectError::UnexpectedFrame(f) if f == Frame::SETTINGS),
            "expected UnexpectedFrame(SETTINGS), got {err:?}"
        );
    }

    #[tokio::test]
    async fn response_read_empty_stream() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let err = ConnectResponse::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, ConnectError::UnexpectedEnd));
    }

    // ---- Truncated payload tests ----

    #[tokio::test]
    async fn request_read_truncated_payload() {
        // Frame header claims 100 bytes but only 5 are present.
        let mut wire = Vec::new();
        Frame::HEADERS.encode(&mut wire);
        VarInt::from_u32(100).encode(&mut wire);
        wire.extend_from_slice(b"short");

        let mut cursor = Cursor::new(wire);
        let err = ConnectRequest::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, ConnectError::UnexpectedEnd));
    }

    #[tokio::test]
    async fn response_read_truncated_payload() {
        let mut wire = Vec::new();
        Frame::HEADERS.encode(&mut wire);
        VarInt::from_u32(100).encode(&mut wire);
        wire.extend_from_slice(b"short");

        let mut cursor = Cursor::new(wire);
        let err = ConnectResponse::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, ConnectError::UnexpectedEnd));
    }

    #[tokio::test]
    async fn request_read_truncated_grease() {
        // GREASE frame claims 100 bytes but only 3 are present.
        let mut wire = Vec::new();
        VarInt::from_u32(0x21).encode(&mut wire);
        VarInt::from_u32(100).encode(&mut wire);
        wire.extend_from_slice(b"abc");

        let mut cursor = Cursor::new(wire);
        let err = ConnectRequest::read(&mut cursor).await.unwrap_err();
        assert!(matches!(err, ConnectError::UnexpectedEnd));
    }
}
