use crate::network_host::response::response_has_null_body;

/// Main fetch checks the filtered response's body before applying SRI's
/// metadata parser. Even ignored metadata cannot authorize a null body.
pub(crate) fn validate_fetch_response_integrity(
    integrity: &str,
    method: &str,
    status: u16,
    response_is_readable: bool,
    bytes: &[u8],
) -> Result<(), String> {
    if integrity.is_empty() {
        return Ok(());
    }
    if !response_is_readable || response_has_null_body(method, status) {
        return Err("fetch: integrity metadata requires a non-null response body".to_owned());
    }
    if !crate::subresource_integrity::response_matches_subresource_integrity_metadata(
        bytes,
        Some(integrity),
        true,
    ) {
        return Err("fetch: response body does not match its integrity metadata".to_owned());
    }
    Ok(())
}
