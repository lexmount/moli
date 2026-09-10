//! The caller's receive/write handles and the owner's I/O endpoint share one
//! lifetime. Pending send completion stays independent of received event capacity.

use super::{CurlWebSocketEvent, CurlWebSocketSend, MAX_PENDING_EVENTS};
use crate::CurlTransferId;
use anyhow::{Context, Result, bail};
use curl::multi::MultiWaker;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{OwnedSemaphorePermit, mpsc, oneshot};

#[derive(Debug)]
pub(super) struct Control {
    cancelled: AtomicBool,
    closed: AtomicBool,
    pub(super) reading: AtomicBool,
    pub(super) send: Mutex<Option<PendingSend>>,
    terminal: Mutex<Option<std::result::Result<(), String>>>,
    waker: MultiWaker,
    #[cfg(test)]
    pub(super) read_blocked: tokio::sync::Notify,
    #[cfg(test)]
    pub(super) write_blocked: tokio::sync::Notify,
    #[cfg(test)]
    pub(super) read_attempts: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub(super) receive_allocations: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub(super) read_waiting: tokio::sync::Notify,
    #[cfg(test)]
    pub(super) owner_thread: Mutex<Option<std::thread::ThreadId>>,
    #[cfg(test)]
    pub(super) pool_waiting: Arc<tokio::sync::Notify>,
}

impl Control {
    fn wake(&self) {
        let _ = self.waker.wakeup();
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.wake();
    }
}

#[derive(Debug)]
pub(super) struct PendingSend {
    pub(super) frame: CurlWebSocketSend,
    pub(super) offset: usize,
    pub(super) completed: oneshot::Sender<usize>,
}

/// Cloneable write/control capability; it does not keep a dropped receiver alive.
#[derive(Clone, Debug)]
pub struct CurlWebSocketSender {
    pub(super) control: Arc<Control>,
}

impl CurlWebSocketSender {
    /// Writes one frame and returns its payload size once libcurl consumes it.
    ///
    /// Only one frame may be pending per connection; overlapping sends return an
    /// error. The caller owns data/control ordering and any message queue.
    /// Completion is independent of reads and received event delivery.
    ///
    /// Dropping an unpolled future submits nothing. Once submitted, the frame
    /// remains pending until written or the connection is cancelled/closed, even
    /// if this future is dropped. Transport failure or cancellation returns an error.
    pub async fn send_frame(&self, frame: CurlWebSocketSend) -> Result<usize> {
        frame.validate()?;
        let completion = {
            let mut send = self.control.send.lock();
            if self.control.closed.load(Ordering::Acquire)
                || self.control.cancelled.load(Ordering::Acquire)
            {
                bail!("WebSocket transport is closed");
            }
            if send.is_some() {
                bail!("WebSocket frame is already pending");
            }
            let (completed, completion) = oneshot::channel();
            *send = Some(PendingSend {
                frame,
                offset: 0,
                completed,
            });
            completion
        };
        self.control.wake();
        completion
            .await
            .context("WebSocket transport closed before frame completion")
    }

    /// Handshake delivery starts paused, so application data cannot outrun Open.
    /// Pausing does not retract queued chunks or a read already in progress.
    /// Send completions progress independently of reads and received events.
    pub fn set_reading(&self, enabled: bool) {
        self.control.reading.store(enabled, Ordering::Release);
        self.control.wake();
    }

    pub fn cancel(&self) {
        self.control.cancel();
    }
}

#[derive(Debug)]
pub struct CurlWebSocketConnection {
    id: CurlTransferId,
    sender: CurlWebSocketSender,
    pub(super) events: mpsc::Receiver<CurlWebSocketEvent>,
    terminal_delivered: bool,
}

impl CurlWebSocketConnection {
    pub(super) fn channel(
        id: CurlTransferId,
        waker: MultiWaker,
        slot: OwnedSemaphorePermit,
    ) -> (Self, SessionIo) {
        let control = Arc::new(Control {
            cancelled: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            reading: AtomicBool::new(false),
            send: Mutex::new(None),
            terminal: Mutex::new(None),
            waker,
            #[cfg(test)]
            read_blocked: tokio::sync::Notify::new(),
            #[cfg(test)]
            write_blocked: tokio::sync::Notify::new(),
            #[cfg(test)]
            read_attempts: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            receive_allocations: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            read_waiting: tokio::sync::Notify::new(),
            #[cfg(test)]
            owner_thread: Mutex::new(None),
            #[cfg(test)]
            pool_waiting: Arc::new(tokio::sync::Notify::new()),
        });
        let (event_tx, events) = mpsc::channel(MAX_PENDING_EVENTS);
        let io = SessionIo {
            events: event_tx,
            control: control.clone(),
            _slot: slot,
        };
        (
            Self {
                id,
                sender: CurlWebSocketSender { control },
                events,
                terminal_delivered: false,
            },
            io,
        )
    }

    pub fn id(&self) -> CurlTransferId {
        self.id
    }
    pub fn sender(&self) -> CurlWebSocketSender {
        self.sender.clone()
    }

    pub async fn recv(&mut self) -> Option<CurlWebSocketEvent> {
        if let Some(event) = self.events.recv().await {
            self.sender.control.wake();
            return Some(event);
        }
        if self.terminal_delivered {
            return None;
        }
        self.terminal_delivered = true;
        let result = self
            .sender
            .control
            .terminal
            .lock()
            .take()
            .unwrap_or_else(|| Err("WebSocket runtime stopped".to_owned()));
        Some(CurlWebSocketEvent::Closed { result })
    }
}

impl Drop for CurlWebSocketConnection {
    fn drop(&mut self) {
        self.sender.cancel();
    }
}

pub(super) struct SessionIo {
    // Release admission before closing the event channel publishes the terminal.
    _slot: OwnedSemaphorePermit,
    pub(super) events: mpsc::Sender<CurlWebSocketEvent>,
    pub(super) control: Arc<Control>,
}

impl SessionIo {
    pub(super) fn cancelled(&self) -> bool {
        self.control.cancelled.load(Ordering::Acquire)
    }
    pub(super) fn finish(&self, result: std::result::Result<(), String>) {
        *self.control.terminal.lock() = Some(result);
    }
}

impl Drop for SessionIo {
    fn drop(&mut self) {
        self.control.closed.store(true, Ordering::Release);
        // Settle the pending write before closing the receive event channel.
        self.control.send.lock().take();
    }
}
