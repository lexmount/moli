use base64::{Engine as _, engine::general_purpose::STANDARD};
use moli_web_mime::{mime_charset, normalize_web_api_mime_type};

mod blob_store;
pub mod data_transfer;
pub mod file;

pub use blob_store::{BlobId, BlobStore, ObjectUrlData, ObjectUrlTarget};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlobLineEndings {
    Transparent,
    Native,
}

impl BlobLineEndings {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "transparent" => Some(Self::Transparent),
            "native" => Some(Self::Native),
            _ => None,
        }
    }
}

pub fn normalize_blob_line_endings(value: &str, endings: BlobLineEndings) -> String {
    normalize_blob_line_endings_with_native_ending(value, endings, "\n")
}

pub fn normalize_blob_line_endings_with_native_ending(
    value: &str,
    endings: BlobLineEndings,
    native_ending: &str,
) -> String {
    match endings {
        BlobLineEndings::Transparent => value.to_owned(),
        BlobLineEndings::Native => value
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', native_ending),
    }
}

pub fn normalize_blob_mime_type(raw: &str) -> String {
    normalize_web_api_mime_type(raw)
}

pub fn clamp_blob_long_long(value: f64) -> i64 {
    if value.is_nan() || value == 0.0 {
        return 0;
    }
    if value <= i64::MIN as f64 {
        return i64::MIN;
    }
    if value >= i64::MAX as f64 {
        return i64::MAX;
    }
    value.round_ties_even() as i64
}

pub fn blob_slice_relative_index(index: i64, size: usize) -> usize {
    if index < 0 {
        size.saturating_sub((-index) as usize)
    } else {
        (index as usize).min(size)
    }
}

pub fn file_reader_data_url(bytes: &[u8], mime_type: &str) -> String {
    let mime = if mime_type.is_empty() {
        "application/octet-stream"
    } else {
        mime_type
    };
    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
}

pub fn file_reader_binary_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

pub fn file_reader_text(
    bytes: &[u8],
    explicit_encoding_label: Option<&str>,
    mime_type: &str,
) -> String {
    let encoding = explicit_encoding_label
        .and_then(file_reader_text_encoding_from_label)
        .or_else(|| file_reader_text_encoding_from_mime_type(mime_type))
        .or_else(|| file_reader_text_encoding_from_bom(bytes))
        .unwrap_or(FileReaderTextEncoding::Utf8);
    let bytes = strip_matching_file_reader_bom(bytes, encoding);
    match encoding {
        FileReaderTextEncoding::Utf8 => {
            let (text, _, _) = encoding_rs::UTF_8.decode(bytes);
            text.into_owned()
        }
        FileReaderTextEncoding::Utf16Le => {
            let (text, _, _) = encoding_rs::UTF_16LE.decode(bytes);
            text.into_owned()
        }
        FileReaderTextEncoding::Utf16Be => {
            let (text, _, _) = encoding_rs::UTF_16BE.decode(bytes);
            text.into_owned()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileReaderTextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

fn file_reader_text_encoding_from_mime_type(mime_type: &str) -> Option<FileReaderTextEncoding> {
    mime_charset(mime_type)
        .as_deref()
        .and_then(file_reader_text_encoding_from_label)
}

fn file_reader_text_encoding_from_label(label: &str) -> Option<FileReaderTextEncoding> {
    let label = label.trim().to_ascii_lowercase();
    match label.as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => Some(FileReaderTextEncoding::Utf8),
        "utf-16" | "utf-16le" => Some(FileReaderTextEncoding::Utf16Le),
        "utf-16be" => Some(FileReaderTextEncoding::Utf16Be),
        _ => None,
    }
}

fn file_reader_text_encoding_from_bom(bytes: &[u8]) -> Option<FileReaderTextEncoding> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some(FileReaderTextEncoding::Utf8)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some(FileReaderTextEncoding::Utf16Be)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Some(FileReaderTextEncoding::Utf16Le)
    } else {
        None
    }
}

