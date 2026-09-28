use moli_web_mime::{extract_response_mime_essence, is_css_mime, is_json_module_mime};

pub(crate) fn validate_json_module_response_mime(
    headers: &[(String, Vec<u8>)],
) -> Result<(), String> {
    validate_module_response_mime(headers, "JSON", is_json_module_mime)
}

pub(crate) fn validate_css_module_response_mime(
    headers: &[(String, Vec<u8>)],
) -> Result<(), String> {
    validate_module_response_mime(headers, "CSS", is_css_mime)
}

fn validate_module_response_mime(
    headers: &[(String, Vec<u8>)],
    expected: &str,
    accepts: fn(&str) -> bool,
) -> Result<(), String> {
    let essence = extract_response_mime_essence(headers);
    if essence.as_deref().is_some_and(accepts) {
        return Ok(());
    }
    let actual = essence
        .as_deref()
        .unwrap_or("missing or invalid Content-Type");
    Err(format!(
        "non-{expected} module response for {expected} import attribute: `{actual}`"
    ))
}
