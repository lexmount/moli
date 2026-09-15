use moli_web_mime::{is_css_mime, is_json_module_mime, is_text_mime, response_header_values};

pub(crate) fn ensure_worker_wasm_module_mime(
    response: &moli_fetch::Response,
) -> Result<(), String> {
    let content_type = super::script_mime::worker_response_content_type(&response.headers);
    let Some(content_type) = content_type else {
        return Err("WebAssembly module response missing Content-Type".to_owned());
    };
    if moli_web_mime::is_webassembly_mime(&content_type) {
        return Ok(());
    }
    Err(format!(
        "WebAssembly module response has unsupported MIME type `{content_type}`"
    ))
}

pub(crate) fn ensure_worker_json_module_mime(
    response: &moli_fetch::Response,
) -> Result<(), String> {
    ensure_worker_json_module_mime_from_headers(&response.headers)
}

pub(super) fn ensure_worker_json_module_mime_from_headers(
    headers: &[(String, String)],
) -> Result<(), String> {
    let content_type = worker_module_response_content_type(headers);
    let Some(content_type) = content_type else {
        return Err(
            "non-JSON module response for JSON import attribute: missing Content-Type".to_owned(),
        );
    };
    if is_json_module_mime(&content_type) {
        return Ok(());
    }
    Err(format!(
        "non-JSON module response for JSON import attribute: `{content_type}`"
    ))
}

pub(crate) fn ensure_worker_css_module_mime(response: &moli_fetch::Response) -> Result<(), String> {
    let content_type = worker_module_response_content_type(&response.headers);
    let Some(content_type) = content_type else {
        return Err(
            "non-CSS module response for CSS import attribute: missing Content-Type".to_owned(),
        );
    };
    if is_css_mime(&content_type) {
        return Ok(());
    }
    Err(format!(
        "non-CSS module response for CSS import attribute: `{content_type}`"
    ))
}

pub(crate) fn ensure_worker_text_module_mime(
    response: &moli_fetch::Response,
) -> Result<(), String> {
    let content_type = worker_module_response_content_type(&response.headers);
    let Some(content_type) = content_type else {
        return Err(
            "non-text module response for text import attribute: missing Content-Type".to_owned(),
        );
    };
    if is_text_mime(&content_type) {
        return Ok(());
    }
    Err(format!(
        "non-text module response for text import attribute: `{content_type}`"
    ))
}

fn worker_module_response_content_type(headers: &[(String, Vec<u8>)]) -> Option<String> {
    response_header_values(headers, "content-type")
        .into_iter()
        .next_back()
}
