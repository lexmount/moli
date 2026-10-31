//! Real TCP admission and handshake barriers, without timing-based sleeps.

use super::*;

struct Listener {
    incoming: tokio::net::TcpListener,
    observer: TcpListener,
}

struct Handshake {
    stream: TcpStream,
    request: String,
}

impl Listener {
    fn bind(address: impl std::net::ToSocketAddrs) -> Self {
        let observer = TcpListener::bind(address).unwrap();
        observer.set_nonblocking(true).unwrap();
        let incoming = tokio::net::TcpListener::from_std(observer.try_clone().unwrap()).unwrap();
        Self { incoming, observer }
    }

    fn new() -> Self {
        Self::bind(("127.0.0.1", 0))
    }

    fn port(&self) -> u16 {
        self.observer.local_addr().unwrap().port()
    }

    fn request(&self, host: &str, address: &str) -> CurlWebSocketRequest {
        let mut request = CurlWebSocketRequest::new(format!("ws://{host}:{}/native", self.port()));
        request.resolve_entries = vec![format!("{host}:{}:{address}", self.port())];
        request.handshake_timeout = DEADLINE;
        request
    }

    async fn stream(&self) -> TcpStream {
        let (stream, _) = timeout(DEADLINE, self.incoming.accept())
            .await
            .unwrap()
            .unwrap();
        let stream = stream.into_std().unwrap();
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(DEADLINE)).unwrap();
        stream.set_write_timeout(Some(DEADLINE)).unwrap();
        stream
    }

    async fn handshake(&self) -> Handshake {
        let mut stream = self.stream().await;
        let request = read_request(&mut stream);
        Handshake { stream, request }
    }

    async fn proxy_handshake(&self) -> Handshake {
        let mut stream = self.stream().await;
        let connect = read_request(&mut stream);
        assert!(connect.starts_with("CONNECT "));
        stream
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .unwrap();
        let request = read_request(&mut stream);
        Handshake { stream, request }
    }

    fn assert_idle(&self) {
        match self.observer.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            result => panic!("queued endpoint must not open a TCP socket: {result:?}"),
        }
    }
}

impl Handshake {
    fn upgrade(&mut self) {
        let key = self
            .request
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("sec-websocket-key")
                    .then_some(value.trim())
            })
            .unwrap();
        write!(self.stream, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", derive_accept_key(key.as_bytes())).unwrap();
    }
}

async fn waiting(connection: &CurlWebSocketConnection) {
    timeout(
        DEADLINE,
        connection.sender().control.endpoint_waiting.notified(),
    )
    .await
    .expect("native TCP endpoint must actually enter the wait queue");
}

#[tokio::test]
async fn aliases_serialize_handshakes_fifo_and_release_before_socket_close() {
    let listener = Listener::new();
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut first = runtime
        .connect(listener.request("first.test", "127.0.0.1"))
        .unwrap();
    let mut first_peer = listener.handshake().await;
    let mut second = runtime
        .connect(listener.request("second.test", "127.0.0.1"))
        .unwrap();
    waiting(&second).await;
    let mut third = runtime
        .connect(listener.request("third.test", "127.0.0.1"))
        .unwrap();
    waiting(&third).await;
    listener.assert_idle();

    first_peer.upgrade();
    opened(&mut first).await;
    let mut second_peer = listener.handshake().await;
    assert!(second_peer.request.contains("Host: second.test:"));
    listener.assert_idle();

    // The first upgraded socket is still usable while the second handshakes.
    first.sender().set_reading(true);
    first_peer.stream.write_all(&[0x81, 2, b'o', b'k']).unwrap();
    assert!(
        matches!(event(&mut first).await, CurlWebSocketEvent::Chunk { data, .. } if data == b"ok")
    );
    second_peer.upgrade();
    opened(&mut second).await;
    let mut third_peer = listener.handshake().await;
    assert!(third_peer.request.contains("Host: third.test:"));
    third_peer.upgrade();
    opened(&mut third).await;
}

#[tokio::test]
async fn cancelled_and_expired_endpoint_waiters_never_connect_or_block_successors() {
    let listener = Listener::new();
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut first = runtime
        .connect(listener.request("held.test", "127.0.0.1"))
        .unwrap();
    let mut first_peer = listener.handshake().await;

    let mut cancelled = runtime
        .connect(listener.request("cancelled.test", "127.0.0.1"))
        .unwrap();
    waiting(&cancelled).await;
    cancelled.sender().cancel();
    assert!(matches!(
        event(&mut cancelled).await,
        CurlWebSocketEvent::Closed { result: Ok(()) }
    ));

    let mut request = listener.request("expired.test", "127.0.0.1");
    request.handshake_timeout = Duration::from_secs(1);
    let mut expired = runtime.connect(request).unwrap();
    waiting(&expired).await;
    assert!(matches!(
        event(&mut expired).await,
        CurlWebSocketEvent::Closed { result: Err(_) }
            | CurlWebSocketEvent::Handshake { result: Err(_), .. }
    ));

    let mut next = runtime
        .connect(listener.request("next.test", "127.0.0.1"))
        .unwrap();
    waiting(&next).await;
    listener.assert_idle();
    first_peer.upgrade();
    opened(&mut first).await;
    let mut peer = listener.handshake().await;
    assert!(peer.request.contains("Host: next.test:"));
    peer.upgrade();
    opened(&mut next).await;
    listener.assert_idle();
}

