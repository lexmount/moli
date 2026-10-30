use super::*;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;

fn tls(alpns: &[&[u8]]) -> (tempfile::NamedTempFile, TlsAcceptor) {
    let key = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let mut ca = tempfile::NamedTempFile::new().unwrap();
    ca.write_all(key.cert.pem().as_bytes()).unwrap();
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![key.cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.key_pair.serialize_der())),
        )
        .unwrap();
    config.alpn_protocols = alpns.iter().map(|alpn| alpn.to_vec()).collect();
    (ca, TlsAcceptor::from(Arc::new(config)))
}

async fn masked_frame(body: &mut h2::RecvStream) -> (u8, Vec<u8>) {
    let mut wire = Vec::new();
    loop {
        let chunk = body.data().await.unwrap().unwrap();
        body.flow_control().release_capacity(chunk.len()).unwrap();
        wire.extend_from_slice(&chunk);
        if wire.len() < 2 {
            continue;
        }
        assert_ne!(wire[1] & 0x80, 0, "client frames remain masked over H2");
        let length = usize::from(wire[1] & 0x7f);
        assert!(length < 126, "small test frame");
        if wire.len() < length + 6 {
            continue;
        }
        assert_eq!(wire.len(), length + 6);
        let payload = wire[6..]
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ wire[2 + index % 4])
            .collect();
        return (wire[0], payload);
    }
}

#[tokio::test]
async fn extended_connect_preserves_headers_masked_frames_and_close() {
    let (ca, tls) = tls(&[b"h2"]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = tls.accept(socket).await.unwrap();
        assert_eq!(socket.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
        let mut builder = h2::server::Builder::new();
        builder.enable_connect_protocol();
        let mut connection = builder.handshake(socket).await.unwrap();
        let (request, mut respond) = connection.accept().await.unwrap().unwrap();
        assert_eq!(request.method(), http::Method::CONNECT);
        assert_eq!(request.version(), http::Version::HTTP_2);
        assert_eq!(request.uri().scheme_str(), Some("https"));
        assert_eq!(
            request.uri().path_and_query().unwrap().as_str(),
            "/echo?x=1"
        );
        assert_eq!(
            request.uri().authority().unwrap().as_str(),
            format!("localhost:{port}")
        );
        assert_eq!(
            request
                .extensions()
                .get::<h2::ext::Protocol>()
                .unwrap()
                .as_str(),
            "websocket"
        );
        for name in ["upgrade", "connection", "sec-websocket-key"] {
            assert!(!request.headers().contains_key(name));
        }
        assert_eq!(request.headers()["origin"], "https://origin.test");
        assert_eq!(request.headers()["cookie"], "session=abc");
        assert_eq!(request.headers()["sec-websocket-version"], "13");
        assert_eq!(request.headers()["sec-websocket-protocol"], "alpha, beta");
        let stream = tokio::spawn(async move {
            let response = http::Response::builder()
                .status(200)
                .header("sec-websocket-protocol", "beta")
                .body(())
                .unwrap();
            let mut send = respond.send_response(response, false).unwrap();
            let mut recv = request.into_body();
            assert_eq!(masked_frame(&mut recv).await, (0x81, b"hello".to_vec()));
            send.send_data(b"\x81\x05hello".as_slice(), false).unwrap();
            assert_eq!(masked_frame(&mut recv).await, (0x88, vec![3, 232]));
            send.send_data(b"\x88\x02\x03\xe8".as_slice(), true)
                .unwrap();
        });
        if let Some(other) = connection.accept().await {
            panic!("unexpected extra H2 stream: {other:?}");
        }
        stream.await.unwrap();
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut request = CurlWebSocketRequest::new(format!("wss://localhost:{port}/echo?x=1"));
    request.tls.ca_cert = Some(ca.path().to_owned());
    request.headers = moli_header_field::HeaderFields::from_utf8(vec![
        ("Origin".into(), "https://origin.test".into()),
        ("Cookie".into(), "session=abc".into()),
        ("Sec-WebSocket-Protocol".into(), "alpha, beta".into()),
    ]);
    let mut connection = runtime.connect(request).unwrap();
    let handshake = event(&mut connection).await;
    assert!(
        matches!(handshake, CurlWebSocketEvent::Handshake { result: Ok(()), ref response, .. }
        if response.starts_with(b"HTTP/2 200")),
        "{handshake:?}"
    );
    let sender = connection.sender();
    sender.set_reading(true);
    sender
        .send_frame(CurlWebSocketSend {
            flags: WsFlags::TEXT,
            data: b"hello".to_vec(),
        })
        .await
        .unwrap();
    assert!(
        matches!(event(&mut connection).await, CurlWebSocketEvent::Chunk { data, frame }
        if data == b"hello" && frame.flags() == WsFlags::TEXT && frame.bytes_left() == 0)
    );
    sender
        .send_frame(CurlWebSocketSend {
            flags: WsFlags::CLOSE,
            data: vec![3, 232],
        })
        .await
        .unwrap();
    assert!(
        matches!(event(&mut connection).await, CurlWebSocketEvent::Chunk { data, frame }
        if data == [3, 232] && frame.flags() == WsFlags::CLOSE)
    );
    drop(connection);
    timeout(DEADLINE, peer).await.unwrap().unwrap();
}

#[tokio::test]
async fn missing_connect_setting_negotiates_http1_without_sending_connect() {
    let (ca, tls) = tls(&[b"h2", b"http/1.1"]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = tls.accept(socket).await.unwrap();
        assert_eq!(socket.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
        let first = tokio::spawn(async move {
            let mut connection = h2::server::handshake(socket).await.unwrap();
            if let Some(request) = connection.accept().await {
                assert!(
                    request.is_err(),
                    "CONNECT requires the peer's opt-in setting"
                );
            }
        });
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tls.accept(socket).await.unwrap();
        assert_eq!(
            socket.get_ref().1.alpn_protocol(),
            Some(b"http/1.1".as_slice())
        );
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("GET /fallback HTTP/1.1\r\n"));
        let key = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("Sec-WebSocket-Key")
                    .then(|| value.trim())
            })
            .unwrap();
        let accept = derive_accept_key(key.as_bytes());
        socket.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).await.unwrap();
        match socket.read_u8().await {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                ) => {}
            other => panic!("cancelled socket must close: {other:?}"),
        }
        first.await.unwrap();
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut request = CurlWebSocketRequest::new(format!("wss://localhost:{port}/fallback"));
    request.tls.ca_cert = Some(ca.path().to_owned());
    let mut connection = runtime.connect(request).unwrap();
    let handshake = event(&mut connection).await;
    assert!(
        matches!(handshake, CurlWebSocketEvent::Handshake { result: Ok(()), ref response, .. }
        if response.starts_with(b"HTTP/1.1 101")),
        "{handshake:?}"
    );
    drop(connection);
    timeout(DEADLINE, peer).await.unwrap().unwrap();
}

