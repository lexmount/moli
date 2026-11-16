use std::borrow::Cow;

pub use moli_fetch_types::header_policy::{
    cors_unsafe_request_header_names, is_cors_safelisted_method,
    is_cors_safelisted_request_content_type, is_cors_safelisted_request_header,
    is_cors_safelisted_request_range, is_cors_unsafe_request_header_byte,
    is_forbidden_request_header_name, is_forbidden_request_header_override_value,
    is_forbidden_response_header_name, is_no_cors_safelisted_request_header,
    split_http_header_list,
};

/// Accept policy is distinct from Fetch's destination: EventSource has an
/// empty destination but requires a dedicated media type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultAcceptKind {
    Navigation,
    Image,
    EventSource,
    TextTrack,
    TextModule,
    Other,
}

/// The browser's default Accept value before interception or network dispatch.
pub fn default_accept_header(kind: DefaultAcceptKind) -> &'static str {
    match kind {
        DefaultAcceptKind::Navigation => {
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"
        }
        DefaultAcceptKind::Image => {
            "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8"
        }
        DefaultAcceptKind::EventSource => "text/event-stream",
        DefaultAcceptKind::TextTrack => "text/vtt,*/*;q=0.1",
        DefaultAcceptKind::TextModule => "text/plain,*/*;q=0.5",
        DefaultAcceptKind::Other => "*/*",
    }
}

// Header strings represent HTTP bytes one-for-one, including obs-text. UTF-8
// decoding would either replace bytes or combine a valid multibyte sequence.
pub(crate) fn decode_http_header_bytes(data: &[u8]) -> Cow<'_, str> {
    moli_header_field::decode_header_value(data)
}

/// Preserve raw Location bytes before URL parsing, as Chromium does. Ordinary
/// HTTP fields still use isomorphic decoding; interpreting these bytes as
/// Unicode would re-encode UTF-8 and corrupt invalid byte sequences.
pub fn encode_redirect_location(value: &[u8]) -> Cow<'_, str> {
    percent_encoding::percent_encode(value, &percent_encoding::AsciiSet::EMPTY).into()
}

pub(crate) fn parse_http_response_header_line(data: &[u8]) -> Option<(String, Vec<u8>)> {
    let end = data
        .iter()
        .rposition(|byte| !matches!(byte, b'\r' | b'\n'))
        .map_or(0, |index| index + 1);
    let line = &data[..end];
    let separator = line.iter().position(|byte| *byte == b':')?;
    let name = trim_http_ows(&line[..separator]);
    if name.is_empty() {
        return None;
    }
    let value = trim_http_ows(&line[separator + 1..]);
    Some((decode_http_header_bytes(name).into_owned(), value.to_vec()))
}

fn trim_http_ows(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(bytes.len());
    let bytes = &bytes[start..];
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t'))
        .map_or(0, |index| index + 1);
    &bytes[..end]
}

pub(crate) fn parse_http_header_line(line: &str) -> Option<(String, String)> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (name, value) = line.split_once(':')?;
    let name = name.trim_matches([' ', '\t']);
    if name.is_empty() {
        return None;
    }
    // Only HTTP OWS is framing. NBSP and NEL are ordinary header bytes.
    Some((name.to_owned(), value.trim_matches([' ', '\t']).to_owned()))
}