#[tokio::test]
async fn failed_cancelled_and_expired_holders_wake_the_next_endpoint() {
    for terminal in ["failure", "cancel", "timeout"] {
        let listener = Listener::new();
        let runtime = CurlWebSocketRuntime::new().unwrap();
        let mut request = listener.request("held.test", "127.0.0.1");
        if terminal == "timeout" {
            request.handshake_timeout = Duration::from_secs(2);
        }
        let mut first = runtime.connect(request).unwrap();
        let mut first_peer = listener.handshake().await;
        let mut next = runtime
            .connect(listener.request("next.test", "127.0.0.1"))
            .unwrap();
        waiting(&next).await;
        listener.assert_idle();
        match terminal {
            "failure" => first_peer
                .stream
                .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .unwrap(),
            "cancel" => first.sender().cancel(),
            "timeout" => {}
            _ => unreachable!(),
        }
        match event(&mut first).await {
            CurlWebSocketEvent::Closed { result } => {
                assert_eq!(result.is_ok(), terminal == "cancel")
            }
            CurlWebSocketEvent::Handshake { result, .. } => assert!(result.is_err()),
            unexpected => panic!("expected opening terminal, got {unexpected:?}"),
        }
        let mut next_peer = listener.handshake().await;
        next_peer.upgrade();
        opened(&mut next).await;
    }
}

#[tokio::test]
async fn distinct_ports_and_native_owners_connect_independently() {
    let listener = Listener::new();
    let other_port = Listener::new();
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut first = runtime
        .connect(listener.request("same.test", "127.0.0.1"))
        .unwrap();
    let _first_peer = listener.handshake().await;
    let mut other = runtime
        .connect(other_port.request("same.test", "127.0.0.1"))
        .unwrap();
    let mut other_peer = other_port.handshake().await;
    other_peer.upgrade();
    opened(&mut other).await;

    let independent = CurlWebSocketRuntime::new().unwrap();
    let mut isolated = independent
        .connect(listener.request("same.test", "127.0.0.1"))
        .unwrap();
    let mut isolated_peer = listener.handshake().await;
    isolated_peer.upgrade();
    opened(&mut isolated).await;
    first.sender().cancel();
    assert!(matches!(
        event(&mut first).await,
        CurlWebSocketEvent::Closed { .. }
    ));
}

#[tokio::test]
async fn proxy_admission_uses_origin_host_instead_of_proxy_address() {
    let proxy = Listener::new();
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let request = |host| {
        let mut request = CurlWebSocketRequest::new(format!("ws://{host}/native"));
        request.proxy = Some(format!("http://127.0.0.1:{}", proxy.port()));
        request.handshake_timeout = DEADLINE;
        request
    };
    let mut first = runtime.connect(request("first.test")).unwrap();
    let mut first_peer = proxy.proxy_handshake().await;
    let mut same_host = runtime.connect(request("first.test")).unwrap();
    waiting(&same_host).await;
    proxy.assert_idle();
    let mut other = runtime.connect(request("other.test")).unwrap();
    let mut other_peer = proxy.proxy_handshake().await;
    assert!(other_peer.request.contains("Host: other.test"));
    other_peer.upgrade();
    opened(&mut other).await;
    first_peer.upgrade();
    opened(&mut first).await;
    let mut same_peer = proxy.proxy_handshake().await;
    same_peer.upgrade();
    opened(&mut same_host).await;
}

#[tokio::test]
async fn shutdown_retires_endpoint_holders_and_waiters() {
    let listener = Listener::new();
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut first = runtime
        .connect(listener.request("first.test", "127.0.0.1"))
        .unwrap();
    let _peer = listener.handshake().await;
    let mut waiting_connection = runtime
        .connect(listener.request("waiting.test", "127.0.0.1"))
        .unwrap();
    waiting(&waiting_connection).await;
    drop(runtime);
    for connection in [&mut first, &mut waiting_connection] {
        assert!(matches!(
            event(connection).await,
            CurlWebSocketEvent::Closed { result: Err(_) }
        ));
    }
    listener.assert_idle();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn endpoint_wait_preserves_resolved_address_fallback() {
    let listener = Listener::new();
    let alternative = Listener::bind(("127.0.0.2", listener.port()));
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut held = runtime
        .connect(listener.request("held.test", "127.0.0.1"))
        .unwrap();
    let mut held_peer = listener.handshake().await;
    let mut fallback = runtime
        .connect(listener.request("fallback.test", "127.0.0.1,127.0.0.2"))
        .unwrap();
    waiting(&fallback).await;
    let mut fallback_peer = alternative.handshake().await;
    fallback_peer.upgrade();
    opened(&mut fallback).await;
    listener.assert_idle();

    let mut next = runtime
        .connect(listener.request("next.test", "127.0.0.1"))
        .unwrap();
    waiting(&next).await;
    held_peer.upgrade();
    opened(&mut held).await;
    let mut next_peer = listener.handshake().await;
    next_peer.upgrade();
    opened(&mut next).await;
    listener.assert_idle();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn mapped_ipv6_and_ipv4_addresses_share_one_endpoint() {
    let listener = Listener::new();
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut first = runtime
        .connect(listener.request("ipv4.test", "127.0.0.1"))
        .unwrap();
    let mut first_peer = listener.handshake().await;
    let mut mapped = runtime
        .connect(listener.request("mapped.test", "[::ffff:127.0.0.1]"))
        .unwrap();
    waiting(&mapped).await;
    listener.assert_idle();
    first_peer.upgrade();
    opened(&mut first).await;
    let mut mapped_peer = listener.handshake().await;
    mapped_peer.upgrade();
    opened(&mut mapped).await;
}
