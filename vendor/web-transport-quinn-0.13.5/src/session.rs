use crate::error::CloseReason;
use std::{
    fmt,
    future::{poll_fn, Future},
    io::Cursor,
    ops::Deref,
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
    task::{Context, Poll, Waker},
    time::Duration,
};

use bytes::{Bytes, BytesMut};
use futures::stream::{FuturesUnordered, Stream, StreamExt};
use kio::{Fan, Waiter};

use crate::{
    proto::{ConnectRequest, ConnectResponse, Frame, StreamUni, VarInt},
    waiters::Parked,
    ClientError, Connected, RecvStream, SendStream, SessionError, Settings, WebTransportError,
};

const H3_MESSAGE_ERROR: u32 = 0x10e;

/// The ALPN the QUIC handshake negotiated, or `None` if there was none, the handshake
/// has not completed, or it is not valid UTF-8.
///
/// Quinn only exposes the ALPN via a downcast of the boxed handshake data, so the result
/// is owned rather than borrowed from the connection.
#[cfg(any(feature = "aws-lc-rs", feature = "ring"))]
fn negotiated_alpn(conn: &quinn::Connection) -> Option<String> {
    let data = conn.handshake_data()?;
    let data = data
        .downcast::<quinn::crypto::rustls::HandshakeData>()
        .ok()?;

    String::from_utf8(data.protocol?).ok()
}

/// `quinn::crypto::rustls` only exists once a TLS backend is enabled, so there is no
/// handshake data type to name here.
///
/// This crate's own builders are gated the same way, so normally there is no connection
/// to read an ALPN from either. A caller that enables a provider on `quinn` directly and
/// brings its own endpoint can still establish one, and [`Session::protocol`] reports
/// `None` for those raw sessions. Enable this crate's `aws-lc-rs` or `ring` feature to
/// get the ALPN.
#[cfg(not(any(feature = "aws-lc-rs", feature = "ring")))]
fn negotiated_alpn(_conn: &quinn::Connection) -> Option<String> {
    None
}

/// An established WebTransport session, acting like a full QUIC connection. See [`quinn::Connection`].
///
/// It is important to remember that WebTransport is layered on top of QUIC:
///   1. Each stream starts with a few bytes identifying the stream type and session ID.
///   2. Errors codes are encoded with the session ID, so they aren't full QUIC error codes.
///   3. Stream IDs may have gaps in them, used by HTTP/3 transparant to the application.
///
/// Deref is used to expose non-overloaded methods on [`quinn::Connection`].
/// These should be safe to use with WebTransport, but file a PR if you find one that isn't.
#[derive(Clone)]
pub struct Session {
    conn: quinn::Connection,

    // The session ID, as determined by the stream ID of the connect request.
    session_id: Option<VarInt>,

    // The accept logic is stateful, so use an Arc<Mutex> to share it.
    accept: Option<Arc<Mutex<SessionAccept>>>,

    // Cache the headers in front of each stream we open.
    header_uni: Bytes,
    header_bi: Bytes,
    header_datagram: Bytes,

    // Keep a reference to the settings and connect stream to avoid closing them until dropped.
    #[allow(dead_code)]
    settings: Option<Arc<Settings>>,

    // The send side of the CONNECT stream, used to write the CloseWebTransportSession capsule.
    // Wrapped in Arc<Mutex<Option<...>>> so close() can take it exactly once.
    connect_send: Arc<Mutex<Option<quinn::SendStream>>>,

    // Session error, set once by either local close() or the background task
    // when a remote CloseWebTransportSession capsule is received.
    // Uses OnceLock for set-once, first-writer-wins semantics with lock-free reads.
    error: Arc<CloseReason>,

    // The request sent by the client, or None for a raw QUIC session.
    request: Option<ConnectRequest>,

    // The response sent by the server, or None for a raw QUIC session.
    response: Option<ConnectResponse>,

    // The ALPN negotiated by the QUIC handshake, for raw QUIC sessions. Quinn only
    // exposes it behind an allocating downcast, so `protocol()` has nothing in the
    // connection to borrow from and needs somewhere to keep the result.
    //
    // Resolved on first read rather than at construction: `raw()` accepts any
    // connection, including one from `into_0rtt()` whose handshake has not completed
    // and so has no ALPN yet. Only a successful read is latched, so an early call
    // cannot pin `None` for the life of the session. Shared across clones.
    //
    // Unused by HTTP/3 sessions, which read the subprotocol out of the response.
    alpn: Arc<OnceLock<String>>,

    // Quinn exposes these only as futures, and those futures hold the `Notify`
    // registration that will wake us — rebuilding one per poll would drop the
    // registration and we would never be woken. Each is per-clone, so cloning the
    // session is what gives you concurrent operations.
    //
    // Only used by `Session::raw`, where there is no `SessionAccept` to forward to
    // and Quinn's own accept futures hold the registration.
    op_accept_uni: crate::op::Op<Result<RecvStream, SessionError>>,
    op_accept_bi: crate::op::Op<Result<(SendStream, RecvStream), SessionError>>,
    op_open_uni: crate::op::Op<Result<SendStream, SessionError>>,
    op_open_bi: crate::op::Op<Result<(SendStream, RecvStream), SessionError>>,
    op_send_datagram: crate::op::Op<Result<(), SessionError>>,
    op_recv_datagram: crate::op::Op<Result<Bytes, SessionError>>,
    op_closed: crate::op::Op<SessionError>,

    // `SessionAccept` holds its waiters weakly, so the handle this clone registered
    // with has to outlive the poll that made it. Per-clone, for the same reason as the
    // ops above.
    parked_accept_uni: Parked,
    parked_accept_bi: Parked,
}

impl Session {
    pub(crate) fn new(conn: quinn::Connection, settings: Settings, connect: Connected) -> Self {
        // The session ID is the stream ID of the CONNECT request.
        let session_id = connect.session_id();

        // Cache the tiny header we write in front of each stream we open.
        let mut header_uni = Vec::new();
        StreamUni::WEBTRANSPORT.encode(&mut header_uni);
        session_id.encode(&mut header_uni);

        let mut header_bi = Vec::new();
        Frame::WEBTRANSPORT.encode(&mut header_bi);
        session_id.encode(&mut header_bi);

        let mut header_datagram = Vec::new();
        session_id.encode(&mut header_datagram);

        let error = Arc::new(CloseReason::http3());

        // Accept logic is stateful, so use an Arc<Mutex> to share it.
        let accept = SessionAccept::new(conn.clone(), session_id, error.clone());

        let this = Self {
            conn,
            accept: Some(Arc::new(Mutex::new(accept))),
            session_id: Some(session_id),
            header_uni: header_uni.into(),
            header_bi: header_bi.into(),
            header_datagram: header_datagram.into(),
            settings: Some(Arc::new(settings)),
            connect_send: Arc::new(Mutex::new(Some(connect.send))),
            error: error.clone(),
            request: Some(connect.request.clone()),
            response: Some(connect.response.clone()),
            alpn: Default::default(),
            op_accept_uni: Default::default(),
            op_accept_bi: Default::default(),
            op_open_uni: Default::default(),
            op_open_bi: Default::default(),
            op_send_datagram: Default::default(),
            op_recv_datagram: Default::default(),
            op_closed: Default::default(),
            parked_accept_uni: Default::default(),
            parked_accept_bi: Default::default(),
        };

        // Run a background task to read capsules from the CONNECT recv stream, on whichever
        // runtime quinn detects, as quinn's own Endpoint::client and Endpoint::server do.
        let runtime = quinn::default_runtime().expect("no async runtime found");
        let conn2 = this.conn.clone();
        runtime.spawn(Box::pin(Self::run_recv(conn2, connect.recv, error)));

        this
    }

