use base64::{Engine, engine::general_purpose::STANDARD};
use url::Url;

use crate::{
    ConnectOptions,
    headers::{apply_connect_context_headers, insert_header_if_absent},
    validate_subprotocols,
};

/// Browser connection intent. Libcurl generates the HTTP upgrade fields.
#[derive(Debug)]
pub(crate) struct PreparedWebSocketRequest {
    pub url: Url,
    pub headers: http::HeaderMap,
}

pub(crate) fn prepare_websocket_request(
    url: &str,
    protocols: &[String],
    context: &ConnectOptions,
) -> Result<PreparedWebSocketRequest, String> {
    let mut url =
        Url::parse(url).map_err(|error| format!("failed to parse WebSocket URL: {error}"))?;
    if !matches!(url.scheme(), "ws" | "wss") || url.host_str().is_none() {
        return Err("WebSocket URL must use ws or wss and include a host".to_owned());
    }
    reject_blocked_websocket_port(&url)?;
    let mut headers = http::HeaderMap::new();
    apply_connect_context_headers(&mut headers, context)
        .map_err(|error| format!("failed to build WebSocket handshake headers: {error}"))?;
    apply_subprotocol_header(&mut headers, protocols)?;
    apply_basic_auth_header(&mut headers, &url)?;
    apply_cookie_header(&mut headers, context)?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    Ok(PreparedWebSocketRequest { url, headers })
}

fn apply_subprotocol_header(
    headers: &mut http::HeaderMap,
    protocols: &[String],
) -> Result<(), String> {
    if protocols.is_empty() {
        return Ok(());
    }
    validate_subprotocols(protocols)
        .map_err(|error| format!("failed to build WebSocket subprotocol header: {error}"))?;
    let value = protocols.join(", ");
    let value = value
        .parse()
        .map_err(|error| format!("failed to build WebSocket subprotocol header: {error}"))?;
    headers.insert(http::header::SEC_WEBSOCKET_PROTOCOL, value);
    Ok(())
}

fn apply_cookie_header(
    headers: &mut http::HeaderMap,
    context: &ConnectOptions,
) -> Result<(), String> {
    let Some(cookie_header) = context.cookie_header.as_deref() else {
        return Ok(());
    };
    insert_header_if_absent(headers, http::header::COOKIE, cookie_header)
        .map_err(|error| format!("failed to build WebSocket cookie header: {error}"))
}

fn apply_basic_auth_header(headers: &mut http::HeaderMap, url: &Url) -> Result<(), String> {
    if url.username().is_empty() && url.password().is_none() {
        return Ok(());
    }
    let username = percent_decode_userinfo_component(url.username());
    let password = url
        .password()
        .map(percent_decode_userinfo_component)
        .unwrap_or_default();
    let value = format!("Basic {}", encode_basic_auth(&username, &password));
    insert_header_if_absent(headers, http::header::AUTHORIZATION, &value)
        .map_err(|error| format!("failed to build WebSocket basic auth header: {error}"))
}

fn reject_blocked_websocket_port(url: &Url) -> Result<(), String> {
    let Some(port) = url.port_or_known_default() else {
        return Ok(());
    };
    if is_blocked_websocket_port(port) {
        return Err(format!("WebSocket target port `{port}` is blocked"));
    }
    Ok(())
}

fn percent_decode_userinfo_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (bytes.get(index + 1), bytes.get(index + 2))
            && let (Some(high), Some(low)) = (hex_value(*high), hex_value(*low))
        {
            out.push((high << 4) | low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn encode_basic_auth(username: &str, password: &str) -> String {
    STANDARD.encode(format!("{username}:{password}"))
}

fn is_blocked_websocket_port(port: u16) -> bool {
    matches!(
        port,
        1 | 7
            | 9
            | 11
            | 13
            | 15
            | 17
            | 19
            | 20
            | 21
            | 22
            | 23
            | 25
            | 37
            | 42
            | 43
            | 53
            | 69
            | 77
            | 79
            | 87
            | 95
            | 101
            | 102
            | 103
            | 104
            | 109
            | 110
            | 111
            | 113
            | 115
            | 117
            | 119
            | 123
            | 135
            | 137
            | 139
            | 143
            | 161
            | 179
            | 389
            | 427
            | 465
            | 512
            | 513
            | 514
            | 515
            | 526
            | 530
            | 531
            | 532
            | 540
            | 548
            | 554
            | 556
            | 563
            | 587
            | 601
            | 636
            | 989
            | 990
            | 993
            | 995
            | 1719
            | 1720
            | 1723
            | 2049
            | 3659
            | 4045
            | 5060
            | 5061
            | 6000
            | 6566
            | 6665..=6669 | 6697 | 10080
    )
}
