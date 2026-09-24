use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, stream::SplitSink};
use moli_bounded_json::{BoundedJsonError, json_string_between_with_limit, to_string_with_limit};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use self::pending_bytes::{PendingByteBudget, PendingByteReservation};

mod pending_bytes;
#[cfg(test)]
mod tests;

// Keep the outbound boundary bounded. A slow frontend is detached instead of
// growing protocol output without limit or blocking the target owner.
const DEFAULT_WRITER_QUEUE_CAPACITY: usize = 1024;
const DEFAULT_MAX_PENDING_WRITER_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_WRITER_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct CdpSocketSink {
    output_tx: mpsc::Sender<SocketWriterCommand>,
    close_tx: tokio::sync::watch::Sender<SocketCloseSignal>,
    pending_byte_budget: PendingByteBudget,
    max_message_bytes: usize,
}

pub(crate) type CdpSocketWriterFinishedReceiver = oneshot::Receiver<()>;

struct ProtocolOutputEnvelope {
    message: String,
    _pending_bytes: PendingByteReservation,
}

enum SocketWriterCommand {
    Output(ProtocolOutputEnvelope),
    CloseAfterFlush,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SocketCloseSignal {
    Open,
    ImmediateClose,
    PeerCloseReceived,
}

pub(crate) fn spawn_socket_sink(
    socket: SplitSink<WebSocket, Message>,
) -> (CdpSocketSink, CdpSocketWriterFinishedReceiver) {
    spawn_socket_sink_with_limits(
        socket,
        DEFAULT_WRITER_QUEUE_CAPACITY,
        DEFAULT_MAX_PENDING_WRITER_BYTES,
        DEFAULT_MAX_WRITER_MESSAGE_BYTES,
    )
}

fn spawn_socket_sink_with_limits(
    socket: SplitSink<WebSocket, Message>,
    queue_capacity: usize,
    max_pending_bytes: usize,
    max_message_bytes: usize,
) -> (CdpSocketSink, CdpSocketWriterFinishedReceiver) {
    let (output_tx, mut output_rx) = mpsc::channel::<SocketWriterCommand>(queue_capacity);
    let (close_tx, mut close_rx) = tokio::sync::watch::channel(SocketCloseSignal::Open);
    let (finished_tx, finished_rx) = oneshot::channel::<()>();
    let pending_byte_budget = PendingByteBudget::new(max_pending_bytes);
    tokio::spawn(async move {
        let mut socket = socket;
        let mut finished_tx = Some(finished_tx);
        let mut close_signal = SocketCloseSignal::Open;
        'writer: loop {
            let command = tokio::select! {
                biased;
                _ = close_rx.changed() => {
                    close_signal = *close_rx.borrow();
                    break;
                }
                command = output_rx.recv() => {
                    let Some(command) = command else {
                        break;
                    };
                    command
                }
            };
            let SocketWriterCommand::Output(envelope) = command else {
                break;
            };
            let ProtocolOutputEnvelope {
                message,
                _pending_bytes,
            } = envelope;
            let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
            if trace_started.is_some() {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "writer_batch_start",
                    messages = 1,
                );
            }
            let send = send_socket_text(&mut socket, message);
            tokio::pin!(send);
            let sent = tokio::select! {
                biased;
                _ = close_rx.changed() => {
                    close_signal = *close_rx.borrow();
                    if close_signal == SocketCloseSignal::PeerCloseReceived {
                        // Do not leave a partially-started SplitSink send in
                        // its slot ahead of tungstenite's automatic close reply.
                        let _ = send.await;
                    }
                    false
                },
                sent = &mut send => sent,
            };
            if !sent {
                if close_signal == SocketCloseSignal::Open {
                    close_signal = SocketCloseSignal::ImmediateClose;
                }
                break 'writer;
            }
            if let Some(started) = trace_started {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "writer_batch_done",
                    messages = 1,
                    elapsed_us = %started.elapsed().as_micros(),
                );
            }
        }
        match close_signal {
            SocketCloseSignal::PeerCloseReceived => {
                let _ = socket.flush().await;
            }
            SocketCloseSignal::Open | SocketCloseSignal::ImmediateClose => {
                let _ = socket.close().await;
            }
        }
        notify_socket_writer_finished(&mut finished_tx);
    });
    (
        CdpSocketSink {
            output_tx,
            close_tx,
            pending_byte_budget,
            max_message_bytes,
        },
        finished_rx,
    )
}

impl CdpSocketSink {
    pub(crate) fn enqueue_owned_message(&self, message: Value) -> bool {
        if *self.close_tx.borrow() != SocketCloseSignal::Open || self.output_tx.is_closed() {
            return false;
        }
        let available_pending_bytes = self.pending_byte_budget.available();
        let serialization_limit = self.max_message_bytes.min(available_pending_bytes);
        let message = match serialize_owned_message_with_limit(message, serialization_limit) {
            Ok(message) => message,
            Err(BoundedJsonError::LimitExceeded { .. }) => {
                tracing::warn!(
                    max_message_bytes = self.max_message_bytes,
                    available_pending_bytes,
                    "closing CDP frontend after oversized protocol message"
                );
                self.close();
                return false;
            }
            Err(BoundedJsonError::Serialization(error)) => {
                tracing::warn!(?error, "failed to serialize CDP protocol output");
                self.close();
                return false;
            }
        };
        let Some(pending_bytes) = self.pending_byte_budget.try_reserve(message.len()) else {
            tracing::warn!(
                message_bytes = message.len(),
                max_pending_bytes = self.pending_byte_budget.limit(),
                "closing CDP frontend after outbound byte budget overflow"
            );
            self.close();
            return false;
        };
        let envelope = ProtocolOutputEnvelope {
            message,
            _pending_bytes: pending_bytes,
        };
        if self
            .output_tx
            .try_send(SocketWriterCommand::Output(envelope))
            .is_err()
        {
            self.close();
            return false;
        }
        true
    }