    // Read capsules from the CONNECT recv stream until it's closed,
    // then record the close error and tear down the connection.
    async fn run_recv(conn: quinn::Connection, recv: quinn::RecvStream, error: Arc<CloseReason>) {
        let (cause, code, reason) = match Self::read_capsules(recv).await {
            Ok(info) => {
                // A clean CONNECT FIN is the default remote close, including
                // when no explicit WT_CLOSE_SESSION capsule preceded it.
                let (code, reason) = info.unwrap_or_default();
                let cause = WebTransportError::Closed(code, reason.clone()).into();
                let code = web_transport_proto::error_to_http3(code).try_into().unwrap();
                (cause, code, reason)
            }
            Err(cause) => {
                // Preserve parser/read failure separately from normal EOF.
                let cause = WebTransportError::CapsuleError(cause.to_string().into()).into();
                (
                    cause,
                    quinn::VarInt::from_u32(H3_MESSAGE_ERROR),
                    "Invalid WebTransport capsule".into(),
                )
            }
        };
        // A local close that already recorded its cause owns the teardown.
        if error.set(cause).is_ok() {
            conn.close(code, reason.as_bytes());
        }
    }

    // Keep reading capsules from the CONNECT recv stream until it's closed.
    // Normal EOF, explicit metadata, and malformed/interrupted capsules remain
    // distinct results all the way to the session's authoritative close cause.
    async fn read_capsules(
        recv: quinn::RecvStream,
    ) -> Result<Option<(u32, String)>, web_transport_proto::CapsuleError> {
        let mut reader = web_transport_proto::Http3CapsuleReader::new(recv);
        loop {
            match reader.read().await? {
                Some(web_transport_proto::Capsule::CloseWebTransportSession {
                    code,
                    reason,
                }) => return Ok(Some((code, reason))),
                Some(web_transport_proto::Capsule::Grease { .. }) => {}
                Some(web_transport_proto::Capsule::Unknown { typ, payload }) => {
                    tracing::warn!(%typ, size = payload.len(), "unknown capsule");
                }
                None => return Ok(None),
            }
        }
    }

    /// Connect using an established QUIC connection if you want to create the connection yourself.
    /// This will only work with a brand new QUIC connection using the HTTP/3 ALPN.
    pub async fn connect(
        conn: quinn::Connection,
        request: impl Into<ConnectRequest>,
    ) -> Result<Session, ClientError> {
        let request = request.into();

        // Perform the H3 handshake by sending/reciving SETTINGS frames.
        let settings = Settings::connect(&conn).await?;

        // Send the HTTP/3 CONNECT request.
        let connect = Connected::open(&conn, request).await?;

        // Return the resulting session with a reference to the control/connect streams.
        // If either stream is closed, then the session will be closed, so we need to keep them around.
        let session = Session::new(conn, settings, connect);

        Ok(session)
    }

    /// Accept a new unidirectional stream. See [`quinn::Connection::accept_uni`].
    pub async fn accept_uni(&self) -> Result<RecvStream, SessionError> {
        if let Some(accept) = &self.accept {
            // `kio::wait` owns the waiter, so dropping this future — a `timeout` that
            // expires, say — also drops its registration in `SessionAccept`.
            kio::wait(|waiter| poll_accept_uni_shared(accept, waiter))
                .await
                .map_err(|e| self.map_error(e))
        } else {
            let recv = self
                .conn
                .accept_uni()
                .await
                .map_err(|e| self.map_error(e))?;
            Ok(RecvStream::new(recv, self.error.clone(), true))
        }
    }

    /// Accept a new bidirectional stream. See [`quinn::Connection::accept_bi`].
    pub async fn accept_bi(&self) -> Result<(SendStream, RecvStream), SessionError> {
        if let Some(accept) = &self.accept {
            kio::wait(|waiter| poll_accept_bi_shared(accept, waiter))
                .await
                .map_err(|e| self.map_error(e))
        } else {
            let (send, recv) = self.conn.accept_bi().await.map_err(|e| self.map_error(e))?;
            Ok((
                SendStream::new(send, self.error.clone(), true),
                RecvStream::new(recv, self.error.clone(), true),
            ))
        }
    }

    /// Open a new unidirectional stream. See [`quinn::Connection::open_uni`].
    pub async fn open_uni(&self) -> Result<SendStream, SessionError> {
        Self::open_uni_owned(
            self.conn.clone(),
            self.header_uni.clone(),
            self.error.clone(),
            self.is_raw(),
        )
        .await
    }

    /// Open a new bidirectional stream. See [`quinn::Connection::open_bi`].
    pub async fn open_bi(&self) -> Result<(SendStream, RecvStream), SessionError> {
        Self::open_bi_owned(
            self.conn.clone(),
            self.header_bi.clone(),
            self.error.clone(),
            self.is_raw(),
        )
        .await
    }

    /// Asynchronously receives an application datagram from the remote peer.
    ///
    /// This method is used to receive an application datagram sent by the remote
    /// peer over the connection.
    /// It waits for a datagram to become available and returns the received bytes.
    pub async fn read_datagram(&self) -> Result<Bytes, SessionError> {
        Self::read_datagram_owned(self.conn.clone(), self.session_id, self.error.clone()).await
    }

    /// Sends an application datagram to the remote peer.
    ///
    /// Datagrams are unreliable and may be dropped or delivered out of order.
    /// The data must be smaller than [`max_datagram_size`](Self::max_datagram_size).
    pub fn send_datagram(&self, data: Bytes) -> Result<(), SessionError> {
        let result = if !self.header_datagram.is_empty() {
            // Unfortunately, we need to allocate/copy each datagram because of the Quinn API.
            // Pls go +1 if you care: https://github.com/quinn-rs/quinn/issues/1724
            let mut buf = BytesMut::with_capacity(self.header_datagram.len() + data.len());

            // Prepend the datagram with the header indicating the session ID.
            buf.extend_from_slice(&self.header_datagram);
            buf.extend_from_slice(&data);

            self.conn.send_datagram(buf.into())
        } else {
            self.conn.send_datagram(data)
        };

        result.map_err(|e| self.map_error(e))?;
        Ok(())
    }

