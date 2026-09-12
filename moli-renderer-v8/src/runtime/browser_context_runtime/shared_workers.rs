use crate::{
    shared_worker_runtime::{
        SharedWorkerLaunchParams, SharedWorkerRuntimeOwnerWake, SharedWorkerRuntimeOwnerWakeSender,
    },
    worker_owner_wake::WorkerOwnerWakeRoutes,
};
use moli_shared_worker::{
    SharedWorkerClientId, SharedWorkerClientOwnerId, SharedWorkerDescriptor, SharedWorkerInstanceId,
};
use parking_lot::Mutex;

use super::RendererBrowserContextRuntime;
use crate::runtime::RendererOwnerLocalHostId;

/// Defers the browser-context SharedWorker registry until the first actual
/// `connect_shared_worker` call. ID allocation and owner routing do not require
/// the registry.
pub(super) struct LazySharedWorkerRuntime {
    state: Mutex<LazySharedWorkerRuntimeState>,
    client_owner_id_allocator: crate::shared_worker_runtime::SharedWorkerClientOwnerIdAllocator,
    worker_lifecycle: crate::runtime::RendererWorkerLifecycleReporter,
    output_transport: crate::runtime::RendererOutputTransportSenderSlot,
}

enum LazySharedWorkerRuntimeState {
    Deferred {
        owner_wake_senders: WorkerOwnerWakeRoutes<SharedWorkerRuntimeOwnerWake>,
        owner_local_host_id: Option<RendererOwnerLocalHostId>,
    },
    Live(crate::shared_worker_runtime::SharedWorkerRuntimeService),
}

impl std::fmt::Debug for LazySharedWorkerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazySharedWorkerRuntime")
            .field("initialized", &self.is_initialized())
            .finish()
    }
}

impl LazySharedWorkerRuntime {
    pub(super) fn new(
        worker_lifecycle: crate::runtime::RendererWorkerLifecycleReporter,
        output_transport: crate::runtime::RendererOutputTransportSenderSlot,
    ) -> Self {
        Self {
            state: Mutex::new(LazySharedWorkerRuntimeState::Deferred {
                owner_wake_senders: WorkerOwnerWakeRoutes::default(),
                owner_local_host_id: None,
            }),
            client_owner_id_allocator: Default::default(),
            worker_lifecycle,
            output_transport,
        }
    }

    pub(super) fn from_service(
        service: crate::shared_worker_runtime::SharedWorkerRuntimeService,
        worker_lifecycle: crate::runtime::RendererWorkerLifecycleReporter,
        output_transport: crate::runtime::RendererOutputTransportSenderSlot,
    ) -> Self {
        service.configure_target_output_streams(worker_lifecycle.clone(), output_transport.clone());
        Self {
            client_owner_id_allocator: service.client_owner_id_allocator(),
            state: Mutex::new(LazySharedWorkerRuntimeState::Live(service)),
            worker_lifecycle,
            output_transport,
        }
    }

    pub(super) fn get_or_init(&self) -> crate::shared_worker_runtime::SharedWorkerRuntimeService {
        let mut state = self.state.lock();
        if let LazySharedWorkerRuntimeState::Live(service) = &*state {
            return service.clone();
        }
        let LazySharedWorkerRuntimeState::Deferred {
            owner_wake_senders,
            owner_local_host_id,
        } = &mut *state
        else {
            unreachable!();
        };
        let owner_wake_senders = std::mem::take(owner_wake_senders);
        let owner_local_host_id = *owner_local_host_id;
        let service = crate::shared_worker_runtime::
            new_shared_worker_runtime_service_with_client_owner_id_allocator(
                self.client_owner_id_allocator.clone(),
            );
        service.configure_target_output_streams(
            self.worker_lifecycle.clone(),
            self.output_transport.clone(),
        );
        for sender in owner_wake_senders.into_senders() {
            service.add_owner_wake_sender(sender);
        }
        if let Some(owner_local_host_id) = owner_local_host_id {
            service.set_owner_local_host_id(owner_local_host_id);
        }
        *state = LazySharedWorkerRuntimeState::Live(service.clone());
        service
    }

    pub(super) fn get(&self) -> Option<crate::shared_worker_runtime::SharedWorkerRuntimeService> {
        let state = self.state.lock();
        let LazySharedWorkerRuntimeState::Live(service) = &*state else {
            return None;
        };
        Some(service.clone())
    }

    pub(super) fn is_initialized(&self) -> bool {
        matches!(*self.state.lock(), LazySharedWorkerRuntimeState::Live(_))
    }

    pub(super) fn allocate_client_owner_id(&self) -> SharedWorkerClientOwnerId {
        self.client_owner_id_allocator.allocate()
    }

