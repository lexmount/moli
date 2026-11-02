//! Native access to the running browser, revoked by instance shutdown.
use std::{fmt, sync::Arc};

use moli_core::ace_page_task::AcePageTask;
pub use moli_protocol::ace_page_task::{
    AcePageTaskError, AcePageTaskResult, BrowserSnapshot, BrowserTargetSnapshot,
};

use super::{
    AppState, BrowserInstanceControl, ProtocolServer, SharedCdpAgentHostDirectory,
    SharedCdpOwnerRegistry,
};

#[derive(Clone)]
pub(super) struct RuntimeReadyCallback(Arc<dyn Fn(BrowserRuntimeHandle) + Send + Sync>);

impl fmt::Debug for RuntimeReadyCallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeReadyCallback(..)")
    }
}

impl ProtocolServer {
    /// Notify the embedder once per serving invocation, after browser state and
    /// the protocol listener are initialized. The callback must return promptly;
    /// the embedder owns any additional services and their supervision.
    pub fn with_runtime_ready(
        mut self,
        callback: impl Fn(BrowserRuntimeHandle) + Send + Sync + 'static,
    ) -> Self {
        self.runtime_ready = Some(RuntimeReadyCallback(Arc::new(callback)));
        self
    }

    pub(super) fn notify_runtime_ready(&self, state: &AppState) {
        if let Some(callback) = &self.runtime_ready {
            (callback.0)(BrowserRuntimeHandle {
                directory: state.cdp_agent_host_directory.clone(),
                registry: state.cdp_owner_registry.clone(),
                control: state.control.clone(),
            });
        }
    }
}

/// Each call resolves its target afresh. Clones grant no page ownership and
/// stop admitting work as soon as instance shutdown is requested.
#[derive(Clone)]
pub struct BrowserRuntimeHandle {
    pub(super) directory: SharedCdpAgentHostDirectory,
    pub(super) registry: SharedCdpOwnerRegistry,
    pub(super) control: BrowserInstanceControl,
}

impl BrowserRuntimeHandle {
    pub async fn targets(
        &self,
    ) -> Result<Vec<moli_protocol::DevToolsTargetInfo>, AcePageTaskError> {
        let _lease = self
            .control
            .connection()
            .ok_or_else(|| AcePageTaskError::new(404, "Browser has stopped"))?;
        let endpoint = self
            .registry
            .shared_owner()
            .map_err(|error| AcePageTaskError::new(404, error))?;
        tokio::select! {
            biased;
            _ = self.control.shutdown_requested() => return Err(AcePageTaskError::new(404, "Browser has stopped")),
            result = endpoint.ensure_default_target_published() => result.map_err(|error| AcePageTaskError::new(404, error))?,
        }
        Ok(self.directory.remote_debugging_target_infos(false))
    }

    pub async fn run_ace_page_task(
        &self,
        target_id: &str,
        task: Box<dyn AcePageTask>,
    ) -> Result<AcePageTaskResult, AcePageTaskError> {
        let _lease = self
            .control
            .connection()
            .ok_or_else(|| AcePageTaskError::new(404, "Browser has stopped"))?;
        let route = self
            .directory
            .lookup_target(target_id)
            .ok_or_else(|| AcePageTaskError::new(404, "Page target is unavailable"))?;
        tokio::select! {
            biased;
            _ = self.control.shutdown_requested() => Err(AcePageTaskError::new(404, "Browser has stopped")),
            result = route.endpoint.run_ace_page_task(target_id.to_owned(), task) => result,
        }
    }
}

#[cfg(test)]
mod tests;