    /// Sends an application datagram, waiting for buffer space if the send buffer is full.
    ///
    /// Unlike [`send_datagram`](Self::send_datagram), this applies backpressure instead of
    /// returning an error when there are too many outstanding datagrams.
    ///
    /// Datagrams are unreliable and may be dropped or delivered out of order.
    /// The data must be smaller than [`max_datagram_size`](Self::max_datagram_size).
    pub async fn send_datagram_wait(&self, data: Bytes) -> Result<(), SessionError> {
        let result = if !self.header_datagram.is_empty() {
            // Unfortunately, we need to allocate/copy each datagram because of the Quinn API.
            // Pls go +1 if you care: https://github.com/quinn-rs/quinn/issues/1724
            let mut buf = BytesMut::with_capacity(self.header_datagram.len() + data.len());

            // Prepend the datagram with the header indicating the session ID.
            buf.extend_from_slice(&self.header_datagram);
            buf.extend_from_slice(&data);

            self.conn.send_datagram_wait(buf.into()).await
        } else {
            self.conn.send_datagram_wait(data).await
        };

        result.map_err(|e| self.map_error(e))?;
        Ok(())
    }

    /// Computes the maximum size of datagrams that may be passed to
    /// [`send_datagram`](Self::send_datagram), or 0 when the peer does not accept datagrams.
    pub fn max_datagram_size(&self) -> usize {
        self.conn
            .max_datagram_size()
            .unwrap_or(0)
            .saturating_sub(self.header_datagram.len())
    }

    /// The number of bytes of available space in the outgoing datagram buffer.
    ///
    /// The session-ID header is subtracted, so this reflects the payload bytes that may be
    /// passed to [`send_datagram`](Self::send_datagram) before it starts dropping datagrams.
    pub fn datagram_send_buffer_space(&self) -> usize {
        self.conn
            .datagram_send_buffer_space()
            .saturating_sub(self.header_datagram.len())
    }

    /// Close the session with an error code and reason.
    ///
    /// When there is a session ID (WebTransport over HTTP/3), a `CloseWebTransportSession`
    /// capsule is written on the CONNECT stream before the QUIC connection is closed.
    /// This allows browser clients to receive the close code and reason via `WebTransport.closed`.
    ///
    /// The capsule write and connection close happen asynchronously in a task spawned on
    /// the runtime [`quinn::default_runtime`] detects. Callers should `await`
    /// [`Session::closed()`] to ensure the capsule has been delivered. Session operations
    /// will fail once the QUIC connection is closed.
    ///
    /// When called where no runtime can be detected, such as outside a tokio runtime with
    /// only quinn's tokio support enabled, the capsule is skipped and the QUIC connection is
    /// closed immediately with the same code and reason.
    pub fn close(&self, code: u32, reason: &[u8]) {
        // Record the local close error. First writer wins — if the background
        // task already set a remote close error, or close() was already called,
        // this is a no-op. A raw peer close that already arrived is latched first.
        if let Some(err) = self.conn.close_reason() {
            self.map_error(err);
        }
        let err = SessionError::ConnectionError(quinn::ConnectionError::LocallyClosed);
        if self.error.set(err).is_err() {
            return;
        }

        if self.session_id.is_some() {
            // Take the send stream for the capsule write.
            let send = self.connect_send.lock().unwrap().take();

            if let Some(send) = send {
                // Without a runtime to run the graceful close on, such as when called from
                // outside one, close the connection now and carry the reason there instead.
                let Some(runtime) = quinn::default_runtime() else {
                    let http3_code: quinn::VarInt = web_transport_proto::error_to_http3(code)
                        .try_into()
                        .unwrap();
                    self.conn.close(http3_code, reason);
                    return;
                };

                let reason = String::from_utf8_lossy(reason).into_owned();
                let conn = self.conn.clone();
                let capsule =
                    web_transport_proto::Capsule::CloseWebTransportSession { code, reason };
                let timeout = (self.rtt() * 3).max(Duration::from_millis(100));

                runtime.clone().spawn(Box::pin(async move {
                    Self::close_with_capsule(conn, send, capsule, code, timeout, runtime).await;
                }));
            }
        } else {
            // Raw QUIC mode: no capsule needed.
            self.conn.close(code.into(), reason);
        }
    }

    /// Write the CloseWebTransportSession capsule, finish the stream, wait for
    /// the peer to close the connection (or timeout), then force-close.
    async fn close_with_capsule(
        conn: quinn::Connection,
        mut send: quinn::SendStream,
        capsule: web_transport_proto::Capsule,
        code: u32,
        timeout: std::time::Duration,
        runtime: Arc<dyn quinn::Runtime>,
    ) {
        let http3_code: quinn::VarInt = web_transport_proto::error_to_http3(code)
            .try_into()
            .unwrap();

        // Encode the capsule, then wrap it in an HTTP/3 DATA frame.
        // In HTTP/3, capsule data is carried inside DATA frames on the CONNECT
        // stream (RFC 9297 Section 3.2).
        let mut capsule_bytes = Vec::new();
        capsule.encode(&mut capsule_bytes);

        let mut frame = Vec::new();
        Frame::DATA.encode(&mut frame);
        let Ok(len) = VarInt::try_from(capsule_bytes.len()) else {
            tracing::warn!("capsule too large to encode as DATA frame");
            conn.close(http3_code, b"");
            return;
        };
        len.encode(&mut frame);
        frame.extend_from_slice(&capsule_bytes);

        // Bound the entire graceful-close sequence (capsule write, FIN,
        // waiting for the peer) with a single timeout.  Without this, an
        // unresponsive peer can cause write_all to block indefinitely when
        // the send buffer fills up and no idle timeout is configured.
        let graceful = async {
            // Write the DATA frame to the CONNECT send stream.
            if let Err(e) = send.write_all(&frame).await {
                tracing::warn!(?e, "failed to write CloseWebTransportSession capsule");
                conn.close(http3_code, b"");
                return;
            }

            // FIN the send stream so the peer knows no more capsules are coming.
            if let Err(e) = send.finish() {
                tracing::warn!(?e, "failed to finish CONNECT send stream");
                conn.close(http3_code, b"");
                return;
            }

            // Wait for the peer to close the CONNECT stream after receiving the capsule.
            conn.closed().await;
        };

        // Timer deadlines are on the runtime's clock, which need not match Instant::now.
        let mut timer = runtime.new_timer(runtime.now() + timeout);
        let expired = poll_fn(|cx| timer.as_mut().poll(cx));

        futures::pin_mut!(graceful, expired);

        if let futures::future::Either::Right(_) = futures::future::select(graceful, expired).await
        {
            tracing::debug!("timeout waiting for peer to close; force-closing connection");
            conn.close(http3_code, b"");
        }
    }

