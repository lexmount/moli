use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};

use tokio::time::timeout;
use tokio_tungstenite::tungstenite::{self, Message, handshake::derive_accept_key};

use super::*;

const DEADLINE: Duration = Duration::from_secs(10);

fn server(handler: impl FnOnce(TcpStream) + Send + 'static) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/native", listener.local_addr().unwrap());
    let task = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(DEADLINE)).unwrap();
        stream.set_write_timeout(Some(DEADLINE)).unwrap();
        handler(stream);
    });
    (url, task)
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        request.push(byte[0]);
        assert!(request.len() < 65536);
    }
    String::from_utf8(request).unwrap()
}

fn upgrade(stream: &mut TcpStream, tail: &[u8]) {
    let request = read_request(stream);
    let key = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("sec-websocket-key")
                .then_some(value.trim())
        })
        .unwrap();
    let mut response = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", derive_accept_key(key.as_bytes())).into_bytes();
    response.extend_from_slice(tail);
    stream.write_all(&response).unwrap();
}

async fn event(connection: &mut CurlWebSocketConnection) -> CurlWebSocketEvent {
    timeout(DEADLINE, connection.recv())
        .await
        .expect("native event deadline")
        .expect("native event")
}

async fn opened(connection: &mut CurlWebSocketConnection) {
    match event(connection).await {
        CurlWebSocketEvent::Handshake {
            request,
            response,
            result,
        } => {
            result.unwrap();
            assert!(request.starts_with(b"GET /native HTTP/1.1\r\n"));
            assert!(response.starts_with(b"HTTP/1.1 101"));
        }
        unexpected => panic!("expected handshake, got {unexpected:?}"),
    }
}