#[tokio::test]
async fn window_updates_resume_a_blocked_send_with_application_reads_paused() {
    let (ca, tls) = tls(&[b"h2"]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release, resume) = oneshot::channel();
    let (complete, received) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = tls.accept(socket).await.unwrap();
        let mut builder = h2::server::Builder::new();
        builder.enable_connect_protocol().initial_window_size(1024);
        let mut connection = builder.handshake::<_, &[u8]>(socket).await.unwrap();
        let (request, mut respond) = connection.accept().await.unwrap().unwrap();
        let stream = tokio::spawn(async move {
            let _send = respond
                .send_response(http::Response::new(()), false)
                .unwrap();
            let mut recv = request.into_body();
            let first = recv.data().await.unwrap().unwrap();
            assert_eq!(first.len(), 1024);
            resume.await.unwrap();
            recv.flow_control().release_capacity(first.len()).unwrap();
            let mut wire = first.to_vec();
            while wire.len() < MAX_SEND_FRAME_BYTES + 14 {
                let chunk = recv.data().await.unwrap().unwrap();
                recv.flow_control().release_capacity(chunk.len()).unwrap();
                wire.extend_from_slice(&chunk);
            }
            assert_eq!(wire.len(), MAX_SEND_FRAME_BYTES + 14);
            assert_eq!(&wire[..2], &[0x82, 0xff]);
            assert_eq!(&wire[2..10], &(MAX_SEND_FRAME_BYTES as u64).to_be_bytes());
            assert!(
                wire[14..]
                    .iter()
                    .enumerate()
                    .all(|(i, byte)| byte ^ wire[10 + i % 4] == 42)
            );
            complete.send(()).unwrap();
            if let Some(chunk) = recv.data().await {
                assert!(chunk.is_err(), "no second frame is sent");
            }
        });
        if let Some(result) = connection.accept().await {
            assert!(result.is_err(), "only one WebSocket stream is admitted");
        }
        stream.await.unwrap();
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut request = CurlWebSocketRequest::new(format!("wss://localhost:{port}/backpressure"));
    request.tls.ca_cert = Some(ca.path().to_owned());
    let mut connection = runtime.connect(request).unwrap();
    assert!(matches!(
        event(&mut connection).await,
        CurlWebSocketEvent::Handshake { result: Ok(()), .. }
    ));
    let sender = connection.sender();
    let sending = sender.send_frame(CurlWebSocketSend {
        flags: WsFlags::BINARY,
        data: vec![42; MAX_SEND_FRAME_BYTES],
    });
    tokio::pin!(sending);
    timeout(DEADLINE, async {
        tokio::select! {
            result = &mut sending => panic!("write must await WINDOW_UPDATE: {result:?}"),
            _ = sender.control.write_blocked.notified() => {},
        }
    })
    .await
    .unwrap();
    release.send(()).unwrap();
    assert_eq!(
        timeout(DEADLINE, sending).await.unwrap().unwrap(),
        MAX_SEND_FRAME_BYTES
    );
    timeout(DEADLINE, received).await.unwrap().unwrap();
    drop(connection);
    timeout(DEADLINE, peer).await.unwrap().unwrap();
}

