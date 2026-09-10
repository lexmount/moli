use std::num::NonZeroUsize;

use curl::easy::{Easy2, Handler, InfoType, WriteError};

use super::*;
use crate::{CurlMultiJob, CurlMultiRuntime, CurlMultiRuntimeConfig, CurlOriginKey};

#[derive(Default, Debug)]
struct HttpCapture {
    body: Vec<u8>,
    owner: Option<thread::ThreadId>,
    headers: Option<tokio::sync::oneshot::Sender<()>>,
    queued: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Handler for HttpCapture {
    fn debug(&mut self, kind: InfoType, data: &[u8]) {
        // Wait for the native connection-pool decision, not wall-clock delay.
        if matches!(kind, InfoType::Text)
            && (data.starts_with(b"No more connections allowed to host")
                || data.starts_with(b"No connections available, total of"))
            && let Some(queued) = self.queued.take()
        {
            queued.send(()).unwrap();
        }
    }

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
    easy.verbose(easy.get_ref().queued.is_some()).unwrap();
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
    thread::JoinHandle<Vec<String>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (release, released) = crossbeam_channel::bounded(1);
    let task = thread::spawn(move || {
        let mut children = Vec::new();
        let mut paths = Vec::new();
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(DEADLINE)).unwrap();
            stream.set_write_timeout(Some(DEADLINE)).unwrap();
            let request = read_request(&mut stream);
            if request.starts_with("GET /stop ") {
                break;
            }
            paths.push(request.split_whitespace().nth(1).unwrap().to_owned());
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
        paths
    });
    (base, release, task)
}

fn stop_peer(base: &str, peer: thread::JoinHandle<Vec<String>>) -> Vec<String> {
    let authority = base.strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(authority).unwrap();
    stream
        .write_all(b"GET /stop HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    peer.join().unwrap()
}

fn config(total: bool) -> CurlMultiRuntimeConfig {
    CurlMultiRuntimeConfig {
        max_active: NonZeroUsize::new(2).unwrap(),
        max_host_connections: (!total).then(|| NonZeroUsize::new(1).unwrap()),
        max_total_connections: total.then(|| NonZeroUsize::new(1).unwrap()),
        ..CurlMultiRuntimeConfig::default()
    }
}

fn websocket_request(base: &str) -> CurlWebSocketRequest {
    let mut request = CurlWebSocketRequest::new(base.replacen("http", "ws", 1) + "/native");
    request.handshake_timeout = DEADLINE;
    request
}

async fn hold_http(runtime: &CurlMultiRuntime<HttpCapture, ()>, base: &str) {
    let (headers, ready) = oneshot::channel();
    runtime
        .http_sender()
        .submit(request(
            &format!("{base}/held"),
            HttpCapture {
                headers: Some(headers),
                ..HttpCapture::default()
            },
            DEADLINE,
        ))
        .unwrap();
    timeout(DEADLINE, ready).await.unwrap().unwrap();
}

async fn pool_waiting(connection: &CurlWebSocketConnection) {
    timeout(
        DEADLINE,
        connection.sender().control.pool_waiting.notified(),
    )
    .await
    .expect("WebSocket must reach the native connection pool wait");
}

async fn echo(connection: &mut CurlWebSocketConnection) {
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
        matches!(event(connection).await, CurlWebSocketEvent::Chunk { data, .. } if data == [42])
    );
}

#[tokio::test]
async fn shared_connection_limits_resume_http_after_websocket_release() {
    for total in [false, true] {
        let (base, _, peer) = peer();
        let (runtime, completed) = CurlMultiRuntime::new(config(total)).unwrap();
        let connector = runtime.websocket_connector();
        let mut connection = connector.connect(websocket_request(&base)).unwrap();
        opened(&mut connection).await;
        let owner = *connection.sender().control.owner_thread.lock();
        let (queued, waiting) = oneshot::channel();
        let id = runtime
            .http_sender()
            .submit(request(
                &(base.clone() + "/queued"),
                HttpCapture {
                    queued: Some(queued),
                    ..HttpCapture::default()
                },
                DEADLINE,
            ))
            .unwrap();
        timeout(DEADLINE, waiting).await.unwrap().unwrap();
        assert!(completed.is_empty());
        // It was silent during admission. Pool pressure must not evict it.
        echo(&mut connection).await;
        drop(connection);
        let completion = completed.recv_timeout(DEADLINE).unwrap();
        assert_eq!(completion.transfer_id, id);
        completion.result.unwrap();
        let easy = completion.easy.unwrap();
        assert_eq!(easy.get_ref().body, b"ok");
        assert_eq!(easy.get_ref().owner, owner);
        // HTTP remains usable on this same native owner after WS teardown.
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
        assert!(connector.connect(websocket_request(&base)).is_err());
        assert_eq!(stop_peer(&base, peer), ["/native", "/queued", "/after"]);
    }
}

