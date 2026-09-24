use moli_shared_worker::{
    SharedWorkerClientId, SharedWorkerClientRemoval, SharedWorkerConnectAction,
    SharedWorkerDescriptor, SharedWorkerInstanceId, SharedWorkerInstanceRemoval, SharedWorkerKey,
    SharedWorkerLoadFailure, SharedWorkerLoadReady, SharedWorkerRegistry,
    SharedWorkerRegistryDiagnostics,
};

use super::host::SharedRendererSharedWorkerHost;

#[derive(Default)]
pub(super) struct SharedWorkerMatchingStore {
    registry: SharedWorkerRegistry<SharedRendererSharedWorkerHost>,
}

impl SharedWorkerMatchingStore {
    pub(super) fn connect(
        &self,
        key: SharedWorkerKey,
        descriptor: SharedWorkerDescriptor,
    ) -> SharedWorkerConnectAction<SharedRendererSharedWorkerHost> {
        self.registry.connect(key, descriptor)
    }

    pub(super) fn finish_loading(
        &self,
        key: &SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
        host: SharedRendererSharedWorkerHost,
    ) -> SharedWorkerLoadReady<SharedRendererSharedWorkerHost> {
        self.registry.finish_loading(key, instance_id, host)
    }

    pub(super) fn fail_loading(
        &self,
        key: &SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerLoadFailure {
        self.registry.fail_loading(key, instance_id)
    }

    pub(super) fn remove_client(
        &self,
        client_id: SharedWorkerClientId,
    ) -> SharedWorkerClientRemoval<SharedRendererSharedWorkerHost> {
        self.registry.remove_client(client_id)
    }

    pub(super) fn remove_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerInstanceRemoval<SharedRendererSharedWorkerHost> {
        self.registry.remove_instance(instance_id)
    }

    pub(super) fn remove_all_instances(
        &self,
    ) -> Vec<SharedWorkerInstanceRemoval<SharedRendererSharedWorkerHost>> {
        self.registry.remove_all_instances()
    }

    pub(super) fn running_host(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<SharedRendererSharedWorkerHost> {
        self.registry.running_instance(instance_id)
    }

    pub(super) fn diagnostics(&self) -> SharedWorkerRegistryDiagnostics {
        self.registry.diagnostics()
    }

    #[cfg(test)]
    pub(super) fn clients_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Vec<SharedWorkerClientId> {
        self.registry.clients_for_instance(instance_id)
    }

    pub(super) fn loading_clients_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Vec<SharedWorkerClientId> {
        self.registry.loading_clients_for_instance(instance_id)
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }
}
