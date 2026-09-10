//! Submission capability for a specific owner, without thread ownership.

use super::{
    CurlWebSocketConnection, CurlWebSocketRequest, SESSION_CAPACITY, connection::SessionIo,
};
use crate::{CurlTransferId, runtime::identity::next_transfer_id};
use anyhow::{Context, Result, bail};
use curl::multi::MultiWaker;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Semaphore;

pub(crate) struct Submission {
    pub(super) id: CurlTransferId,
    pub(super) request: CurlWebSocketRequest,
    pub(super) io: SessionIo,
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
    #[cfg(test)]
    pub(super) fn available_session_slots(&self) -> usize {
        self.inner.slots.available_permits()
    }

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
        let (connection, io) = CurlWebSocketConnection::channel(id, self.inner.waker.clone(), slot);
        self.inner
            .submissions
            .try_send(Submission { id, request, io })
            .map_err(|_| anyhow::anyhow!("curl WebSocket runtime cannot accept a session"))?;
        let _ = self.inner.waker.wakeup();
        Ok(connection)
    }
}
