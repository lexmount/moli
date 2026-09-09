//! Browser-compatible text, document, form, script, and URL encoding helpers.

mod document;
mod form;
mod labels;
mod legacy_text;
mod percent;
mod query;
mod script;

pub use document::{
    HtmlDocumentStreamingDecoder, LegacyEncodingDetector, decode_html_document,
    decode_html_document_with_fallback,
};
pub use form::{
    encode_text_for_legacy_web, form_output_encoding, form_output_encoding_for_label,
    form_submission_encoding, form_urlencoded_serialize_pairs, is_charset_sentinel_name,
};
pub use labels::{
    charset_from_content_type, charset_from_headers, encoding_for_label,
    encoding_for_response_charset, encoding_from_response_headers,
};
pub use legacy_text::decode_text_for_legacy_web;
pub use query::encode_url_query_for_legacy_web;
pub use script::{decode_classic_script_source, decode_utf8};

#[cfg(test)]
mod tests;
