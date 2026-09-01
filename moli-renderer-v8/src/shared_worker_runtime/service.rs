use std::{
    fmt,
    sync::{Arc, OnceLock, Weak},
};

use moli_shared_worker::{
    SharedWorkerClientId, SharedWorkerClientOwnerId, SharedWorkerClientRemoval,
    SharedWorkerConnectAction, SharedWorkerDescriptor, SharedWorkerInstanceId,
    SharedWorkerInstanceRemoval, SharedWorkerKey, SharedWorkerLoadFailure, SharedWorkerLoadReady,
    SharedWorkerRegistryDiagnostics,
};
use parking_lot::Mutex;
use tracing::trace;

use crate::runtime::RendererOwnerLocalHostId;

use super::{
    host::SharedRendererSharedWorkerHost,
    instances::SharedWorkerHostStore,
    matching::{SharedWorkerClientOwnerIdAllocator, SharedWorkerMatchingStore},
    owner_wake::{
        SharedWorkerOwnerWake, SharedWorkerRuntimeOwnerWake, SharedWorkerRuntimeOwnerWakeSender,
    },
    service_lane::SharedWorkerServiceLane,
    target_output_streams::SharedWorkerTargetOutputStreams,
};

pub(crate) fn new_shared_worker_runtime_service_with_client_owner_id_allocator(
    client_owner_id_allocator: SharedWorkerClientOwnerIdAllocator,
) -> SharedWorkerRuntimeService {
    SharedWorkerRuntimeService {
        inner: Arc::new(SharedWorkerRuntimeInner {
            matching: Arc::new(SharedWorkerMatchingStore::with_client_owner_id_allocator(
                client_owner_id_allocator,
            )),
            ..SharedWorkerRuntimeInner::default()
        }),
    }
}

#[derive(Clone)]
pub(crate) struct SharedWorkerRuntimeService {
    inner: Arc<SharedWorkerRuntimeInner>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct WeakSharedWorkerRuntimeService {
    inner: Weak<SharedWorkerRuntimeInner>,
}

#[derive(Default)]
struct SharedWorkerRuntimeInner {
    matching: Arc<SharedWorkerMatchingStore>,
    hosts: Arc<SharedWorkerHostStore>,
    service_lane: Arc<SharedWorkerServiceLane>,
    owner_wake: SharedWorkerOwnerWake,
    owner_local_host_id: Mutex<Option<RendererOwnerLocalHostId>>,
    target_output_streams: OnceLock<SharedWorkerTargetOutputStreams>,
}

impl SharedWorkerRuntimeService {
    pub(crate) fn client_owner_id_allocator(&self) -> SharedWorkerClientOwnerIdAllocator {
        self.inner.matching.client_owner_id_allocator()
    }

    pub(crate) fn configure_target_output_streams(
        &self,
        browser_context_runtime_id: crate::runtime::RendererBrowserContextRuntimeId,
        transport: crate::runtime::RendererOutputTransportSenderSlot,
    ) {
        self.inner
            .target_output_streams
            .set(SharedWorkerTargetOutputStreams::new(
                browser_context_runtime_id,
                transport,
            ))
            .unwrap_or_else(|_| {
                panic!("SharedWorker target output streams configured more than once")
            });
    }

    pub(crate) fn bind_target_output_transport(
        &self,
        transport: crate::runtime::RendererOutputTransportSender,
    ) {
        self.target_output_streams().bind_transport(transport);
    }

