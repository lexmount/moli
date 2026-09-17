use moli_content_type::parse_mime_type;

pub use moli_content_type::{
    mime_charset, mime_essence, mime_parameter, normalize_web_api_mime_type,
};

pub fn parse_mime(input: &str) -> Option<mime::Mime> {
    parse_mime_type(input).and_then(|mime| mime.to_string().parse().ok())
}

pub fn request_header_content_type_essence(input: &str) -> Option<String> {
    // Fetch parses one MIME type here, rather than extracting a type from a
    // comma-separated response header. Commas invalidate the type/subtype,
    // but are allowed in parameters.
    mime_essence(input)
}
