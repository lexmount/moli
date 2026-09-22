mod classification;
mod data_url;
mod destination;
mod headers;
mod media;
mod parse;
mod resource;
mod response_policy;
mod sniffing;

pub use classification::{
    is_audio_mime, is_audio_mime_essence, is_binary_document_mime_type, is_css_mime,
    is_css_style_element_type_attribute, is_css_stylesheet_type_hint, is_dom_parser_xml_mime,
    is_font_mime, is_font_mime_essence, is_form_urlencoded_mime, is_html_document_mime,
    is_image_mime, is_image_mime_essence, is_javascript_mime, is_javascript_mime_essence,
    is_json_module_mime, is_multipart_form_data_mime, is_png_image_mime, is_png_image_mime_essence,
    is_supported_document_mime_type, is_svg_image_mime, is_svg_image_mime_essence, is_text_mime,
    is_text_mime_essence, is_video_mime, is_video_mime_essence, is_webassembly_mime,
    multipart_form_data_boundary,
};
pub use data_url::{
    data_url_body_and_computed_mime_type, data_url_body_and_mime_type, data_url_mime_type,
};
pub use destination::FetchDestination;
pub use headers::{
    effective_response_mime_essence, effective_response_mime_type,
    normalize_response_blob_mime_type, response_blob_mime_type, response_content_type,
    response_document_content_type, response_header_value, response_header_values,
    response_headers_indicate_attachment_download, response_headers_indicate_binary_document,
    response_headers_indicate_raw_document,
};
pub use media::{MediaMimeSupport, is_media_source_type_supported, media_mime_support};
pub use parse::{
    mime_charset, mime_essence, mime_parameter, normalize_web_api_mime_type, parse_mime,
    request_header_content_type_essence,
};
pub use resource::{
    known_url_path_mime_essence, resource_mime_essence_for_path, resource_mime_essence_for_url,
};
pub use response_policy::{
    ScriptResponseMimeError, check_script_response_mime, computed_response_mime_type,
    determine_nosniff, should_opaque_response_be_blocked_by_orb,
    should_opaque_response_be_blocked_by_orb_with_body, should_response_be_blocked_due_to_nosniff,
    should_script_like_response_be_blocked_due_to_mime_type,
};
pub use sniffing::{
    MimeSniffingContext, RESOURCE_HEADER_BYTE_LIMIT, computed_mime_type, resource_header,
    sniff_archive_mime_type, sniff_audio_video_mime_type, sniff_font_mime_type,
    sniff_image_mime_type, sniff_text_or_binary, sniff_unknown_mime_type,
};

#[cfg(test)]
mod tests;
