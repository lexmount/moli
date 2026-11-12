use moli_header_field::HeaderFields;

use crate::ConnectOptions;

pub(crate) fn apply_connect_context_headers(
    headers: &mut HeaderFields,
    context: &ConnectOptions,
) -> Result<(), http::header::InvalidHeaderValue> {
    // Preserve explicit field spelling, bytes, order and duplicates. Embedding
    // fields may override browser defaults, but never protocol-control fields.
    let mut extra_headers = HeaderFields::default();
    for (name, value) in &context.extra_headers {
        if is_websocket_control_header(name) {
            continue;
        }
        if http::header::HeaderName::from_bytes(name.as_bytes()).is_err() {
            continue;
        }
        http::HeaderValue::from_bytes(value)?;
        extra_headers.push((name.clone(), value.clone()));
    }
    headers.overlay(extra_headers);
    insert_header_if_absent(headers, "Origin", &context.origin)?;
    insert_header_if_absent(headers, "User-Agent", &context.user_agent)?;
    Ok(())
}

pub(crate) fn header_map_entries(headers: &http::HeaderMap) -> HeaderFields {
    HeaderFields::from_bytes(
        headers
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), value.as_bytes().to_vec()))
            .collect(),
    )
}

pub(crate) fn insert_header_if_absent(
    headers: &mut HeaderFields,
    name: &str,
    value: &str,
) -> Result<(), http::header::InvalidHeaderValue> {
    if !headers
        .iter()
        .any(|(field, _)| field.eq_ignore_ascii_case(name))
    {
        let value: http::HeaderValue = value.parse()?;
        headers.push((name.to_owned(), value.as_bytes().to_vec()));
    }
    Ok(())
}

fn is_websocket_control_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "connection"
            | "upgrade"
            | "sec-websocket-accept"
            | "sec-websocket-extensions"
            | "sec-websocket-key"
            | "sec-websocket-protocol"
            | "sec-websocket-version"
    )
}
