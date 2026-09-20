use crate::classification::is_binary_document_mime_type;
use crate::parse::{mime_essence, normalize_web_api_mime_type};

pub fn response_header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    response_header_values(headers, name).into_iter().next()
}

pub fn response_header_values(headers: &[(String, String)], name: &str) -> Vec<String> {
    let Some(name) = parsed_header_name(name) else {
        return Vec::new();
    };
    headers
        .iter()
        .filter(|(header_name, _)| header_name_matches(header_name, &name))
        .map(|(_, value)| value.to_owned())
        .collect()
}

pub fn response_content_type(headers: &[(String, String)]) -> Option<String> {
    response_header_value(headers, "content-type")
}

/// The MIME record returned by Fetch's "extract a MIME type" algorithm.
///
/// Combine Content-Type fields before splitting so quoted strings, including
/// unterminated ones, can span fields. Ignore invalid MIME values and `*/*`.
/// Repeated values with the same essence inherit the first value's charset
/// when their own charset is absent.
pub fn extract_response_mime_type(
    headers: &[(String, String)],
) -> Option<moli_content_type::MimeType> {
    let combined = response_header_values(headers, "content-type").join(", ");
    let mut quoted = false;
    let mut escaped = false;
    let mut essence = None;
    let mut charset = None;
    let mut mime_type = None;
    for value in combined.split(|character| {
        if escaped {
            escaped = false;
            false
        } else if quoted && character == '\\' {
            escaped = true;
            false
        } else if character == '"' {
            quoted = !quoted;
            false
        } else {
            character == ',' && !quoted
        }
    }) {
        let Some(mut parsed) = moli_content_type::parse_mime_type(value) else {
            continue;
        };
        let parsed_essence = parsed.essence();
        if parsed_essence == "*/*" {
            continue;
        }
        if essence.as_ref() != Some(&parsed_essence) {
            charset = parsed.parameter("charset").map(str::to_owned);
            essence = Some(parsed_essence);
        } else if parsed.parameter("charset").is_none()
            && let Some(charset) = &charset
        {
            parsed.set_parameter("charset", charset);
        }
        mime_type = Some(parsed);
    }
    mime_type
}

/// The essence returned by Fetch's "extract a MIME type" algorithm.
pub fn extract_response_mime_essence(headers: &[(String, String)]) -> Option<String> {
    extract_response_mime_type(headers).map(|mime| mime.essence())
}

pub fn response_headers_indicate_attachment_download(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        let Ok(name) = http::HeaderName::from_bytes(name.as_bytes()) else {
            return false;
        };
        name == http::header::CONTENT_DISPOSITION
            && content_disposition::parse_content_disposition(value).disposition
                == content_disposition::DispositionType::Attachment
    })
}

pub fn response_headers_indicate_binary_document(headers: &[(String, String)]) -> bool {
    response_content_type(headers)
        .as_deref()
        .is_some_and(is_binary_document_mime_type)
}

pub fn response_headers_indicate_raw_document(headers: &[(String, String)]) -> bool {
    response_headers_indicate_attachment_download(headers)
        || response_headers_indicate_binary_document(headers)
}

pub fn response_document_content_type(headers: &[(String, String)]) -> Option<String> {
    let content_type = response_header_values(headers, "content-type")
        .into_iter()
        .last()?;
    (!content_type.trim().is_empty())
        .then(|| mime_essence(&content_type))
        .flatten()
}

pub fn effective_response_mime_type(
    headers: &[(String, String)],
    override_mime_type: Option<&str>,
) -> Option<String> {
    override_mime_type
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| response_content_type(headers))
}

pub fn effective_response_mime_essence(
    headers: &[(String, String)],
    override_mime_type: Option<&str>,
) -> Option<String> {
    effective_response_mime_type(headers, override_mime_type)
        .as_deref()
        .and_then(mime_essence)
}

pub fn response_blob_mime_type(headers: &[(String, String)]) -> String {
    normalize_response_blob_mime_type(response_content_type(headers).as_deref())
}

pub fn normalize_response_blob_mime_type(content_type: Option<&str>) -> String {
    content_type
        .map(normalize_web_api_mime_type)
        .unwrap_or_default()
}

fn parsed_header_name(name: &str) -> Option<http::HeaderName> {
    http::HeaderName::from_bytes(name.as_bytes()).ok()
}

fn header_name_matches(candidate: &str, expected: &http::HeaderName) -> bool {
    parsed_header_name(candidate).is_some_and(|candidate| candidate == *expected)
}