#[tokio::test]
async fn native_upgrade_retains_socket_and_same_packet_empty_frame() {
    let (url, task) = server(|mut stream| {
        // Empty text followed by a binary frame, coalesced with the handshake.
        upgrade(&mut stream, &[0x81, 0, 0x82, 3, 1, 2, 3]);
        assert_eq!(
            stream.read(&mut [0]).unwrap(),
            0,
            "receiver drop releases TCP"
        );
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut connection = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
    opened(&mut connection).await;
    assert!(
        connection.events.is_empty(),
        "application reads start paused"
    );
    connection.sender().set_reading(true);
    match event(&mut connection).await {
        CurlWebSocketEvent::Chunk { data, frame } => {
            assert!(data.is_empty());
            assert_eq!(frame.flags(), WsFlags::TEXT);
            assert_eq!(frame.bytes_left(), 0);
        }
        unexpected => panic!("expected empty text, got {unexpected:?}"),
    }
    match event(&mut connection).await {
        CurlWebSocketEvent::Chunk { data, frame } => {
            assert_eq!(data, [1, 2, 3]);
            assert_eq!(frame.flags(), WsFlags::BINARY);
        }
        unexpected => panic!("expected binary, got {unexpected:?}"),
    }
    drop(connection);
    task.join().unwrap();
}

#[tokio::test]
async fn native_fragmented_send_completes_without_echo() {
    let payload: Vec<_> = (0..MAX_SEND_FRAME_BYTES * 2)
        .map(|i| (i % 251) as u8)
        .collect();
    let expected = payload.clone();
    let (url, task) = server(move |stream| {
        let mut socket = tungstenite::accept(stream).unwrap();
        assert_eq!(socket.read().unwrap(), Message::Binary(expected.into()));
        assert_eq!(socket.read().unwrap(), Message::Text("".into()));
        assert!(socket.read().unwrap().is_close());
        assert!(matches!(
            socket.flush(),
            Ok(()) | Err(tungstenite::Error::ConnectionClosed)
        ));
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut connection = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
    opened(&mut connection).await;
    let sender = connection.sender();
    for (i, data) in payload.chunks(MAX_SEND_FRAME_BYTES).enumerate() {
        let send = sender.send_frame(CurlWebSocketSend {
            data: data.to_vec(),
            flags: if i == 0 {
                WsFlags::BINARY | WsFlags::CONT
            } else {
                WsFlags::BINARY
            },
        });
        assert_eq!(
            timeout(DEADLINE, send).await.unwrap().unwrap(),
            MAX_SEND_FRAME_BYTES
        );
    }
    let empty = sender.send_frame(CurlWebSocketSend {
        data: Vec::new(),
        flags: WsFlags::TEXT,
    });
    assert_eq!(timeout(DEADLINE, empty).await.unwrap().unwrap(), 0);
    let close = sender.send_frame(CurlWebSocketSend {
        data: 1000u16.to_be_bytes().to_vec(),
        flags: WsFlags::CLOSE,
    });
    assert_eq!(timeout(DEADLINE, close).await.unwrap().unwrap(), 2);
    sender.set_reading(true);
    assert!(
        matches!(event(&mut connection).await, CurlWebSocketEvent::Chunk { frame, .. } if frame.flags() == WsFlags::CLOSE)
    );
    task.join().unwrap();
}

#[tokio::test]
async fn native_full_delivery_queue_does_not_block_other_sessions_or_cancellation() {
    let (url, task) = server(|mut stream| {
        let tail: Vec<_> = (0..32).flat_map(|_| [0x82, 1, 7]).collect();
        upgrade(&mut stream, &tail);
        let result = stream.read(&mut [0]);
        assert!(
            matches!(result, Ok(0))
                || result.is_err_and(|e| e.kind() == std::io::ErrorKind::ConnectionReset)
        );
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut blocked = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
    opened(&mut blocked).await;
    blocked.sender().set_reading(true);
    timeout(DEADLINE, blocked.sender.control.read_blocked.notified())
        .await
        .unwrap();
    assert_eq!(blocked.events.len(), MAX_PENDING_EVENTS);

    let (other_url, other_task) = server(|mut stream| {
        upgrade(&mut stream, &[0x81, 2, b'o', b'k']);
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    });
    let mut other = runtime
        .connect(CurlWebSocketRequest::new(other_url))
        .unwrap();
    assert_ne!(blocked.id(), other.id());
    opened(&mut other).await;
    other.sender().set_reading(true);
    assert!(
        matches!(event(&mut other).await, CurlWebSocketEvent::Chunk { data, .. } if data == b"ok")
    );
    drop(blocked);
    task.join().unwrap();
    drop(other);
    other_task.join().unwrap();
}

#[tokio::test]
async fn native_queue_capacity_restoration_wakes_owner() {
    let (url, task) = server(|mut stream| {
        let tail: Vec<_> = (0..32).flat_map(|i| [0x82, 1, i]).collect();
        upgrade(&mut stream, &tail);
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut connection = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
    opened(&mut connection).await;
    connection.sender().set_reading(true);
    timeout(DEADLINE, connection.sender.control.read_blocked.notified())
        .await
        .unwrap();
    for expected in 0..32 {
        assert!(
            matches!(event(&mut connection).await, CurlWebSocketEvent::Chunk { data, .. } if data == [expected])
        );
    }
    drop(connection);
    task.join().unwrap();
}

#[tokio::test]
async fn native_pending_handshake_cancel_and_deadline_release_socket() {
    for cancel in [true, false] {
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let (url, task) = server(move |mut stream| {
            read_request(&mut stream);
            accepted_tx.send(()).unwrap();
            assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        });
        let runtime = CurlWebSocketRuntime::new().unwrap();
        let mut request = CurlWebSocketRequest::new(url);
        if !cancel {
            request.handshake_timeout = Duration::from_millis(100);
        }
        let mut connection = runtime.connect(request).unwrap();
        timeout(DEADLINE, accepted_rx).await.unwrap().unwrap();
        if cancel {
            connection.sender().cancel();
        }
        loop {
            if let CurlWebSocketEvent::Closed { result } = event(&mut connection).await {
                if !cancel {
                    assert!(result.is_err());
                }
                break;
            }
        }
        assert!(connection.recv().await.is_none());
        task.join().unwrap();
    }
}

#[tokio::test]
async fn native_partial_write_retries_preserve_payload_and_control_boundaries() {
    const FRAMES: usize = 128;
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let (url, task) = server(move |stream| {
        let mut socket = tungstenite::accept(stream).unwrap();
        resume_rx.recv_timeout(DEADLINE).unwrap();
        let mut i = 0;
        let mut ping_seen = false;
        while i < FRAMES {
            let message = socket.read().unwrap();
            if let Message::Ping(payload) = &message {
                assert_eq!(&payload[..], b"between frames");
                assert!(!ping_seen);
                ping_seen = true;
                continue;
            }
            let expected = vec![(i % 251) as u8; MAX_SEND_FRAME_BYTES];
            assert_eq!(message, Message::Binary(expected.into()));
            i += 1;
        }
        assert!(ping_seen);
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut connection = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
    opened(&mut connection).await;
    let sender = connection.sender();
    let sends = async {
        let mut ping_sent = false;
        for i in 0..FRAMES {
            let send = sender.send_frame(CurlWebSocketSend {
                flags: WsFlags::BINARY,
                data: vec![(i % 251) as u8; MAX_SEND_FRAME_BYTES],
            });
            tokio::pin!(send);
            let count = tokio::select! {
                biased;
                _ = sender.control.write_blocked.notified(), if !ping_sent => {
                    // The caller schedules control only after the partial frame
                    // finishes. The native layer must preserve its write cursor.
                    resume_tx.send(()).unwrap();
                    let count = send.await.unwrap();
                    assert_eq!(sender.send_frame(CurlWebSocketSend {
                        flags: WsFlags::PING,
                        data: b"between frames".to_vec(),
                    }).await.unwrap(), b"between frames".len());
                    ping_sent = true;
                    count
                },
                count = &mut send => count.unwrap(),
            };
            assert_eq!(count, MAX_SEND_FRAME_BYTES);
        }
        assert!(ping_sent, "server backpressure must reach native writer");
    };
    timeout(DEADLINE, sends).await.unwrap();
    task.join().unwrap();
}

#[test]
fn native_wss_configuration_validates_chain_and_hostname() {
    use rustls::{
        ServerConfig, ServerConnection, StreamOwned,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let config = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificate.cert.der().to_vec())],
                PrivateKeyDer::from(PrivatePkcs8KeyDer::from(
                    certificate.key_pair.serialize_der(),
                )),
            )
            .unwrap(),
    );
    for (host, trusted, succeeds) in [
        ("localhost", true, true),
        ("127.0.0.1", true, false),
        ("localhost", false, false),
    ] {
        let config = config.clone();
        let (url, task) = server(move |stream| {
            let tls = StreamOwned::new(ServerConnection::new(config).unwrap(), stream);
            assert_eq!(tungstenite::accept(tls).is_ok(), succeeds);
        });
        let url = url.replacen("ws://127.0.0.1", &format!("wss://{host}"), 1);
        let request = CurlWebSocketRequest::new(url);
        let mut easy = super::request::configure(&request).unwrap();
        if trusted {
            easy.ssl_cainfo_blob(certificate.cert.pem().as_bytes())
                .unwrap();
        }
        let result = easy.perform();
        if succeeds {
            result.unwrap();
        } else {
            assert!(
                result.unwrap_err().is_peer_failed_verification(),
                "certificate policy must reject the connection"
            );
        }
        drop(easy);
        task.join().unwrap();
    }
}

#[tokio::test]
async fn native_sends_progress_with_full_receive_queue() {
    let (url, task) = server(|mut stream| {
        let tail: Vec<_> = (0..MAX_PENDING_EVENTS).flat_map(|_| [0x82, 1, 7]).collect();
        upgrade(&mut stream, &tail);
        let mut socket = tungstenite::WebSocket::from_raw_socket(
            stream,
            tungstenite::protocol::Role::Server,
            None,
        );
        assert_eq!(
            socket.read().unwrap(),
            Message::Binary(vec![9; 2 * MAX_SEND_FRAME_BYTES].into())
        );
        assert!(matches!(
            socket.read(),
            Err(tungstenite::Error::Protocol(
                tungstenite::error::ProtocolError::ResetWithoutClosingHandshake
            ))
        ));
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut connection = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
    opened(&mut connection).await;
    let sender = connection.sender();
    sender.set_reading(true);
    timeout(DEADLINE, sender.control.read_blocked.notified())
        .await
        .unwrap();
    assert_eq!(connection.events.len(), MAX_PENDING_EVENTS);
    for flags in [WsFlags::BINARY | WsFlags::CONT, WsFlags::BINARY] {
        let send = sender.send_frame(CurlWebSocketSend {
            flags,
            data: vec![9; MAX_SEND_FRAME_BYTES],
        });
        assert_eq!(
            timeout(DEADLINE, send).await.unwrap().unwrap(),
            MAX_SEND_FRAME_BYTES
        );
    }
    assert_eq!(connection.events.len(), MAX_PENDING_EVENTS);
    drop(connection);
    task.join().unwrap();
}

#[tokio::test]
async fn native_send_rejects_overlap_until_current_frame_completes() {
    for abandon in [false, true] {
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let (url, task) = server(move |stream| {
            resume_rx.recv_timeout(DEADLINE).unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            assert_eq!(socket.read().unwrap(), Message::Text("x".into()));
            socket.send(Message::Text("ack".into())).unwrap();
            assert_eq!(
                socket.read().unwrap(),
                Message::Ping(b"control".to_vec().into())
            );
            assert_eq!(socket.read().unwrap(), Message::Text("next".into()));
            // Unpolled or rejected sends must not appear on the wire.
            assert!(socket.read().is_err());
        });
        let runtime = CurlWebSocketRuntime::new().unwrap();
        let mut connection = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
        let sender = connection.sender();
        drop(sender.send_frame(CurlWebSocketSend {
            flags: WsFlags::TEXT,
            data: b"unpolled".to_vec(),
        }));
        let mut first = Box::pin(sender.send_frame(CurlWebSocketSend {
            flags: WsFlags::TEXT,
            data: b"x".to_vec(),
        }));
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                std::future::Future::poll(first.as_mut(), cx).is_pending()
            ))
            .await
        );
        let first = if abandon {
            drop(first);
            None
        } else {
            Some(first)
        };
        let other = sender.clone();
        for flags in [WsFlags::TEXT, WsFlags::PING] {
            let error = other
                .send_frame(CurlWebSocketSend {
                    flags,
                    data: b"overlapping".to_vec(),
                })
                .await
                .unwrap_err();
            assert_eq!(error.to_string(), "WebSocket frame is already pending");
        }
        resume_tx.send(()).unwrap();
        opened(&mut connection).await;
        if let Some(first) = first {
            assert_eq!(timeout(DEADLINE, first).await.unwrap().unwrap(), 1);
        }
        sender.set_reading(true);
        // Native completion precedes reading the server's acknowledgment, even
        // when the send future was dropped. The slot can now be reused.
        assert!(matches!(event(&mut connection).await,
            CurlWebSocketEvent::Chunk { data, .. } if data == b"ack"));
        let control = sender.send_frame(CurlWebSocketSend {
            flags: WsFlags::PING,
            data: b"control".to_vec(),
        });
        assert_eq!(timeout(DEADLINE, control).await.unwrap().unwrap(), 7);
        let next = sender.send_frame(CurlWebSocketSend {
            flags: WsFlags::TEXT,
            data: b"next".to_vec(),
        });
        assert_eq!(timeout(DEADLINE, next).await.unwrap().unwrap(), 4);
        drop(connection);
        task.join().unwrap();
    }
}

#[tokio::test]
async fn native_cancel_or_failure_settles_send_during_handshake() {
    for cancel in [true, false] {
        let (accepted_tx, accepted_rx) = oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let (url, task) = server(move |mut stream| {
            read_request(&mut stream);
            accepted_tx.send(()).unwrap();
            if cancel {
                assert_eq!(stream.read(&mut [0]).unwrap(), 0);
            } else {
                finish_rx.recv_timeout(DEADLINE).unwrap();
            }
        });
        let runtime = CurlWebSocketRuntime::new().unwrap();
        let mut connection = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
        timeout(DEADLINE, accepted_rx).await.unwrap().unwrap();
        let sender = connection.sender();
        let mut send = Box::pin(sender.send_frame(CurlWebSocketSend {
            flags: WsFlags::TEXT,
            data: b"pending".to_vec(),
        }));
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                std::future::Future::poll(send.as_mut(), cx).is_pending()
            ))
            .await
        );
        if cancel {
            sender.cancel();
        } else {
            finish_tx.send(()).unwrap();
        }
        assert!(timeout(DEADLINE, send).await.unwrap().is_err());
        loop {
            if let CurlWebSocketEvent::Closed { result } = event(&mut connection).await {
                if !cancel {
                    assert!(result.is_err());
                }
                break;
            }
        }
        assert!(
            sender
                .send_frame(CurlWebSocketSend {
                    flags: WsFlags::TEXT,
                    data: Vec::new(),
                })
                .await
                .is_err()
        );
        drop(connection);
        task.join().unwrap();
    }
}
