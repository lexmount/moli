use crate::{Event, FrameOpcode, spawn_standalone_connection, test_support::*};
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
    let handle = spawn_standalone_connection(70, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    handle.send_text("without echo".to_owned()).unwrap();
    assert_send_completed(&mut rx, 70, FrameOpcode::Text, 12).await;

    timeout(Duration::from_secs(3), received_rx)
        .await
        .unwrap()
        .unwrap();
    handle.close(Some(1000), String::new()).unwrap();
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
    let handle = spawn_standalone_connection(71, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    handle
        .close(Some(3001), "not acknowledged".to_owned())
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
    // UTF-8 U+1F642 spans fragments, with empty continuation and control frames.
    let frames = vec![
        0x01, 2, 0xf0, 0x9f, 0x00, 0, 0x89, 2, b'h', b'i', 0x8a, 0, 0x80, 2, 0x99, 0x82, 0x81, 0,
        0x82, 0,
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
    let handle = spawn_standalone_connection(72, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    assert_text_message(&mut rx, 72, "🙂").await;
    assert_text_message(&mut rx, 72, "").await;
    assert_binary_message(&mut rx, 72, &[]).await;
    handle.close(Some(1000), String::new()).unwrap();
    assert_closing(&mut rx, 72).await;
    assert_close(&mut rx, 72, 1000, "", true).await;
    server.await.unwrap();
}

#[tokio::test]
async fn native_invalid_frames_and_messages_fail_before_delivery() {
    let cases = [
        (vec![0x83, 0], "WebSocket receive failed:"),
        // A native framing failure must also discard a partially assembled message.
        (
            vec![0x01, 1, b'a', 0x82, 1, b'b'],
            "WebSocket receive failed:",
        ),
        (vec![0x81, 2, 0xc0, 0xaf], "text is not valid UTF-8"), // overlong
        (vec![0x81, 1, 0xf0], "text is not valid UTF-8"),       // incomplete
        (
            vec![0x01, 1, 0xf0, 0x80, 1, b'a'],
            "text is not valid UTF-8",
        ),
        (vec![0x88, 1, 0x03], "close payload has invalid length"),
        (vec![0x88, 2, 0x03, 0xee], "invalid close code 1006"),
        (
            vec![0x88, 3, 0x03, 0xe8, 0xff],
            "close reason is not valid UTF-8",
        ),
        // A 16 MiB + 1 frame header followed by one byte is enough to reject it.
        (
            vec![0x82, 127, 0, 0, 0, 0, 1, 0, 0, 1, 7],
            "frame exceeds size limit",
        ),
        // The largest legal wire length must also fail before buffering payload.
        (
            vec![0x82, 127, 127, 255, 255, 255, 255, 255, 255, 255, 7],
            "frame exceeds size limit",
        ),
    ];
    for (frames, expected_error) in cases {
        let (url, server) = frames_server(frames, async |mut socket| {
            assert!(matches!(socket.next().await, None | Some(Err(_))));
        })
        .await;
        let (tx, mut rx) = mpsc::channel(8);
        let _handle =
            spawn_standalone_connection(73, url, Vec::new(), test_websocket_context(), tx);
        recv_open_event(&mut rx).await;
        match timeout(Duration::from_secs(3), rx.recv()).await.unwrap() {
            Some(Event::Error { message, .. }) => {
                assert!(
                    message.contains(expected_error),
                    "{message}: expected {expected_error}"
                );
            }
            unexpected => panic!("expected {expected_error} before delivery, got {unexpected:?}"),
        }
        assert_close(&mut rx, 73, 1006, "", false).await;
        server.await.unwrap();
    }
}

#[tokio::test]
async fn native_message_size_limit_counts_fragments_and_resets_after_delivery() {
    const FRAME_BYTES: usize = 16 * 1024 * 1024;
    for overflow in [false, true] {
        let (url, server) = frames_server(Vec::new(), async move |mut socket| {
            // Accept exactly 64 MiB; leave 1 MiB available in the rejection case.
            let payload = vec![0xab; FRAME_BYTES];
            for (index, first) in [0x02, 0x00, 0x00, 0x00].into_iter().enumerate() {
                let size = if overflow && index == 3 {
                    FRAME_BYTES - 1024 * 1024
                } else {
                    FRAME_BYTES
                };
                let mut header = vec![first, 127];
                header.extend_from_slice(&(size as u64).to_be_bytes());
                socket.get_mut().write_all(&header).await.unwrap();
                socket.get_mut().write_all(&payload[..size]).await.unwrap();
            }
            if overflow {
                // Only one byte arrives, but the declared 2 MiB final frame would
                // exceed the message limit. Do not wait for its remaining payload.
                socket
                    .get_mut()
                    .write_all(&[0x80, 127, 0, 0, 0, 0, 0, 0x20, 0, 0, 7])
                    .await
                    .unwrap();
                assert!(matches!(socket.next().await, None | Some(Err(_))));
            } else {
                socket.get_mut().write_all(&[0x80, 0]).await.unwrap();
                socket
                    .send(Message::Text("next message".into()))
                    .await
                    .unwrap();
                assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
                let _ = socket.flush().await;
            }
        })
        .await;
        let (tx, mut rx) = mpsc::channel(8);
        let handle = spawn_standalone_connection(76, url, Vec::new(), test_websocket_context(), tx);
        recv_open_event(&mut rx).await;
        let event = timeout(Duration::from_secs(10), rx.recv()).await.unwrap();
        if overflow {
            assert!(matches!(event, Some(Event::Error { message, .. })
                if message == "WebSocket message exceeds size limit"));
            assert_close(&mut rx, 76, 1006, "", false).await;
        } else {
            let Some(Event::BinaryMessage { data, .. }) = event else {
                panic!("expected complete binary message, got {event:?}");
            };
            assert_eq!(data.len(), 4 * FRAME_BYTES);
            assert!(data.iter().all(|byte| *byte == 0xab));
            assert_text_message(&mut rx, 76, "next message").await;
            handle.close(Some(1000), String::new()).unwrap();
            assert_closing(&mut rx, 76).await;
            assert_close(&mut rx, 76, 1000, "", true).await;
        }
        timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
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
    let message = format!("a{}", "🙂".repeat(20_000));
    let expected = message.clone();
    let (url, server) = frames_server(Vec::new(), async move |mut socket| {
        socket.send(Message::Text(message.into())).await.unwrap();
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        let _ = socket.flush().await;
    })
    .await;
    let (tx, mut rx) = mpsc::channel(8);
    let handle = spawn_standalone_connection(74, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    assert_text_message(&mut rx, 74, &expected).await;
    handle.close(Some(1000), String::new()).unwrap();
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
    let handle = spawn_standalone_connection(75, url, Vec::new(), test_websocket_context(), sink);
    recv_open_event(&mut rx).await;
    timeout(Duration::from_secs(3), blocked.notified())
        .await
        .unwrap();
    handle.close(Some(1000), String::new()).unwrap();
    timeout(Duration::from_secs(3), released_rx)
        .await
        .expect("physical close must not wait for the event sink or closing timeout")
        .unwrap();
    assert!(handle.is_closed());
    assert_eq!(handle.send_binary(vec![1]), Err(crate::SendError::Closed));
    resume.add_permits(1);
    assert_text_message(&mut rx, 75, "blocked").await;
    assert_closing(&mut rx, 75).await;
    assert_close(&mut rx, 75, 1000, "", true).await;
    server.await.unwrap();
}
