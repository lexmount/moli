use crate::handshake::{HandshakeResponse, parse_handshake_response, validate_handshake_response};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::protocol::Role};

const MAX_WEBSOCKET_HANDSHAKE_RESPONSE_SIZE: usize = 64 * 1024;

pub(crate) async fn browser_client_handshake(
    request: http::Request<()>,
    mut stream: MaybeTlsStream<TcpStream>,
) -> Result<
    (
        WebSocketStream<MaybeTlsStream<TcpStream>>,
        HandshakeResponse,
    ),
    String,
> {
    write_handshake_request(&mut stream, &request).await?;
    let (response, tail) = read_handshake_response(&mut stream).await?;
    validate_handshake_response(&request, &response)?;
    let stream = WebSocketStream::from_partially_read(stream, tail, Role::Client, None).await;
    Ok((stream, response))
}

async fn write_handshake_request(
    stream: &mut MaybeTlsStream<TcpStream>,
    request: &http::Request<()>,
) -> Result<(), String> {
    let request_target = request
        .uri()
        .path_and_query()
        .map(|path| path.as_str())
        .unwrap_or("/");
    let mut raw = format!("GET {request_target} HTTP/1.1\r\n").into_bytes();
    for (name, value) in request.headers() {
        raw.extend_from_slice(name.as_str().as_bytes());
        raw.extend_from_slice(b": ");
        raw.extend_from_slice(value.as_bytes());
        raw.extend_from_slice(b"\r\n");
    }
    raw.extend_from_slice(b"\r\n");
    stream
        .write_all(&raw)
        .await
        .map_err(|error| format!("failed to write WebSocket handshake request: {error}"))?;
    stream
        .flush()
        .await
        .map_err(|error| format!("failed to flush WebSocket handshake request: {error}"))
}

async fn read_handshake_response(
    stream: &mut MaybeTlsStream<TcpStream>,
) -> Result<(HandshakeResponse, Vec<u8>), String> {
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 512];
    loop {
        let count = stream
            .read(&mut chunk)
            .await
            .map_err(|error| format!("failed to read WebSocket handshake response: {error}"))?;
        if count == 0 {
            return Err("WebSocket server closed during handshake".to_owned());
        }
        raw.extend_from_slice(&chunk[..count]);
        if let Some(header_end) = find_header_end(&raw) {
            let tail = raw[header_end..].to_vec();
            let response = parse_handshake_response(&raw[..header_end - 4])?;
            return Ok((response, tail));
        }
        if raw.len() > MAX_WEBSOCKET_HANDSHAKE_RESPONSE_SIZE {
            return Err("WebSocket handshake response is too large".to_owned());
        }
    }
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}
