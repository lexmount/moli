use moli_shared_worker::SharedWorkerInstanceId;

use super::service::SharedWorkerRuntimeService;
use crate::runtime::RendererWorkerInspectionEndpoint;

impl SharedWorkerRuntimeService {
    pub(crate) fn inspection_endpoint(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<RendererWorkerInspectionEndpoint> {
        let host = self.running_host_for_instance(instance_id)?;
        Some(RendererWorkerInspectionEndpoint::new(
            host.running_devtools_handle()?,
            Some(host.target_output().clone()),
            "SharedWorkerRuntimeUnavailable",
        ))
    }
}
