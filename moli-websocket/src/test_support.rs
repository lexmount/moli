mod tls;
pub use tls::TlsWebSocketFixture;

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    time::{Duration, timeout},
};
use tokio_tungstenite::tungstenite::{
    handshake::server::{Callback, ErrorResponse, Request, Response},
    protocol::{CloseFrame, Message},
};

use crate::{
    ConnectOptions, Event, FrameOpcode, proxy::websocket_proxy_url_with_env, spawn_connection,
};

struct InfallibleHandshakeCallback<F>(F);

impl<F> Callback for InfallibleHandshakeCallback<F>
where
    F: FnOnce(&Request, Response) -> Response,
{
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        Ok((self.0)(request, response))
    }
}

pub fn test_websocket_context() -> ConnectOptions {
    ConnectOptions {
        connector: None,
        origin: "https://example.com".to_owned(),
        user_agent: "Moli-WebSocket-Test/1.0".to_owned(),
        extra_headers: Vec::new(),
        http_proxy: None,
        http_no_proxy: None,
        proxy_bearer_token: None,
        tls: Default::default(),
        cookie_header: None,
    }
}

pub fn test_websocket_proxy_url_with_env(
    url: &url::Url,
    context: &ConnectOptions,
    env: &[(&str, &str)],
) -> Option<String> {
    websocket_proxy_url_with_env(url, context, |name| {
        env.iter()
            .find_map(|(env_name, value)| (*env_name == name).then(|| (*value).to_owned()))
    })
    .expect("websocket proxy url should resolve")
    .map(|url| url.to_string())
}

pub async fn websocket_raw_handshake_failure_message(
    path: &'static str,
    response: &'static [u8],
) -> String {
    let (url, server) = spawn_raw_websocket_response_server(path, response).await;
    let message = websocket_handshake_failure_message(url, Vec::new()).await;
    server
        .await
        .expect("raw websocket response server should finish");
    message
}

pub async fn websocket_computed_accept_handshake_failure_message(
    path: &'static str,
    headers: Vec<&'static str>,
    protocols: Vec<String>,
) -> String {
    let (url, server) = spawn_computed_accept_websocket_response_server(path, headers).await;
    let message = websocket_handshake_failure_message(url, protocols).await;
    server
        .await
        .expect("computed websocket response server should finish");
    message
}

async fn websocket_handshake_failure_message(url: String, protocols: Vec<String>) -> String {
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let _command_tx = spawn_connection(90, url, protocols, test_websocket_context(), event_tx);
    recv_handshake_failure_events(&mut event_rx).await
}

pub struct OpenEvent {
    pub socket_id: u64,
    pub protocol: String,
}

pub async fn recv_open_event(event_rx: &mut mpsc::Receiver<Event>) -> OpenEvent {
    loop {
        let event = timeout(Duration::from_secs(3), event_rx.recv())
            .await
            .expect("websocket event should arrive")
            .expect("websocket event channel should stay open");
        match event {
            Event::Open {
                socket_id,
                protocol,
                ..
            } => {
                return OpenEvent {
                    socket_id,
                    protocol,
                };
            }
            Event::Error { message, .. } => {
                panic!("websocket unexpectedly failed before open: {message}")
            }
            Event::HandshakeResponse { .. } => {}
            Event::Close { code, reason, .. } => {
                panic!("websocket unexpectedly closed before open: {code} {reason}")
            }
            Event::TextMessage { .. }
            | Event::BinaryMessage { .. }
            | Event::SendCompleted { .. }
            | Event::Closing { .. } => {}
        }
    }
}

pub async fn recv_handshake_failure_events(event_rx: &mut mpsc::Receiver<Event>) -> String {
    let error_message = loop {
        let event = timeout(Duration::from_secs(3), event_rx.recv())
            .await
            .expect("websocket failure event should arrive")
            .expect("websocket event channel should stay open");
        match event {
            Event::Error { message, .. } => break message,
            Event::Open { .. } => panic!("websocket unexpectedly opened"),
            Event::HandshakeResponse { .. } => panic!("websocket unexpectedly paused"),
            Event::Close { code, reason, .. } => {
                panic!("websocket closed before error: {code} {reason}")
            }
            Event::TextMessage { .. }
            | Event::BinaryMessage { .. }
            | Event::SendCompleted { .. }
            | Event::Closing { .. } => {}
        }
    };

    loop {
        let event = timeout(Duration::from_secs(3), event_rx.recv())
            .await
            .expect("websocket close event should arrive")
            .expect("websocket event channel should stay open");
        match event {
            Event::Close {
                code,
                reason,
                was_clean,
                ..
            } => {
                assert_eq!(code, 1006);
                assert!(reason.is_empty());
                assert!(!was_clean);
                return error_message;
            }
            Event::Open { .. } => panic!("websocket unexpectedly opened after error"),
            Event::HandshakeResponse { .. } => panic!("websocket unexpectedly paused after error"),
            Event::Error { message, .. } => {
                panic!("websocket emitted a second error before close: {message}")
            }
            Event::TextMessage { .. }
            | Event::BinaryMessage { .. }
            | Event::SendCompleted { .. }
            | Event::Closing { .. } => {}
        }
    }
}