fn strip_matching_file_reader_bom(bytes: &[u8], encoding: FileReaderTextEncoding) -> &[u8] {
    match encoding {
        FileReaderTextEncoding::Utf8 if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) => &bytes[3..],
        FileReaderTextEncoding::Utf16Be if bytes.starts_with(&[0xFE, 0xFF]) => &bytes[2..],
        FileReaderTextEncoding::Utf16Le if bytes.starts_with(&[0xFF, 0xFE]) => &bytes[2..],
        _ => bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_line_endings_parse_standard_tokens() {
        assert_eq!(
            BlobLineEndings::parse("transparent"),
            Some(BlobLineEndings::Transparent)
        );
        assert_eq!(
            BlobLineEndings::parse("native"),
            Some(BlobLineEndings::Native)
        );
        assert_eq!(BlobLineEndings::parse("Native"), None);
        assert_eq!(BlobLineEndings::parse("unknown"), None);
    }

    #[test]
    fn blob_line_endings_normalize_native_newlines() {
        assert_eq!(
            normalize_blob_line_endings("a\r\nb\rc\nd", BlobLineEndings::Native),
            "a\nb\nc\nd"
        );
        assert_eq!(
            normalize_blob_line_endings_with_native_ending(
                "a\r\nb\rc\nd",
                BlobLineEndings::Native,
                "\r\n"
            ),
            "a\r\nb\r\nc\r\nd"
        );
        assert_eq!(
            normalize_blob_line_endings("a\r\nb", BlobLineEndings::Transparent),
            "a\r\nb"
        );
    }

    #[test]
    fn blob_slice_clamped_long_long_uses_webidl_half_to_even_rounding() {
        assert_eq!(clamp_blob_long_long(f64::NAN), 0);
        assert_eq!(clamp_blob_long_long(f64::INFINITY), i64::MAX);
        assert_eq!(clamp_blob_long_long(f64::NEG_INFINITY), i64::MIN);
        assert_eq!(clamp_blob_long_long(0.5), 0);
        assert_eq!(clamp_blob_long_long(1.5), 2);
        assert_eq!(clamp_blob_long_long(2.5), 2);
        assert_eq!(clamp_blob_long_long(3.5), 4);
        assert_eq!(clamp_blob_long_long(-1.5), -2);
    }

    #[test]
    fn blob_slice_relative_index_clamps_to_size() {
        assert_eq!(blob_slice_relative_index(3, 10), 3);
        assert_eq!(blob_slice_relative_index(20, 10), 10);
        assert_eq!(blob_slice_relative_index(-3, 10), 7);
        assert_eq!(blob_slice_relative_index(-20, 10), 0);
    }

    #[test]
    fn blob_mime_type_uses_web_api_normalization() {
        assert_eq!(
            normalize_blob_mime_type("Text/Plain; Charset=UTF-8"),
            "text/plain; charset=utf-8"
        );
        assert_eq!(normalize_blob_mime_type("text/plain\n"), "");
    }

    #[test]
    fn file_reader_data_url_defaults_empty_mime_type_to_octet_stream() {
        assert_eq!(
            file_reader_data_url(b"hello", ""),
            "data:application/octet-stream;base64,aGVsbG8="
        );
    }

    #[test]
    fn file_reader_data_url_preserves_blob_mime_type() {
        assert_eq!(
            file_reader_data_url(b"hello", "text/plain;charset=utf-8"),
            "data:text/plain;charset=utf-8;base64,aGVsbG8="
        );
    }

    #[test]
    fn file_reader_binary_string_maps_bytes_to_code_units() {
        assert_eq!(file_reader_binary_string(&[0x41, 0x00, 0xFF]), "A\0\u{ff}");
    }

    #[test]
    fn file_reader_text_encoding_uses_mime_charset_parser() {
        assert_eq!(
            file_reader_text_encoding_from_mime_type("Text/Plain ; Charset=\"utf-16le\""),
            Some(FileReaderTextEncoding::Utf16Le)
        );
        assert_eq!(
            file_reader_text_encoding_from_mime_type("text/plain; charset=utf-16be"),
            Some(FileReaderTextEncoding::Utf16Be)
        );
        assert_eq!(
            file_reader_text_encoding_from_mime_type("charset=utf-8"),
            None,
            "invalid MIME strings should not provide a charset"
        );
    }

    #[test]
    fn file_reader_text_prefers_explicit_and_mime_encoding_before_bom() {
        assert_eq!(
            file_reader_text(&[b'A', 0x00], Some("utf-8"), "text/plain;charset=utf-16le"),
            "A\0"
        );
        assert_eq!(
            file_reader_text(&[b'A', 0x00], None, "text/plain;charset=utf-16le"),
            "A"
        );
        assert_eq!(file_reader_text(&[0xFF, 0xFE, b'A', 0x00], None, ""), "A");
    }
}
