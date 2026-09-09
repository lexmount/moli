//! Persistent native WebSocket sessions, owned by a separate libcurl multi thread.
//!
//! This layer transports frames. Browser handshake policy, message assembly and
//! close-handshake semantics belong to the caller. Dropping the receiver cancels
//! its session, including DNS and handshake work, independently of queue capacity.

mod owner;
mod request;
#[cfg(test)]
mod tests;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use curl::multi::MultiWaker;
use parking_lot::Mutex;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};

use crate::{
    CurlDnsResolution, CurlTlsConfig, CurlTransferId, runtime::identity::next_transfer_id,
};
pub use curl::easy::{WsFlags, WsFrame};

/// Fragment large messages above this layer to bound native write residence.
pub const MAX_SEND_FRAME_BYTES: usize = 64 * 1024;
const MAX_PENDING_EVENTS: usize = 8;
const DATA_CAPACITY: usize = 8;
const CONTROL_CAPACITY: usize = 4;
const SESSION_CAPACITY: usize = 255;

#[derive(Debug)]
pub struct CurlWebSocketRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// An already resolved proxy policy. None explicitly disables environment proxies.
    pub proxy: Option<String>,
    pub proxy_headers: Vec<(String, String)>,
    pub tls: CurlTlsConfig,
    pub dns_resolution: CurlDnsResolution,
    pub handshake_timeout: Duration,
}

