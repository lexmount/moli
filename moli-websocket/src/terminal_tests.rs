use crate::{
    Command, Event, EventSender, SendError,
    commands::{MAX_QUEUED_BYTES, MAX_QUEUED_MESSAGES, command_channel},
    connection::run_websocket_connection,
    handle::HandshakeDecision,
    test_support::{spawn_raw_websocket_response_server, test_websocket_context},
};
use std::sync::Arc;
use tokio::{
    io::AsyncReadExt,
    net::TcpListener,
    sync::{Semaphore, mpsc, oneshot},
    time::{Duration, timeout},
};

const DEADLINE: Duration = Duration::from_secs(3);

fn blocked_terminal_sink() -> (EventSender, mpsc::UnboundedReceiver<Event>, Arc<Semaphore>) {
    let (events, incoming) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let sink_release = release.clone();
    let sink = EventSender::with_async_sink(move |event| {
        let release = sink_release.clone();
        let events = events.clone();
        async move {
            let terminal = matches!(
                event,
                Event::Closing { .. } | Event::Error { .. } | Event::Close { .. }
            );
            events.send(event).unwrap();
            if terminal {
                release.acquire().await.unwrap().forget();
            }
            true
        }
    });
    (sink, incoming, release)
}

async fn next(events: &mut mpsc::UnboundedReceiver<Event>) -> Event {
    timeout(DEADLINE, events.recv()).await.unwrap().unwrap()
}

#[tokio::test]
async fn synthetic_terminal_discards_commands_before_blocked_delivery() {
    for terminal in [
        Command::Close {
            code: Some(1000),
            reason: String::new(),
        },
        Command::ServerClose {
            code: Some(1000),
            reason: String::new(),
        },
        Command::Fail("failed".to_owned()),
    ] {
        let (commands, receiver) = command_channel();
        commands.send(terminal).unwrap();
        commands.send(Command::SendBinary(vec![7; 4096])).unwrap();
        commands
            .send(Command::SendText("discarded".into()))
            .unwrap();
        let (sink, mut events, release) = blocked_terminal_sink();
        let task = tokio::spawn(crate::synthetic::run_synthetic_websocket_connection(
            98,
            receiver,
            sink,
            Vec::new(),
            101,
            Vec::new(),
        ));
        assert!(matches!(next(&mut events).await, Event::Open { .. }));
        let first = next(&mut events).await;
        assert!(commands.is_closed());
        assert_eq!(
            commands.send(Command::SendBinary(vec![1])),
            Err(SendError::Closed)
        );
        assert_eq!(
            commands.available_data_capacity(),
            (MAX_QUEUED_BYTES, MAX_QUEUED_MESSAGES)
        );
        release.add_permits(3);
        timeout(DEADLINE, task).await.unwrap().unwrap().unwrap();
        let mut closes = usize::from(matches!(first, Event::Close { .. }));
        while let Some(event) = events.recv().await {
            assert!(
                matches!(event, Event::Close { .. }),
                "unexpected terminal event: {event:?}"
            );
            closes += 1;
        }
        assert_eq!(closes, 1);
    }
}

#[tokio::test]
async fn failed_handshake_closes_commands_before_blocked_error() {
    let (url, server) = spawn_raw_websocket_response_server(
        "terminal-admission",
        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n",
    )
    .await;
    // Also cover request validation, which ends before a native handle exists.
    for url in [url, "not a websocket URL".into()] {
        let (commands, receiver) = command_channel();
        let (sink, mut events, release) = blocked_terminal_sink();
        let task = tokio::spawn(run_websocket_connection(
            crate::runtime::standalone_connector().unwrap(),
            99,
            url,
            Vec::new(),
            test_websocket_context(),
            receiver,
            sink,
            None,
        ));
        assert!(matches!(next(&mut events).await, Event::Error { .. }));
        assert!(commands.is_closed());
        assert_eq!(
            commands.send(Command::SendBinary(vec![1])),
            Err(SendError::Closed)
        );
        release.add_permits(2);
        assert!(matches!(
            next(&mut events).await,
            Event::Close { code: 1006, .. }
        ));
        timeout(DEADLINE, task).await.unwrap().unwrap().unwrap();
        assert!(events.recv().await.is_none());
    }
    server.await.unwrap();
}

#[tokio::test]
async fn close_during_handshake_releases_tcp_before_blocked_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/cancel-opening", listener.local_addr().unwrap());
    let (commands, receiver) = command_channel();
    let (sink, mut events, release) = blocked_terminal_sink();
    let task = tokio::spawn(run_websocket_connection(
        crate::runtime::standalone_connector().unwrap(),
        100,
        url,
        Vec::new(),
        test_websocket_context(),
        receiver,
        sink,
        None,
    ));
    let (mut stream, _) = timeout(DEADLINE, listener.accept()).await.unwrap().unwrap();
    // Confirm the HTTP request arrived, then hold back the upgrade response.
    let mut request = Vec::new();
    timeout(DEADLINE, async {
        while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await.unwrap());
        }
    })
    .await
    .unwrap();
    commands
        .send(Command::Close {
            code: Some(1000),
            reason: String::new(),
        })
        .unwrap();
    assert!(matches!(next(&mut events).await, Event::Error { .. }));
    assert!(commands.is_closed());
    assert_eq!(
        commands.send(Command::SendBinary(vec![1])),
        Err(SendError::Closed)
    );
    assert_eq!(
        timeout(DEADLINE, stream.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    release.add_permits(2);
    assert!(matches!(
        next(&mut events).await,
        Event::Close { code: 1006, .. }
    ));
    timeout(DEADLINE, task).await.unwrap().unwrap().unwrap();
    assert!(events.recv().await.is_none());
}

#[tokio::test]
async fn rejected_handshake_decision_closes_commands_before_blocked_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/reject-open", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_eq!(socket.get_mut().read(&mut [0]).await.unwrap(), 0);
    });
    let (commands, receiver) = command_channel();
    let (decision, decide) = oneshot::channel();
    let (sink, mut events, release) = blocked_terminal_sink();
    let task = tokio::spawn(run_websocket_connection(
        crate::runtime::standalone_connector().unwrap(),
        101,
        url,
        Vec::new(),
        test_websocket_context(),
        receiver,
        sink,
        Some(decide),
    ));
    assert!(matches!(
        next(&mut events).await,
        Event::HandshakeResponse { .. }
    ));
    decision
        .send(HandshakeDecision::Fail("rejected".into()))
        .unwrap();
    assert!(matches!(next(&mut events).await, Event::Error { .. }));
    assert!(commands.is_closed());
    assert_eq!(
        commands.send(Command::SendBinary(vec![1])),
        Err(SendError::Closed)
    );
    timeout(DEADLINE, server).await.unwrap().unwrap();
    release.add_permits(2);
    assert!(matches!(
        next(&mut events).await,
        Event::Close { code: 1006, .. }
    ));
    timeout(DEADLINE, task).await.unwrap().unwrap().unwrap();
    assert!(events.recv().await.is_none());
}
