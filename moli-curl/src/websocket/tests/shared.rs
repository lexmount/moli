use std::num::NonZeroUsize;

use curl::easy::{Easy2, Handler, WriteError};

use super::*;
use crate::{CurlMultiJob, CurlOriginKey};

#[derive(Default, Debug)]
struct HttpCapture {
    body: Vec<u8>,
    owner: Option<thread::ThreadId>,
    headers: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Handler for HttpCapture {
    fn header(&mut self, data: &[u8]) -> bool {
        if data == b"\r\n"
            && let Some(sender) = self.headers.take()
        {
            let _ = sender.send(());
        }
        true
    }

    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        self.owner = Some(thread::current().id());
        self.body.extend_from_slice(data);
        Ok(data.len())
    }
}

fn request(url: &str, handler: HttpCapture, timeout: Duration) -> CurlMultiJob<HttpCapture, ()> {
    let mut easy = Easy2::new(handler);
    easy.url(url).unwrap();
    easy.proxy("").unwrap();
    let url = url::Url::parse(url).unwrap();
    CurlMultiJob {
        easy,
        context: (),
        origin: Some(CurlOriginKey {
            scheme: url.scheme().to_owned(),
            host: url.host_str().unwrap().to_owned(),
            port: url.port_or_known_default(),
        }),
        deadline: Some(std::time::Instant::now() + timeout),
        dns_resolution: CurlDnsResolution::curl_managed(),
        priority: 1,
        label: "mixed test".to_owned(),
    }
}

// HTTP and WS use the same authority so both per-host and total pool caps are
// exercised. /held keeps an HTTP connection occupied until explicitly released.
fn peer() -> (
    String,
    crossbeam_channel::Sender<()>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (release, released) = crossbeam_channel::bounded(1);
    let task = thread::spawn(move || {
        let mut children = Vec::new();
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(DEADLINE)).unwrap();
            stream.set_write_timeout(Some(DEADLINE)).unwrap();
            let request = read_request(&mut stream);
            if request.starts_with("GET /stop ") {
                break;
            }
            let released = released.clone();
            children.push(thread::spawn(move || {
                if request.starts_with("GET /native ") {
                    let key = request.lines().find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("sec-websocket-key").then_some(value.trim())
                    }).unwrap();
                    write!(stream, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", derive_accept_key(key.as_bytes())).unwrap();
                    let mut socket = tungstenite::WebSocket::from_raw_socket(stream, tungstenite::protocol::Role::Server, None);
                    match socket.read() {
                        Ok(message) => {
                            socket.send(message).unwrap();
                            assert_eq!(socket.get_mut().read(&mut [0]).unwrap(), 0);
                        }
                        Err(tungstenite::Error::Protocol(tungstenite::error::ProtocolError::ResetWithoutClosingHandshake)) => {}
                        result => panic!("unexpected peer read: {result:?}"),
                    }
                } else {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n").unwrap();
                    if request.starts_with("GET /held ") {
                        released.recv_timeout(DEADLINE).unwrap();
                    }
                    stream.write_all(b"ok").unwrap();
                }
            }));
        }
        for child in children {
            child.join().unwrap();
        }
    });
    (base, release, task)
}