async fn recv_next_event(event_rx: &mut mpsc::Receiver<Event>) -> Event {
    timeout(Duration::from_secs(3), event_rx.recv())
        .await
        .expect("websocket event should arrive")
        .expect("websocket event channel should stay open")
}

pub async fn assert_send_completed(
    event_rx: &mut mpsc::Receiver<Event>,
    expected_socket_id: u64,
    expected_opcode: FrameOpcode,
    expected_payload_length: usize,
) {
    match recv_next_event(event_rx).await {
        Event::SendCompleted {
            socket_id,
            opcode,
            payload_length,
        } => {
            assert_eq!(socket_id, expected_socket_id);
            assert_eq!(opcode, expected_opcode);
            assert_eq!(payload_length, expected_payload_length);
        }
        event => panic!("expected send completion, got {event:?}"),
    }
}

pub async fn assert_text_message(
    event_rx: &mut mpsc::Receiver<Event>,
    expected_socket_id: u64,
    expected_data: &str,
) {
    match recv_next_event(event_rx).await {
        Event::TextMessage { socket_id, data } => {
            assert_eq!(socket_id, expected_socket_id);
            assert_eq!(data, expected_data);
        }
        event => panic!("expected text message event, got {event:?}"),
    }
}

pub async fn assert_binary_message(
    event_rx: &mut mpsc::Receiver<Event>,
    expected_socket_id: u64,
    expected_data: &[u8],
) {
    match recv_next_event(event_rx).await {
        Event::BinaryMessage { socket_id, data } => {
            assert_eq!(socket_id, expected_socket_id);
            assert_eq!(data, expected_data);
        }
        event => panic!("expected binary message event, got {event:?}"),
    }
}

pub async fn assert_closing(event_rx: &mut mpsc::Receiver<Event>, expected_socket_id: u64) {
    match recv_next_event(event_rx).await {
        Event::Closing { socket_id } => assert_eq!(socket_id, expected_socket_id),
        event => panic!("expected closing event, got {event:?}"),
    }
}

pub async fn assert_close(
    event_rx: &mut mpsc::Receiver<Event>,
    expected_socket_id: u64,
    expected_code: u16,
    expected_reason: &str,
    expected_was_clean: bool,
) {
    match recv_next_event(event_rx).await {
        Event::Close {
            socket_id,
            code,
            reason,
            was_clean,
        } => {
            assert_eq!(socket_id, expected_socket_id);
            assert_eq!(code, expected_code);
            assert_eq!(reason, expected_reason);
            assert_eq!(was_clean, expected_was_clean);
        }
        event => panic!("expected close event, got {event:?}"),
    }
}

pub async fn spawn_header_capture_websocket_server() -> (
    String,
    oneshot::Receiver<Vec<(String, String)>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket header server");
    let addr = listener.local_addr().expect("websocket header addr");
    let (headers_tx, headers_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept websocket client");
        let mut headers_tx = Some(headers_tx);
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            InfallibleHandshakeCallback(move |request: &Request, mut response: Response| {
                let headers = header_entries(request.headers());
                if let Some(headers_tx) = headers_tx.take() {
                    let _ = headers_tx.send(headers);
                }
                if let Some(protocol) = request.headers().get(http::header::SEC_WEBSOCKET_PROTOCOL)
                {
                    response
                        .headers_mut()
                        .insert(http::header::SEC_WEBSOCKET_PROTOCOL, protocol.clone());
                }
                response
            }),
        )
        .await
        .expect("accept websocket upgrade");
        let _ = socket.close(None).await;
    });
    (format!("ws://{addr}/headers"), headers_rx, handle)
}

