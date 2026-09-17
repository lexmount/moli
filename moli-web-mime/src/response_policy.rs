use crate::classification::{
    is_audio_mime_essence, is_css_mime, is_font_mime_essence, is_image_mime_essence,
    is_javascript_mime, is_video_mime_essence,
};
use crate::destination::FetchDestination;
use crate::headers::response_header_value;
use crate::parse::{mime_charset, mime_essence};
use crate::sniffing::{MimeSniffingContext, computed_mime_type, sniff_image_mime_type};

pub fn determine_nosniff(headers: &[(String, Vec<u8>)]) -> bool {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-content-type-options"))
        .is_some_and(|(_, value)| {
            moli_header_field::decode_header_value(value)
                .split(',')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("nosniff"))
        })
}

pub fn should_response_be_blocked_due_to_nosniff(
    headers: &[(String, Vec<u8>)],
    destination: FetchDestination,
) -> bool {
    if !determine_nosniff(headers) {
        return false;
    }

    let content_type = response_header_value(headers, "content-type");
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
    let content_type = response_header_value(headers, "content-type");
    if determine_nosniff(headers)
        && content_type
            .as_deref()
            .is_none_or(|content_type| content_type.trim().is_empty())
    {
        return true;
    }

    let Some(content_type) = content_type else {
        return false;
    };
    let Some(essence) = mime_essence(&content_type) else {
        return determine_nosniff(headers);
    };
    if is_javascript_mime(&content_type) || is_css_mime(&content_type) || essence == "image/svg+xml"
    {
        return false;
    }

    essence == "text/plain"
        || essence == "text/html"
        || essence == "text/xml"
        || essence == "application/xml"
        || essence == "application/xhtml+xml"
        || essence == "application/json"
        || essence == "text/json"
        || essence.ends_with("+json")
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
        || matches!(
            essence.as_str(),
            "application/msexcel"
                | "application/mspowerpoint"
                | "application/msword"
                | "application/msword-template"
                | "application/vnd.apple.mpegurl"
                | "application/vnd.ces-quickpoint"
                | "application/vnd.ces-quicksheet"
                | "application/vnd.ces-quickword"
                | "application/vnd.ms-excel"
                | "application/vnd.ms-excel.sheet.macroenabled.12"
                | "application/vnd.ms-powerpoint"
                | "application/vnd.ms-powerpoint.presentation.macroenabled.12"
                | "application/vnd.ms-word"
                | "application/vnd.ms-word.document.12"
                | "application/vnd.ms-word.document.macroenabled.12"
                | "application/vnd.msword"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                | "application/vnd.openxmlformats-officedocument.presentationml.template"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.template"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.template"
                | "application/vnd.presentation-openxml"
                | "application/vnd.presentation-openxmlm"
                | "application/vnd.spreadsheet-openxml"
                | "application/vnd.wordprocessing-openxml"
        )
}

pub fn should_opaque_response_be_blocked_by_orb_with_body(
    headers: &[(String, Vec<u8>)],
    body: &[u8],
) -> bool {
    if !should_opaque_response_be_blocked_by_orb(headers) {
        return false;
    }

    if sniff_image_mime_type(body).is_some() {
        return false;
    }

    let Some(content_type) = response_header_value(headers, "content-type") else {
        return true;
    };
    let Some(essence) = mime_essence(&content_type) else {
        return true;
    };
    if is_json_like_mime_essence(&essence)
        && response_body_looks_like_orb_allowed_javascript(&content_type, body)
    {
        return false;
    }

    true
}

pub fn computed_response_mime_type(
    headers: &[(String, Vec<u8>)],
    context: MimeSniffingContext,
    body: &[u8],
) -> String {
    let content_type = response_header_value(headers, "content-type");
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
        let supplied_mime_type = response_header_value(headers, "content-type")
            .as_deref()
            .and_then(mime_essence)
            .unwrap_or_default();
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
    let Some(content_type) = response_header_value(headers, "content-type") else {
        return false;
    };
    let Some(essence) = mime_essence(&content_type) else {
        return false;
    };
    is_audio_mime_essence(&essence)
        || is_image_mime_essence(&essence)
        || is_video_mime_essence(&essence)
        || essence == "text/csv"
}

fn is_json_like_mime_essence(essence: &str) -> bool {
    essence == "application/json" || essence == "text/json" || essence.ends_with("+json")
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
    let units = body.chunks_exact(2).map(|chunk| {
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
