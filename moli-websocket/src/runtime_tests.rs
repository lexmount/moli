use crate::{
    Event, SendError, spawn_connection,
    test_support::{spawn_text_echo_websocket_server, test_websocket_context},
};
use moli_curl::websocket::CurlWebSocketRuntime;
use tokio::{
    sync::mpsc,
    time::{Duration, timeout},
};

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn provided_native_connector_shutdown_ends_browser_session() {
    let (url, server) = spawn_text_echo_websocket_server().await;
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut context = test_websocket_context();
    context.connector = Some(runtime.connector());
    let (events, mut incoming) = mpsc::channel(8);
    let handle = spawn_connection(1000, url, Vec::new(), context, events);
    assert!(matches!(next(&mut incoming).await, Event::Open { .. }));
    handle.send_text("shared owner".into()).unwrap();
    assert!(matches!(
        next(&mut incoming).await,
        Event::SendCompleted { .. }
    ));
    assert!(
        matches!(next(&mut incoming).await, Event::TextMessage { data, .. } if data == "shared owner")
    );
    drop(runtime);
    assert!(matches!(next(&mut incoming).await, Event::Error { .. }));
    assert!(matches!(
        next(&mut incoming).await,
        Event::Close { code: 1006, .. }
    ));
    assert!(handle.is_closed());
    assert_eq!(handle.send_binary(vec![1]), Err(SendError::Closed));
    timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn closed_provided_connector_never_falls_back_to_global_owner() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("ws://{}/must-not-connect", listener.local_addr().unwrap());
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut context = test_websocket_context();
    context.connector = Some(runtime.connector());
    drop(runtime);
    let (events, mut incoming) = mpsc::channel(8);
    let handle = spawn_connection(1001, url, Vec::new(), context, events);
    assert!(
        matches!(next(&mut incoming).await, Event::Error { message, .. } if message.contains("curl WebSocket runtime is closed"))
    );
    assert!(matches!(
        next(&mut incoming).await,
        Event::Close { code: 1006, .. }
    ));
    assert!(handle.is_closed());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