#[tokio::test]
async fn shared_connection_limits_keep_quiet_websocket_through_http_timeout() {
    for total in [false, true] {
        let (base, _, peer) = peer();
        let (runtime, completed) = CurlMultiRuntime::new(config(total)).unwrap();
        let mut connection = runtime
            .websocket_connector()
            .connect(websocket_request(&base))
            .unwrap();
        opened(&mut connection).await;
        let id = runtime
            .http_sender()
            .submit(request(
                &(base.clone() + "/timed-out"),
                HttpCapture::default(),
                Duration::from_millis(200),
            ))
            .unwrap();
        let completion = completed.recv_timeout(DEADLINE).unwrap();
        assert_eq!(completion.transfer_id, id);
        assert!(completion.result.unwrap_err().chain().any(|error| {
            error
                .downcast_ref::<curl::Error>()
                .is_some_and(curl::Error::is_operation_timedout)
        }));
        echo(&mut connection).await;
        drop(connection);
        drop(runtime);
        assert_eq!(stop_peer(&base, peer), ["/native"]);
    }
}

#[tokio::test]
async fn shared_connection_limits_resume_websocket_after_http_release() {
    for total in [false, true] {
        let (base, release, peer) = peer();
        let (runtime, completed) = CurlMultiRuntime::new(config(total)).unwrap();
        hold_http(&runtime, &base).await;
        let mut connection = runtime
            .websocket_connector()
            .connect(websocket_request(&base))
            .unwrap();
        pool_waiting(&connection).await;
        assert!(connection.events.is_empty());
        release.send(()).unwrap();
        completed.recv_timeout(DEADLINE).unwrap().result.unwrap();
        opened(&mut connection).await;
        echo(&mut connection).await;
        drop(connection);
        drop(runtime);
        assert_eq!(stop_peer(&base, peer), ["/held", "/native"]);
    }
}

#[tokio::test]
async fn shared_connection_limits_cancel_waiting_websocket_and_release_admission() {
    for total in [false, true] {
        let (base, release, peer) = peer();
        let (runtime, completed) = CurlMultiRuntime::new(config(total)).unwrap();
        hold_http(&runtime, &base).await;
        let connector = runtime.websocket_connector();
        let mut connection = connector.connect(websocket_request(&base)).unwrap();
        pool_waiting(&connection).await;
        assert_eq!(connector.available_session_slots(), SESSION_CAPACITY - 1);
        connection.sender().cancel();
        assert!(matches!(
            event(&mut connection).await,
            CurlWebSocketEvent::Closed { result: Ok(()) }
        ));
        // Terminal delivery guarantees the native admission has been released,
        // even while the application's receiver and connector remain alive.
        assert_eq!(connector.available_session_slots(), SESSION_CAPACITY);
        release.send(()).unwrap();
        completed.recv_timeout(DEADLINE).unwrap().result.unwrap();
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
        // The cancelled waiter must never issue its upgrade after capacity returns.
        assert_eq!(stop_peer(&base, peer), ["/held", "/after"]);
    }
}

#[tokio::test]
async fn shared_connection_limits_include_pool_wait_in_websocket_deadline() {
    for total in [false, true] {
        let (base, release, peer) = peer();
        let (runtime, completed) = CurlMultiRuntime::new(config(total)).unwrap();
        hold_http(&runtime, &base).await;
        let connector = runtime.websocket_connector();
        let mut request = websocket_request(&base);
        request.handshake_timeout = Duration::from_millis(200);
        let mut connection = connector.connect(request).unwrap();
        pool_waiting(&connection).await;
        let mut timed_out = false;
        while let Some(event) = timeout(DEADLINE, connection.recv()).await.unwrap() {
            match event {
                CurlWebSocketEvent::Handshake {
                    result: Err(error), ..
                }
                | CurlWebSocketEvent::Closed { result: Err(error) } => {
                    let error = error.to_ascii_lowercase();
                    timed_out |= error.contains("timed out") || error.contains("timeout");
                }
                other => panic!("unexpected event while waiting for pool capacity: {other:?}"),
            }
        }
        assert!(timed_out);
        assert_eq!(connector.available_session_slots(), SESSION_CAPACITY);
        release.send(()).unwrap();
        completed.recv_timeout(DEADLINE).unwrap().result.unwrap();
        drop(runtime);
        assert_eq!(stop_peer(&base, peer), ["/held"]);
    }
}

#[tokio::test]
async fn shared_owner_shutdown_releases_open_and_waiting_websockets_with_live_connector() {
    for total in [false, true] {
        let (base, _, peer) = peer();
        let (runtime, _) = CurlMultiRuntime::<HttpCapture, ()>::new(config(total)).unwrap();
        let connector = runtime.websocket_connector();
        let mut connection = connector.connect(websocket_request(&base)).unwrap();
        opened(&mut connection).await;
        let mut waiting = connector.connect(websocket_request(&base)).unwrap();
        pool_waiting(&waiting).await;
        // Keep the capabilities alive. Neither owns the native thread's lifetime.
        drop(runtime);
        for connection in [&mut connection, &mut waiting] {
            assert!(matches!(
                event(connection).await,
                CurlWebSocketEvent::Closed { result: Err(_) }
            ));
        }
        assert_eq!(connector.available_session_slots(), SESSION_CAPACITY);
        assert!(connector.connect(websocket_request(&base)).is_err());
        assert_eq!(stop_peer(&base, peer), ["/native"]);
    }
}