pub async fn spawn_child_document_and_header_capture_websocket_server(
    child_path: &'static str,
    child_html: &'static str,
) -> (
    String,
    oneshot::Receiver<Vec<(String, String)>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind child document websocket server");
    let addr = listener
        .local_addr()
        .expect("child document websocket addr");
    let (headers_tx, headers_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let (mut document_stream, _) = listener.accept().await.expect("accept child document");
        let request = read_http_headers(&mut document_stream).await;
        assert!(
            request.starts_with(&format!("GET {child_path} ")),
            "unexpected child document request: {request:?}"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            child_html.len(),
            child_html
        );
        document_stream
            .write_all(response.as_bytes())
            .await
            .expect("write child document response");

        let (stream, _) = listener.accept().await.expect("accept child websocket");
        let mut headers_tx = Some(headers_tx);
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            InfallibleHandshakeCallback(move |request: &Request, response: Response| {
                let mut headers = header_entries(request.headers());
                headers.push((":path".to_owned(), request.uri().path().to_owned()));
                if let Some(headers_tx) = headers_tx.take() {
                    let _ = headers_tx.send(headers);
                }
                response
            }),
        )
        .await
        .expect("accept child websocket upgrade");
        let _ = socket.close(None).await;
    });
    (format!("http://{addr}"), headers_rx, handle)
}

pub async fn spawn_raw_websocket_response_server(
    path: &'static str,
    response: &'static [u8],
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind raw websocket response server");
    let addr = listener.local_addr().expect("raw websocket response addr");
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept raw websocket response client");
        let _request = read_http_headers(&mut stream).await;
        stream
            .write_all(response)
            .await
            .expect("write raw websocket response");
    });
    (format!("ws://{addr}/{path}"), handle)
}

pub async fn spawn_computed_accept_websocket_response_server(
    path: &'static str,
    headers: Vec<&'static str>,
) -> (String, tokio::task::JoinHandle<()>) {
    spawn_computed_accept_websocket_response_with_body_server(path, headers, b"").await
}

pub async fn spawn_computed_accept_websocket_response_with_body_server(
    path: &'static str,
    headers: Vec<&'static str>,
    body: &'static [u8],
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind computed websocket response body server");
    let addr = listener
        .local_addr()
        .expect("computed websocket response body addr");
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept computed websocket response body client");
        let request = read_http_headers(&mut stream).await;
        let key = websocket_request_header(&request, "sec-websocket-key")
            .expect("websocket request should include Sec-WebSocket-Key");
        let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
        let mut response = "HTTP/1.1 101 Switching Protocols\r\n".to_owned();
        for header in headers {
            response.push_str(header);
            response.push_str("\r\n");
        }
        response.push_str("Sec-WebSocket-Accept: ");
        response.push_str(&accept);
        response.push_str("\r\n\r\n");
        let mut packet = response.into_bytes();
        packet.extend_from_slice(body);
        stream
            .write_all(&packet)
            .await
            .expect("write computed handshake and frames");
        if body.first().is_some_and(|opcode| opcode & 0x0f == 8) {
            let mut socket = tokio_tungstenite::WebSocketStream::from_raw_socket(
                stream,
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            )
            .await;
            assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        }
    });
    (format!("ws://{addr}/{path}"), handle)
}

pub async fn spawn_tls_header_capture_websocket_server() -> (
    String,
    oneshot::Receiver<Vec<(String, String)>>,
    tokio::task::JoinHandle<()>,
) {
    use tokio_rustls::rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("self-signed websocket certificate");
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("websocket tls config");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket tls header server");
    let addr = listener.local_addr().expect("websocket tls header addr");
    let (headers_tx, headers_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept websocket client");
        let stream = acceptor.accept(stream).await.expect("accept websocket tls");
        let mut headers_tx = Some(headers_tx);
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            InfallibleHandshakeCallback(move |request: &Request, response: Response| {
                let headers = header_entries(request.headers());
                if let Some(headers_tx) = headers_tx.take() {
                    let _ = headers_tx.send(headers);
                }
                response
            }),
        )
        .await
        .expect("accept websocket tls upgrade");
        let _ = socket.close(None).await;
    });
    (format!("wss://{addr}/headers"), headers_rx, handle)
}

