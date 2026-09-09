use crate::{
    ConnectOptions,
    handshake::{HandshakeResponse, parse_handshake_response, validate_handshake_response},
    headers::header_map_entries,
    proxy::{append_proxy_connect_header, websocket_proxy_url},
};
use moli_curl::{
    CurlDnsResolution,
    websocket::{
        CurlWebSocketConnection, CurlWebSocketEvent, CurlWebSocketRequest, CurlWebSocketRuntime,
    },
};
use moli_dns_resolver::DnsTarget;
use std::sync::OnceLock;

pub(crate) async fn open_websocket_stream(
    mut request: http::Request<()>,
    context: &ConnectOptions,
) -> Result<
    (
        CurlWebSocketConnection,
        HandshakeResponse,
        Vec<(String, String)>,
    ),
    String,
> {
    let proxy = websocket_proxy_url(request.uri(), context)?;
    let mut native = CurlWebSocketRequest::new(request.uri().to_string());
    native.headers = header_map_entries(request.headers());
    // libcurl appends the Upgrade token itself for a native WS request.
    native
        .headers
        .retain(|(name, _)| !name.eq_ignore_ascii_case("connection"));
    native.proxy = proxy.map(|url| url.to_string());
    // WebSocket opening handshakes use credentials=include, including across
    // origins: https://websockets.spec.whatwg.org/#opening-handshake
    native.tls = context.tls.clone();
    if native.proxy.is_some() {
        let mut validated = String::new();
        append_proxy_connect_header(&mut validated, "User-Agent", &context.user_agent)?;
        native
            .proxy_headers
            .push(("User-Agent".to_owned(), context.user_agent.clone()));
        if let Some(token) = &context.proxy_bearer_token {
            let value = format!("Bearer {token}");
            append_proxy_connect_header(&mut validated, "Proxy-Authorization", &value)?;
            native
                .proxy_headers
                .push(("Proxy-Authorization".to_owned(), value));
        }
    } else {
        let url = url::Url::parse(&native.url).map_err(|error| error.to_string())?;
        if let Some(url::Host::Domain(host)) = url.host() {
            native.dns_resolution = CurlDnsResolution::resolve_origin(
                DnsTarget::new(
                    host,
                    url.port_or_known_default()
                        .ok_or("WebSocket URL has no port")?,
                ),
                Vec::new(),
            );
        }
    }
    static RUNTIME: OnceLock<Result<CurlWebSocketRuntime, String>> = OnceLock::new();
    let runtime = RUNTIME
        .get_or_init(|| CurlWebSocketRuntime::new().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(Clone::clone)?;
    let mut connection = runtime.connect(native).map_err(|error| error.to_string())?;
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
            *request.headers_mut() = actual_request_headers(&actual)?;
            validate_handshake_response(&request, &response)?;
            result?;
            let headers = header_map_entries(request.headers());
            Ok((connection, response, headers))
        }
        Some(CurlWebSocketEvent::Closed { result }) => Err(result
            .err()
            .unwrap_or_else(|| "WebSocket closed during handshake".to_owned())),
        _ => Err("WebSocket transport stopped during handshake".to_owned()),
    }
}

fn actual_request_headers(raw: &[u8]) -> Result<http::HeaderMap, String> {
    let raw = std::str::from_utf8(raw)
        .map_err(|error| format!("invalid WebSocket request headers: {error}"))?;
    let mut lines = raw.split("\r\n");
    if !lines.next().is_some_and(|line| line.starts_with("GET ")) {
        return Err("WebSocket native request headers are missing".to_owned());
    }
    let mut headers = http::HeaderMap::new();
    for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or("invalid WebSocket request header")?;
        let name = http::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| error.to_string())?;
        let value =
            http::header::HeaderValue::from_str(value.trim()).map_err(|error| error.to_string())?;
        headers.append(name, value);
    }
    Ok(headers)
}