#[tokio::test]
async fn peer_reset_rejects_a_write_waiting_for_window_update() {
    let (ca, tls) = tls(&[b"h2"]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (reset, trigger) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = tls.accept(socket).await.unwrap();
        let mut builder = h2::server::Builder::new();
        builder.enable_connect_protocol().initial_window_size(1024);
        let mut connection = builder.handshake::<_, &[u8]>(socket).await.unwrap();
        let (request, mut respond) = connection.accept().await.unwrap().unwrap();
        let stream = tokio::spawn(async move {
            let mut send = respond
                .send_response(http::Response::new(()), false)
                .unwrap();
            let mut recv = request.into_body();
            let first = recv.data().await.unwrap().unwrap();
            assert_eq!(first.len(), 1024);
            trigger.await.unwrap();
            send.send_reset(h2::Reason::CANCEL);
        });
        if let Some(result) = connection.accept().await {
            assert!(
                result.is_err(),
                "no replacement request after opening a WebSocket"
            );
        }
        stream.await.unwrap();
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut request = CurlWebSocketRequest::new(format!("wss://localhost:{port}/reset"));
    request.tls.ca_cert = Some(ca.path().to_owned());
    let mut connection = runtime.connect(request).unwrap();
    assert!(matches!(
        event(&mut connection).await,
        CurlWebSocketEvent::Handshake { result: Ok(()), .. }
    ));
    let sender = connection.sender();
    let sending = sender.send_frame(CurlWebSocketSend {
        flags: WsFlags::BINARY,
        data: vec![42; MAX_SEND_FRAME_BYTES],
    });
    tokio::pin!(sending);
    timeout(DEADLINE, async {
        tokio::select! {
            result = &mut sending => panic!("write must wait for the peer's window: {result:?}"),
            _ = sender.control.write_blocked.notified() => {},
        }
    })
    .await
    .unwrap();
    reset.send(()).unwrap();
    assert!(timeout(DEADLINE, sending).await.unwrap().is_err());
    drop(connection);
    timeout(DEADLINE, peer).await.unwrap().unwrap();
}

#[tokio::test]
async fn remote_end_stream_follows_the_last_buffered_control_frame() {
    let (ca, tls) = tls(&[b"h2"]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = tls.accept(socket).await.unwrap();
        let mut builder = h2::server::Builder::new();
        builder.enable_connect_protocol();
        let mut connection = builder.handshake(socket).await.unwrap();
        let (_request, mut respond) = connection.accept().await.unwrap().unwrap();
        let mut send = respond
            .send_response(http::Response::new(()), false)
            .unwrap();
        send.send_data(b"\x89\x04ping".as_slice(), true).unwrap();
        if let Some(result) = connection.accept().await {
            assert!(result.is_err(), "no second stream after remote EOF");
        }
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut request = CurlWebSocketRequest::new(format!("wss://localhost:{port}/eof"));
    request.tls.ca_cert = Some(ca.path().to_owned());
    let mut connection = runtime.connect(request).unwrap();
    assert!(matches!(
        event(&mut connection).await,
        CurlWebSocketEvent::Handshake { result: Ok(()), .. }
    ));
    connection.sender().set_reading(true);
    let received = event(&mut connection).await;
    assert!(
        matches!(&received, CurlWebSocketEvent::Chunk { data, frame }
        if data == b"ping" && frame.flags() == WsFlags::PING),
        "{received:?}"
    );
    assert!(matches!(
        event(&mut connection).await,
        CurlWebSocketEvent::Closed { .. }
    ));
    drop(connection);
    timeout(DEADLINE, peer).await.unwrap().unwrap();
}

#[tokio::test]
async fn received_messages_cross_multiple_stream_windows_without_losing_bytes() {
    const PAYLOAD: usize = 2 * 1024 * 1024;
    let (ca, tls) = tls(&[b"h2"]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = tls.accept(socket).await.unwrap();
        let mut builder = h2::server::Builder::new();
        builder.enable_connect_protocol();
        let mut connection = builder.handshake(socket).await.unwrap();
        let (_request, mut respond) = connection.accept().await.unwrap().unwrap();
        let mut send = respond
            .send_response(http::Response::new(()), false)
            .unwrap();
        let mut wire = vec![0x82, 127];
        wire.extend_from_slice(&(PAYLOAD as u64).to_be_bytes());
        wire.resize(PAYLOAD + 10, 42);
        send.send_data(std::io::Cursor::new(wire), true).unwrap();
        if let Some(result) = connection.accept().await {
            assert!(result.is_err(), "only one receive stream is expected");
        }
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut request = CurlWebSocketRequest::new(format!("wss://localhost:{port}/receive"));
    request.tls.ca_cert = Some(ca.path().to_owned());
    let mut connection = runtime.connect(request).unwrap();
    assert!(matches!(
        event(&mut connection).await,
        CurlWebSocketEvent::Handshake { result: Ok(()), .. }
    ));
    connection.sender().set_reading(true);
    let mut received = 0;
    while received < PAYLOAD {
        let next = event(&mut connection).await;
        match next {
            CurlWebSocketEvent::Chunk { data, frame } => {
                assert_eq!(frame.flags(), WsFlags::BINARY);
                assert!(data.iter().all(|byte| *byte == 42));
                received += data.len();
            }
            unexpected => panic!("received {received}/{PAYLOAD} before {unexpected:?}"),
        }
    }
    assert_eq!(received, PAYLOAD);
    drop(connection);
    timeout(DEADLINE, peer).await.unwrap().unwrap();
}

#[tokio::test]
async fn rejected_connect_finishes_before_its_handshake_deadline() {
    let (ca, tls) = tls(&[b"h2"]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = tls.accept(socket).await.unwrap();
        let mut builder = h2::server::Builder::new();
        builder.enable_connect_protocol();
        let mut connection = builder.handshake::<_, &[u8]>(socket).await.unwrap();
        let (_request, mut respond) = connection.accept().await.unwrap().unwrap();
        respond
            .send_response(
                http::Response::builder().status(404).body(()).unwrap(),
                true,
            )
            .unwrap();
        if let Some(result) = connection.accept().await {
            assert!(result.is_err(), "a rejected CONNECT is not retried");
        }
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut request = CurlWebSocketRequest::new(format!("wss://localhost:{port}/missing"));
    request.tls.ca_cert = Some(ca.path().to_owned());
    // The event's deadline expires first if HTTP errors remain pending until
    // the native opening timeout instead of being delivered immediately.
    request.handshake_timeout = DEADLINE * 2;
    let mut connection = runtime.connect(request).unwrap();
    let handshake = event(&mut connection).await;
    assert!(
        matches!(handshake, CurlWebSocketEvent::Handshake { result: Err(_), ref response, .. }
        if response.starts_with(b"HTTP/2 404")),
        "{handshake:?}"
    );
    drop(connection);
    timeout(DEADLINE, peer).await.unwrap().unwrap();
}
