use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::{sync::mpsc, task::AbortHandle};

use crate::Command;

/// The producer's ownership of a WebSocket connection.
///
/// Clones keep the connection alive. Dropping the last handle or calling
/// `cancel` releases the connection even while its handshake, write, or event
/// sink is waiting. Use `Command::Close` for the graceful closing handshake.
#[derive(Clone, Debug)]
pub struct ConnectionHandle {
    inner: Arc<ConnectionControl>,
}

#[derive(Debug)]
struct ConnectionControl {
    command_tx: mpsc::UnboundedSender<Command>,
    cancelled: AtomicBool,
    task: AbortOnDrop,
}

impl ConnectionHandle {
    pub(crate) fn new(command_tx: mpsc::UnboundedSender<Command>, task: AbortHandle) -> Self {
        Self {
            inner: Arc::new(ConnectionControl {
                command_tx,
                cancelled: AtomicBool::new(false),
                task: AbortOnDrop::new(task),
            }),
        }
    }

    /// Enqueues a command without waiting for network I/O or event delivery.
    pub fn send(&self, command: Command) -> Result<(), CommandSendError> {
        if self.inner.cancelled.load(Ordering::Acquire) {
            return Err(CommandSendError(command));
        }
        self.inner
            .command_tx
            .send(command)
            .map_err(|error| CommandSendError(error.0))
    }

    /// Cancels all work owned by this connection, including blocked delivery.
    ///
    /// Cancellation is idempotent, affects all clones, and does not enqueue a
    /// graceful Close or publish further browser events.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.task.abort();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire) || self.inner.command_tx.is_closed()
    }
}

/// A command rejected because the connection is no longer available.
#[derive(Debug)]
pub struct CommandSendError(pub Command);

impl std::fmt::Display for CommandSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebSocket connection is closed")
    }
}

impl std::error::Error for CommandSendError {}

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
