use super::*;

/// A real multi-connection peer for the shared Window/Worker stream contract.
pub struct StreamWebSocketFixture {
    pub base_url: String,
    server: tokio::task::JoinHandle<()>,
}

impl StreamWebSocketFixture {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.unwrap();
                        clients.spawn(async move {
                            let protocol = Arc::new(std::sync::atomic::AtomicBool::new(false));
                            let callback_protocol = protocol.clone();
                            let socket = tokio_tungstenite::accept_hdr_async(
                                stream,
                                InfallibleHandshakeCallback(move |request: &Request, mut response: Response| {
                                    if request.uri().path() == "/protocol_array" {
                                        callback_protocol.store(true, std::sync::atomic::Ordering::Relaxed);
                                        response.headers_mut().insert("sec-websocket-protocol", "alpha".parse().unwrap());
                                    }
                                    response
                                }),
                            ).await;
                            let Ok(mut socket) = socket else {return;};
                            if protocol.load(std::sync::atomic::Ordering::Relaxed) {
                                let _ = socket.send(Message::Text("alpha".into())).await;
                                let _ = socket.close(None).await;
                                let _ = socket.next().await;
                                return;
                            }
                            while let Some(message) = socket.next().await {
                                match message {
                                    Ok(Message::Text(text)) => {
                                        if socket.send(Message::Text(text)).await.is_err() {break;}
                                    }
                                    Ok(Message::Binary(bytes)) => {
                                        if socket.send(Message::Binary(bytes)).await.is_err() {break;}
                                    }
                                    Ok(Message::Close(_)) => {let _ = socket.flush().await;break;}
                                    Ok(Message::Ping(bytes)) => {let _ = socket.send(Message::Pong(bytes)).await;}
                                    Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
                                    Err(_) => break,
                                }
                            }
                        });
                    }
                    completed = clients.join_next(), if !clients.is_empty() => {
                        completed.unwrap().unwrap();
                    }
                }
            }
        });
        Self { base_url, server }
    }
}

impl Drop for StreamWebSocketFixture {
    fn drop(&mut self) {
        // Dropping JoinSet on the server task also cancels every accepted peer.
        self.server.abort();
    }
}

/// Pong proves the client consumed three frames, enough to fill readable,
/// parked and ingress slots without requiring a sleep or an internal test hook.
pub async fn spawn_parked_stream_websocket_server()
-> (String, oneshot::Receiver<()>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/parked", listener.local_addr().unwrap());
    let (pong_tx, pong_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        for word in ["first", "second", "third"] {
            socket.send(Message::Text(word.into())).await.unwrap();
        }
        socket
            .send(Message::Ping(b"parked".to_vec().into()))
            .await
            .unwrap();
        let mut pong_tx = Some(pong_tx);
        while let Some(message) = socket.next().await {
            match message {
                Ok(Message::Pong(bytes)) => {
                    assert_eq!(&bytes[..], b"parked");
                    if let Some(sender) = pong_tx.take() {
                        let _ = sender.send(());
                    }
                }
                Ok(Message::Close(_)) => {
                    let _ = socket.flush().await;
                    break;
                }
                Err(_) => break,
                other => panic!("unexpected client frame: {other:?}"),
            }
        }
    });
    (url, pong_rx, server)
}
