use crate::classification::{
    is_audio_mime_essence, is_css_mime, is_font_mime_essence, is_image_mime_essence,
    is_javascript_mime, is_json_mime_essence, is_office_document_mime_essence,
    is_video_mime_essence,
};
use crate::destination::FetchDestination;
use crate::headers::{extract_response_mime_essence, extract_response_mime_type};
use crate::parse::mime_charset;
use crate::sniffing::{MimeSniffingContext, computed_mime_type, sniff_image_mime_type};

pub fn determine_nosniff(headers: &[(String, Vec<u8>)]) -> bool {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-content-type-options"))
        .is_some_and(|(_, value)| {
            moli_header_field::decode_header_value(value)
                .split(',')
                .next()
                .is_some_and(|value| {
                    value
                        .trim_matches(['\t', ' '])
                        .eq_ignore_ascii_case("nosniff")
                })
        })
}

/// Supplies navigation MIME metadata before a body decision is available.
///
/// Without a supplied type, `nosniff` makes the response a plain-text document.
/// Use [`crate::NavigationMimeSniffer`] to select a parser: supplied unknown and
/// plain-text types can still require a body prefix. This navigation default
/// does not apply to image or other subresources.
pub fn navigation_response_mime_type_from_headers(headers: &[(String, Vec<u8>)]) -> Option<String> {
    extract_response_mime_essence(headers)
        .or_else(|| determine_nosniff(headers).then(|| "text/plain".to_owned()))
}

pub fn should_response_be_blocked_due_to_nosniff(
    headers: &[(String, Vec<u8>)],
    destination: FetchDestination,
) -> bool {
    if !determine_nosniff(headers) {
        return false;
    }

    let content_type = extract_response_mime_essence(headers);
    if destination.is_script_like() {
        return content_type
            .as_deref()
            .is_none_or(|content_type| !is_javascript_mime(content_type));
    }
    if destination == FetchDestination::Style {
        return content_type
            .as_deref()
            .is_none_or(|content_type| !is_css_mime(content_type));
    }
    false
}

pub fn should_opaque_response_be_blocked_by_orb(headers: &[(String, Vec<u8>)]) -> bool {
    let Some(essence) = extract_response_mime_essence(headers) else {
        return determine_nosniff(headers);
    };
    if is_javascript_mime(&essence) || is_css_mime(&essence) || essence == "image/svg+xml" {
        return false;
    }

    essence == "text/plain"
        || essence == "text/html"
        || essence == "text/xml"
        || essence == "application/xml"
        || essence == "application/xhtml+xml"
        || is_json_mime_essence(&essence)
        || essence == "application/dash+xml"
        || essence == "application/gzip"
        || essence == "application/x-gzip"
        || essence == "application/pdf"
        || essence == "application/zip"
        || essence == "application/x-protobuf"
        || essence == "application/x-protobuffer"
        || essence == "audio/mpegurl"
        || essence == "multipart/byteranges"
        || essence == "multipart/signed"
        || essence == "text/event-stream"
        || essence == "text/csv"
        || essence == "text/vtt"
        || is_font_mime_essence(&essence)
        || is_office_document_mime_essence(&essence)
        || matches!(
            essence.as_str(),
            "application/vnd.apple.mpegurl"
                | "application/vnd.ces-quickpoint"
                | "application/vnd.ces-quicksheet"
                | "application/vnd.ces-quickword"
        )
}

pub fn should_opaque_response_be_blocked_by_orb_with_body(
    headers: &[(String, Vec<u8>)],
    body: &[u8],
) -> bool {
    if !should_opaque_response_be_blocked_by_orb(headers) {
        return false;
    }

    // A nosniff response rejected by the header check cannot be rescued by an
    // image signature or JavaScript-looking bytes in its body.
    if determine_nosniff(headers) {
        return true;
    }

    if sniff_image_mime_type(body).is_some() {
        return false;
    }

    let Some(mime) = extract_response_mime_type(headers) else {
        return true;
    };
    let essence = mime.essence();
    !(is_json_mime_essence(&essence)
        && response_body_looks_like_orb_allowed_javascript(&mime.to_string(), body))
}