fn stop_peer(base: &str, peer: thread::JoinHandle<()>) {
    let authority = base.strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(authority).unwrap();
    stream
        .write_all(b"GET /stop HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    peer.join().unwrap();
}

fn config(total: bool) -> CurlMultiRuntimeConfig {
    CurlMultiRuntimeConfig {
        max_active: NonZeroUsize::new(2).unwrap(),
        max_host_connections: (!total).then(|| NonZeroUsize::new(1).unwrap()),
        max_total_connections: total.then(|| NonZeroUsize::new(1).unwrap()),
        ..CurlMultiRuntimeConfig::default()
    }
}

#[tokio::test]
async fn shared_owner_keeps_http_and_websocket_pools_independent() {
    for total in [false, true] {
        let (base, _, peer) = peer();
        let (runtime, completed) = CurlMultiRuntime::new(config(total)).unwrap();
        let connector = runtime.websocket_connector();
        let mut connections = Vec::new();
        // Neither the HTTP host cap nor its total cap of 1 limits these sessions.
        for _ in 0..2 {
            let mut request = CurlWebSocketRequest::new(base.replacen("http", "ws", 1) + "/native");
            request.handshake_timeout = DEADLINE;
            let mut connection = connector.connect(request).unwrap();
            opened(&mut connection).await;
            connections.push(connection);
        }
        runtime
            .http_sender()
            .submit(request(
                &(base.clone() + "/ok"),
                HttpCapture::default(),
                DEADLINE,
            ))
            .unwrap();
        let completion = completed.recv_timeout(DEADLINE).unwrap();
        completion.result.unwrap();
        let easy = completion.easy.unwrap();
        assert_eq!(easy.get_ref().body, b"ok");
        for mut connection in connections {
            assert_eq!(
                *connection.sender.control.owner_thread.lock(),
                easy.get_ref().owner
            );
            connection.sender().set_reading(true);
            connection
                .sender()
                .send_frame(CurlWebSocketSend {
                    flags: WsFlags::BINARY,
                    data: vec![42],
                })
                .await
                .unwrap();
            assert!(
                matches!(event(&mut connection).await, CurlWebSocketEvent::Chunk { data, .. } if data == [42])
            );
            drop(connection);
        }
        // Retiring WS does not retire the shared owner or its HTTP pool.
        runtime
            .http_sender()
            .submit(request(
                &(base.clone() + "/after"),
                HttpCapture::default(),
                DEADLINE,
            ))
            .unwrap();
        completed.recv_timeout(DEADLINE).unwrap().result.unwrap();
        drop(runtime);
        assert!(
            connector
                .connect(CurlWebSocketRequest::new(
                    base.replacen("http", "ws", 1) + "/native"
                ))
                .is_err()
        );
        stop_peer(&base, peer);
    }
}

#[tokio::test]
async fn shared_owner_preserves_http_connection_caps_with_live_websocket() {
    for total in [false, true] {
        let (base, release, peer) = peer();
        let (runtime, completed) = CurlMultiRuntime::new(config(total)).unwrap();
        let connector = runtime.websocket_connector();
        let mut connection = connector
            .connect(CurlWebSocketRequest::new(
                base.replacen("http", "ws", 1) + "/native",
            ))
            .unwrap();
        opened(&mut connection).await;
        let (headers, ready) = tokio::sync::oneshot::channel();
        let first = runtime
            .http_sender()
            .submit(request(
                &(base.clone() + "/held"),
                HttpCapture {
                    headers: Some(headers),
                    ..HttpCapture::default()
                },
                DEADLINE,
            ))
            .unwrap();
        timeout(DEADLINE, ready).await.unwrap().unwrap();
        // Both HTTP jobs fit max_active=2. Only the connection cap keeps the
        // second waiting until its own deadline, while the first is held open.
        let second = runtime
            .http_sender()
            .submit(request(
                &(base.clone() + "/queued"),
                HttpCapture::default(),
                Duration::from_millis(200),
            ))
            .unwrap();
        let completion = completed.recv_timeout(DEADLINE).unwrap();
        assert_eq!(completion.transfer_id, second);
        assert!(completion.result.unwrap_err().chain().any(|error| {
            error
                .downcast_ref::<curl::Error>()
                .is_some_and(curl::Error::is_operation_timedout)
        }));
        release.send(()).unwrap();
        let completion = completed.recv_timeout(DEADLINE).unwrap();
        assert_eq!(completion.transfer_id, first);
        completion.result.unwrap();
        drop(connection);
        runtime.shutdown();
        stop_peer(&base, peer);
    }
}

#[tokio::test]
async fn shared_owner_shutdown_releases_websocket_with_live_connector() {
    let (base, _, peer) = peer();
    let (runtime, _) = CurlMultiRuntime::<HttpCapture, ()>::new(config(true)).unwrap();
    let connector = runtime.websocket_connector();
    let mut connection = connector
        .connect(CurlWebSocketRequest::new(
            base.replacen("http", "ws", 1) + "/native",
        ))
        .unwrap();
    opened(&mut connection).await;
    // Keep both capabilities alive. Neither owns the native thread's lifetime.
    drop(runtime);
    while let Some(event) = timeout(DEADLINE, connection.recv()).await.unwrap() {
        if let CurlWebSocketEvent::Closed { result } = event {
            assert!(result.is_err());
        }
    }
    assert!(
        connector
            .connect(CurlWebSocketRequest::new(
                "ws://127.0.0.1:1/native".to_owned()
            ))
            .is_err()
    );
    stop_peer(&base, peer);
}