    pub(super) fn add_owner_wake_sender(&self, sender: SharedWorkerRuntimeOwnerWakeSender) {
        let mut state = self.state.lock();
        match &mut *state {
            LazySharedWorkerRuntimeState::Deferred {
                owner_wake_senders, ..
            } => owner_wake_senders.register(sender),
            LazySharedWorkerRuntimeState::Live(service) => service.add_owner_wake_sender(sender),
        }
    }

    pub(super) fn set_owner_local_host_id(&self, owner_local_host_id: RendererOwnerLocalHostId) {
        let mut state = self.state.lock();
        match &mut *state {
            LazySharedWorkerRuntimeState::Deferred {
                owner_local_host_id: slot,
                ..
            } => *slot = Some(owner_local_host_id),
            LazySharedWorkerRuntimeState::Live(service) => {
                service.set_owner_local_host_id(owner_local_host_id)
            }
        }
    }
}

impl RendererBrowserContextRuntime {
    pub(super) fn shared_worker_runtime_if_initialized(
        &self,
    ) -> Option<crate::shared_worker_runtime::SharedWorkerRuntimeService> {
        self.inner.shared_worker_runtime.get()
    }

    pub(crate) fn add_shared_worker_owner_wake_sender(
        &self,
        sender: SharedWorkerRuntimeOwnerWakeSender,
    ) {
        self.inner
            .shared_worker_runtime
            .add_owner_wake_sender(sender);
    }

    pub(crate) fn set_shared_worker_owner_local_host_id(
        &self,
        owner_local_host_id: RendererOwnerLocalHostId,
    ) {
        self.inner
            .shared_worker_runtime
            .set_owner_local_host_id(owner_local_host_id);
    }

    pub(crate) fn connect_shared_worker(
        &self,
        descriptor: SharedWorkerDescriptor,
        params: SharedWorkerLaunchParams,
    ) -> SharedWorkerClientId {
        self.inner
            .shared_worker_runtime
            .get_or_init()
            .connect(descriptor, params)
    }

    pub(crate) fn next_shared_worker_client_owner_id(&self) -> SharedWorkerClientOwnerId {
        self.inner.shared_worker_runtime.allocate_client_owner_id()
    }

    pub(crate) fn drain_shared_worker_service_lane(&self) -> usize {
        self.inner
            .shared_worker_runtime
            .get()
            .map_or(0, |runtime| runtime.drain_service_lane())
    }

    pub fn close_shared_worker(&self, instance_id: SharedWorkerInstanceId) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| runtime.close_instance(instance_id))
    }

    pub fn shared_worker_client_document(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<(
        RendererOwnerLocalHostId,
        crate::runtime::RendererDocumentToken,
    )> {
        self.shared_worker_runtime_if_initialized()?
            .client_document(instance_id)
    }

    pub(crate) fn remove_shared_worker_client(&self, client_id: SharedWorkerClientId) {
        if let Some(runtime) = self.shared_worker_runtime_if_initialized() {
            runtime.remove_client(client_id);
        }
    }
}

#[cfg(test)]
mod owner_wake_retirement_tests {
    use super::*;

    #[test]
    fn deferred_worker_routes_are_bounded_without_service_initialization() {
        let context = RendererBrowserContextRuntime::new_for_test();
        let (peer_tx, _peer_rx) = crate::shared_worker_runtime::shared_worker_owner_wake_channel();
        context.add_shared_worker_owner_wake_sender(peer_tx);
        for _ in 0..64 {
            let (sender, receiver) =
                crate::shared_worker_runtime::shared_worker_owner_wake_channel();
            context.add_shared_worker_owner_wake_sender(sender);
            {
                let state = context.inner.shared_worker_runtime.state.lock();
                let LazySharedWorkerRuntimeState::Deferred {
                    owner_wake_senders, ..
                } = &*state
                else {
                    panic!("wake registration must not initialize worker services");
                };
                assert_eq!(
                    owner_wake_senders.len_for_test(),
                    2,
                    "only the peer and current renderer remain"
                );
            }
            drop(receiver);
        }
        let (closed, receiver) = crate::shared_worker_runtime::shared_worker_owner_wake_channel();
        drop(receiver);
        context.add_shared_worker_owner_wake_sender(closed);
        let state = context.inner.shared_worker_runtime.state.lock();
        let LazySharedWorkerRuntimeState::Deferred {
            owner_wake_senders, ..
        } = &*state
        else {
            unreachable!();
        };
        assert_eq!(
            owner_wake_senders.len_for_test(),
            1,
            "closed admission must not reintroduce stale routes"
        );
    }
}
