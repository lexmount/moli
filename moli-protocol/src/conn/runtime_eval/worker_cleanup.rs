use super::*;
use anyhow::{Context as _, Result};

/// One native teardown receipt, irrespective of how many Inspector references
/// the session owns. No Browser-side object/group enumeration is involved.
pub(crate) struct SessionWorkerCleanup {
    receipt: Option<tokio::sync::oneshot::Receiver<Result<()>>>,
}

pub(crate) struct CompletedSessionWorkerCleanup {
    result: Result<()>,
}

impl CdpConnection {
    pub(crate) fn prepare_session_worker_cleanup(
        &mut self,
        session_id: &str,
    ) -> SessionWorkerCleanup {
        let receipt = match self.session_route(Some(session_id)) {
            Some(CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            }) => self
                .browser_context_by_id(&browser_context_id)
                .and_then(|context| {
                    let target = context.shared_worker_target(&target_id)?;
                    context
                        .renderer_runtime()
                        .detach_shared_worker_runtime_inspector_session(
                            target.renderer_instance_id,
                            Some(session_id.to_owned()),
                        )
                }),
            Some(CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            }) => self
                .browser_context_by_id(&browser_context_id)
                .and_then(|context| {
                    let target = context.dedicated_worker_target(&target_id)?;
                    context
                        .renderer_runtime()
                        .detach_dedicated_worker_runtime_inspector_session(
                            target.renderer_instance_id,
                            Some(session_id.to_owned()),
                        )
                }),
            Some(CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id,
                target_id,
            }) => self
                .browser_context_by_id(&browser_context_id)
                .and_then(|context| {
                    let target = context.service_worker_target(&target_id)?;
                    context
                        .renderer_runtime()
                        .detach_service_worker_runtime_inspector_session(
                            target.renderer_version_id,
                            Some(session_id.to_owned()),
                        )
                }),
            _ => None,
        };
        SessionWorkerCleanup { receipt }
    }
}

impl SessionWorkerCleanup {
    pub(crate) async fn wait(self) -> CompletedSessionWorkerCleanup {
        let result = match self.receipt {
            Some(receipt) => receipt
                .await
                .context("Worker Inspector teardown receipt was cancelled")
                .and_then(|result| result),
            // A Worker with no live renderer cannot hold a native session.
            None => Ok(()),
        };
        CompletedSessionWorkerCleanup { result }
    }
}

impl CompletedSessionWorkerCleanup {
    pub(crate) fn finish(self) -> Result<()> {
        self.result
    }
}
