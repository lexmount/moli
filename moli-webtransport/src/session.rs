use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;
use web_transport_quinn::{SessionError, WebTransportError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseInfo {
    pub code: u32,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub enum CloseReason {
    Local(CloseInfo),
    Remote(CloseInfo),
    OwnerRetired,
    Failed(SessionError),
}

struct Inner {
    endpoint: EndpointOwner,
    native: web_transport_quinn::Session,
    owner: CancellationToken,
    cancellation: AbortHandle,
    executor: tokio::runtime::Handle,
    local_close: Mutex<Option<CloseInfo>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.cancellation.abort();
    }
}

/// Cancellation of a handshake future must close its socket even when Quinn
/// background tasks still retain a connection reference.
pub(crate) struct EndpointOwner(pub(crate) quinn::Endpoint);

impl Drop for EndpointOwner {
    fn drop(&mut self) {
        self.0.close(quinn::VarInt::from_u32(0x100), b"");
    }
}

/// A real HTTP/3 session, retained while any byte stream still uses it.
#[derive(Clone)]
pub struct Session(Arc<Inner>);

impl Session {
    pub(crate) fn new(
        endpoint: EndpointOwner,
        native: web_transport_quinn::Session,
        owner: CancellationToken,
    ) -> Self {
        let owner = owner.child_token();
        let executor = tokio::runtime::Handle::current();
        let connection = std::ops::Deref::deref(&native).clone();
        let cancelled = owner.clone();
        let cancellation = executor
            .spawn(async move {
                cancelled.cancelled().await;
                connection.close(quinn::VarInt::from_u32(0x100), b"");
            })
            .abort_handle();
        Self(Arc::new(Inner {
            endpoint,
            native,
            owner,
            cancellation,
            executor,
            local_close: Mutex::new(None),
        }))
    }

    pub fn protocol(&self) -> Option<&str> {
        self.0.native.protocol()
    }

    pub fn max_datagram_size(&self) -> usize {
        self.0.native.max_datagram_size()
    }

    pub async fn open_bi(&self) -> Result<(SendStream, RecvStream), SessionError> {
        let (send, recv) = self.0.native.open_bi().await?;
        Ok((self.send_stream(send), self.recv_stream(recv)))
    }

    pub async fn open_uni(&self) -> Result<SendStream, SessionError> {
        self.0
            .native
            .open_uni()
            .await
            .map(|stream| self.send_stream(stream))
    }

    pub async fn accept_bi(&self) -> Result<(SendStream, RecvStream), SessionError> {
        let (send, recv) = self.0.native.accept_bi().await?;
        Ok((self.send_stream(send), self.recv_stream(recv)))
    }

    pub async fn accept_uni(&self) -> Result<RecvStream, SessionError> {
        self.0
            .native
            .accept_uni()
            .await
            .map(|stream| self.recv_stream(stream))
    }

    pub fn send_datagram(&self, bytes: Bytes) -> Result<(), SessionError> {
        self.0.native.send_datagram(bytes)
    }

    pub async fn read_datagram(&self) -> Result<Bytes, SessionError> {
        self.0.native.read_datagram().await
    }

    pub fn close(&self, info: CloseInfo) {
        if self.0.owner.is_cancelled() || self.0.native.close_reason().is_some() {
            return;
        }
        let mut local = self.0.local_close.lock();
        if local.is_some() {
            return;
        }
        *local = Some(info.clone());
        let mut end = info.reason.len().min(1024);
        while !info.reason.is_char_boundary(end) {
            end -= 1;
        }
        let _entered = self.0.executor.enter();
        self.0
            .native
            .close(info.code, &info.reason.as_bytes()[..end]);
        // Graceful closure must survive the JS session wrapper being collected.
        // Owner retirement can still abort its real QUIC connection immediately.
        let session = self.clone();
        drop(self.0.executor.spawn(async move {
            session.0.native.closed().await;
        }));
    }

    /// Retire this execution context's session immediately. Retained JS/native
    /// stream handles cannot keep it alive, and sibling sessions are unaffected.
    pub fn cancel(&self) {
        self.0.owner.cancel();
        self.0.endpoint.0.close(quinn::VarInt::from_u32(0x100), b"");
    }

    pub async fn closed(&self) -> CloseReason {
        let result = self.0.native.closed().await;
        if self.0.owner.is_cancelled() {
            return CloseReason::OwnerRetired;
        }
        if let SessionError::WebTransportError(WebTransportError::Closed(code, reason)) = result {
            return CloseReason::Remote(CloseInfo { code, reason });
        }
        if matches!(
            result,
            SessionError::ConnectionError(quinn::ConnectionError::LocallyClosed)
        ) && let Some(info) = self.0.local_close.lock().clone()
        {
            return CloseReason::Local(info);
        }
        CloseReason::Failed(result)
    }

    fn send_stream(&self, stream: web_transport_quinn::SendStream) -> SendStream {
        SendStream {
            stream,
            _session: self.clone(),
        }
    }

    fn recv_stream(&self, stream: web_transport_quinn::RecvStream) -> RecvStream {
        RecvStream {
            stream,
            _session: self.clone(),
        }
    }
}

pub struct SendStream {
    stream: web_transport_quinn::SendStream,
    _session: Session,
}

impl std::ops::Deref for SendStream {
    type Target = web_transport_quinn::SendStream;
    fn deref(&self) -> &Self::Target {
        &self.stream
    }
}

impl std::ops::DerefMut for SendStream {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.stream
    }
}

pub struct RecvStream {
    stream: web_transport_quinn::RecvStream,
    _session: Session,
}

impl std::ops::Deref for RecvStream {
    type Target = web_transport_quinn::RecvStream;
    fn deref(&self) -> &Self::Target {
        &self.stream
    }
}

impl std::ops::DerefMut for RecvStream {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.stream
    }
}
