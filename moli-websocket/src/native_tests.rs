use crate::{Command, Event, FrameOpcode, spawn_connection, test_support::*};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::AsyncWriteExt,
    net::TcpListener,
    sync::{mpsc, oneshot},
    time::{Duration, timeout},
};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn native_send_accounting_does_not_require_a_server_echo() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/no-echo", listener.local_addr().unwrap());
    let (received_tx, received_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Text("without echo".into())
        );
        received_tx.send(()).unwrap();
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        let _ = socket.flush().await;
    });
    let (tx, mut rx) = mpsc::channel(8);
    let handle = spawn_connection(70, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    handle
        .send(Command::SendText("without echo".to_owned()))
        .unwrap();
    assert_frame_sent(&mut rx, 70, FrameOpcode::Text, 12).await;
    assert_buffered_amount_consumed(&mut rx, 70, 12).await;
    timeout(Duration::from_secs(3), received_rx)
        .await
        .unwrap()
        .unwrap();
    handle
        .send(Command::Close {
            code: Some(1000),
            reason: String::new(),
        })
        .unwrap();
    assert_closing(&mut rx, 70).await;
    assert_close(&mut rx, 70, 1000, "", true).await;
    server.await.unwrap();
}

#[tokio::test]
async fn native_local_close_without_peer_close_is_abnormal_once() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/missing-close", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        // Intentionally drop without flushing tungstenite's queued Close reply.
    });
    let (tx, mut rx) = mpsc::channel(8);
    let handle = spawn_connection(71, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    handle
        .send(Command::Close {
            code: Some(3001),
            reason: "not acknowledged".to_owned(),
        })
        .unwrap();
    assert_closing(&mut rx, 71).await;
    assert!(matches!(
        timeout(Duration::from_secs(3), rx.recv()).await.unwrap(),
        Some(Event::Error { .. })
    ));
    assert_close(&mut rx, 71, 1006, "", false).await;
    assert!(
        timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .is_none(),
        "there is only one terminal Close"
    );
    server.await.unwrap();
}

async fn frames_server<F, Fut>(frames: Vec<u8>, handler: F) -> (String, tokio::task::JoinHandle<()>)
where
    F: FnOnce(tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/frames", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        socket.get_mut().write_all(&frames).await.unwrap();
        handler(socket).await;
    });
    (url, server)
}

#[tokio::test]
async fn native_fragmented_utf8_and_ping_are_assembled_as_one_message() {
    // UTF-8 U+1F642 spans continuation frames, with a Ping in between.
    let frames = vec![
        0x01, 2, 0xf0, 0x9f, 0x89, 2, b'h', b'i', 0x80, 2, 0x99, 0x82, 0x82, 0,
    ];
    let (url, server) = frames_server(frames, async |mut socket| {
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Pong(b"hi".to_vec().into())
        );
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        let _ = socket.flush().await;
    })
    .await;
    let (tx, mut rx) = mpsc::channel(8);
    let handle = spawn_connection(72, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    assert_text_message(&mut rx, 72, "🙂").await;
    assert_binary_message(&mut rx, 72, &[]).await;
    handle
        .send(Command::Close {
            code: Some(1000),
            reason: String::new(),
        })
        .unwrap();
    assert_closing(&mut rx, 72).await;
    assert_close(&mut rx, 72, 1000, "", true).await;
    server.await.unwrap();
}

#[tokio::test]
async fn native_invalid_utf8_close_code_and_frame_size_fail_without_message_delivery() {
    let cases = [
        vec![0x81, 2, 0xc0, 0xaf],       // overlong UTF-8
        vec![0x88, 2, 0x03, 0xee],       // reserved close code 1006
        vec![0x88, 3, 0x03, 0xe8, 0xff], // invalid close reason
        // A 16 MiB + 1 frame header followed by one byte is enough to reject it.
        vec![0x82, 127, 0, 0, 0, 0, 1, 0, 0, 1, 7],
    ];
    for frames in cases {
        let (url, server) = frames_server(frames, async |mut socket| {
            assert!(matches!(socket.next().await, None | Some(Err(_))));
        })
        .await;
        let (tx, mut rx) = mpsc::channel(8);
        let _handle = spawn_connection(73, url, Vec::new(), test_websocket_context(), tx);
        recv_open_event(&mut rx).await;
        assert!(matches!(
            timeout(Duration::from_secs(3), rx.recv()).await.unwrap(),
            Some(Event::Error { .. })
        ));
        assert_close(&mut rx, 73, 1006, "", false).await;
        server.await.unwrap();
    }
}

#[tokio::test]
async fn native_unsolicited_extensions_are_rejected_before_open() {
    let message = websocket_computed_accept_handshake_failure_message(
        "extensions",
        vec![
            "Upgrade: websocket",
            "Connection: Upgrade",
            "Sec-WebSocket-Extensions: permessage-deflate",
        ],
        Vec::new(),
    )
    .await;
    assert!(message.contains("unrequested extension"), "{message}");
}

#[tokio::test]
async fn native_large_incoming_frame_keeps_chunk_offsets_and_utf8() {
    let message = "🙂".repeat(20_000);
    let expected = message.clone();
    let (url, server) = frames_server(Vec::new(), async move |mut socket| {
        socket.send(Message::Text(message.into())).await.unwrap();
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        let _ = socket.flush().await;
    })
    .await;
    let (tx, mut rx) = mpsc::channel(8);
    let handle = spawn_connection(74, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    assert_text_message(&mut rx, 74, &expected).await;
    handle
        .send(Command::Close {
            code: Some(1000),
            reason: String::new(),
        })
        .unwrap();
    assert_closing(&mut rx, 74).await;
    assert_close(&mut rx, 74, 1000, "", true).await;
    server.await.unwrap();
}

#[tokio::test]
async fn native_close_handshake_finishes_while_message_sink_is_blocked() {
    use std::sync::Arc;
    use tokio::{
        io::AsyncReadExt,
        sync::{Notify, Semaphore},
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/blocked-delivery", listener.local_addr().unwrap());
    let (released_tx, released_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        socket.send(Message::Text("blocked".into())).await.unwrap();
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        let _ = socket.flush().await;
        assert_eq!(socket.get_mut().read(&mut [0]).await.unwrap(), 0);
        released_tx.send(()).unwrap();
    });
    let blocked = Arc::new(Notify::new());
    let resume = Arc::new(Semaphore::new(0));
    let (events, mut rx) = mpsc::channel(8);
    let sink = crate::EventSender::with_async_sink({
        let blocked = blocked.clone();
        let resume = resume.clone();
        move |event| {
            let blocked = blocked.clone();
            let resume = resume.clone();
            let events = events.clone();
            async move {
                if matches!(event, Event::TextMessage { .. }) {
                    blocked.notify_one();
                    resume.acquire().await.unwrap().forget();
                }
                events.send(event).await.is_ok()
            }
        }
    });
    let handle = spawn_connection(75, url, Vec::new(), test_websocket_context(), sink);
    recv_open_event(&mut rx).await;
    timeout(Duration::from_secs(3), blocked.notified())
        .await
        .unwrap();
    handle
        .send(Command::Close {
            code: Some(1000),
            reason: String::new(),
        })
        .unwrap();
    timeout(Duration::from_secs(3), released_rx)
        .await
        .expect("physical close must not wait for the event sink or closing timeout")
        .unwrap();
    resume.add_permits(1);
    assert_text_message(&mut rx, 75, "blocked").await;
    assert_closing(&mut rx, 75).await;
    assert_close(&mut rx, 75, 1000, "", true).await;
    server.await.unwrap();
}
