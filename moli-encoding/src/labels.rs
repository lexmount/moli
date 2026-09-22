use encoding_rs::Encoding;
use moli_content_type::parse_response_content_type;

/// Gets an encoding from a label using the Encoding Standard's preprocessing.
///
/// `encoding_rs` removes leading and trailing ASCII whitespace before matching
/// the label. Use this for web-platform label sources such as a classic
/// script's `charset` attribute.
pub fn encoding_for_label(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.as_bytes())
}

/// Looks up a charset extracted from Chromium response transport metadata.
///
/// Chromium's network parser has already removed its HTTP LWS set (SP and HT)
/// before Blink performs an exact encoding-name lookup. `encoding_rs` would
/// apply additional Encoding Standard whitespace preprocessing, so reject any
/// remaining leading or trailing ASCII whitespace first.
pub fn encoding_for_response_charset(label: &str) -> Option<&'static Encoding> {
    let bytes = label.as_bytes();
    if bytes.first().is_some_and(|byte| byte.is_ascii_whitespace())
        || bytes.last().is_some_and(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }

    Encoding::for_label(bytes)
}

/// The `charset` parameter of a `Content-Type` value.
///
/// Response metadata is parsed with Chromium network-layer tolerances before
/// the label is handed to `encoding_rs`. In particular, quoted separators stay
/// inside their parameter and unterminated quoted values remain recoverable.
pub fn charset_from_content_type(value: &str) -> Option<String> {
    let parsed = parse_response_content_type(value)?;
    let charset = parsed.charset()?;
    (!charset.is_empty()).then(|| charset.to_owned())
}

pub fn charset_from_headers(headers: &[(String, Vec<u8>)]) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .and_then(|(_, value)| {
            charset_from_content_type(&moli_header_field::decode_header_value(value))
        })
}

/// Selects Chromium's transport encoding from HTTP response headers.
pub fn encoding_from_response_headers(headers: &[(String, Vec<u8>)]) -> Option<&'static Encoding> {
    charset_from_headers(headers)
        .as_deref()
        .and_then(encoding_for_response_charset)
}
