//! Bounded synchronous admission. Reservations live until native completion,
//! rejection or cancellation, rather than being released at channel dequeue.
use crate::{Command, CommandSendError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc};

pub(crate) const MAX_QUEUED_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_QUEUED_MESSAGES: usize = 256;
const MAX_CONTROL_BYTES: usize = 64 * 1024;

#[derive(Debug)]
struct Admission {
    bytes: Arc<Semaphore>,
    messages: Arc<Semaphore>,
    controls: Arc<Semaphore>,
    close_queued: AtomicBool,
    failed: AtomicBool,
    failure: Notify,
}

#[derive(Debug)]
pub(crate) struct Reservation {
    _permits: Vec<OwnedSemaphorePermit>,
}

pub(crate) struct QueuedCommand {
    pub command: Command,
    pub reservation: Reservation,
}

#[derive(Debug)]
pub(crate) struct CommandPort {
    tx: mpsc::UnboundedSender<QueuedCommand>,
    admission: Arc<Admission>,
}

pub(crate) struct CommandReceiver {
    rx: mpsc::UnboundedReceiver<QueuedCommand>,
    admission: Arc<Admission>,
    failure_delivered: bool,
}

pub(crate) fn command_channel() -> (CommandPort, CommandReceiver) {
    let (tx, rx) = mpsc::unbounded_channel();
    let admission = Arc::new(Admission {
        bytes: Arc::new(Semaphore::new(MAX_QUEUED_BYTES)),
        messages: Arc::new(Semaphore::new(MAX_QUEUED_MESSAGES)),
        controls: Arc::new(Semaphore::new(8)),
        close_queued: AtomicBool::new(false),
        failed: AtomicBool::new(false),
        failure: Notify::new(),
    });
    (
        CommandPort {
            tx,
            admission: admission.clone(),
        },
        CommandReceiver {
            rx,
            admission,
            failure_delivered: false,
        },
    )
}

impl CommandPort {
    pub fn send(&self, command: Command) -> Result<(), CommandSendError> {
        if self.is_closed() {
            return Err(CommandSendError(command));
        }
        if matches!(command, Command::Close { .. })
            && self.admission.close_queued.load(Ordering::Acquire)
        {
            return Ok(());
        }
        let reservation = match self.reserve(&command) {
            Some(reservation) => reservation,
            None => {
                self.admission.failed.store(true, Ordering::Release);
                self.admission.failure.notify_one();
                return Err(CommandSendError(command));
            }
        };
        if matches!(command, Command::Close { .. })
            && self.admission.close_queued.swap(true, Ordering::AcqRel)
        {
            return Ok(());
        }
        self.tx
            .send(QueuedCommand {
                command,
                reservation,
            })
            .map_err(|error| CommandSendError(error.0.command))
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed() || self.admission.failed.load(Ordering::Acquire)
    }

    fn reserve(&self, command: &Command) -> Option<Reservation> {
        let bytes = match command {
            Command::SendText(text) | Command::ReceiveText(text) => Some(text.len()),
            Command::SendBinary(data) | Command::ReceiveBinary(data) => Some(data.len()),
            _ => None,
        };
        let mut permits = Vec::with_capacity(2);
        if let Some(bytes) = bytes {
            if bytes > MAX_QUEUED_BYTES {
                return None;
            }
            permits.push(self.admission.messages.clone().try_acquire_owned().ok()?);
            permits.push(
                self.admission
                    .bytes
                    .clone()
                    .try_acquire_many_owned(bytes as u32)
                    .ok()?,
            );
        } else {
            let size = match command {
                Command::Close { reason, .. }
                | Command::ServerClose { reason, .. }
                | Command::FailOpen(reason) => reason.len(),
                Command::ContinueOpen {
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
                _ => unreachable!(),
            };
            if size > MAX_CONTROL_BYTES {
                return None;
            }
            // One Close always has admission even when data/control quotas are full.
            if !matches!(command, Command::Close { .. }) {
                permits.push(self.admission.controls.clone().try_acquire_owned().ok()?);
            }
        }
        Some(Reservation { _permits: permits })
    }
}

impl CommandReceiver {
    pub async fn recv(&mut self) -> Option<QueuedCommand> {
        loop {
            if !self.failure_delivered && self.admission.failed.load(Ordering::Acquire) {
                self.failure_delivered = true;
                return Some(QueuedCommand {
                    command: Command::FailOpen("WebSocket send queue capacity exceeded".to_owned()),
                    reservation: Reservation {
                        _permits: Vec::new(),
                    },
                });
            }
            tokio::select! {
                biased;
                _ = self.admission.failure.notified(), if !self.failure_delivered => {}
                command = self.rx.recv() => return command,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dequeued_messages_keep_admission_until_completion() {
        let (port, mut receiver) = command_channel();
        for _ in 0..MAX_QUEUED_MESSAGES {
            port.send(Command::SendText(String::new())).unwrap();
        }
        let pending_native_send = receiver.recv().await.unwrap();
        assert!(
            port.send(Command::SendText(String::new())).is_err(),
            "dequeue must not release the zero-length message count"
        );
        assert!(
            matches!(receiver.recv().await.unwrap().command, Command::FailOpen(_)),
            "overflow is delivered even when the data queue is full"
        );
        drop(pending_native_send);
    }

    #[tokio::test]
    async fn completion_releases_capacity_and_close_has_independent_admission() {
        let (port, mut receiver) = command_channel();
        for _ in 0..MAX_QUEUED_MESSAGES {
            port.send(Command::SendBinary(vec![7])).unwrap();
        }
        drop(receiver.recv().await.unwrap());
        port.send(Command::SendBinary(vec![8])).unwrap();
        port.send(Command::Close {
            code: Some(1000),
            reason: String::new(),
        })
        .unwrap();
        for _ in 0..MAX_QUEUED_MESSAGES {
            assert!(matches!(
                receiver.recv().await.unwrap().command,
                Command::SendBinary(_)
            ));
        }
        assert!(
            matches!(
                receiver.recv().await.unwrap().command,
                Command::Close { .. }
            ),
            "graceful close follows all previously admitted data"
        );
    }
}