    /// Wait until the session is closed, returning the error. See [`quinn::Connection::closed`].
    ///
    /// If the peer sent a `CloseWebTransportSession` capsule, the returned error will be
    /// [`WebTransportError::Closed`] with the code and reason from the capsule.
    ///
    /// Unlike [`quinn::Connection::closed`], this does **not** return early when
    /// [`close()`](Self::close) has been called. It waits for the underlying QUIC
    /// connection to shut down, ensuring the `CloseWebTransportSession` capsule has
    /// been delivered. Use [`close_reason()`](Self::close_reason) for a non-blocking check.
    pub async fn closed(&self) -> SessionError {
        self.map_error(self.conn.closed().await)
    }

    /// Return why the session was closed, or None if it's not closed. See [`quinn::Connection::close_reason`].
    pub fn close_reason(&self) -> Option<SessionError> {
        self.conn.close_reason().map(|e| self.map_error(e))
    }

    /// Whether this is a raw QUIC session, with no HTTP/3 layer.
    fn is_raw(&self) -> bool {
        self.session_id.is_none()
    }

    /// Replace connection-level errors with the stored session error if available.
    fn map_error(&self, e: impl Into<SessionError>) -> SessionError {
        map_error_with(&self.error, e)
    }

    // The owned-argument form, for futures that outlive the borrow that created them.
    fn map_error_owned(error: &CloseReason, e: impl Into<SessionError>) -> SessionError {
        map_error_with(error, e)
    }
}

/// Replace connection-level errors with the stored session error if available.
fn map_error_with(stored: &CloseReason, e: impl Into<SessionError>) -> SessionError {
    stored.map(e.into())
}

impl Session {
    async fn write_full(send: &mut quinn::SendStream, buf: &[u8]) -> Result<(), SessionError> {
        match send.write_all(buf).await {
            Ok(_) => Ok(()),
            Err(quinn::WriteError::ConnectionLost(err)) => Err(err.into()),
            Err(err) => Err(WebTransportError::WriteError(err).into()),
        }
    }

    /// Create a new session from a raw QUIC connection.
    ///
    /// This is used to pretend like a QUIC connection is a WebTransport session,
    /// making it easier to support WebTransport and raw QUIC simultaneously.
    ///
    /// There is no HTTP/3 exchange, so [`Self::request`] and [`Self::response`] both
    /// return `None`. [`Self::protocol`] reports the ALPN negotiated by the QUIC
    /// handshake, read from `conn` on demand — a connection that is still handshaking
    /// (from `into_0rtt`, say) is fine, it just has no ALPN to report until it is done.
    pub fn raw(conn: quinn::Connection) -> Self {
        Self {
            error: Arc::new(CloseReason::raw(conn.clone())),
            conn,
            session_id: None,
            header_uni: Default::default(),
            header_bi: Default::default(),
            header_datagram: Default::default(),
            accept: None,
            op_accept_uni: Default::default(),
            op_accept_bi: Default::default(),
            op_open_uni: Default::default(),
            op_open_bi: Default::default(),
            op_send_datagram: Default::default(),
            op_recv_datagram: Default::default(),
            op_closed: Default::default(),
            parked_accept_uni: Default::default(),
            parked_accept_bi: Default::default(),
            settings: None,
            connect_send: Arc::new(Mutex::new(None)),
            request: None,
            response: None,
            alpn: Default::default(),
        }
    }

    /// Returns the [`ConnectRequest`] if this session was established over HTTP/3,
    /// or `None` for a raw QUIC session.
    pub fn request(&self) -> Option<&ConnectRequest> {
        self.request.as_ref()
    }

    /// Returns the [`ConnectResponse`] if this session was established over HTTP/3,
    /// or `None` for a raw QUIC session.
    pub fn response(&self) -> Option<&ConnectResponse> {
        self.response.as_ref()
    }

    /// Returns the application protocol negotiated for this session.
    ///
    /// For an HTTP/3 session this is the subprotocol the server selected via
    /// `WT-Available-Protocols`; for a raw QUIC session it is the negotiated ALPN.
    /// `None` if neither was negotiated, the ALPN is not valid UTF-8, or the raw
    /// connection is still handshaking.
    pub fn protocol(&self) -> Option<&str> {
        if let Some(response) = &self.response {
            return response.protocol.as_deref();
        }

        if let Some(alpn) = self.alpn.get() {
            return Some(alpn);
        }

        // Latch only a successful read, so a call made while the connection is still
        // handshaking cannot pin `None` for the life of the session.
        let alpn = negotiated_alpn(&self.conn)?;
        Some(self.alpn.get_or_init(|| alpn))
    }

    /// Return connection-level statistics.
    pub fn stats(&self) -> SessionStats {
        SessionStats {
            stats: self.conn.stats(),
            rtt: self.conn.rtt(),
            pacing_rate: self.conn.congestion_state().metrics().pacing_rate,
        }
    }
}

impl Deref for Session {
    type Target = quinn::Connection;

    fn deref(&self) -> &Self::Target {
        &self.conn
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.conn.fmt(f)
    }
}

impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        self.conn.stable_id() == other.conn.stable_id()
    }
}

impl Eq for Session {}

// Poll the shared accept state, then wake the *other* accepters once the lock is
// released.
//
// `SessionAccept` does not wake them itself: a waker is free to resume its task
// inline, and the first thing a resumed accepter does is take this same lock. An
// arrival or a failure is exactly what the others are parked waiting to retry
// after, so `Ready` is the signal.
fn poll_accept_uni_shared(
    accept: &Mutex<SessionAccept>,
    waiter: &Waiter,
) -> Poll<Result<RecvStream, SessionError>> {
    let (result, waiters, hold) = {
        let mut accept = accept.lock().unwrap();
        let waiters = accept.uni_waiters.clone();

        // The poll below drives the shared accept futures with this list's waker, and one
        // of them may wake it inline. `Fan` holds those back while this guard is alive.
        let hold = waiters.hold();
        let result = accept.poll_accept_uni(waiter);

        (result, waiters, hold)
    };

    // Dropped after the accept lock, never before: that is where a held-back wake is
    // delivered, and delivering it under the lock is the hazard the hold exists for.
    drop(hold);

    if result.is_ready() {
        waiters.wake();
    }

    result
}

fn poll_accept_bi_shared(
    accept: &Mutex<SessionAccept>,
    waiter: &Waiter,
) -> Poll<Result<(SendStream, RecvStream), SessionError>> {
    let (result, waiters, hold) = {
        let mut accept = accept.lock().unwrap();
        let waiters = accept.bi_waiters.clone();

        // The poll below drives the shared accept futures with this list's waker, and one
        // of them may wake it inline. `Fan` holds those back while this guard is alive.
        let hold = waiters.hold();
        let result = accept.poll_accept_bi(waiter);

        (result, waiters, hold)
    };

    // Dropped after the accept lock, never before: that is where a held-back wake is
    // delivered, and delivering it under the lock is the hazard the hold exists for.
    drop(hold);

    if result.is_ready() {
        waiters.wake();
    }

    result
}