pub async fn spawn_http_connect_proxy() -> (
    String,
    oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket proxy");
    let addr = listener.local_addr().expect("websocket proxy addr");
    let (request_tx, request_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.expect("accept proxy client");
        let request = read_http_headers(&mut client).await;
        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("CONNECT target")
            .to_owned();
        let _ = request_tx.send(request);
        let mut upstream = TcpStream::connect(&target)
            .await
            .expect("connect proxy upstream");
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .expect("write proxy CONNECT response");
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    });
    (format!("http://{addr}"), request_rx, handle)
}

pub async fn spawn_http_connect_proxy_response(
    response: &'static [u8],
) -> (
    String,
    oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket proxy response");
    let addr = listener
        .local_addr()
        .expect("websocket proxy response addr");
    let (request_tx, request_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let (mut client, _) = listener
            .accept()
            .await
            .expect("accept proxy response client");
        let request = read_http_headers(&mut client).await;
        let _ = request_tx.send(request);
        client
            .write_all(response)
            .await
            .expect("write proxy CONNECT response");
    });
    (format!("http://{addr}"), request_rx, handle)
}

pub async fn spawn_text_echo_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    spawn_text_binary_echo_websocket_server().await
}

pub async fn spawn_text_binary_echo_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket echo server");
    let addr = listener.local_addr().expect("websocket echo addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept websocket client");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("accept websocket upgrade");
        while let Some(message) = socket.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    socket
                        .send(Message::Text(text))
                        .await
                        .expect("echo websocket text");
                }
                Ok(Message::Binary(bytes)) => {
                    socket
                        .send(Message::Binary(bytes))
                        .await
                        .expect("echo websocket binary");
                }
                Ok(Message::Close(_frame)) => {
                    // Reading Close has already queued the echo; flush it before dropping TCP.
                    let _ = socket.flush().await;
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    socket
                        .send(Message::Pong(payload))
                        .await
                        .expect("websocket pong");
                }
                Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
                Err(_) => break,
            }
        }
    });
    (format!("ws://{addr}/echo"), handle)
}

pub async fn spawn_triggered_text_websocket_server() -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Sender<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind triggered websocket server");
    let addr = listener.local_addr().expect("triggered websocket addr");
    let (opened_tx, opened_rx) = oneshot::channel();
    let (message_tx, message_rx) = oneshot::channel::<String>();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept triggered websocket client");
        let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        let _ = opened_tx.send(());
        let Ok(message) = message_rx.await else {
            return;
        };
        let _ = socket.send(Message::Text(message.into())).await;
        let _ = socket.close(None).await;
    });
    (
        format!("ws://{addr}/triggered-text"),
        opened_rx,
        message_tx,
        handle,
    )
}

pub async fn spawn_subprotocol_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket subprotocol server");
    let addr = listener.local_addr().expect("websocket subprotocol addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket subprotocol client");
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            InfallibleHandshakeCallback(|request: &Request, mut response: Response| {
                let requested = request
                    .headers()
                    .get("sec-websocket-protocol")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default();
                if requested
                    .split(',')
                    .map(str::trim)
                    .any(|protocol| protocol == "superchat")
                {
                    response.headers_mut().insert(
                        "sec-websocket-protocol",
                        tokio_tungstenite::tungstenite::http::HeaderValue::from_static("superchat"),
                    );
                }
                response
            }),
        )
        .await
        .expect("accept websocket subprotocol upgrade");
        let _ = socket.close(None).await;
    });
    (format!("ws://{addr}/subprotocol"), handle)
}

pub async fn spawn_server_close_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    spawn_server_close_websocket_server_with_frame(Some((3001, "server done".to_owned()))).await
}

pub async fn spawn_server_close_websocket_server_with_frame(
    close_frame: Option<(u16, String)>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket server-close server");
    let addr = listener.local_addr().expect("websocket server-close addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket server-close client");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("accept websocket server-close upgrade");
        let frame = close_frame.map(|(code, reason)| CloseFrame {
            code: code.into(),
            reason: reason.into(),
        });
        let _ = socket.send(Message::Close(frame)).await;
        let _ = socket.next().await;
    });
    (format!("ws://{addr}/server-close"), handle)
}

