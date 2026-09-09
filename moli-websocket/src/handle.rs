use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use crate::commands::{CommandPort, MAX_CONTROL_BYTES};
use tokio::sync::oneshot;
use tokio::task::AbortHandle;

use crate::Command;

/// The producer's ownership of a WebSocket connection.
///
/// Clones keep the connection alive. Dropping the last handle or calling
/// `cancel` releases the connection even while its handshake, write, or event
/// sink is waiting. Use `close` for the graceful closing handshake.
#[derive(Clone, Debug)]
pub struct ConnectionHandle {
    inner: Arc<ConnectionControl>,
}

#[derive(Debug)]
struct ConnectionControl {
    command_tx: CommandPort,
    cancelled: AtomicBool,
    task: AbortOnDrop,
}

impl ConnectionHandle {
    pub(crate) fn new(command_tx: CommandPort, task: AbortHandle) -> Self {
        Self {
            inner: Arc::new(ConnectionControl {
                command_tx,
                cancelled: AtomicBool::new(false),
                task: AbortOnDrop::new(task),
            }),
        }
    }

    /// Admits browser-validated text without waiting for network I/O or delivery.
    pub fn send_text(&self, text: String) -> Result<(), SendError> {
        self.send(Command::SendText(text))
    }

    /// Admits browser-validated binary data without waiting for network I/O.
    pub fn send_binary(&self, data: Vec<u8>) -> Result<(), SendError> {
        self.send(Command::SendBinary(data))
    }

    /// Requests graceful closure after previously admitted messages are sent.
    pub fn close(&self, code: Option<u16>, reason: String) -> Result<(), SendError> {
        self.send(Command::Close { code, reason })
    }

    pub(crate) fn synthetic_peer(&self) -> SyntheticPeer {
        SyntheticPeer {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Enqueues a command without waiting for network I/O or event delivery.
    pub(crate) fn send(&self, command: Command) -> Result<(), SendError> {
        if self.inner.cancelled.load(Ordering::Acquire) {
            return Err(SendError::Closed);
        }
        self.inner.command_tx.send(command)
    }

    /// Cancels all work owned by this connection, including blocked delivery.
    ///
    /// Cancellation is idempotent, affects all clones, and does not enqueue a
    /// graceful Close or publish further browser events.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.task.abort();
    }

    /// Whether command admission has ended, independently of JS readyState.
    pub fn is_closed(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire) || self.inner.command_tx.is_closed()
    }
}

/// Why synchronous connection admission failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    Closed,
    CapacityExceeded,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Closed => "WebSocket connection is closed",
            Self::CapacityExceeded => "WebSocket queue capacity exceeded",
        })
    }
}
impl std::error::Error for SendError {}

/// A single decision for a connection paused after its native handshake.
///
/// Decisions apply after the handshake response. Dropping the controller
/// without deciding fails the opening connection. It does not keep the
/// connection alive after the last `ConnectionHandle` is dropped.
#[derive(Debug)]
pub struct HandshakeController {
    decision: oneshot::Sender<HandshakeDecision>,
}

#[derive(Debug)]
pub(crate) enum HandshakeDecision {
    Continue {
        response_status: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    },
    Fail(String),
}

impl HandshakeController {
    pub(crate) fn new(decision: oneshot::Sender<HandshakeDecision>) -> Self {
        Self { decision }
    }

    pub fn continue_open(
        self,
        response_status: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> Result<(), SendError> {
        self.decide(HandshakeDecision::Continue {
            response_status,
            response_headers,
        })
    }

    pub fn fail(self, message: String) -> Result<(), SendError> {
        self.decide(HandshakeDecision::Fail(message))
    }

    fn decide(self, decision: HandshakeDecision) -> Result<(), SendError> {
        let bytes = match &decision {
            HandshakeDecision::Continue {
                response_headers, ..
            } => response_headers
                .as_ref()
                .map(|headers| {
                    headers.iter().fold(0usize, |sum, (name, value)| {
                        sum.saturating_add(name.len())
                            .saturating_add(value.len())
                            .saturating_add(4)
                    })
                })
                .unwrap_or(0),
            HandshakeDecision::Fail(message) => message.len(),
        };
        if bytes > MAX_CONTROL_BYTES {
            let _ = self.decision.send(HandshakeDecision::Fail(
                "WebSocket handshake decision capacity exceeded".to_owned(),
            ));
            return Err(SendError::CapacityExceeded);
        }
        self.decision.send(decision).map_err(|_| SendError::Closed)
    }
}

/// The injecting peer of a synthetic connection. Only synthetic creation
/// returns this capability; retaining it does not keep the client alive.
#[derive(Clone, Debug)]
pub struct SyntheticPeer {
    inner: Weak<ConnectionControl>,
}

impl SyntheticPeer {
    pub fn send_text(&self, text: String) -> Result<(), SendError> {
        self.send(Command::ReceiveText(text))
    }
    pub fn send_binary(&self, data: Vec<u8>) -> Result<(), SendError> {
        self.send(Command::ReceiveBinary(data))
    }
    pub fn close(&self, code: Option<u16>, reason: String) -> Result<(), SendError> {
        self.send(Command::ServerClose { code, reason })
    }
    fn send(&self, command: Command) -> Result<(), SendError> {
        let inner = self.inner.upgrade().ok_or(SendError::Closed)?;
        ConnectionHandle { inner }.send(command)
    }
}

/// Child tasks must not outlive the connection task when it is cancelled.
#[derive(Debug)]
pub(crate) struct AbortOnDrop(AbortHandle);

impl AbortOnDrop {
    pub(crate) fn new(task: AbortHandle) -> Self {
        Self(task)
    }

    fn abort(&self) {
        self.0.abort();
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.abort();
    }
}