// Type aliases just so clippy doesn't complain about the complexity.
type AcceptUni = dyn Stream<Item = Result<quinn::RecvStream, quinn::ConnectionError>> + Send;
type AcceptBi = dyn Stream<Item = Result<(quinn::SendStream, quinn::RecvStream), quinn::ConnectionError>>
    + Send;
type PendingUni = dyn Future<Output = Result<(StreamUni, quinn::RecvStream), SessionError>> + Send;
type PendingBi = dyn Future<Output = Result<Option<(quinn::SendStream, quinn::RecvStream)>, SessionError>>
    + Send;

// Logic just for accepting streams, which is annoying because of the stream header.
pub struct SessionAccept {
    session_id: VarInt,

    // Shared session error for propagation to accepted streams.
    error: Arc<CloseReason>,

    // We also need to keep a reference to the qpack streams if the endpoint (incorrectly) creates them.
    // Again, this is just so they don't get closed until we drop the session.
    qpack_encoder: Option<quinn::RecvStream>,
    qpack_decoder: Option<quinn::RecvStream>,

    accept_uni: Pin<Box<AcceptUni>>,
    accept_bi: Pin<Box<AcceptBi>>,

    // Keep track of work being done to read/write the WebTransport stream header.
    pending_uni: FuturesUnordered<Pin<Box<PendingUni>>>,
    pending_bi: FuturesUnordered<Pin<Box<PendingBi>>>,

    // Waiters from concurrent callers of accept_bi / accept_uni.
    // Every clone of the session polls this one struct, so an arrival has to be fanned
    // out: each caller registers here and all of them are woken when a stream lands —
    // by the caller that saw it, once it has released the lock on this struct.
    bi_waiters: Fan,
    uni_waiters: Fan,

    // `Waker::from(waiters.clone())`, cached so Quinn's accept futures are polled with
    // the same waker every time. That waker outlives every caller, so an accepter that
    // drops its future cannot take the wakeup path with it, and Quinn holds one
    // registration rather than one per caller.
    bi_waker: Waker,
    uni_waker: Waker,
}

impl SessionAccept {
    pub(crate) fn new(
        conn: quinn::Connection,
        session_id: VarInt,
        error: Arc<CloseReason>,
    ) -> Self {
        // Create a stream that just outputs new streams, so it's easy to call from poll.
        let accept_uni = Box::pin(futures::stream::unfold(conn.clone(), |conn| async {
            Some((conn.accept_uni().await, conn))
        }));

        let accept_bi = Box::pin(futures::stream::unfold(conn, |conn| async {
            Some((conn.accept_bi().await, conn))
        }));

        let bi_waiters = Fan::new();
        let uni_waiters = Fan::new();
        let bi_waker = bi_waiters.waker();
        let uni_waker = uni_waiters.waker();

        Self {
            session_id,
            error,

            qpack_decoder: None,
            qpack_encoder: None,

            accept_uni,
            accept_bi,

            pending_uni: FuturesUnordered::new(),
            pending_bi: FuturesUnordered::new(),

            bi_waiters,
            uni_waiters,
            bi_waker,
            uni_waker,
        }
    }

    /// Poll for the next unidirectional WebTransport stream.
    ///
    /// `waiter` is parked until a stream arrives, the accept fails, or the caller drops
    /// it. The registration is weak and owned by the caller: keep the [`Waiter`] alive
    /// until it is woken, or it will be reclaimed and nothing will wake you. Drive this
    /// with [`kio::wait`], which holds the waiter inside the future it builds.
    ///
    /// A `Ready` here means every *other* parked accepter should be woken so it can
    /// retry. This does not do that itself — see `poll_accept_uni_shared`, which wakes
    /// them once the lock on this struct is released.
    //
    // Poll-based because we accept and decode streams in parallel. In async land this
    // would be a `tokio::JoinSet`, but that needs a runtime; `FuturesUnordered` is
    // runtime-agnostic.
    pub fn poll_accept_uni(&mut self, waiter: &Waiter) -> Poll<Result<RecvStream, SessionError>> {
        // Register before polling, not on the way out: the shared waker can fire from
        // Quinn's driver at any point below, and a wake that lands before the caller is
        // on the list would be lost.
        self.uni_waiters.register(waiter);

        let waker = self.uni_waker.clone();
        let cx = &mut Context::from_waker(&waker);

        loop {
            // Accept any new streams.
            if let Poll::Ready(Some(res)) = self.accept_uni.poll_next_unpin(cx) {
                // Start decoding the header and add the future to the list of pending streams.
                let recv = match res {
                    Ok(recv) => recv,
                    Err(e) => {
                        return Poll::Ready(Err(e.into()));
                    }
                };
                let pending = Self::decode_uni(recv, self.session_id);
                self.pending_uni.push(Box::pin(pending));

                continue;
            }

            // Poll the list of pending streams.
            let (typ, recv) = match self.pending_uni.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(res))) => res,
                Poll::Ready(Some(Err(err))) => {
                    log_header_error(err, "unidirectional");
                    continue;
                }
                Poll::Ready(None) | Poll::Pending => return Poll::Pending,
            };

            // Decide if we keep looping based on the type.
            match typ {
                StreamUni::WEBTRANSPORT => {
                    let recv = RecvStream::new(recv, self.error.clone(), false);
                    return Poll::Ready(Ok(recv));
                }
                StreamUni::QPACK_DECODER => {
                    self.qpack_decoder = Some(recv);
                }
                StreamUni::QPACK_ENCODER => {
                    self.qpack_encoder = Some(recv);
                }
                _ => {
                    // ignore unknown streams
                    tracing::debug!(?typ, "ignoring unknown unidirectional stream");
                }
            }
        }
    }

    // Reads the stream header, returning the stream type.
    async fn decode_uni(
        mut recv: quinn::RecvStream,
        expected_session: VarInt,
    ) -> Result<(StreamUni, quinn::RecvStream), SessionError> {
        // Read the VarInt at the start of the stream.
        let typ = StreamUni(read_varint(&mut recv).await?);

        if typ == StreamUni::WEBTRANSPORT {
            // Read the session_id and validate it
            let session_id = read_varint(&mut recv).await?;
            if session_id != expected_session {
                return Err(WebTransportError::UnknownSession.into());
            }
        }

        // We need to keep a reference to the qpack streams if the endpoint (incorrectly) creates them, so return everything.
        Ok((typ, recv))
    }

    /// Poll for the next bidirectional WebTransport stream.
    ///
    /// The same contract as [`poll_accept_uni`](Self::poll_accept_uni): the `waiter`
    /// registration is weak and owned by the caller, and a `Ready` is what the other
    /// parked accepters need to be woken for.
    pub fn poll_accept_bi(
        &mut self,
        waiter: &Waiter,
    ) -> Poll<Result<(SendStream, RecvStream), SessionError>> {
        // Register before polling; see `poll_accept_uni`.
        self.bi_waiters.register(waiter);

        let waker = self.bi_waker.clone();
        let cx = &mut Context::from_waker(&waker);

        loop {
            // Accept any new streams.
            if let Poll::Ready(Some(res)) = self.accept_bi.poll_next_unpin(cx) {
                // Start decoding the header and add the future to the list of pending streams.
                let (send, recv) = match res {
                    Ok(pair) => pair,
                    Err(e) => {
                        return Poll::Ready(Err(e.into()));
                    }
                };
                let pending = Self::decode_bi(send, recv, self.session_id);
                self.pending_bi.push(Box::pin(pending));

                continue;
            }

            // Poll the list of pending streams.
            let res = match self.pending_bi.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(res))) => res,
                Poll::Ready(Some(Err(err))) => {
                    log_header_error(err, "bidirectional");
                    continue;
                }
                Poll::Ready(None) | Poll::Pending => return Poll::Pending,
            };

            if let Some((send, recv)) = res {
                // Wrap the streams in our own types for correct error codes.
                let send = SendStream::new(send, self.error.clone(), false);
                let recv = RecvStream::new(recv, self.error.clone(), false);
                return Poll::Ready(Ok((send, recv)));
            }

            // Keep looping if it's a stream we want to ignore.
        }
    }

    // Reads the stream header, returning Some if it's a WebTransport stream.
    async fn decode_bi(
        send: quinn::SendStream,
        mut recv: quinn::RecvStream,
        expected_session: VarInt,
    ) -> Result<Option<(quinn::SendStream, quinn::RecvStream)>, SessionError> {
        let typ = read_varint(&mut recv).await?;
        if Frame(typ) != Frame::WEBTRANSPORT {
            tracing::debug!(?typ, "ignoring unknown bidirectional stream");
            return Ok(None);
        }

        // Read the session ID and validate it.
        let session_id = read_varint(&mut recv).await?;
        if session_id != expected_session {
            return Err(WebTransportError::UnknownSession.into());
        }

        Ok(Some((send, recv)))
    }
}