    pub(crate) fn close(&self) {
        self.close_tx.send_if_modified(|signal| {
            if *signal == SocketCloseSignal::PeerCloseReceived {
                return false;
            }
            *signal = SocketCloseSignal::ImmediateClose;
            true
        });
    }

    pub(crate) fn peer_close_received(&self) {
        let _ = self.close_tx.send(SocketCloseSignal::PeerCloseReceived);
    }

    pub(crate) fn close_after_flush(&self) {
        if self
            .output_tx
            .try_send(SocketWriterCommand::CloseAfterFlush)
            .is_err()
        {
            self.close();
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        let (output_tx, _output_rx) = mpsc::channel(1);
        let (close_tx, _close_rx) = tokio::sync::watch::channel(SocketCloseSignal::Open);
        Self {
            output_tx,
            close_tx,
            pending_byte_budget: PendingByteBudget::new(DEFAULT_MAX_PENDING_WRITER_BYTES),
            max_message_bytes: DEFAULT_MAX_WRITER_MESSAGE_BYTES,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_stalled_writer_for_test(
        queue_capacity: usize,
    ) -> (Self, StalledCdpSocketWriter) {
        let (output_tx, output_rx) = mpsc::channel(queue_capacity);
        let (close_tx, close_rx) = tokio::sync::watch::channel(SocketCloseSignal::Open);
        (
            Self {
                output_tx,
                close_tx,
                pending_byte_budget: PendingByteBudget::new(DEFAULT_MAX_PENDING_WRITER_BYTES),
                max_message_bytes: DEFAULT_MAX_WRITER_MESSAGE_BYTES,
            },
            StalledCdpSocketWriter {
                output_rx,
                close_rx,
            },
        )
    }
}

struct OwnedOuterHtmlResponse {
    id: u64,
    outer_html: String,
    session_id: Option<String>,
}

fn serialize_owned_message_with_limit(
    message: Value,
    limit: usize,
) -> Result<String, BoundedJsonError> {
    let response = match take_owned_outer_html_response(message) {
        Ok(response) => response,
        Err(message) => return to_string_with_limit(&message, limit),
    };

    let prefix = format!("{{\"id\":{},\"result\":{{\"outerHTML\":\"", response.id);
    let suffix = if let Some(session_id) = response.session_id {
        let encoded_session_id = to_string_with_limit(&session_id, limit)?;
        let mut suffix = String::with_capacity(encoded_session_id.len() + 15);
        suffix.push_str("\"},\"sessionId\":");
        suffix.push_str(&encoded_session_id);
        suffix.push('}');
        suffix
    } else {
        "\"}}".to_owned()
    };
    json_string_between_with_limit(response.outer_html, &prefix, &suffix, limit)
}

fn take_owned_outer_html_response(message: Value) -> Result<OwnedOuterHtmlResponse, Value> {
    let Value::Object(object) = &message else {
        return Err(message);
    };
    let Some(id) = object.get("id").and_then(Value::as_u64) else {
        return Err(message);
    };
    let session_id = match object.get("sessionId") {
        Some(Value::String(session_id)) => Some(session_id),
        Some(_) => return Err(message),
        None => None,
    };
    if object.len() != 2 + usize::from(session_id.is_some()) {
        return Err(message);
    }
    let Some(result) = object.get("result").and_then(Value::as_object) else {
        return Err(message);
    };
    if result.len() != 1 || !matches!(result.get("outerHTML"), Some(Value::String(_))) {
        return Err(message);
    }

    let Value::Object(mut object) = message else {
        unreachable!("the message shape was validated above");
    };
    let session_id = object.remove("sessionId").map(|value| {
        let Value::String(session_id) = value else {
            unreachable!("the session id shape was validated above");
        };
        session_id
    });
    let Value::Object(mut result) = object
        .remove("result")
        .expect("the result field was validated above")
    else {
        unreachable!("the result shape was validated above");
    };
    let Value::String(outer_html) = result
        .remove("outerHTML")
        .expect("the outerHTML field was validated above")
    else {
        unreachable!("the outerHTML shape was validated above");
    };
    Ok(OwnedOuterHtmlResponse {
        id,
        outer_html,
        session_id,
    })
}

#[cfg(test)]
pub(crate) struct StalledCdpSocketWriter {
    output_rx: mpsc::Receiver<SocketWriterCommand>,
    close_rx: tokio::sync::watch::Receiver<SocketCloseSignal>,
}

#[cfg(test)]
impl StalledCdpSocketWriter {
    pub(crate) fn take_message(&mut self) -> serde_json::Value {
        let command = self
            .output_rx
            .try_recv()
            .expect("stalled CDP writer should have queued output");
        let SocketWriterCommand::Output(envelope) = command else {
            panic!("stalled CDP writer received close before output");
        };
        serde_json::from_str(&envelope.message).expect("queued CDP output JSON")
    }

    pub(crate) fn is_open(&self) -> bool {
        *self.close_rx.borrow() == SocketCloseSignal::Open
    }
}

async fn send_socket_text(socket: &mut SplitSink<WebSocket, Message>, text: String) -> bool {
    socket.send(Message::Text(text.into())).await.is_ok()
}

fn notify_socket_writer_finished(finished_tx: &mut Option<oneshot::Sender<()>>) {
    if let Some(finished_tx) = finished_tx.take() {
        let _ = finished_tx.send(());
    }
}
