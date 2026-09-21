use crate::{
    ConnectOptions,
    handshake::{HandshakeResponse, parse_handshake_response, validate_handshake_response},
    headers::header_map_entries,
    proxy::websocket_proxy_route,
    request::PreparedWebSocketRequest,
};
use moli_curl::{
    CurlDnsResolution, HostResolveOverrides,
    websocket::{
        CurlWebSocketConnection, CurlWebSocketConnector, CurlWebSocketEvent, CurlWebSocketRequest,
    },
};

pub(crate) struct HandshakeInfo {
    pub request_headers: http::HeaderMap,
    pub response: HandshakeResponse,
}

pub(crate) struct OpenedConnection {
    pub connection: CurlWebSocketConnection,
    pub handshake: HandshakeInfo,
}

pub(crate) async fn open_websocket_connection(
    connector: &CurlWebSocketConnector,
    request: PreparedWebSocketRequest,
    context: &ConnectOptions,
) -> Result<OpenedConnection, String> {
    let proxy_route = websocket_proxy_route(&request.url, context)?;
    let host_resolve = HostResolveOverrides::parse(&context.http_host_resolve)
        .map_err(|error| error.to_string())?;
    // Resolve exactly the endpoint this process will connect to: the target
    // when direct, or the proxy when target DNS belongs to that proxy.
    let dns_endpoint = proxy_route
        .connection_dns_endpoint(&request.url, &host_resolve)
        .map_err(|error| error.to_string())?;
    let resolve_entries = host_resolve.normalized_entries();
    let mut native = CurlWebSocketRequest::new(request.url.to_string());
    native.headers = header_map_entries(&request.headers);
    native.proxy = proxy_route.proxy().map(|proxy| proxy.curl_url().to_owned());
    native.resolve_entries = resolve_entries.clone();
    // WebSocket opening handshakes use credentials=include, including across
    // origins: https://websockets.spec.whatwg.org/#opening-handshake
    native.tls = context.tls.clone();
    if let Some(proxy) = proxy_route.proxy() {
        if proxy.scheme().uses_http_headers() {
            native
                .proxy_headers
                .push(("User-Agent".to_owned(), context.user_agent.clone()));
            if let Some(token) = &context.proxy_bearer_token {
                native
                    .proxy_headers
                    .push(("Proxy-Authorization".to_owned(), format!("Bearer {token}")));
            }
        } else if context.proxy_bearer_token.is_some() {
            return Err("proxy bearer authentication requires an HTTP(S) proxy".to_owned());
        }
    }
    if let Some(endpoint) = dns_endpoint {
        native.dns_resolution =
            CurlDnsResolution::resolve_endpoint(endpoint.target().clone(), resolve_entries);
    }
    let mut connection = connector
        .connect(native)
        .map_err(|error| error.to_string())?;
    match connection.recv().await {
        Some(CurlWebSocketEvent::Handshake {
            request: actual,
            response,
            result,
        }) => {
            if response.is_empty() {
                return Err(result
                    .err()
                    .unwrap_or_else(|| "WebSocket handshake response is empty".to_owned()));
            }
            let response = parse_handshake_response(&response)?;
            let request_headers = actual_request_headers(&actual)?;
            validate_handshake_response(&request_headers, &response)?;
            result?;
            Ok(OpenedConnection {
                connection,
                handshake: HandshakeInfo {
                    request_headers,
                    response,
                },
            })
        }
        Some(CurlWebSocketEvent::Closed { result }) => Err(result
            .err()
            .unwrap_or_else(|| "WebSocket closed during handshake".to_owned())),
        _ => Err("WebSocket transport stopped during handshake".to_owned()),
    }
}

fn actual_request_headers(raw: &[u8]) -> Result<http::HeaderMap, String> {
    if !raw.starts_with(b"GET ") {
        return Err("WebSocket native request headers are missing".to_owned());
    }
    let mut headers = http::HeaderMap::new();
    for line in raw.split(|byte| *byte == b'\n').skip(1) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let separator = line
            .iter()
            .position(|byte| *byte == b':')
            .ok_or_else(|| "malformed WebSocket request header".to_owned())?;
        let (name, value) = line.split_at(separator);
        let name = http::header::HeaderName::from_bytes(name).map_err(|error| error.to_string())?;
        let value = http::header::HeaderValue::from_bytes(value[1..].trim_ascii())
            .map_err(|error| error.to_string())?;
        headers.append(name, value);
    }
    Ok(headers)
}
