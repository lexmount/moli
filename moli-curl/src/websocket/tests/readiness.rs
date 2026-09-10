use super::*;

#[tokio::test]
async fn native_active_sessions_do_not_probe_idle_sockets() {
    let (idle_url, idle_task) = server(|stream| {
        let mut socket = tungstenite::accept(stream).unwrap();
        assert_eq!(socket.read().unwrap(), Message::Text("wake".into()));
        socket.send(Message::Text("awake".into())).unwrap();
        assert!(socket.read().is_err());
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut idle = runtime
        .connect(CurlWebSocketRequest::new(idle_url))
        .unwrap();
    opened(&mut idle).await;
    let idle_sender = idle.sender();
    idle_sender.set_reading(true);
    timeout(DEADLINE, idle_sender.control.read_waiting.notified())
        .await
        .unwrap();
    let before = idle_sender.control.read_attempts.load(Ordering::Acquire);

    let (active_url, active_task) = server(|mut stream| {
        let tail: Vec<_> = (0..128).flat_map(|i| [0x82, 1, i]).collect();
        upgrade(&mut stream, &tail);
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    });
    let mut active = runtime
        .connect(CurlWebSocketRequest::new(active_url))
        .unwrap();
    opened(&mut active).await;
    active.sender().set_reading(true);
    for expected in 0..128 {
        assert!(matches!(event(&mut active).await,
            CurlWebSocketEvent::Chunk { data, .. } if data == [expected]));
    }
    assert_eq!(
        idle_sender.control.read_attempts.load(Ordering::Acquire),
        before,
        "another connection's progress and delivery wakes must not reprobe an idle socket"
    );

    // A new send wakes this idle session independently of its waiting reader.
    assert_eq!(
        timeout(
            DEADLINE,
            idle_sender.send_frame(CurlWebSocketSend {
                flags: WsFlags::TEXT,
                data: b"wake".to_vec(),
            })
        )
        .await
        .unwrap()
        .unwrap(),
        4
    );
    assert!(matches!(event(&mut idle).await,
        CurlWebSocketEvent::Chunk { data, .. } if data == b"awake"));
    drop(idle);
    drop(active);
    idle_task.join().unwrap();
    active_task.join().unwrap();
}

#[tokio::test]
async fn native_socket_readiness_is_serviced_during_continuous_traffic() {
    let (url, task) = server(|stream| {
        let mut socket = tungstenite::accept(stream).unwrap();
        while socket
            .send(Message::Binary(vec![3; MAX_SEND_FRAME_BYTES].into()))
            .is_ok()
        {}
    });
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    let (quiet_url, quiet_task) = server(move |mut stream| {
        upgrade(&mut stream, &[]);
        signal_rx.recv_timeout(DEADLINE).unwrap();
        stream.write_all(&[0x81, 2, b'o', b'k']).unwrap();
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut quiet = runtime
        .connect(CurlWebSocketRequest::new(quiet_url))
        .unwrap();
    opened(&mut quiet).await;
    quiet.sender().set_reading(true);
    timeout(DEADLINE, quiet.sender.control.read_waiting.notified())
        .await
        .unwrap();
    let mut active = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
    opened(&mut active).await;
    active.sender().set_reading(true);
    let (started_tx, started_rx) = oneshot::channel();
    let (stop_tx, stop_rx) = oneshot::channel();
    let drain = tokio::spawn(async move {
        let mut started = Some(started_tx);
        tokio::pin!(stop_rx);
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                next = active.recv() => {
                    assert!(matches!(next, Some(CurlWebSocketEvent::Chunk { data, .. })
                        if !data.is_empty() && data.iter().all(|byte| *byte == 3)));
                    if let Some(started) = started.take() { started.send(()).unwrap(); }
                }
            }
        }
    });
    timeout(DEADLINE, started_rx).await.unwrap().unwrap();
    signal_tx.send(()).unwrap();
    assert!(matches!(event(&mut quiet).await,
        CurlWebSocketEvent::Chunk { data, .. } if data == b"ok"));
    stop_tx.send(()).unwrap();
    timeout(DEADLINE, drain).await.unwrap().unwrap();
    drop(quiet);
    task.join().unwrap();
    quiet_task.join().unwrap();
}

#[tokio::test]
async fn native_wss_buffered_frames_resume_after_backpressure() {
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
    let (url, task) = server(move |stream| {
        let tls = StreamOwned::new(ServerConnection::new(config).unwrap(), stream);
        let mut socket = tungstenite::accept(tls).unwrap();
        // One TLS write coalesces enough frames to exceed the delivery queue.
        let wire: Vec<_> = (0..32).flat_map(|i| [0x82, 1, i]).collect();
        socket.get_mut().write_all(&wire).unwrap();
        socket.get_mut().flush().unwrap();
        assert!(socket.read().is_err());
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut request = CurlWebSocketRequest::new(url.replacen("ws://", "wss://", 1));
    // This fixture tests TLS buffering; certificate policy has separate coverage.
    request.tls.verify = false;
    let mut connection = runtime.connect(request).unwrap();
    opened(&mut connection).await;
    let sender = connection.sender();
    sender.set_reading(true);
    timeout(DEADLINE, sender.control.read_blocked.notified())
        .await
        .unwrap();
    sender.set_reading(false);
    for expected in 0..MAX_PENDING_EVENTS {
        assert!(matches!(event(&mut connection).await,
            CurlWebSocketEvent::Chunk { data, .. } if data == [expected as u8]));
    }
    // No further server writes: resume must also drain libcurl/TLS buffers.
    sender.set_reading(true);
    for expected in MAX_PENDING_EVENTS..32 {
        assert!(matches!(event(&mut connection).await,
            CurlWebSocketEvent::Chunk { data, .. } if data == [expected as u8]));
    }
    drop(connection);
    task.join().unwrap();
}

#[tokio::test]
async fn native_peer_eof_wakes_waiting_reader() {
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let (url, task) = server(move |mut stream| {
        upgrade(&mut stream, &[]);
        finish_rx.recv_timeout(DEADLINE).unwrap();
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut connection = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
    opened(&mut connection).await;
    connection.sender().set_reading(true);
    timeout(DEADLINE, connection.sender.control.read_waiting.notified())
        .await
        .unwrap();
    finish_tx.send(()).unwrap();
    assert!(matches!(
        event(&mut connection).await,
        CurlWebSocketEvent::Closed { result: Ok(()) }
    ));
    assert!(connection.recv().await.is_none());
    task.join().unwrap();
}

#[tokio::test]
async fn native_peer_shutdown_settles_blocked_writer_with_reads_paused() {
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let (url, task) = server(move |stream| {
        let _socket = tungstenite::accept(stream).unwrap();
        finish_rx.recv_timeout(DEADLINE).unwrap();
    });
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut connection = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
    opened(&mut connection).await;
    let sender = connection.sender();
    let send_until_blocked = async {
        for _ in 0..128 {
            let send = sender.send_frame(CurlWebSocketSend {
                flags: WsFlags::BINARY,
                data: vec![9; MAX_SEND_FRAME_BYTES],
            });
            tokio::pin!(send);
            tokio::select! {
                biased;
                _ = sender.control.write_blocked.notified() => {
                    finish_tx.send(()).unwrap();
                    assert!(send.await.is_err());
                    return;
                }
                result = &mut send => { assert_eq!(result.unwrap(), MAX_SEND_FRAME_BYTES); }
            }
        }
        panic!("server backpressure must reach the writer");
    };
    timeout(DEADLINE, send_until_blocked).await.unwrap();
    assert!(matches!(
        event(&mut connection).await,
        CurlWebSocketEvent::Closed { result: Err(_) }
    ));
    task.join().unwrap();
}
