use super::super::{ids::ServiceWorkerVersionId, version::ServiceWorkerVersionRunningState};
use super::ServiceWorkerRuntimeService;
use crate::runtime::{RendererServiceWorkerRunIdentity, RendererWorkerInspectionEndpoint};

impl ServiceWorkerRuntimeService {
    pub(crate) fn inspection_endpoint(
        &self,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
    ) -> Option<RendererWorkerInspectionEndpoint> {
        let state = self.inner.state.lock();
        let version = state.versions.get(&version_id)?;
        let host = match &version.running_state {
            ServiceWorkerVersionRunningState::Starting { host }
            | ServiceWorkerVersionRunningState::Running { host } => host,
            ServiceWorkerVersionRunningState::Stopped => return None,
        };
        if &host.run_identity() != run {
            return None;
        }
        Some(RendererWorkerInspectionEndpoint::new(
            host.inspection_handle()?,
            state.target_output_journal(version_id),
            "ServiceWorkerRuntimeUnavailable",
        ))
    }

    pub(in crate::service_worker_runtime) fn record_worker_execution_ready(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
    ) {
        self.inner
            .state
            .lock()
            .record_target_execution_ready(version_id, run);
    }
}
