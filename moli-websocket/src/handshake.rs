//! Browser handshake validation, independent of the socket transport.
use base64::{Engine, engine::general_purpose::STANDARD};

fn derive_accept_key(key: &[u8]) -> String {
    let mut value = key.to_vec();
    value.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    STANDARD.encode(moli_crypto::sha1_digest(&value))
}

pub(crate) type HandshakeResponse = http::Response<()>;

pub(crate) fn parse_handshake_response(raw_headers: &[u8]) -> Result<HandshakeResponse, String> {
    let headers = std::str::from_utf8(raw_headers)
        .map_err(|error| format!("WebSocket handshake response is not UTF-8: {error}"))?;
    let mut lines = headers.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| "WebSocket handshake response is missing status line".to_owned())?;
    let mut status_parts = status_line.splitn(3, ' ');
    let version = status_parts.next().unwrap_or_default();
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(format!(
            "WebSocket handshake response has unsupported HTTP version `{version}`"
        ));
    }
    let status = status_parts
        .next()
        .ok_or_else(|| "WebSocket handshake response is missing status code".to_owned())?
        .parse::<u16>()
        .map_err(|error| format!("WebSocket handshake response has invalid status: {error}"))?;
    let mut response = HandshakeResponse::new(());
    *response.status_mut() = http::StatusCode::from_u16(status)
        .map_err(|error| format!("WebSocket handshake response has invalid status: {error}"))?;

    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!(
                "WebSocket handshake response has malformed header `{line}`"
            ));
        };
        let name =
            http::header::HeaderName::from_bytes(name.trim().as_bytes()).map_err(|error| {
                format!("WebSocket handshake response has invalid header name: {error}")
            })?;
        let value = http::header::HeaderValue::from_str(value.trim()).map_err(|error| {
            format!("WebSocket handshake response has invalid header value: {error}")
        })?;
        response.headers_mut().append(name, value);
    }

    Ok(response)
}

pub(crate) fn validate_handshake_response(
    request: &http::HeaderMap,
    response: &HandshakeResponse,
) -> Result<(), String> {
    if response.status() != http::StatusCode::SWITCHING_PROTOCOLS {
        return Err(format!(
            "WebSocket server returned HTTP status {}",
            response.status()
        ));
    }
    if !header_values_contain_token(response.headers(), http::header::UPGRADE, "websocket") {
        return Err("WebSocket handshake response is missing `Upgrade: websocket`".to_owned());
    }
    if !header_values_contain_token(response.headers(), http::header::CONNECTION, "upgrade") {
        return Err("WebSocket handshake response is missing `Connection: Upgrade`".to_owned());
    }
    let key = request
        .get(http::header::SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "WebSocket request is missing Sec-WebSocket-Key".to_owned())?;
    let expected_accept = derive_accept_key(key.as_bytes());
    let accept = response
        .headers()
        .get(http::header::SEC_WEBSOCKET_ACCEPT)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "WebSocket handshake response is missing Sec-WebSocket-Accept".to_owned())?;
    if accept.trim() != expected_accept {
        return Err("WebSocket handshake response has invalid Sec-WebSocket-Accept".to_owned());
    }
    if response
        .headers()
        .get_all(http::header::SEC_WEBSOCKET_ACCEPT)
        .iter()
        .count()
        != 1
    {
        return Err("WebSocket server returned multiple Sec-WebSocket-Accept values".to_owned());
    }
    if response
        .headers()
        .contains_key(http::header::SEC_WEBSOCKET_EXTENSIONS)
    {
        return Err("WebSocket server selected an unrequested extension".to_owned());
    }
    validate_response_subprotocol(request, response)
}

fn header_values_contain_token(
    headers: &http::HeaderMap,
    name: http::header::HeaderName,
    token: &str,
) -> bool {
    headers.get_all(name).iter().any(|value| {
        value
            .to_str()
            .ok()
            .into_iter()
            .flat_map(|value| value.split(','))
            .any(|value| value.trim().eq_ignore_ascii_case(token))
    })
}

fn validate_response_subprotocol(
    request: &http::HeaderMap,
    response: &HandshakeResponse,
) -> Result<(), String> {
    let selected_protocols = response
        .headers()
        .get_all(http::header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .map(|value| {
            value
                .to_str()
                .map(|value| value.trim())
                .map_err(|error| format!("WebSocket response protocol is invalid: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if selected_protocols.is_empty() {
        return Ok(());
    }
    if selected_protocols.len() != 1 {
        return Err("WebSocket server selected multiple subprotocols".to_owned());
    }
    let selected = selected_protocols[0];
    if selected.is_empty() {
        return Err("WebSocket server selected an empty subprotocol".to_owned());
    }
    if selected.contains(',') {
        return Err("WebSocket server selected multiple subprotocols".to_owned());
    }

    let requested = requested_subprotocols(request)?;
    if requested.iter().any(|protocol| protocol == selected) {
        Ok(())
    } else {
        Err(format!(
            "WebSocket server selected unrequested subprotocol `{selected}`"
        ))
    }
}

fn requested_subprotocols(request: &http::HeaderMap) -> Result<Vec<String>, String> {
    let mut protocols = Vec::new();
    for value in request.get_all(http::header::SEC_WEBSOCKET_PROTOCOL).iter() {
        let value = value
            .to_str()
            .map_err(|error| format!("WebSocket request protocol is invalid: {error}"))?;
        protocols.extend(
            value
                .split(',')
                .map(str::trim)
                .filter(|protocol| !protocol.is_empty())
                .map(ToOwned::to_owned),
        );
    }
    Ok(protocols)
}