impl CurlWebSocketRequest {
    pub fn new(url: String) -> Self {
        Self {
            url,
            headers: Vec::new(),
            proxy: None,
            proxy_headers: Vec::new(),
            tls: CurlTlsConfig::default(),
            dns_resolution: CurlDnsResolution::curl_managed(),
            handshake_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub enum CurlWebSocketEvent {
    Handshake {
        /// Actual outgoing GET request, excluding proxy CONNECT headers.
        request: Vec<u8>,
        /// Final HTTP response block, preserving duplicate headers.
        response: Vec<u8>,
        result: std::result::Result<(), String>,
    },
    Chunk {
        data: Vec<u8>,
        frame: WsFrame,
    },
    /// Emitted once, after all previously admitted events. Ok means TCP EOF;
    /// it does not assert that a WebSocket close handshake was completed.
    Closed {
        result: std::result::Result<(), String>,
    },
}

#[derive(Debug)]
pub struct CurlWebSocketSend {
    pub flags: WsFlags,
    pub data: Vec<u8>,
}

impl CurlWebSocketSend {
    fn is_control(&self) -> bool {
        [WsFlags::PING, WsFlags::PONG, WsFlags::CLOSE].contains(&self.flags)
    }

    fn validate(&self) -> Result<()> {
        let data_flags = WsFlags::from_bits(self.flags.bits() & !WsFlags::CONT.bits());
        if !self.is_control() && data_flags != WsFlags::TEXT && data_flags != WsFlags::BINARY {
            bail!("invalid WebSocket send flags");
        }
        let limit = if self.is_control() {
            125
        } else {
            MAX_SEND_FRAME_BYTES
        };
        if self.data.len() > limit {
            bail!("WebSocket send frame exceeds {limit} bytes");
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Control {
    cancelled: AtomicBool,
    closed: AtomicBool,
    reading: AtomicBool,
    data_slots: Arc<Semaphore>,
    control_slots: Arc<Semaphore>,
    terminal: Mutex<Option<std::result::Result<(), String>>>,
    waker: MultiWaker,
    #[cfg(test)]
    read_blocked: tokio::sync::Notify,
    #[cfg(test)]
    write_blocked: tokio::sync::Notify,
}

impl Control {
    fn wake(&self) {
        let _ = self.waker.wakeup();
    }
    fn close_admission(&self) {
        self.data_slots.close();
        self.control_slots.close();
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.close_admission();
        self.wake();
    }
}

/// Completion of one admitted frame, independent of the receive event queue.
///
/// A retained receipt holds one send slot until it is consumed or dropped.
/// The native write retains the same slot until the frame completes or fails.
#[derive(Debug)]
pub struct CurlWebSocketSendReceipt {
    completion: oneshot::Receiver<usize>,
    _reservation: Arc<OwnedSemaphorePermit>,
}

impl CurlWebSocketSendReceipt {
    /// Waits until libcurl consumes the full frame payload and returns its size.
    /// Transport failure or cancellation before completion returns an error.
    pub async fn wait(self) -> Result<usize> {
        self.completion
            .await
            .context("WebSocket transport closed before frame completion")
    }
}

#[derive(Debug)]
struct QueuedSend {
    frame: CurlWebSocketSend,
    completed: oneshot::Sender<usize>,
    _reservation: Arc<OwnedSemaphorePermit>,
}

/// Cloneable write/control capability; it does not keep a dropped receiver alive.
#[derive(Clone, Debug)]
pub struct CurlWebSocketSender {
    data: mpsc::Sender<QueuedSend>,
    control_frames: mpsc::Sender<QueuedSend>,
    control: Arc<Control>,
}

impl CurlWebSocketSender {
    /// Admits one frame, returning a separate native completion receipt.
    ///
    /// Admission waits for a bounded slot covering queued frames, native writes
    /// and unconsumed receipts. Cancelling this future before it returns does
    /// not submit a frame. Dropping its receipt does not cancel an admitted frame.
    pub async fn enqueue_frame(
        &self,
        frame: CurlWebSocketSend,
    ) -> Result<CurlWebSocketSendReceipt> {
        frame.validate()?;
        let (queue, slots) = if frame.is_control() {
            (&self.control_frames, &self.control.control_slots)
        } else {
            (&self.data, &self.control.data_slots)
        };
        let reservation = Arc::new(
            slots
                .clone()
                .acquire_owned()
                .await
                .context("WebSocket transport is closed")?,
        );
        if self.control.closed.load(Ordering::Acquire)
            || self.control.cancelled.load(Ordering::Acquire)
        {
            bail!("WebSocket transport is closed");
        }
        let (completed, completion) = oneshot::channel();
        // A slot also covers the queued frame, so the queue cannot be full here.
        queue
            .try_send(QueuedSend {
                frame,
                completed,
                _reservation: reservation.clone(),
            })
            .context("WebSocket transport cannot accept a frame")?;
        self.control.wake();
        Ok(CurlWebSocketSendReceipt {
            completion,
            _reservation: reservation,
        })
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
    events: mpsc::Receiver<CurlWebSocketEvent>,
    terminal_delivered: bool,
}

impl CurlWebSocketConnection {
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

struct SessionIo {
    events: mpsc::Sender<CurlWebSocketEvent>,
    data: mpsc::Receiver<QueuedSend>,
    control_frames: mpsc::Receiver<QueuedSend>,
    control: Arc<Control>,
    _slot: OwnedSemaphorePermit,
}

impl SessionIo {
    fn cancelled(&self) -> bool {
        self.control.cancelled.load(Ordering::Acquire)
    }
    fn finish(&self, result: std::result::Result<(), String>) {
        *self.control.terminal.lock() = Some(result);
    }
}

impl Drop for SessionIo {
    fn drop(&mut self) {
        self.control.closed.store(true, Ordering::Release);
        self.control.close_admission();
    }
}

struct Submission {
    id: CurlTransferId,
    request: CurlWebSocketRequest,
    io: SessionIo,
}

#[derive(Clone, Debug)]
pub struct CurlWebSocketRuntime {
    inner: Arc<RuntimeInner>,
}

#[derive(Debug)]
struct RuntimeInner {
    submissions: crossbeam_channel::Sender<Submission>,
    slots: Arc<Semaphore>,
    waker: MultiWaker,
    shutdown: Arc<AtomicBool>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl CurlWebSocketRuntime {
    pub fn new() -> Result<Self> {
        let (submissions, rx) = crossbeam_channel::bounded(SESSION_CAPACITY);
        let (waker_tx, waker_rx) = crossbeam_channel::bounded(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let owner_shutdown = shutdown.clone();
        let thread = thread::Builder::new()
            .name("moli-curl-websocket".to_owned())
            .spawn(move || owner::run(rx, waker_tx, owner_shutdown))
            .context("failed to start curl WebSocket owner")?;
        let waker = waker_rx
            .recv()
            .context("curl WebSocket owner failed to start")?;
        Ok(Self {
            inner: Arc::new(RuntimeInner {
                submissions,
                slots: Arc::new(Semaphore::new(SESSION_CAPACITY)),
                waker,
                shutdown,
                thread: Mutex::new(Some(thread)),
            }),
        })
    }

    pub fn connect(&self, request: CurlWebSocketRequest) -> Result<CurlWebSocketConnection> {
        if self.inner.shutdown.load(Ordering::Acquire) {
            bail!("curl WebSocket runtime is closed");
        }
        let slot = self
            .inner
            .slots
            .clone()
            .try_acquire_owned()
            .context("too many curl WebSocket sessions")?;
        let id = next_transfer_id()?;
        let control = Arc::new(Control {
            cancelled: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            reading: AtomicBool::new(false),
            data_slots: Arc::new(Semaphore::new(DATA_CAPACITY)),
            control_slots: Arc::new(Semaphore::new(CONTROL_CAPACITY)),
            terminal: Mutex::new(None),
            waker: self.inner.waker.clone(),
            #[cfg(test)]
            read_blocked: tokio::sync::Notify::new(),
            #[cfg(test)]
            write_blocked: tokio::sync::Notify::new(),
        });
        let (event_tx, events) = mpsc::channel(MAX_PENDING_EVENTS);
        let (data_tx, data) = mpsc::channel(DATA_CAPACITY);
        let (control_tx, control_frames) = mpsc::channel(CONTROL_CAPACITY);
        let io = SessionIo {
            events: event_tx,
            data,
            control_frames,
            control: control.clone(),
            _slot: slot,
        };
        self.inner
            .submissions
            .try_send(Submission { id, request, io })
            .map_err(|_| anyhow::anyhow!("curl WebSocket runtime cannot accept a session"))?;
        control.wake();
        Ok(CurlWebSocketConnection {
            id,
            sender: CurlWebSocketSender {
                data: data_tx,
                control_frames: control_tx,
                control,
            },
            events,
            terminal_delivered: false,
        })
    }
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.waker.wakeup();
        if let Some(thread) = self.thread.get_mut().take() {
            let _ = thread.join();
        }
    }
}
