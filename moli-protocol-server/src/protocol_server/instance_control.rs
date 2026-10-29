use super::instance_shutdown::{InstanceConnection, InstanceShutdown};

/// An operation on the entire running browser instance, independent of a
/// protocol session or renderer's command queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstanceCommand {
    Shutdown,
}

/// A thread-safe control handle for one invocation of
/// [`super::ProtocolServer::serve_with_control`].
///
/// The server owns shutdown and resource cleanup. Adapters and native callers
/// only request it; cloning this handle neither creates a task nor keeps a
/// protocol session or renderer alive.
#[derive(Clone, Debug, Default)]
pub struct BrowserInstanceControl {
    shutdown: InstanceShutdown,
}

impl BrowserInstanceControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Submit an idempotent lifecycle command without waiting for page work.
    /// Wait for the server's serving future to finish to observe cleanup.
    pub fn request(&self, command: InstanceCommand) {
        match command {
            InstanceCommand::Shutdown => self.shutdown.request(),
        }
    }

    /// Observe shutdown, including a request made before this call.
    pub async fn shutdown_requested(&self) {
        self.shutdown.requested().await;
    }

    pub(super) fn connection(&self) -> Option<InstanceConnection> {
        self.shutdown.connection()
    }

    pub(super) async fn drained(&self) {
        self.shutdown.drained().await;
    }
}
