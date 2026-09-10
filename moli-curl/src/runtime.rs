//! Owns the common native thread; HTTP and WebSocket handles only submit work.

mod config;
pub(crate) mod diagnostics;
pub(crate) mod identity;
mod owner;
#[cfg(test)]
mod tests;

use crate::{CurlHttpSender, CurlMultiCompletion, CurlMultiJob, websocket::CurlWebSocketConnector};
use anyhow::{Context, Result};
use crossbeam_channel::Receiver;
use curl::easy::Handler;
use parking_lot::Mutex;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

pub use config::CurlMultiRuntimeConfig;
pub use identity::CurlTransferId;
use owner::CurlRuntimeOwner;

#[derive(Debug)]
pub(crate) enum CurlRuntimeCommand<H: Handler, C> {
    Request {
        transfer_id: CurlTransferId,
        job: CurlMultiJob<H, C>,
    },
    Shutdown,
}

/// Owns one native thread. Request handles cannot extend its lifetime or join it.
#[derive(Debug)]
pub struct CurlMultiRuntime<H: Handler + Send + 'static, C: Send + 'static> {
    http: CurlHttpSender<H, C>,
    websocket_connector: CurlWebSocketConnector,
    #[cfg(test)]
    owner_started: Arc<AtomicBool>,
    owner_handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl<H: Handler + Send + 'static, C: Send + 'static> CurlMultiRuntime<H, C> {
    pub fn new(
        config: CurlMultiRuntimeConfig,
    ) -> Result<(Self, Receiver<CurlMultiCompletion<H, C>>)> {
        config.validate()?;
        let (command_tx, command_rx) = crossbeam_channel::unbounded();
        let (completion_tx, completion_rx) = crossbeam_channel::unbounded();
        let (waker_tx, waker_rx) = crossbeam_channel::bounded(1);
        let (websocket_tx, websocket_rx) = CurlWebSocketConnector::channel();
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let owner_started = Arc::new(AtomicBool::new(false));
        let thread_name = config.thread_name.clone();
        let owner_shutdown = shutdown_requested.clone();
        #[cfg(test)]
        let started = owner_started.clone();
        let owner_handle = thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                CurlRuntimeOwner::run(
                    config,
                    command_rx,
                    completion_tx,
                    waker_tx,
                    owner_shutdown,
                    websocket_rx,
                    #[cfg(test)]
                    started,
                )
            })
            .context("failed to spawn curl multi runtime owner thread")?;
        let owner_waker = waker_rx
            .recv()
            .context("curl multi runtime owner did not publish a waker")?;
        let runtime = Self {
            websocket_connector: CurlWebSocketConnector::new(
                websocket_tx,
                owner_waker.clone(),
                shutdown_requested.clone(),
            ),
            http: CurlHttpSender {
                command_tx,
                owner_waker,
                shutdown_requested,
            },
            #[cfg(test)]
            owner_started,
            owner_handle: Mutex::new(Some(owner_handle)),
        };
        Ok((runtime, completion_rx))
    }

    pub fn http_sender(&self) -> CurlHttpSender<H, C> {
        self.http.clone()
    }

    pub fn shutdown(&self) {
        if !self.http.shutdown_requested.swap(true, Ordering::SeqCst) {
            let _ = self.http.command_tx.send(CurlRuntimeCommand::Shutdown);
            let _ = self.http.owner_waker.wakeup();
        }
        if let Some(owner_handle) = self.owner_handle.lock().take() {
            let _ = owner_handle.join();
        }
    }

    /// Creates WebSockets on this runtime's owner and Multi. The capability
    /// cannot keep the runtime alive or shut down unrelated HTTP transfers.
    pub fn websocket_connector(&self) -> CurlWebSocketConnector {
        self.websocket_connector.clone()
    }

    #[cfg(test)]
    pub fn owner_count_for_testing(&self) -> usize {
        usize::from(self.owner_started.load(Ordering::SeqCst))
    }
}

impl<H: Handler + Send + 'static, C: Send + 'static> Drop for CurlMultiRuntime<H, C> {
    fn drop(&mut self) {
        self.shutdown();
    }
}