pub async fn spawn_close_after_goodbye_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket close-after-goodbye server");
    let addr = listener
        .local_addr()
        .expect("websocket close-after-goodbye addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket close-after-goodbye client");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("accept websocket close-after-goodbye upgrade");
        while let Some(message) = socket.next().await {
            match message {
                Ok(Message::Text(text)) if text.as_str() == "Goodbye" => {
                    let _ = socket
                        .send(Message::Close(Some(CloseFrame {
                            code: 1000.into(),
                            reason: "goodbye".into(),
                        })))
                        .await;
                    // Keep reading until the client's reply, including a data
                    // fragment that was already being sent when we closed.
                    while let Some(message) = socket.next().await {
                        if matches!(message, Ok(Message::Close(_)) | Err(_)) {
                            break;
                        }
                    }
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    let _ = socket.send(Message::Pong(payload)).await;
                }
                Ok(Message::Close(_frame)) => {
                    // Reading Close has already queued the echo; flush it before dropping TCP.
                    let _ = socket.flush().await;
                    break;
                }
                Ok(Message::Text(_))
                | Ok(Message::Binary(_))
                | Ok(Message::Pong(_))
                | Ok(Message::Frame(_)) => {}
                Err(_) => break,
            }
        }
    });
    (format!("ws://{addr}/close-after-goodbye"), handle)
}

pub async fn spawn_abrupt_close_after_open_websocket_server()
-> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket abrupt-close server");
    let addr = listener.local_addr().expect("websocket abrupt-close addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket abrupt-close client");
        let socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("accept websocket abrupt-close upgrade");
        drop(socket);
    });
    (format!("ws://{addr}/abrupt-close"), handle)
}

pub async fn spawn_cookie_echo_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket cookie echo server");
    let addr = listener.local_addr().expect("websocket cookie echo addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket cookie echo client");
        let (cookie_tx, cookie_rx) = oneshot::channel();
        let mut cookie_tx = Some(cookie_tx);
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            InfallibleHandshakeCallback(move |request: &Request, response: Response| {
                let cookie = request
                    .headers()
                    .get("cookie")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                if let Some(cookie_tx) = cookie_tx.take() {
                    let _ = cookie_tx.send(cookie);
                }
                response
            }),
        )
        .await
        .expect("accept websocket cookie echo upgrade");
        let cookie = cookie_rx.await.unwrap_or_default();
        let _ = socket.send(Message::Text(cookie.into())).await;
        let _ = socket.close(None).await;
    });
    (format!("ws://{addr}/echo-cookie"), handle)
}

pub async fn spawn_sleeping_handshake_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket sleeping handshake server");
    let addr = listener
        .local_addr()
        .expect("websocket sleeping handshake addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket sleeping handshake client");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        while let Some(message) = socket.next().await {
            if matches!(message, Ok(Message::Close(_)) | Err(_)) {
                break;
            }
        }
    });
    (format!("ws://{addr}/handshake-sleep"), handle)
}

pub async fn spawn_delayed_passive_close_websocket_server() -> (String, tokio::task::JoinHandle<()>)
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket delayed passive close server");
    let addr = listener
        .local_addr()
        .expect("websocket delayed passive close addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket delayed passive close client");
        let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        while let Some(message) = socket.next().await {
            match message {
                Ok(Message::Close(_frame)) => {
                    tokio::time::sleep(Duration::from_millis(1000)).await;
                    // Reading Close has already queued the echo; flush it before dropping TCP.
                    let _ = socket.flush().await;
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    let _ = socket.send(Message::Pong(payload)).await;
                }
                Ok(Message::Text(_))
                | Ok(Message::Binary(_))
                | Ok(Message::Pong(_))
                | Ok(Message::Frame(_)) => {}
                Err(_) => break,
            }
        }
    });
    (format!("ws://{addr}/delayed-passive-close"), handle)
}

pub async fn spawn_backpressure_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket backpressure server");
    let addr = listener.local_addr().expect("websocket backpressure addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket backpressure client");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("accept websocket backpressure upgrade");
        while let Some(message) = socket.next().await {
            match message.expect("websocket backpressure message") {
                Message::Binary(bytes) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    socket
                        .send(Message::Text(bytes.len().to_string().into()))
                        .await
                        .expect("send websocket backpressure ack");
                }
                Message::Text(text) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    socket
                        .send(Message::Text(text.len().to_string().into()))
                        .await
                        .expect("send websocket backpressure text ack");
                }
                Message::Close(_frame) => {
                    // Reading Close has already queued the echo; flush it before dropping TCP.
                    let _ = socket.flush().await;
                    break;
                }
                Message::Ping(payload) => {
                    let _ = socket.send(Message::Pong(payload)).await;
                }
                Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    });
    (format!("ws://{addr}/backpressure"), handle)
}

