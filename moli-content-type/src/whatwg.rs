use data_url::mime::Mime as WhatwgMime;
use std::fmt;

/// A MIME type parsed according to the WHATWG MIME Sniffing Standard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MimeType {
    inner: WhatwgMime,
}

impl MimeType {
    /// The lower-case MIME top-level type.
    pub fn type_(&self) -> &str {
        &self.inner.type_
    }

    /// The lower-case MIME subtype.
    pub fn subtype(&self) -> &str {
        &self.inner.subtype
    }

    /// The lower-case `type/subtype` MIME essence.
    pub fn essence(&self) -> String {
        format!("{}/{}", self.type_(), self.subtype())
    }

    /// Parsed parameters in header order.
    pub fn parameters(&self) -> &[(String, String)] {
        &self.inner.parameters
    }

    /// The first parameter with the given ASCII-case-insensitive name.
    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.inner
            .parameters
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Mutably borrows the first parameter with the given ASCII-case-insensitive name.
    pub fn parameter_mut(&mut self, name: &str) -> Option<&mut String> {
        self.inner
            .parameters
            .iter_mut()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    }
}

impl fmt::Display for MimeType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(formatter)
    }
}

/// Parses a standards-facing MIME value using WHATWG validation,
/// normalization, and parameter recovery rules.
///
/// Use this whenever a web-platform algorithm says to parse a MIME type. That
/// includes MIME classification and sniffing even when the input originated
/// in an HTTP response header. Use [`crate::parse_response_content_type`] only
/// for Chromium's separate transport-metadata interpretation.
pub fn parse_mime_type(input: &str) -> Option<MimeType> {
    input.parse().ok().map(|inner| MimeType { inner })
}

/// Returns the MIME essence of a valid WHATWG MIME value.
pub fn mime_essence(input: &str) -> Option<String> {
    parse_mime_type(input).map(|mime| mime.essence())
}

/// Returns one parsed MIME parameter by ASCII-case-insensitive name.
pub fn mime_parameter(input: &str, name: &str) -> Option<String> {
    parse_mime_type(input)?.parameter(name).map(str::to_owned)
}

/// Returns the parsed `charset` MIME parameter.
pub fn mime_charset(input: &str) -> Option<String> {
    mime_parameter(input, "charset")
}

/// Normalizes a Blob/File-style Web API MIME string without parsing its
/// structure.
///
/// This only validates the permitted byte range and lowercases ASCII. It does
/// not produce a [`MimeType`] and is not a substitute for
/// [`parse_mime_type`].
pub fn normalize_web_api_mime_type(raw: &str) -> String {
    if raw.is_empty() || raw.bytes().any(|byte| !(0x20..=0x7e).contains(&byte)) {
        return String::new();
    }
    raw.to_ascii_lowercase()
}