// Read a stream header VarInt, keeping the read's real cause. `VarInt::read` reports any
// failure as a truncation, which would hide a stream the peer reset before its header
// arrived.
async fn read_varint(recv: &mut quinn::RecvStream) -> Result<VarInt, WebTransportError> {
    let mut buf = [0u8; 8];
    recv.read_exact(&mut buf[..1]).await?;

    // The first two bits encode the length.
    let size = 1 << (buf[0] >> 6);
    recv.read_exact(&mut buf[1..size]).await?;

    Ok(VarInt::decode(&mut &buf[..size]).expect("a complete varint"))
}

// Without reliable reset, a stream the peer resets early loses its header with it, which
// is routine for some applications. Anything else is a misbehaving peer.
fn log_header_error(err: SessionError, direction: &'static str) {
    match err {
        SessionError::WebTransportError(WebTransportError::ReadError(
            quinn::ReadExactError::ReadError(
                quinn::ReadError::Reset(_) | quinn::ReadError::ConnectionLost(_),
            ),
        )) => tracing::debug!(?err, direction, "stream closed before its header"),
        _ => tracing::warn!(?err, direction, "failed to decode stream header"),
    }
}

pub struct SessionStats {
    stats: quinn::ConnectionStats,
    rtt: std::time::Duration,
    pacing_rate: Option<u64>,
}

impl web_transport_trait::Stats for SessionStats {
    fn bytes_sent(&self) -> Option<u64> {
        Some(self.stats.udp_tx.bytes)
    }

    fn bytes_received(&self) -> Option<u64> {
        Some(self.stats.udp_rx.bytes)
    }

    fn bytes_lost(&self) -> Option<u64> {
        Some(self.stats.path.lost_bytes)
    }

    fn packets_sent(&self) -> Option<u64> {
        Some(self.stats.udp_tx.datagrams)
    }

    fn packets_received(&self) -> Option<u64> {
        Some(self.stats.udp_rx.datagrams)
    }

    fn packets_lost(&self) -> Option<u64> {
        Some(self.stats.path.lost_packets)
    }

    fn rtt(&self) -> Option<std::time::Duration> {
        Some(self.rtt)
    }

    fn estimated_send_rate(&self) -> Option<u64> {
        // BBR reports 0 until its first rate sample.
        if let Some(pacing_rate) = self.pacing_rate.filter(|rate| *rate > 0) {
            return Some(pacing_rate);
        }
        let rtt_secs = self.rtt.as_secs_f64();
        if self.stats.path.cwnd > 0 && rtt_secs > 0.0 {
            Some((self.stats.path.cwnd as f64 * 8.0 / rtt_secs) as u64)
        } else {
            None
        }
    }
}

impl web_transport_trait::Session for Session {
    type SendStream = SendStream;
    type RecvStream = RecvStream;
    type Error = SessionError;

    async fn accept_uni(&self) -> Result<Self::RecvStream, Self::Error> {
        Self::accept_uni(self).await
    }

    async fn accept_bi(&self) -> Result<(Self::SendStream, Self::RecvStream), Self::Error> {
        Self::accept_bi(self).await
    }

    async fn open_bi(&self) -> Result<(Self::SendStream, Self::RecvStream), Self::Error> {
        Self::open_bi(self).await
    }

    async fn open_uni(&self) -> Result<Self::SendStream, Self::Error> {
        Self::open_uni(self).await
    }

    fn close(&self, code: u32, reason: &str) {
        Self::close(self, code, reason.as_bytes());
    }

    async fn closed(&self) -> Self::Error {
        Self::closed(self).await
    }

    fn send_datagram(&self, data: Bytes) -> Result<(), Self::Error> {
        Self::send_datagram(self, data)
    }

    async fn recv_datagram(&self) -> Result<Bytes, Self::Error> {
        Self::read_datagram(self).await
    }

    fn max_datagram_size(&self) -> usize {
        Self::max_datagram_size(self)
    }

    fn protocol(&self) -> Option<&str> {
        Self::protocol(self)
    }

    #[allow(refining_impl_trait)]
    fn stats(&self) -> SessionStats {
        Self::stats(self)
    }
}

impl Session {
    // Open plus stream-header write, over owned arguments so the result can live in
    // a retained future. Mirrors `open_uni`/`open_bi`, including the priority dance.
    async fn open_uni_owned(
        conn: quinn::Connection,
        header: Bytes,
        error: Arc<CloseReason>,
        raw: bool,
    ) -> Result<SendStream, SessionError> {
        let mut send = conn
            .open_uni()
            .await
            .map_err(|e| Self::map_error_owned(&error, e))?;

        // Max priority for the header so the application cannot queue lower-priority
        // data in front of it, then back to the default.
        send.set_priority(i32::MAX).ok();
        Self::write_full(&mut send, &header)
            .await
            .map_err(|e| Self::map_error_owned(&error, e))?;
        send.set_priority(0).ok();

        Ok(SendStream::new(send, error, raw))
    }

