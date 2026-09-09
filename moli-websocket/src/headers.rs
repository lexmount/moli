use crate::ConnectOptions;

pub(crate) fn apply_connect_context_headers(
    headers: &mut http::HeaderMap,
    context: &ConnectOptions,
) -> Result<(), http::header::InvalidHeaderValue> {
    // Embedding-provided headers may intentionally override browser defaults
    // such as Origin/User-Agent, but never protocol-control handshake fields.
    for (name, value) in &context.extra_headers {
        if is_websocket_control_header(name) {
            continue;
        }
        let Ok(name) = http::header::HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        headers.insert(name, value.parse()?);
    }
    insert_header_if_absent(headers, http::header::ORIGIN, &context.origin)?;
    insert_header_if_absent(headers, http::header::USER_AGENT, &context.user_agent)?;
    Ok(())
}

pub(crate) fn header_map_entries(headers: &http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}

pub(crate) fn insert_header_if_absent(
    headers: &mut http::HeaderMap,
    name: http::header::HeaderName,
    value: &str,
) -> Result<(), http::header::InvalidHeaderValue> {
    if !headers.contains_key(&name) {
        headers.insert(name, value.parse()?);
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
