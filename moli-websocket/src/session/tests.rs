use super::CLOSE_TIMEOUT;
use crate::{Command, Event, FrameOpcode, spawn_connection, test_support::*};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    net::{TcpListener, TcpSocket},
    sync::{mpsc, oneshot},
    time::{Duration, timeout},
};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn native_close_waits_for_queued_data_before_starting_timeout() {
    const MESSAGE_BYTES: usize = 32 * 1024 * 1024;
    // Set the receive window before accepting TCP, then stop reading after the
    // upgrade. The admitted message must outlive the closing-handshake timeout.
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(128 * 1024).unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let listener = socket.listen(1).unwrap();
    let url = format!("ws://{}/drain-before-close", listener.local_addr().unwrap());
    let (resume_tx, resume_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        if resume_rx.await.is_err() {
            return;
        }
        let message = socket.next().await.unwrap().unwrap();
        let Message::Binary(data) = message else {
            panic!("all queued data must precede Close");
        };
        assert_eq!(data.len(), MESSAGE_BYTES);
        assert!(
            data.iter()
                .enumerate()
                .all(|(i, byte)| *byte == (i % 251) as u8)
        );
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        socket.flush().await.unwrap();
    });
    let (tx, mut rx) = mpsc::channel(8);
    let handle = spawn_connection(80, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    handle
        .send(Command::SendBinary(
            (0..MESSAGE_BYTES).map(|i| (i % 251) as u8).collect(),
        ))
        .unwrap();
    handle
        .send(Command::Close {
            code: Some(1000),
            reason: String::new(),
        })
        .unwrap();
    // Closing confirms the session has processed Close, while TCP backpressure
    // prevents the preceding message from completing. No handshake timer applies.
    assert_closing(&mut rx, 80).await;
    let premature = timeout(CLOSE_TIMEOUT + Duration::from_secs(1), rx.recv()).await;
    assert!(
        premature.is_err(),
        "queued data was interrupted: {premature:?}"
    );
    resume_tx.send(()).unwrap();
    assert_frame_sent(&mut rx, 80, FrameOpcode::Binary, MESSAGE_BYTES).await;
    assert_buffered_amount_consumed(&mut rx, 80, MESSAGE_BYTES).await;
    assert_close(&mut rx, 80, 1000, "", true).await;
    timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn native_close_still_times_out_when_peer_never_answers_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/unanswered-close", listener.local_addr().unwrap());
    let (close_tx, close_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        close_tx.send(()).unwrap();
        // Keep TCP open without flushing tungstenite's queued Close reply.
        let _ = finish_rx.await;
    });
    let (tx, mut rx) = mpsc::channel(8);
    let handle = spawn_connection(81, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    handle
        .send(Command::Close {
            code: Some(1000),
            reason: String::new(),
        })
        .unwrap();
    assert_closing(&mut rx, 81).await;
    timeout(Duration::from_secs(3), close_rx)
        .await
        .unwrap()
        .unwrap();
    let event = timeout(CLOSE_TIMEOUT + Duration::from_secs(1), rx.recv())
        .await
        .expect("a sent Close still has a handshake deadline")
        .unwrap();
    assert!(
        matches!(event, Event::Error { socket_id: 81, ref message } if message == "WebSocket closing handshake timed out"),
        "expected closing handshake timeout, got {event:?}"
    );
    assert_close(&mut rx, 81, 1006, "", false).await;
    assert!(
        timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .is_none()
    );
    finish_tx.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn native_fragmented_sends_complete_while_incoming_sink_is_blocked() {
    use std::sync::Arc;
    use tokio::sync::{Notify, Semaphore};

    const MESSAGE_BYTES: usize = 128 * 1024;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/independent-sends", listener.local_addr().unwrap());
    let blocked = Arc::new(Notify::new());
    let resume = Arc::new(Semaphore::new(0));
    let (received_tx, received_rx) = oneshot::channel();
    let server_resume = resume.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        socket.send(Message::Text("blocked".into())).await.unwrap();
        for expected in [17, 29] {
            let Message::Binary(data) = socket.next().await.unwrap().unwrap() else {
                panic!("expected a complete binary message before any Close");
            };
            assert_eq!(data.len(), MESSAGE_BYTES);
            assert!(data.iter().all(|byte| *byte == expected));
        }
        // The sink can only resume after both multi-frame messages reach the
        // server. Sending Close earlier would bypass the original deadlock.
        server_resume.add_permits(1);
        received_tx.send(()).unwrap();
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        socket.flush().await.unwrap();
    });
    let (events, mut rx) = mpsc::channel(8);
    let sink = crate::EventSender::with_async_sink({
        let blocked = blocked.clone();
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
    let handle = spawn_connection(82, url, Vec::new(), test_websocket_context(), sink);
    recv_open_event(&mut rx).await;
    timeout(Duration::from_secs(3), blocked.notified())
        .await
        .unwrap();
    for byte in [17, 29] {
        handle
            .send(Command::SendBinary(vec![byte; MESSAGE_BYTES]))
            .unwrap();
    }
    timeout(Duration::from_secs(3), received_rx)
        .await
        .expect("all fragments must be sent before the incoming sink resumes")
        .unwrap();
    assert_text_message(&mut rx, 82, "blocked").await;
    for _ in 0..2 {
        assert_frame_sent(&mut rx, 82, FrameOpcode::Binary, MESSAGE_BYTES).await;
        assert_buffered_amount_consumed(&mut rx, 82, MESSAGE_BYTES).await;
    }
    handle
        .send(Command::Close {
            code: Some(1000),
            reason: String::new(),
        })
        .unwrap();
    assert_closing(&mut rx, 82).await;
    assert_close(&mut rx, 82, 1000, "", true).await;
    server.await.unwrap();
}

#[tokio::test]
async fn completed_send_notifications_bound_further_message_scheduling() {
    use super::{Assembler, Closing, Session};
    use crate::commands::{MAX_QUEUED_MESSAGES, command_channel};
    use moli_curl::websocket::MAX_SEND_FRAME_BYTES;

    let mut session = Session {
        socket_id: 83,
        outbox: Default::default(),
        outgoing: Default::default(),
        pongs: Default::default(),
        assembler: Assembler::default(),
        closing: Closing::default(),
        terminal: false,
    };
    let (port, mut commands) = command_channel();
    // Complete a full admission window while the application's notifications
    // remain undelivered. Their residence must not grow with new send calls.
    for _ in 0..MAX_QUEUED_MESSAGES {
        port.send(Command::SendText("sent".to_owned())).unwrap();
        session.command(commands.recv().await.unwrap());
        let (_, flight) = session
            .next_frame()
            .expect("bounded sends can make progress");
        session.sent(flight);
    }
    port.send(Command::SendBinary(vec![7; 2 * MAX_SEND_FRAME_BYTES]))
        .unwrap();
    session.command(commands.recv().await.unwrap());
    assert!(
        session.next_frame().is_none(),
        "pending completion notifications must bound new messages"
    );
    assert!(matches!(
        session.outbox.pop_front(),
        Some(Event::FrameSent { .. })
    ));
    for _ in 0..2 {
        let (frame, flight) = session
            .next_frame()
            .expect("notification delivery permits the next complete message");
        assert_eq!(frame.data.len(), MAX_SEND_FRAME_BYTES);
        session.sent(flight);
    }
    assert!(session.outgoing.is_empty());
    assert!(!session.terminal);
    assert_eq!(
        session
            .outbox
            .iter()
            .filter(|event| matches!(event, Event::FrameSent { .. }))
            .count(),
        MAX_QUEUED_MESSAGES
    );
}
