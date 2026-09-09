use std::sync::OnceLock;

use crate::commands::command_channel;

use crate::{
    CommandSender, ConnectOptions, ConnectionHandle, connection::run_websocket_connection,
    events::EventSender, events::send_error_and_close,
    synthetic::run_synthetic_websocket_connection,
};

pub fn spawn_connection(
    socket_id: u64,
    url: String,
    protocols: Vec<String>,
    context: ConnectOptions,
    event_tx: impl Into<EventSender>,
) -> CommandSender {
    let event_tx = event_tx.into();
    let (command_tx, command_rx) = command_channel();
    let task = websocket_runtime().spawn(async move {
        let _ = run_websocket_connection(socket_id, url, protocols, context, command_rx, event_tx)
            .await;
    });
    ConnectionHandle::new(command_tx, task.abort_handle())
}

pub fn spawn_failed_connection(
    socket_id: u64,
    message: String,
    event_tx: impl Into<EventSender>,
) -> CommandSender {
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
) -> CommandSender {
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
    ConnectionHandle::new(command_tx, task.abort_handle())
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
