use std::sync::Arc;

use tokio::sync::watch;

/// Server-wide graceful shutdown signal for the CDP protocol server.
///
/// A valid browser-level `Browser.close` command requests shutdown through
/// this handle. The signal is deliberately process-lifetime state owned by the
/// protocol server rather than any CDP owner, so target/page frontends cannot
/// reach it and the outward transport only observes a single idempotent latch.
#[derive(Clone)]
pub(crate) struct ShutdownCoordinator {
    shutdown_tx: Arc<watch::Sender<bool>>,
}

impl ShutdownCoordinator {
    pub(crate) fn new() -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            shutdown_tx: Arc::new(shutdown_tx),
        }
    }

    /// Requests graceful server shutdown. Repeated requests are idempotent.
    pub(crate) fn request(&self) {
        self.shutdown_tx.send_if_modified(|requested| {
            if *requested {
                return false;
            }
            *requested = true;
            true
        });
    }

    /// Resolves once shutdown has been requested.
    pub(super) async fn wait(&self) {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        if *shutdown_rx.borrow() {
            return;
        }
        let _ = shutdown_rx.changed().await;
    }

    #[cfg(test)]
    pub(crate) fn is_requested(&self) -> bool {
        *self.shutdown_tx.borrow()
    }
}
