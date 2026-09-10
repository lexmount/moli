use super::*;
use moli_curl::websocket::{
    CurlWebSocketConnection, CurlWebSocketEvent, CurlWebSocketRequest, CurlWebSocketSend, WsFlags,
};
use std::io::Write;
use tokio_rustls::rustls;
use tokio_tungstenite::tungstenite::{self, Message};

const DEADLINE: Duration = Duration::from_secs(10);

fn tls_config() -> Arc<rustls::ServerConfig> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert.cert.der().to_vec())],
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der())),
        )
        .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Arc::new(config)
}

fn ws_peer<S: Read + Write>(stream: S, sent: oneshot::Sender<()>, shutdown: bool) {
    let mut socket =
        tungstenite::accept(stream).unwrap_or_else(|error| panic!("WS handshake: {error}"));
    socket
        .send(Message::Binary(vec![7; 64 * 1024].into()))
        .unwrap();
    sent.send(()).unwrap();
    if shutdown {
        match socket.read() {
            Err(tungstenite::Error::Protocol(
                tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
            )) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                ) => {}
            result => panic!("expected owner shutdown to release the peer: {result:?}"),
        }
        return;
    }
    assert_eq!(socket.read().unwrap(), Message::Text("client".into()));
    socket
        .close(Some(tungstenite::protocol::CloseFrame {
            code: tungstenite::protocol::frame::coding::CloseCode::Normal,
            reason: "".into(),
        }))
        .unwrap();
    assert!(matches!(socket.read().unwrap(), Message::Close(_)));
}

