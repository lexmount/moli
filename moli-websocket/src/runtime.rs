use std::sync::OnceLock;

use crate::commands::command_channel;

use crate::{
    ConnectOptions, ConnectionHandle, HandshakeController, SyntheticPeer,
    connection::run_websocket_connection, events::EventSender, events::send_error_and_close,
    handle::HandshakeDecision, synthetic::run_synthetic_websocket_connection,
};

pub fn spawn_connection(
    socket_id: u64,
    url: String,
    protocols: Vec<String>,
    context: ConnectOptions,
    event_tx: impl Into<EventSender>,
) -> ConnectionHandle {
    spawn_native_connection(socket_id, url, protocols, context, event_tx.into(), None)
}

/// Starts a native connection whose browser Open awaits one explicit decision.
pub fn spawn_connection_with_handshake_pause(
    socket_id: u64,
    url: String,
    protocols: Vec<String>,
    context: ConnectOptions,
    event_tx: impl Into<EventSender>,
) -> (ConnectionHandle, HandshakeController) {
    let (decision, receiver) = tokio::sync::oneshot::channel();
    let connection = spawn_native_connection(
        socket_id,
        url,
        protocols,
        context,
        event_tx.into(),
        Some(receiver),
    );
    (connection, HandshakeController::new(decision))
}

fn spawn_native_connection(
    socket_id: u64,
    url: String,
    protocols: Vec<String>,
    context: ConnectOptions,
    event_tx: EventSender,
    decision: Option<tokio::sync::oneshot::Receiver<HandshakeDecision>>,
) -> ConnectionHandle {
    let (command_tx, command_rx) = command_channel();
    let task = websocket_runtime().spawn(async move {
        let _ = run_websocket_connection(
            socket_id, url, protocols, context, command_rx, event_tx, decision,
        )
        .await;
    });
    ConnectionHandle::new(command_tx, task.abort_handle())
}

pub fn spawn_failed_connection(
    socket_id: u64,
    message: String,
    event_tx: impl Into<EventSender>,
) -> ConnectionHandle {
    let event_tx = event_tx.into();
    let (command_tx, _command_rx) = command_channel();
    let task = websocket_runtime().spawn(async move {
        let _ = send_error_and_close(&event_tx, socket_id, message).await;
    });
    ConnectionHandle::new(command_tx, task.abort_handle())
}

pub fn spawn_synthetic_connection(
    socket_id: u64,
    request_headers: Vec<(String, String)>,
    response_status: u16,
    response_headers: Vec<(String, String)>,
    event_tx: impl Into<EventSender>,
) -> (ConnectionHandle, SyntheticPeer) {
    let event_tx = event_tx.into();
    let (command_tx, command_rx) = command_channel();
    let task = websocket_runtime().spawn(async move {
        let _ = run_synthetic_websocket_connection(
            socket_id,
            command_rx,
            event_tx,
            request_headers,
            response_status,
            response_headers,
        )
        .await;
    });
    let connection = ConnectionHandle::new(command_tx, task.abort_handle());
    let peer = connection.synthetic_peer();
    (connection, peer)
}

fn websocket_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("moli-websocket-runtime")
            .build()
            .expect("failed to build moli websocket runtime")
    })
}
