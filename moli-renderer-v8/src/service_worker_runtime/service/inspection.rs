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
        let ServiceWorkerVersionRunningState::Running { host } = &version.running_state else {
            return None;
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
}