async fn event(connection: &mut CurlWebSocketConnection) -> CurlWebSocketEvent {
    tokio::time::timeout(DEADLINE, connection.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn mixed_http2_and_websocket(tls: bool, shutdown: bool) -> Result<()> {
    let tls_config = tls_config();
    let h2_listener = TcpListener::bind("127.0.0.1:0").await?;
    let h2_url = format!("https://{}", h2_listener.local_addr()?);
    let (held_tx, held_rx) = oneshot::channel();
    let (release_tx, mut release_rx) = oneshot::channel();
    let acceptor = tokio_rustls::TlsAcceptor::from(tls_config.clone());
    let h2_task = tokio::spawn(async move {
        // Exactly one TCP connection must carry all three HTTP/2 streams.
        let (stream, _) = h2_listener.accept().await.unwrap();
        let stream = acceptor.accept(stream).await.unwrap();
        assert_eq!(stream.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
        let mut connection = h2::server::handshake(stream).await.unwrap();
        let mut held = None;
        let mut held_tx = Some(held_tx);
        let mut paths = Vec::new();
        loop {
            tokio::select! {
                _ = &mut release_rx, if held.is_some() => {
                    let mut send: h2::SendStream<_> = held.take().unwrap();
                    send.send_data("held".into(), true).unwrap();
                }
                incoming = connection.accept() => {
                    let Some(Ok((request, mut respond))) = incoming else { break; };
                    let path = request.uri().path().to_owned();
                    let response = http::Response::builder().status(200).body(()).unwrap();
                    let mut send = respond.send_response(response, false).unwrap();
                    if path == "/held" {
                        held = Some(send);
                        held_tx.take().unwrap().send(()).unwrap();
                    } else {
                        send.send_data("fast".into(), true).unwrap();
                    }
                    paths.push(path);
                }
            }
        }
        paths
    });

    let ws_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let ws_url = format!(
        "{}://{}/native",
        if tls { "wss" } else { "ws" },
        ws_listener.local_addr()?
    );
    let (sent_tx, sent_rx) = oneshot::channel();
    let ws_task = thread::spawn(move || {
        let (stream, _) = ws_listener.accept().unwrap();
        stream.set_read_timeout(Some(DEADLINE)).unwrap();
        stream.set_write_timeout(Some(DEADLINE)).unwrap();
        if tls {
            ws_peer(
                rustls::StreamOwned::new(
                    rustls::ServerConnection::new(tls_config).unwrap(),
                    stream,
                ),
                sent_tx,
                shutdown,
            );
        } else {
            ws_peer(stream, sent_tx, shutdown);
        }
    });
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    config.set_transport_connection_limits(Some(1), Some(1), Some(8));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let handle = client.handle();
    let held_url = format!("{h2_url}/held");
    let held = tokio::spawn(async move { handle.fetch(Request::get(&held_url)?).await });
    tokio::time::timeout(DEADLINE, held_rx).await??;

    let mut request = CurlWebSocketRequest::new(ws_url);
    request.tls.verify = !tls;
    let mut connection = client.handle().websocket_connector().connect(request)?;
    assert!(matches!(
        event(&mut connection).await,
        CurlWebSocketEvent::Handshake { result: Ok(()), .. }
    ));
    // The server has queued a payload, but this connection is still paused.
    tokio::time::timeout(DEADLINE, sent_rx).await??;
    let fast = tokio::time::timeout(
        DEADLINE,
        client.fetch(Request::get(&format!("{h2_url}/fast"))?),
    )
    .await??;
    assert_eq!(
        fast.negotiated_http_version,
        Some(NegotiatedHttpVersion::Http2)
    );
    assert_eq!(fast.body_text(), "fast");
    assert!(!held.is_finished(), "a separate stream remains blocked");

    if shutdown {
        drop(client);
        assert!(tokio::time::timeout(DEADLINE, held).await??.is_err());
        assert!(matches!(
            event(&mut connection).await,
            CurlWebSocketEvent::Closed { result: Err(_) }
        ));
        ws_task.join().unwrap();
        assert_eq!(
            tokio::time::timeout(DEADLINE, h2_task).await??,
            ["/held", "/fast"]
        );
        return Ok(());
    }

    connection.sender().set_reading(true);
    let mut payload = Vec::new();
    while payload.len() < 64 * 1024 {
        let CurlWebSocketEvent::Chunk { data, .. } = event(&mut connection).await else {
            panic!("expected data");
        };
        payload.extend(data);
    }
    assert_eq!(payload, vec![7; 64 * 1024]);
    connection
        .sender()
        .send_frame(CurlWebSocketSend {
            flags: WsFlags::TEXT,
            data: b"client".to_vec(),
        })
        .await?;
    let CurlWebSocketEvent::Chunk { data, frame } = event(&mut connection).await else {
        panic!("expected Close");
    };
    assert!(frame.flags().contains(WsFlags::CLOSE));
    assert_eq!(data, 1000_u16.to_be_bytes());
    connection
        .sender()
        .send_frame(CurlWebSocketSend {
            flags: WsFlags::CLOSE,
            data,
        })
        .await?;
    drop(connection);
    ws_task.join().unwrap();

    // WS close does not complete or cancel either HTTP/2 stream.
    release_tx.send(()).unwrap();
    let response = tokio::time::timeout(DEADLINE, held).await???;
    assert_eq!(
        response.negotiated_http_version,
        Some(NegotiatedHttpVersion::Http2)
    );
    assert_eq!(response.body_text(), "held");
    let after = tokio::time::timeout(
        DEADLINE,
        client.fetch(Request::get(&format!("{h2_url}/after"))?),
    )
    .await??;
    assert_eq!(
        after.negotiated_http_version,
        Some(NegotiatedHttpVersion::Http2)
    );
    assert_eq!(after.body_text(), "fast");
    drop(client);
    assert_eq!(
        tokio::time::timeout(DEADLINE, h2_task).await??,
        ["/held", "/fast", "/after"]
    );
    Ok(())
}

#[tokio::test]
async fn http2_multiplexing_survives_paused_websocket_and_close() -> Result<()> {
    mixed_http2_and_websocket(false, false).await
}

#[tokio::test]
async fn http2_multiplexing_survives_paused_wss_and_close() -> Result<()> {
    mixed_http2_and_websocket(true, false).await
}

#[tokio::test]
async fn shared_fetch_shutdown_cancels_pending_http2_and_paused_wss() -> Result<()> {
    mixed_http2_and_websocket(true, true).await
}
