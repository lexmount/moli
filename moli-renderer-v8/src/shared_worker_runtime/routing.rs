use std::sync::Arc;

use moli_shared_worker::SharedWorkerInstanceId;

use super::{
    host::SharedRendererSharedWorkerHost,
    service::{SharedWorkerRuntimeService, WeakSharedWorkerRuntimeService},
};

impl SharedWorkerRuntimeService {
    pub(crate) fn client_document(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<(
        crate::runtime::RendererOwnerLocalHostId,
        crate::runtime::RendererDocumentToken,
    )> {
        self.running_host_for_instance(instance_id)?
            .worker_host_bridge_sender()
            .map(|client| client.document_source())
    }

    pub(super) fn running_host_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<SharedRendererSharedWorkerHost> {
        self.running_matching_host(instance_id)
    }
}

impl WeakSharedWorkerRuntimeService {
    pub(super) fn running_host_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<SharedRendererSharedWorkerHost> {
        self.upgrade()
            .and_then(|service| service.running_host_for_instance(instance_id))
    }

    pub(super) fn is_running_host(&self, host: &SharedRendererSharedWorkerHost) -> bool {
        self.running_host_for_instance(host.instance_id())
            .is_some_and(|running| Arc::ptr_eq(&running, host))
    }
}