    async fn open_bi_owned(
        conn: quinn::Connection,
        header: Bytes,
        error: Arc<CloseReason>,
        raw: bool,
    ) -> Result<(SendStream, RecvStream), SessionError> {
        let (mut send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| Self::map_error_owned(&error, e))?;

        send.set_priority(i32::MAX).ok();
        Self::write_full(&mut send, &header)
            .await
            .map_err(|e| Self::map_error_owned(&error, e))?;
        send.set_priority(0).ok();

        Ok((
            SendStream::new(send, error.clone(), raw),
            RecvStream::new(recv, error, raw),
        ))
    }

    fn send_datagram_framed(
        &mut self,
        cx: &mut Context<'_>,
        payload: &[u8],
    ) -> Poll<Result<(), SessionError>> {
        // Resume an in-flight send before touching `payload`. Framing first would
        // allocate on every poll and then discard it, and would quietly ignore a
        // caller that retried with a different datagram — the retained future
        // already owns the one it started with.
        if let Some(result) = self.op_send_datagram.poll_pending(cx) {
            return result;
        }

        let conn = self.conn.clone();
        // Quinn's datagram API needs an owned `Bytes` either way, and the HTTP/3 path
        // has to copy regardless to prepend the session ID.
        let payload = Self::frame_datagram(&self.header_datagram, payload);
        let error = self.error.clone();

        // `send_datagram_wait` is what makes this pollable rather than a drop: it
        // parks until the transport has room. Retained, because its `Notify`
        // registration lives in the future.
        self.op_send_datagram.poll(cx, move || async move {
            conn.send_datagram_wait(payload)
                .await
                .map_err(|e| Session::map_error_owned(&error, e))
        })
    }

    // Prepend the session ID, as `send_datagram`/`send_datagram_wait` do.
    //
    // Unfortunately, we need to allocate/copy each datagram because of the Quinn API.
    // Pls go +1 if you care: https://github.com/quinn-rs/quinn/issues/1724
    fn frame_datagram(header: &Bytes, data: &[u8]) -> Bytes {
        let mut buf = BytesMut::with_capacity(header.len() + data.len());
        buf.extend_from_slice(header);
        buf.extend_from_slice(data);
        buf.into()
    }

    async fn read_datagram_owned(
        conn: quinn::Connection,
        session_id: Option<VarInt>,
        error: Arc<CloseReason>,
    ) -> Result<Bytes, SessionError> {
        let mut datagram = conn
            .read_datagram()
            .await
            .map_err(|e| Self::map_error_owned(&error, e))?;

        let mut cursor = Cursor::new(&datagram);

        if let Some(session_id) = session_id {
            // We have to check and strip the session ID from the datagram.
            let actual_id =
                VarInt::decode(&mut cursor).map_err(|_| WebTransportError::UnknownSession)?;
            if actual_id != session_id {
                return Err(WebTransportError::UnknownSession.into());
            }
        }

        // Return the datagram without the session ID.
        Ok(datagram.split_off(cursor.position() as usize))
    }
}

impl web_transport_trait::poll::Session for Session {
    type SendStream = SendStream;
    type RecvStream = RecvStream;
    type Error = SessionError;

    // Accept forwards natively: `SessionAccept` already drives a persistent stream and
    // keeps a waiter list, so the only thing to retain is our registration in it.
    fn poll_accept_uni(&mut self, cx: &mut Context<'_>) -> Poll<Result<RecvStream, SessionError>> {
        if let Some(accept) = self.accept.clone() {
            return self
                .parked_accept_uni
                .poll(cx, |waiter| poll_accept_uni_shared(&accept, waiter));
        }

        let conn = self.conn.clone();
        let error = self.error.clone();

        self.op_accept_uni.poll(cx, move || async move {
            let recv = conn
                .accept_uni()
                .await
                .map_err(|e| Session::map_error_owned(&error, e))?;
            Ok(RecvStream::new(recv, error, true))
        })
    }

    fn poll_accept_bi(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(SendStream, RecvStream), SessionError>> {
        if let Some(accept) = self.accept.clone() {
            return self
                .parked_accept_bi
                .poll(cx, |waiter| poll_accept_bi_shared(&accept, waiter));
        }

        let conn = self.conn.clone();
        let error = self.error.clone();

        self.op_accept_bi.poll(cx, move || async move {
            let (send, recv) = conn
                .accept_bi()
                .await
                .map_err(|e| Session::map_error_owned(&error, e))?;
            Ok((
                SendStream::new(send, error.clone(), true),
                RecvStream::new(recv, error, true),
            ))
        })
    }

    fn poll_open_uni(&mut self, cx: &mut Context<'_>) -> Poll<Result<SendStream, SessionError>> {
        let conn = self.conn.clone();
        let header = self.header_uni.clone();
        let error = self.error.clone();
        let raw = self.is_raw();

        self.op_open_uni.poll(cx, move || async move {
            Session::open_uni_owned(conn, header, error, raw).await
        })
    }

    fn poll_open_bi(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(SendStream, RecvStream), SessionError>> {
        let conn = self.conn.clone();
        let header = self.header_bi.clone();
        let error = self.error.clone();
        let raw = self.is_raw();

        self.op_open_bi.poll(cx, move || async move {
            Session::open_bi_owned(conn, header, error, raw).await
        })
    }

    fn poll_send_datagram(
        &mut self,
        cx: &mut Context<'_>,
        payload: &[u8],
    ) -> Poll<Result<(), SessionError>> {
        self.send_datagram_framed(cx, payload)
    }

    fn poll_recv_datagram(&mut self, cx: &mut Context<'_>) -> Poll<Result<Bytes, SessionError>> {
        let conn = self.conn.clone();
        let session_id = self.session_id;
        let error = self.error.clone();

        self.op_recv_datagram.poll(cx, move || {
            Session::read_datagram_owned(conn, session_id, error)
        })
    }

    fn max_datagram_size(&self) -> usize {
        Self::max_datagram_size(self)
    }

    fn protocol(&self) -> Option<&str> {
        Self::protocol(self)
    }

    fn close(&mut self, code: u32, reason: &str) {
        Self::close(self, code, reason.as_bytes());
    }

    fn poll_closed(&mut self, cx: &mut Context<'_>) -> Poll<SessionError> {
        let conn = self.conn.clone();
        let error = self.error.clone();

        self.op_closed.poll(cx, move || async move {
            Session::map_error_owned(&error, conn.closed().await)
        })
    }

    #[allow(refining_impl_trait)]
    fn stats(&self) -> SessionStats {
        Self::stats(self)
    }
}

#[cfg(all(test, any(feature = "aws-lc-rs", feature = "ring")))]
mod tests {
    use std::{net::Ipv4Addr, sync::Arc};

    use rcgen::{CertifiedKey, KeyPair};
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

    use super::*;

    const SESSION: VarInt = VarInt::from_u32(0);

    /// A connected client and server QUIC connection, plus the endpoints that drive them.
    async fn pair() -> (quinn::Connection, quinn::Connection, [quinn::Endpoint; 2]) {
        pair_with(Default::default()).await
    }

    /// [`pair`], with the client using `transport`.
    async fn pair_with(
        transport: quinn::TransportConfig,
    ) -> (quinn::Connection, quinn::Connection, [quinn::Endpoint; 2]) {
        #[cfg(all(feature = "aws-lc-rs", feature = "ring"))]
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KeyPair::serialize_der(
            &signing_key,
        )));

        let server_config =
            quinn::ServerConfig::with_single_cert(vec![cert.der().clone()], key).unwrap();
        let server =
            quinn::Endpoint::server(server_config, (Ipv4Addr::LOCALHOST, 0).into()).unwrap();

        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert.der().clone()).unwrap();
        let mut client_config =
            quinn::ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();
        client_config.transport_config(Arc::new(transport));
        let client = quinn::Endpoint::client((Ipv4Addr::LOCALHOST, 0).into()).unwrap();

