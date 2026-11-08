use crate::parse::find_bytes;
use crate::types::{MultipartFormDataPart, MultipartFormDataPartValue};
use encoding_rs::Encoding;
use moli_encoding::encode_text_for_legacy_web;
use std::borrow::Cow;

pub fn serialize_multipart_form_data_with_prefix(
    parts: &[MultipartFormDataPart],
    boundary_prefix: &str,
) -> (Vec<u8>, String) {
    serialize_multipart_form_data_with_encoding(parts, boundary_prefix, encoding_rs::UTF_8)
}

pub fn serialize_multipart_form_data_with_encoding(
    parts: &[MultipartFormDataPart],
    boundary_prefix: &str,
    encoding: &'static Encoding,
) -> (Vec<u8>, String) {
    let boundary = multipart_boundary(parts, boundary_prefix, encoding);
    let body = serialize_multipart_form_data_body(parts, &boundary, encoding);
    (body, format!("multipart/form-data; boundary={boundary}"))
}

pub(crate) fn serialize_multipart_form_data_body(
    parts: &[MultipartFormDataPart],
    boundary: &str,
    encoding: &'static Encoding,
) -> Vec<u8> {
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(b"--");
        body.extend_from_slice(boundary.as_bytes());
        body.extend_from_slice(b"\r\nContent-Disposition: form-data; name=\"");
        let name = normalize_multipart_newlines(&part.name);
        let name = encode_text_for_legacy_web(&name, encoding);
        body.extend_from_slice(&escape_multipart_name(&name));
        body.push(b'"');
        match &part.value {
            MultipartFormDataPartValue::Text(value) => {
                body.extend_from_slice(b"\r\n\r\n");
                let value = normalize_multipart_newlines(value);
                body.extend_from_slice(&encode_text_for_legacy_web(&value, encoding));
            }
            MultipartFormDataPartValue::Blob {
                filename,
                content_type,
                body: value,
            } => {
                body.extend_from_slice(b"; filename=\"");
                // Filenames are encoded and escaped, but their line breaks
                // are not normalized like field names and text values.
                let filename = encode_text_for_legacy_web(filename, encoding);
                body.extend_from_slice(&escape_multipart_name(&filename));
                body.push(b'"');
                let content_type = if content_type.is_empty() {
                    super::types::DEFAULT_MULTIPART_BLOB_CONTENT_TYPE
                } else {
                    content_type
                };
                body.extend_from_slice(b"\r\nContent-Type: ");
                body.extend_from_slice(content_type.as_bytes());
                body.extend_from_slice(b"\r\n\r\n");
                body.extend_from_slice(value);
            }
        }
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");
    body
}

fn multipart_boundary(
    parts: &[MultipartFormDataPart],
    boundary_prefix: &str,
    encoding: &'static Encoding,
) -> String {
    multipart_boundary_with_candidate(parts, encoding, |salt| {
        multipart_boundary_candidate(parts, salt, boundary_prefix)
    })
}

pub(crate) fn multipart_boundary_with_candidate(
    parts: &[MultipartFormDataPart],
    encoding: &'static Encoding,
    mut candidate: impl FnMut(u64) -> String,
) -> String {
    let mut salt = 0u64;
    loop {
        let boundary = candidate(salt);
        if !multipart_boundary_collides(parts, &boundary, encoding) {
            return boundary;
        }
        salt = salt.wrapping_add(1);
    }
}

fn multipart_boundary_candidate(
    parts: &[MultipartFormDataPart],
    salt: u64,
    boundary_prefix: &str,
) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for part in parts {
        hash_part(&mut hash, part.name.bytes());
        match &part.value {
            MultipartFormDataPartValue::Text(value) => {
                hash_part(&mut hash, value.bytes());
            }
            MultipartFormDataPartValue::Blob {
                filename,
                content_type,
                body,
            } => {
                hash_part(&mut hash, filename.bytes());
                hash_part(&mut hash, content_type.bytes());
                hash_part(&mut hash, body.iter().copied());
            }
        }
    }
    if salt > 0 {
        hash_part(&mut hash, salt.to_le_bytes());
    }
    format!("{boundary_prefix}{hash:016x}")
}

fn hash_part(hash: &mut u64, bytes: impl IntoIterator<Item = u8>) {
    for byte in bytes {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    *hash ^= 0;
    *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
}

pub(crate) fn multipart_boundary_collides(
    parts: &[MultipartFormDataPart],
    boundary: &str,
    encoding: &'static Encoding,
) -> bool {
    parts.iter().any(|part| {
        let name = normalize_multipart_newlines(&part.name);
        let name = encode_text_for_legacy_web(&name, encoding);
        find_bytes(&escape_multipart_name(&name), boundary.as_bytes()).is_some()
            || match &part.value {
                MultipartFormDataPartValue::Text(value) => {
                    let value = normalize_multipart_newlines(value);
                    find_bytes(
                        &encode_text_for_legacy_web(&value, encoding),
                        boundary.as_bytes(),
                    )
                    .is_some()
                }
                MultipartFormDataPartValue::Blob {
                    filename,
                    content_type,
                    body,
                } => {
                    let filename = encode_text_for_legacy_web(filename, encoding);
                    find_bytes(&escape_multipart_name(&filename), boundary.as_bytes()).is_some()
                        || content_type.contains(boundary)
                        || find_bytes(body, boundary.as_bytes()).is_some()
                }
            }
    })
}

pub(crate) fn escape_multipart_name(value: &[u8]) -> Vec<u8> {
    let mut escaped = Vec::with_capacity(value.len());
    for &byte in value {
        match byte {
            b'\n' => escaped.extend_from_slice(b"%0A"),
            b'\r' => escaped.extend_from_slice(b"%0D"),
            b'"' => escaped.extend_from_slice(b"%22"),
            _ => escaped.push(byte),
        }
    }
    escaped
}

fn normalize_multipart_newlines(value: &str) -> Cow<'_, str> {
    if !value.bytes().any(|byte| matches!(byte, b'\r' | b'\n')) {
        return Cow::Borrowed(value);
    }
    let mut normalized = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                normalized.push_str("\r\n");
                if chars.peek() == Some(&'\n') {
                    let _ = chars.next();
                }
            }
            '\n' => normalized.push_str("\r\n"),
            _ => normalized.push(ch),
        }
    }
    Cow::Owned(normalized)
}
