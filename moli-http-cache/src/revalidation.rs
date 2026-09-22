/// Builds conditional request headers from cached response validators.
pub fn validation_headers_from_headers(
    record_headers: &[(String, Vec<u8>)],
) -> Vec<(String, Vec<u8>)> {
    let mut headers = Vec::new();
    if let Some((_, etag)) = record_headers.iter().find(|(name, _)| name == "etag") {
        headers.push(("If-None-Match".to_owned(), etag.clone()));
    }
    if let Some((_, last_modified)) = record_headers
        .iter()
        .find(|(name, _)| name == "last-modified")
    {
        headers.push(("If-Modified-Since".to_owned(), last_modified.clone()));
    }
    headers
}

/// Merges 304 response metadata into cached headers while preserving the
/// cached representation body.
pub fn merge_not_modified_headers(
    cached_headers: &[(String, Vec<u8>)],
    not_modified_headers: &[(String, Vec<u8>)],
) -> Vec<(String, Vec<u8>)> {
    let mut headers = cached_headers.to_vec();
    let connection_nominated_headers = connection_nominated_header_names(not_modified_headers);
    for (name, value) in not_modified_headers {
        if should_skip_not_modified_header(name, &connection_nominated_headers) {
            continue;
        }
        if let Some(existing) = headers
            .iter_mut()
            .find(|(existing_name, _)| existing_name.eq_ignore_ascii_case(name))
        {
            existing.1 = value.clone();
        } else {
            headers.push((name.clone(), value.clone()));
        }
    }
    headers
}

fn should_skip_not_modified_header(name: &str, connection_nominated_headers: &[String]) -> bool {
    let lower_name = name.to_ascii_lowercase();
    connection_nominated_headers
        .iter()
        .any(|nominated| nominated == &lower_name)
        || matches!(
            lower_name.as_str(),
            "content-length"
                | "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
        )
}

fn connection_nominated_header_names(headers: &[(String, Vec<u8>)]) -> Vec<String> {
    let headers = moli_header_field::headers_to_byte_strings(headers);
    // Connection can name extra hop-by-hop fields; a 304 must not promote
    // those transient fields into cached representation metadata.
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("connection"))
        .flat_map(|(_, value)| value.split(','))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}