        let connecting = client
            .connect_with(client_config, server.local_addr().unwrap(), "localhost")
            .unwrap();
        let accept = async { server.accept().await.unwrap().await.unwrap() };
        let (client_conn, server_conn) = tokio::join!(async { connecting.await.unwrap() }, accept);

        (client_conn, server_conn, [client, server])
    }

    fn assert_reset(err: SessionError) {
        assert!(
            matches!(
                &err,
                SessionError::WebTransportError(WebTransportError::ReadError(
                    quinn::ReadExactError::ReadError(quinn::ReadError::Reset(code))
                )) if code.into_inner() == 7
            ),
            "{err:?}"
        );
    }

    /// A uni stream reset before its header arrived reports the reset, not a foreign session.
    #[tokio::test]
    async fn uni_reset_before_header() {
        let (client, server, _endpoints) = pair().await;

        // The stream type alone, so the reset also cuts off the session ID.
        let mut send = client.open_uni().await.unwrap();
        send.write_all(&[0x40, 0x54]).await.unwrap();
        send.reset(7u32.into()).unwrap();

        let recv = server.accept_uni().await.unwrap();
        let err = SessionAccept::decode_uni(recv, SESSION)
            .await
            .err()
            .unwrap();
        assert_reset(err);
    }

    /// A bi stream reset before its header arrived reports the reset, not a foreign session.
    #[tokio::test]
    async fn bi_reset_before_header() {
        let (client, server, _endpoints) = pair().await;

        let (mut send, _recv) = client.open_bi().await.unwrap();
        send.reset(7u32.into()).unwrap();

        let (send, recv) = server.accept_bi().await.unwrap();
        let err = SessionAccept::decode_bi(send, recv, SESSION)
            .await
            .err()
            .unwrap();
        assert_reset(err);
    }

    /// A stream that names another session is still reported as `UnknownSession`.
    #[tokio::test]
    async fn uni_other_session() {
        let (client, server, _endpoints) = pair().await;

        let mut send = client.open_uni().await.unwrap();
        send.write_all(&[0x40, 0x54, 0x04]).await.unwrap();
        send.finish().unwrap();

        let recv = server.accept_uni().await.unwrap();
        let err = SessionAccept::decode_uni(recv, SESSION)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(
                err,
                SessionError::WebTransportError(WebTransportError::UnknownSession)
            ),
            "{err:?}"
        );
    }

    /// A peer without datagram support reports a max datagram size of 0 instead of panicking.
    #[tokio::test]
    async fn no_datagram_support() {
        let mut transport = quinn::TransportConfig::default();
        transport.datagram_receive_buffer_size(None);
        let (_client, server, _endpoints) = pair_with(transport).await;

        assert_eq!(Session::raw(server).max_datagram_size(), 0);
    }

    /// How far [`OffsetRuntime`]'s clock runs ahead of the real one, well past any close timeout.
    const OFFSET: Duration = Duration::from_secs(60 * 60);

    /// Quinn's tokio runtime with a clock that runs [`OFFSET`] ahead of `Instant::now`, as
    /// tokio's paused time or a simulator can. It records every deadline it is asked to time.
    #[derive(Debug, Default)]
    struct OffsetRuntime {
        deadlines: Mutex<Vec<std::time::Instant>>,
    }

    impl quinn::Runtime for OffsetRuntime {
        fn new_timer(&self, deadline: std::time::Instant) -> Pin<Box<dyn quinn::AsyncTimer>> {
            self.deadlines.lock().unwrap().push(deadline);

            // Move the deadline onto the real clock; one already in this clock's past fires at once.
            let remaining = deadline.saturating_duration_since(self.now());
            quinn::TokioRuntime.new_timer(std::time::Instant::now() + remaining)
        }

        fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
            quinn::TokioRuntime.spawn(future)
        }

        fn wrap_udp_socket(
            &self,
            socket: std::net::UdpSocket,
        ) -> std::io::Result<Arc<dyn quinn::AsyncUdpSocket>> {
            quinn::TokioRuntime.wrap_udp_socket(socket)
        }

        fn now(&self) -> std::time::Instant {
            std::time::Instant::now() + OFFSET
        }
    }

    /// The graceful close bounds its wait for the peer with a timer on the runtime it runs
    /// on, so the deadline has to be ahead of that runtime's clock rather than `Instant::now`.
    #[tokio::test]
    async fn close_deadline_follows_the_runtime_clock() {
        use quinn::Runtime;

        let (client, _server, _endpoints) = pair().await;
        let (send, _recv) = client.open_bi().await.unwrap();

        let runtime = Arc::new(OffsetRuntime::default());
        let before = runtime.now();

        // The peer never closes, so only the timer can end the graceful close.
        let capsule = web_transport_proto::Capsule::CloseWebTransportSession {
            code: 0,
            reason: "bye".into(),
        };
        Session::close_with_capsule(
            client.clone(),
            send,
            capsule,
            0,
            Duration::from_millis(100),
            runtime.clone(),
        )
        .await;

        let deadlines = runtime.deadlines.lock().unwrap().clone();
        assert_eq!(deadlines.len(), 1, "close did not time out on the runtime");
        assert!(
            deadlines[0] > before,
            "the close deadline is {:?} behind the runtime's clock",
            before - deadlines[0]
        );
        assert!(client.close_reason().is_some(), "the timeout did not close");
    }

    /// Sessions and requests do not store the runtime, so they stay unwind safe.
    #[test]
    fn unwind_safe() {
        fn assert<T: std::panic::UnwindSafe + std::panic::RefUnwindSafe>() {}
        assert::<Session>();
        assert::<crate::Request>();
    }
}
