//! Persistent native WebSocket sessions, owned by a separate libcurl multi thread.
//!
//! This layer transports frames. Browser handshake policy, message assembly and
//! close-handshake semantics belong to the caller. Dropping the receiver cancels
//! its session, including DNS and handshake work, independently of queue capacity.
//!
//! Internally, the shared runtime drives DNS and scheduling; session owns the native
//! handle and its Opening/Open/ReceivedClose lifecycle; scheduling holds I/O
//! admission state and maps polled sockets back to their sessions. Returning
//! AGAIN parks that I/O until its socket is signalled. Application wakeups only
//! resume paused work, such as a new frame or restored receive capacity.

mod connection_pool;
mod diagnostics;
pub(crate) mod registry;
mod request;
mod scheduling;
mod session;
#[cfg(test)]
mod tests;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use curl::multi::MultiWaker;
use parking_lot::Mutex;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};

use crate::{
    CurlDnsResolution, CurlMultiRuntime, CurlMultiRuntimeConfig, CurlTlsConfig, CurlTransferId,
    runtime::identity::next_transfer_id,
};
pub use curl::easy::{WsFlags, WsFrame};

/// Fragment large messages above this layer to bound native write residence.
pub const MAX_SEND_FRAME_BYTES: usize = 64 * 1024;
const MAX_PENDING_EVENTS: usize = 8;
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
    /// Decoded payload from libcurl with raw mode and automatic Pong disabled.
    /// Invalid opcodes/RSV, masking, fragmentation and control sizes fail at the
    /// native decoder. Each chunk belongs to one frame: len equals data.len(),
    /// offsets advance from zero, and bytes_left counts the remaining payload.
    /// Empty frames yield an empty chunk. Continuations retain TEXT/BINARY;
    /// CONT marks every non-final fragment, including empty ones.
    /// UTF-8, Close contents and application size limits belong to the caller.
    Chunk { data: Vec<u8>, frame: WsFrame },
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
    send: Mutex<Option<PendingSend>>,
    terminal: Mutex<Option<std::result::Result<(), String>>>,
    waker: MultiWaker,
    #[cfg(test)]
    read_blocked: tokio::sync::Notify,
    #[cfg(test)]
    write_blocked: tokio::sync::Notify,
    #[cfg(test)]
    read_attempts: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    receive_allocations: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    read_waiting: tokio::sync::Notify,
    #[cfg(test)]
    owner_thread: Mutex<Option<std::thread::ThreadId>>,
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
struct PendingSend {
    frame: CurlWebSocketSend,
    offset: usize,
    completed: oneshot::Sender<usize>,
}

/// Cloneable write/control capability; it does not keep a dropped receiver alive.
#[derive(Clone, Debug)]
pub struct CurlWebSocketSender {
    control: Arc<Control>,
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
        // Settle the pending write before closing the receive event channel.
        self.control.send.lock().take();
    }
}

pub(crate) struct Submission {
    id: CurlTransferId,
    request: CurlWebSocketRequest,
    io: SessionIo,
}

/// Admission capability for one native owner. Clones do not keep that owner
/// alive; shutting down its runtime closes connections and rejects new ones.
#[derive(Clone, Debug)]
pub struct CurlWebSocketConnector {
    inner: Arc<ConnectorInner>,
}

#[derive(Debug)]
struct ConnectorInner {
    submissions: crossbeam_channel::Sender<Submission>,
    slots: Arc<Semaphore>,
    waker: MultiWaker,
    shutdown: Arc<AtomicBool>,
}

impl CurlWebSocketConnector {
    pub(crate) fn channel() -> (
        crossbeam_channel::Sender<Submission>,
        crossbeam_channel::Receiver<Submission>,
    ) {
        crossbeam_channel::bounded(SESSION_CAPACITY)
    }

    pub(crate) fn new(
        submissions: crossbeam_channel::Sender<Submission>,
        waker: MultiWaker,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            inner: Arc::new(ConnectorInner {
                submissions,
                slots: Arc::new(Semaphore::new(SESSION_CAPACITY)),
                waker,
                shutdown,
            }),
        }
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
            send: Mutex::new(None),
            terminal: Mutex::new(None),
            waker: self.inner.waker.clone(),
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
        });
        let (event_tx, events) = mpsc::channel(MAX_PENDING_EVENTS);
        let io = SessionIo {
            events: event_tx,
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
            sender: CurlWebSocketSender { control },
            events,
            terminal_delivered: false,
        })
    }
}

/// Standalone owner for callers without an existing HTTP runtime. Uses the
/// same Multi driver as HTTP, with no HTTP submissions.
#[derive(Debug)]
pub struct CurlWebSocketRuntime {
    runtime: CurlMultiRuntime<StandaloneHandler, ()>,
}

#[derive(Debug)]
struct StandaloneHandler;
impl curl::easy::Handler for StandaloneHandler {}

impl CurlWebSocketRuntime {
    pub fn new() -> Result<Self> {
        let (runtime, _) = CurlMultiRuntime::new(CurlMultiRuntimeConfig {
            thread_name: "moli-curl-websocket".to_owned(),
            poll_interval: Duration::from_secs(1),
            ..CurlMultiRuntimeConfig::default()
        })?;
        Ok(Self { runtime })
    }

    pub fn connector(&self) -> CurlWebSocketConnector {
        self.runtime.websocket_connector()
    }

    pub fn connect(&self, request: CurlWebSocketRequest) -> Result<CurlWebSocketConnection> {
        self.connector().connect(request)
    }
}