pub async fn spawn_send_backpressure_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket send-backpressure server");
    let addr = listener
        .local_addr()
        .expect("websocket send-backpressure addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket send-backpressure client");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("accept websocket send-backpressure upgrade");
        tokio::time::sleep(Duration::from_millis(2000)).await;
        if let Some(message) = socket.next().await {
            match message.expect("websocket send-backpressure message") {
                Message::Binary(_) | Message::Text(_) => {
                    let _ = socket.close(None).await;
                }
                Message::Close(_frame) => {
                    // Reading Close has already queued the echo; flush it before dropping TCP.
                    let _ = socket.flush().await;
                }
                Message::Ping(payload) => {
                    let _ = socket.send(Message::Pong(payload)).await;
                }
                Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    });
    (format!("ws://{addr}/send-backpressure"), handle)
}

pub async fn spawn_receive_backpressure_websocket_server() -> (String, tokio::task::JoinHandle<()>)
{
    const LARGE_MESSAGE_COUNT: usize = 32;
    const LARGE_MESSAGE_SIZE: usize = 1024 * 1024;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket receive-backpressure server");
    let addr = listener
        .local_addr()
        .expect("websocket receive-backpressure addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket receive-backpressure client");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("accept websocket receive-backpressure upgrade");
        socket
            .send(Message::Text("".into()))
            .await
            .expect("send websocket receive-backpressure primer");
        let payload = vec![b'x'; LARGE_MESSAGE_SIZE];
        let start = tokio::time::Instant::now();
        for _ in 0..LARGE_MESSAGE_COUNT {
            socket
                .send(Message::Binary(payload.clone().into()))
                .await
                .expect("send websocket receive-backpressure payload");
        }
        let elapsed = start.elapsed().as_secs_f64();
        socket
            .send(Message::Text(format!("{elapsed:.3}").into()))
            .await
            .expect("send websocket receive-backpressure elapsed");
        let _ = socket.close(None).await;
    });
    (format!("ws://{addr}/receive-backpressure"), handle)
}

pub async fn spawn_set_cookie_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket set-cookie server");
    let addr = listener.local_addr().expect("websocket set-cookie addr");
    let handle = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket set-cookie client");
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            InfallibleHandshakeCallback(|_request: &Request, mut response: Response| {
                response.headers_mut().insert(
                    "set-cookie",
                    tokio_tungstenite::tungstenite::http::HeaderValue::from_static(
                        "ws_response_cookie=ok; Path=/",
                    ),
                );
                response
            }),
        )
        .await
        .expect("accept websocket set-cookie upgrade");
        let _ = socket.close(None).await;
    });
    (format!("ws://{addr}/set-cookie"), handle)
}

pub async fn spawn_dropping_websocket_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket drop server");
    let addr = listener.local_addr().expect("websocket drop addr");
    let handle = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.expect("accept websocket client");
    });
    (format!("ws://{addr}/drop"), handle)
}

async fn read_http_headers(stream: &mut TcpStream) -> String {
    let mut request_bytes = Vec::new();
    let mut chunk = [0_u8; 512];
    loop {
        let count = stream.read(&mut chunk).await.expect("read HTTP request");
        assert!(count > 0, "client closed before HTTP headers completed");
        request_bytes.extend_from_slice(&chunk[..count]);
        if request_bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&request_bytes).into_owned()
}

fn websocket_request_header(request: &str, name: &str) -> Option<String> {
    request.lines().find_map(|line| {
        let (header_name, value) = line.split_once(':')?;
        header_name
            .trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

fn header_entries(headers: &http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

pub fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find_map(|(header_name, value)| (header_name == name).then(|| value.clone()))
}

/// Observes physical transport retirement without cooperating in a Close handshake.
pub async fn spawn_transport_retirement_websocket_server() -> (String, tokio::task::JoinHandle<()>)
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind retirement server");
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let terminal = timeout(Duration::from_secs(3), socket.next())
            .await
            .expect("retirement must physically release TCP");
        assert!(
            terminal.is_none() || terminal.is_some_and(|event| event.is_err()),
            "retirement cancels without a graceful Close"
        );
    });
    (format!("ws://{addr}/retirement"), server)
}