    pub(super) fn open_target_output_stream(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> crate::runtime::RendererTurnOutputJournal {
        self.target_output_streams().open(instance_id)
    }

    pub(super) fn retire_target_output_stream(&self, instance_id: SharedWorkerInstanceId) {
        self.target_output_streams().retire(instance_id);
    }

    fn target_output_streams(&self) -> &SharedWorkerTargetOutputStreams {
        self.inner
            .target_output_streams
            .get()
            .expect("SharedWorker runtime must be bound to a BrowserContext before use")
    }

    #[cfg(test)]
    pub(super) fn ensure_target_output_streams_for_test(&self) {
        let _ = self
            .inner
            .target_output_streams
            .set(SharedWorkerTargetOutputStreams::new(
                crate::runtime::RendererBrowserContextRuntimeId::new_for_testing(0),
                crate::runtime::RendererOutputTransportSenderSlot::default(),
            ));
    }

    pub(crate) fn downgrade(&self) -> WeakSharedWorkerRuntimeService {
        WeakSharedWorkerRuntimeService {
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub(super) fn service_lane(&self) -> &SharedWorkerServiceLane {
        self.inner.service_lane.as_ref()
    }

    pub(crate) fn add_owner_wake_sender(&self, sender: SharedWorkerRuntimeOwnerWakeSender) {
        self.inner.owner_wake.add_owner_wake_sender(sender.clone());
        if self.pending_service_lane_event_count() > 0 {
            sender.signal(SharedWorkerRuntimeOwnerWake::ServiceLane);
        }
    }

    pub(crate) fn set_owner_local_host_id(&self, owner_local_host_id: RendererOwnerLocalHostId) {
        *self.inner.owner_local_host_id.lock() = Some(owner_local_host_id);
    }

    pub(super) fn owner_local_host_id(&self) -> Option<RendererOwnerLocalHostId> {
        *self.inner.owner_local_host_id.lock()
    }

    pub(super) fn required_owner_local_host_id(&self) -> RendererOwnerLocalHostId {
        self.owner_local_host_id()
            .expect("SharedWorker runtime must be attached to a renderer owner before use")
    }

    pub(super) fn signal_service_lane_wake(&self) -> bool {
        self.inner.owner_wake.signal_service_lane_wake()
    }

    #[cfg(test)]
    pub(crate) fn next_client_owner_id(&self) -> SharedWorkerClientOwnerId {
        self.inner.matching.next_client_owner_id()
    }

    pub(super) fn connect_matching(
        &self,
        key: SharedWorkerKey,
        descriptor: SharedWorkerDescriptor,
        client_owner_id: SharedWorkerClientOwnerId,
    ) -> SharedWorkerConnectAction<SharedRendererSharedWorkerHost> {
        self.inner
            .matching
            .connect(key, descriptor, client_owner_id)
    }

    pub(super) fn finish_loading_matching(
        &self,
        key: &SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
        host: SharedRendererSharedWorkerHost,
    ) -> SharedWorkerLoadReady<SharedRendererSharedWorkerHost> {
        self.inner.matching.finish_loading(key, instance_id, host)
    }

    pub(super) fn fail_loading_matching(
        &self,
        key: &SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerLoadFailure {
        self.inner.matching.fail_loading(key, instance_id)
    }

    pub(super) fn remove_matching_client(
        &self,
        client_id: SharedWorkerClientId,
    ) -> SharedWorkerClientRemoval<SharedRendererSharedWorkerHost> {
        self.inner.matching.remove_client(client_id)
    }

    pub(super) fn remove_matching_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerInstanceRemoval<SharedRendererSharedWorkerHost> {
        self.inner.matching.remove_instance(instance_id)
    }

    pub(super) fn remove_all_matching_instances(
        &self,
    ) -> Vec<SharedWorkerInstanceRemoval<SharedRendererSharedWorkerHost>> {
        self.inner.matching.remove_all_instances()
    }

    pub(super) fn running_matching_host(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<SharedRendererSharedWorkerHost> {
        self.inner.matching.running_host(instance_id)
    }

    pub(super) fn loading_clients_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Vec<SharedWorkerClientId> {
        self.inner
            .matching
            .loading_clients_for_instance(instance_id)
    }

    pub(super) fn loading_host(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<SharedRendererSharedWorkerHost> {
        self.inner.hosts.loading_host(instance_id)
    }

    pub(super) fn insert_loading_host(
        &self,
        instance_id: SharedWorkerInstanceId,
        host: SharedRendererSharedWorkerHost,
    ) {
        self.inner.hosts.insert_loading_host(instance_id, host);
    }

    pub(super) fn remove_loading_host(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<SharedRendererSharedWorkerHost> {
        self.inner.hosts.remove_loading_host(instance_id)
    }

    pub(super) fn take_context_shutdown_hosts_from_stores(
        &self,
    ) -> Vec<super::host_removal::SharedWorkerRemovedHost> {
        self.inner
            .hosts
            .take_context_shutdown_hosts(|| self.remove_all_matching_instances())
    }

    pub(super) fn matching_diagnostics(&self) -> SharedWorkerRegistryDiagnostics {
        self.inner.matching.diagnostics()
    }

    pub(super) fn loading_host_count(&self) -> usize {
        self.inner.hosts.loading_host_count()
    }

    #[cfg(test)]
    pub(super) fn matching_clients_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Vec<SharedWorkerClientId> {
        self.inner.matching.clients_for_instance(instance_id)
    }

    #[cfg(test)]
    pub(super) fn matching_is_empty(&self) -> bool {
        self.inner.matching.is_empty()
    }

    #[cfg(test)]
    pub(super) fn active_owner_ids_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Vec<SharedWorkerClientOwnerId> {
        self.inner
            .matching
            .active_owner_ids_for_instance(instance_id)
    }

    #[cfg(test)]
    pub(super) fn owner_lifecycle_is_empty(&self) -> bool {
        self.inner.matching.owner_lifecycle_is_empty()
    }

    #[cfg(test)]
    pub(super) fn loading_hosts_empty(&self) -> bool {
        self.inner.hosts.is_empty()
    }
}

impl Default for SharedWorkerRuntimeService {
    fn default() -> Self {
        Self {
            inner: Arc::new(SharedWorkerRuntimeInner::default()),
        }
    }
}

impl WeakSharedWorkerRuntimeService {
    pub(super) fn upgrade(&self) -> Option<SharedWorkerRuntimeService> {
        self.inner
            .upgrade()
            .map(|inner| SharedWorkerRuntimeService { inner })
    }

    #[cfg(test)]
    pub(super) fn is_alive(&self) -> bool {
        self.inner.strong_count() > 0
    }

    pub(super) fn signal_service_lane_wake(&self) -> bool {
        let Some(service) = self.upgrade() else {
            trace!("dropping shared worker service lane wake because runtime service is gone");
            return false;
        };
        service.signal_service_lane_wake()
    }
}

impl fmt::Debug for SharedWorkerRuntimeService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedWorkerRuntimeService")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use moli_shared_worker::SharedWorkerInstanceId;

    use crate::shared_worker_runtime::{SharedWorkerRuntimeOwnerWake, test_support};

    #[test]
    fn new_owner_wake_sender_receives_pending_service_lane_after_stale_sender() {
        let service = test_support::runtime_service();
        let stale_rx = test_support::install_owner_wake_sender(&service);
        let instance_id = SharedWorkerInstanceId::from_u64(31);
        drop(stale_rx);

        assert!(
            service
                .downgrade()
                .enqueue_service_lane_worker_closed(instance_id)
        );
        assert!(
            !service.downgrade().signal_service_lane_wake(),
            "the stale sender should be pruned without delivering the pending service-lane wake"
        );
        assert_eq!(service.pending_service_lane_event_count(), 1);

        let mut fresh_rx = test_support::install_owner_wake_sender(&service);

        assert!(
            matches!(
                fresh_rx.try_recv(),
                Ok(SharedWorkerRuntimeOwnerWake::ServiceLane)
            ),
            "a replacement owner must be woken for service-lane events that were queued while no live owner sender existed"
        );
        assert_eq!(service.drain_service_lane(), 1);
        assert_eq!(service.pending_service_lane_event_count(), 0);
    }
}
