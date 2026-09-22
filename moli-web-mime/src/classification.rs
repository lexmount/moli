use crate::parse::{mime_essence, mime_parameter};

pub fn is_html_document_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| mime == "text/html")
}

pub fn is_dom_parser_xml_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| {
        matches!(
            mime.as_str(),
            "text/xml" | "application/xml" | "application/xhtml+xml" | "image/svg+xml"
        )
    })
}

pub fn is_javascript_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| is_javascript_mime_essence(&mime))
}

pub fn is_javascript_mime_essence(input: &str) -> bool {
    matches!(
        input,
        "application/ecmascript"
            | "application/javascript"
            | "application/x-ecmascript"
            | "application/x-javascript"
            | "text/ecmascript"
            | "text/javascript"
            | "text/javascript1.0"
            | "text/javascript1.1"
            | "text/javascript1.2"
            | "text/javascript1.3"
            | "text/javascript1.4"
            | "text/javascript1.5"
            | "text/jscript"
            | "text/livescript"
            | "text/x-ecmascript"
            | "text/x-javascript"
    )
}

pub fn is_css_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| mime == "text/css")
}

pub fn is_text_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| is_text_mime_essence(&mime))
}

pub fn is_text_mime_essence(input: &str) -> bool {
    input.starts_with("text/")
}

/// Whether a `style` element's raw `type` attribute selects classic CSS.
///
/// HTML's update-a-style-block algorithm compares the untrimmed attribute
/// value directly; unlike a MIME type hint, parameters and surrounding ASCII
/// whitespace are not accepted.
pub fn is_css_style_element_type_attribute(value: Option<&str>) -> bool {
    value.is_none_or(|value| value.is_empty() || value.eq_ignore_ascii_case("text/css"))
}

/// Whether a stylesheet link or processing-instruction MIME hint selects CSS.
pub fn is_css_stylesheet_type_hint(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return true;
    };
    let value = value.trim();
    value.is_empty() || is_css_mime(value)
}

pub fn is_image_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| is_image_mime_essence(&mime))
}

pub fn is_image_mime_essence(input: &str) -> bool {
    input.starts_with("image/")
}

pub fn is_png_image_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| is_png_image_mime_essence(&mime))
}

pub fn is_png_image_mime_essence(input: &str) -> bool {
    input == "image/png"
}

pub fn is_svg_image_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| is_svg_image_mime_essence(&mime))
}

pub fn is_svg_image_mime_essence(input: &str) -> bool {
    input == "image/svg+xml"
}

pub fn is_audio_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| is_audio_mime_essence(&mime))
}

pub fn is_audio_mime_essence(input: &str) -> bool {
    input.starts_with("audio/")
}

pub fn is_video_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| is_video_mime_essence(&mime))
}

pub fn is_video_mime_essence(input: &str) -> bool {
    input.starts_with("video/")
}

pub fn is_font_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| is_font_mime_essence(&mime))
}

pub fn is_font_mime_essence(input: &str) -> bool {
    input.starts_with("font/")
}

pub fn is_form_urlencoded_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| mime == "application/x-www-form-urlencoded")
}

pub fn is_multipart_form_data_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| mime == "multipart/form-data")
}

pub fn multipart_form_data_boundary(input: &str) -> Option<String> {
    is_multipart_form_data_mime(input)
        .then(|| mime_parameter(input, "boundary"))
        .flatten()
}

pub fn is_json_module_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| mime == "application/json" || mime.ends_with("+json"))
}

pub fn is_webassembly_mime(input: &str) -> bool {
    mime_essence(input).is_some_and(|mime| mime == "application/wasm")
}

/// Returns whether a navigation response can be represented by Moli's
/// lightweight Document surface. Raw main-resource handling is deliberately
/// broader: binary responses can be returned to a caller without becoming a
/// Document, while a child navigation has no such raw-content owner.
pub fn is_supported_document_mime_type(input: &str) -> bool {
    let Some(essence) = mime_essence(input) else {
        return false;
    };
    is_text_mime_essence(&essence)
        || is_image_mime_essence(&essence)
        || is_audio_mime_essence(&essence)
        || is_video_mime_essence(&essence)
        || is_javascript_mime_essence(&essence)
        || essence == "application/json"
        || essence.ends_with("+json")
        || matches!(
            essence.as_str(),
            "application/xml"
                | "application/xhtml+xml"
                | "application/atom+xml"
                | "application/rss+xml"
        )
        || essence.ends_with("+xml")
}

pub fn is_binary_document_mime_type(input: &str) -> bool {
    let Some(essence) = mime_essence(input) else {
        return false;
    };
    is_audio_mime_essence(&essence)
        || is_font_mime_essence(&essence)
        || is_image_mime_essence(&essence)
        || is_video_mime_essence(&essence)
        || matches!(
            essence.as_str(),
            "application/gzip"
                | "application/octet-stream"
                | "application/pdf"
                | "application/vnd.ms-fontobject"
                | "application/x-7z-compressed"
                | "application/x-bzip2"
                | "application/x-gzip"
                | "application/x-rar-compressed"
                | "application/x-tar"
                | "application/zip"
                | "application/wasm"
        )
}