pub fn computed_response_mime_type(
    headers: &[(String, Vec<u8>)],
    context: MimeSniffingContext,
    body: &[u8],
) -> String {
    let content_type = extract_response_mime_type(headers).map(|mime| mime.to_string());
    computed_mime_type(
        content_type.as_deref(),
        determine_nosniff(headers),
        context,
        body,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptResponseMimeError {
    Nosniff,
    Unsupported(String),
}

pub fn check_script_response_mime(
    headers: &[(String, Vec<u8>)],
    body: &[u8],
    destination: FetchDestination,
    require_javascript_mime: bool,
) -> Result<(), ScriptResponseMimeError> {
    if should_response_be_blocked_due_to_nosniff(headers, destination) {
        return Err(ScriptResponseMimeError::Nosniff);
    }

    if require_javascript_mime {
        // A script-context sniffing default cannot satisfy an explicit
        // JavaScript MIME requirement, including when Content-Type is absent
        // or cannot be parsed.
        let supplied_mime_type = extract_response_mime_essence(headers).unwrap_or_default();
        return is_javascript_mime(&supplied_mime_type)
            .then_some(())
            .ok_or(ScriptResponseMimeError::Unsupported(supplied_mime_type));
    }

    if should_script_like_response_be_blocked_due_to_mime_type(headers) {
        let computed_mime_type =
            computed_response_mime_type(headers, MimeSniffingContext::Script, body);
        return Err(ScriptResponseMimeError::Unsupported(computed_mime_type));
    }
    Ok(())
}

pub fn should_script_like_response_be_blocked_due_to_mime_type(
    headers: &[(String, Vec<u8>)],
) -> bool {
    let Some(essence) = extract_response_mime_essence(headers) else {
        return false;
    };
    is_audio_mime_essence(&essence)
        || is_image_mime_essence(&essence)
        || is_video_mime_essence(&essence)
        || essence == "text/csv"
}

fn response_body_looks_like_orb_allowed_javascript(content_type: &str, body: &[u8]) -> bool {
    let text = decode_orb_script_candidate(content_type, body);
    let trimmed = text.trim_start_matches(|ch: char| ch.is_whitespace());
    if trimmed.is_empty() {
        return false;
    }
    if matches!(trimmed.as_bytes().first(), Some(b'{' | b'[')) {
        return false;
    }

    trimmed.starts_with("\"use strict\"")
        || trimmed.starts_with("'use strict'")
        || trimmed.starts_with("function ")
        || trimmed.starts_with("function\t")
        || trimmed.starts_with("function\n")
        || trimmed.starts_with("function(")
        || trimmed.starts_with("async function")
        || trimmed.starts_with("import ")
        || trimmed.starts_with("export ")
        || trimmed.starts_with("var ")
        || trimmed.starts_with("let ")
        || trimmed.starts_with("const ")
        || trimmed.starts_with("self.")
        || trimmed.starts_with("window.")
        || trimmed.starts_with("globalThis.")
        || trimmed.starts_with("!function")
        || trimmed.starts_with("(function")
        || trimmed.starts_with("(()=>")
        || trimmed.starts_with("(() =>")
}

fn decode_orb_script_candidate(content_type: &str, body: &[u8]) -> String {
    let charset = mime_charset(content_type).map(|value| value.to_ascii_lowercase());
    if body.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16_body(&body[2..], false);
    }
    if body.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16_body(&body[2..], true);
    }
    match charset.as_deref() {
        Some("utf-16") | Some("utf-16le") => decode_utf16_body(body, true),
        Some("utf-16be") => decode_utf16_body(body, false),
        _ => String::from_utf8_lossy(body).into_owned(),
    }
}

fn decode_utf16_body(body: &[u8], little_endian: bool) -> String {
    let units = body.as_chunks::<2>().0.iter().map(|chunk| {
        if little_endian {
            u16::from_le_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], chunk[1]])
        }
    });
    std::char::decode_utf16(units)
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}
