use crate::network_host::response::response_has_null_body;

/// Even unsupported integrity metadata requires a complete readable body
/// before Main Fetch may hand the response to its consumer.
pub(crate) fn fetch_integrity_requires_full_body(integrity: &str) -> bool {
    !integrity.is_empty()
}

/// Main fetch checks the filtered response's body before applying SRI's
/// metadata parser. Even ignored metadata cannot authorize a null body.
pub(crate) fn validate_fetch_response_integrity(
    integrity: &str,
    method: &str,
    status: u16,
    response_is_readable: bool,
    bytes: &[u8],
) -> Result<(), String> {
    if !fetch_integrity_requires_full_body(integrity) {
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
