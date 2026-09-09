use anyhow::{Result, bail};
use curl::easy::{Easy2, Handler, HttpVersion, InfoType, List, WsOptions};

use super::CurlWebSocketRequest;

const MAX_HEADERS: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct Handshake {
    pub request: Vec<u8>,
    pub response: Vec<u8>,
    pub error: Option<String>,
    proxy_connect: bool,
    header_bytes: usize,
}

impl Handler for Handshake {
    fn header(&mut self, data: &[u8]) -> bool {
        self.header_bytes = self.header_bytes.saturating_add(data.len());
        if self.header_bytes > MAX_HEADERS {
            self.error = Some("WebSocket handshake headers are too large".to_owned());
            return false;
        }
        if self.proxy_connect {
            if data.starts_with(b"HTTP/") {
                let line = String::from_utf8_lossy(data);
                let status = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|status| status.parse::<u16>().ok());
                if status.is_some_and(|status| status > 200) {
                    self.error = Some(format!("WebSocket proxy CONNECT failed: {}", line.trim()));
                    return false;
                }
            }
            return true;
        }
        if data.starts_with(b"HTTP/") {
            self.response.clear();
        }
        self.response.extend_from_slice(data);
        true
    }

    fn debug(&mut self, kind: InfoType, data: &[u8]) {
        if matches!(kind, InfoType::HeaderOut) {
            self.proxy_connect = data.starts_with(b"CONNECT ");
            if data.starts_with(b"GET ") && data.len() <= MAX_HEADERS {
                self.request = data.to_vec();
            }
        }
    }
}

pub(super) fn configure(request: &CurlWebSocketRequest) -> Result<Easy2<Handshake>> {
    let url = url::Url::parse(&request.url)?;
    if !matches!(url.scheme(), "ws" | "wss") {
        bail!("WebSocket URL must use ws or wss");
    }
    if request.handshake_timeout.is_zero() {
        bail!("WebSocket handshake timeout must be positive");
    }
    let mut easy = Easy2::new(Handshake::default());
    easy.url(&request.url)?;
    easy.http_version(HttpVersion::V11)?;
    easy.follow_location(false)?;
    easy.signal(false)?;
    easy.ws_connect_only(true)?;
    easy.ws_options(WsOptions::new().no_auto_pong(true))?;
    easy.ssl_verify_peer(request.tls_verify)?;
    easy.ssl_verify_host(request.tls_verify)?;
    easy.proxy(request.proxy.as_deref().unwrap_or(""))?;
    // The browser has already applied no_proxy; do not evaluate environment policy twice.
    easy.noproxy("")?;
    easy.http_proxy_tunnel(request.proxy.is_some())?;
    easy.separate_proxy_headers(true)?;
    easy.http_headers(headers(&request.headers)?)?;
    easy.proxy_headers(headers(&request.proxy_headers)?)?;
    // Capture HeaderOut through our handler; never print debug or credentials.
    easy.verbose(true)?;
    Ok(easy)
}

fn headers(entries: &[(String, String)]) -> Result<List> {
    let mut list = List::new();
    let mut size = 0usize;
    for (name, value) in entries {
        if name.is_empty()
            || name
                .bytes()
                .any(|b| !b.is_ascii_alphanumeric() && !b"!#$%&'*+-.^_`|~".contains(&b))
            || value.bytes().any(|b| matches!(b, 0 | b'\r' | b'\n'))
        {
            bail!("invalid WebSocket request header");
        }
        size = size
            .saturating_add(name.len())
            .saturating_add(value.len())
            .saturating_add(4);
        if size > MAX_HEADERS {
            bail!("WebSocket request headers are too large");
        }
        // curl uses a semicolon to request an empty header instead of removing it.
        list.append(&if value.is_empty() {
            format!("{name};")
        } else {
            format!("{name}: {value}")
        })?;
    }
    Ok(list)
}
