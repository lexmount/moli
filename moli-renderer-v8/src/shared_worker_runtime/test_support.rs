use std::{cell::RefCell, sync::Arc};

use moli_shared_worker::{
    SharedWorkerClientId, SharedWorkerConnectAction, SharedWorkerDescriptor,
    SharedWorkerInstanceId, SharedWorkerInstanceRemoval, SharedWorkerKey, SharedWorkerLoadReady,
    SharedWorkerSameSiteCookies,
};
use moli_storage_key::{MoliStorageKey, StoragePartitionRelation};

use super::{
    host::{RendererSharedWorkerHost, SharedRendererSharedWorkerHost},
    owner_wake::{SharedWorkerRuntimeOwnerWake, shared_worker_owner_wake_channel},
    service::{SharedWorkerRuntimeService, WeakSharedWorkerRuntimeService},
};

pub(super) struct SharedWorkerPageClientHarness {
    sources: RefCell<crate::page_task_queue::RendererPageOwnedTaskSources>,
    owner: crate::message_port_runtime::MessagePortOwner,
    shared_worker_client_event_realm:
        crate::page_task_queue::RendererPageSharedWorkerClientEventRealmSender,
    worker_host_bridge_sender: crate::page_task_queue::RendererWorkerHostBridgeEventSender,
    _wake_rx: tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
}

impl SharedWorkerPageClientHarness {
    pub(super) fn new() -> Self {
        let page_id = crate::PageId::new_for_testing(9201);
        let root_document = crate::runtime::RendererDocumentToken::new_for_testing(page_id, 1);
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let (sources, routes) = crate::page_task_queue::RendererPageOwnedTaskSources::new(
            crate::page_task_queue::PageRuntimeWakeSignal::default(),
            crate::page_task_queue::RendererOwnerWakeSender::new(
                wake_tx,
                crate::runtime::RendererPageToken::new_for_testing(page_id),
            ),
        );
        let execution_context = crate::native_bridge::WindowExecutionContextIdentity::new(
            crate::native_bridge::WindowExecutionContextOwner::Frame(
                crate::frame_owner_model::LocalWindowId(1),
            ),
            crate::native_bridge::OwnerDispatchScope::Top,
            crate::native_bridge::RuntimeObservableContextToken::from_raw(1),
            crate::native_bridge::WindowExecutionContextAccessPolicy::EnforceWebOrigin,
        );
        let owner = crate::message_port_runtime::MessagePortOwner::Page(
            routes
                .message_port_delivery_sender(root_document)
                .bind_execution_context(execution_context),
        );
        let shared_worker_client_event_realm = routes
            .shared_worker_client_event_sender(root_document)
            .bind_execution_context(execution_context);
        let worker_host_bridge_sender = routes.worker_host_bridge_event_sender(root_document);
        Self {
            sources: RefCell::new(sources),
            owner,
            shared_worker_client_event_realm,
            worker_host_bridge_sender,
            _wake_rx: wake_rx,
        }
    }

    pub(super) fn owner(&self) -> crate::message_port_runtime::MessagePortOwner {
        self.owner.clone()
    }

    pub(super) fn shared_worker_client_event_realm(
        &self,
    ) -> crate::page_task_queue::RendererPageSharedWorkerClientEventRealmSender {
        self.shared_worker_client_event_realm.clone()
    }

    pub(super) fn shared_worker_client_event_producer(
        &self,
        client_id: SharedWorkerClientId,
    ) -> crate::page_task_queue::RendererPageSharedWorkerClientEventProducer {
        self.shared_worker_client_event_realm.bind_client(client_id)
    }

    pub(super) fn worker_host_bridge_sender(
        &self,
    ) -> crate::page_task_queue::RendererWorkerHostBridgeEventSender {
        self.worker_host_bridge_sender.clone()
    }

    pub(super) fn pop_shared_worker_client_event(
        &self,
    ) -> Option<crate::page_task_queue::RendererPageSharedWorkerClientEventTask> {
        let descriptor =
            self.sources
                .borrow_mut()
                .ready_descriptors()
                .into_iter()
                .find(|descriptor| {
                    matches!(
                    descriptor,
                    crate::page_task_queue::RendererPageReadyDescriptor::SharedWorkerClientEvent {
                        ..
                    }
                )
                })?;
        let task = self.sources.borrow_mut().take_task(descriptor);
        let crate::page_task_queue::RendererPageSchedulerTask::SharedWorkerClientEvent(task) = task
        else {
            panic!("SharedWorker descriptor dequeued a different task variant")
        };
        Some(task)
    }

    pub(super) fn shared_worker_client(
        &self,
        client_id: SharedWorkerClientId,
        client_port_id: crate::types::MessagePortId,
        worker_port_id: crate::types::MessagePortId,
        message_port_registry: crate::message_port_runtime::SharedMessagePortRegistry,
    ) -> crate::shared_worker_runtime::client::RendererSharedWorkerClient {
        crate::shared_worker_runtime::client::RendererSharedWorkerClient {
            client_port_id,
            worker_port_id,
            message_port_registry,
            client_event_producer: self.shared_worker_client_event_producer(client_id),
            worker_host_bridge_sender: self.worker_host_bridge_sender(),
        }
    }
}

pub(super) fn page_message_port_pair(
    registry: &crate::message_port_runtime::SharedMessagePortRegistry,
) -> (
    crate::types::MessagePortId,
    crate::types::MessagePortId,
    SharedWorkerPageClientHarness,
) {
    let owner = SharedWorkerPageClientHarness::new();
    let (client_port_id, worker_port_id) =
        registry.create_entangled_message_port_pair(owner.owner());
    (client_port_id, worker_port_id, owner)
}

pub(super) fn shared_worker_key() -> SharedWorkerKey {
    SharedWorkerKey::new(
        MoliStorageKey::new(
            "https://example.test".to_owned(),
            "https://example.test".to_owned(),
            None,
            StoragePartitionRelation::FirstParty,
        ),
        "https://example.test/worker.js".to_owned(),
        "loader".to_owned(),
        SharedWorkerSameSiteCookies::All,
    )
}

pub(super) fn loading_host(
    instance_id: SharedWorkerInstanceId,
    key: &SharedWorkerKey,
) -> Arc<RendererSharedWorkerHost> {
    let target_output = crate::runtime::RendererTurnOutputJournal::new(
        crate::runtime::RendererOutputStreamIdentity::new_shared_worker(
            crate::runtime::RendererBrowserContextRuntimeId::new_for_testing(0),
            instance_id.as_u64(),
        ),
    );
    Arc::new(RendererSharedWorkerHost::new_loading(
        instance_id,
        crate::runtime::RendererOwnerLocalHostId::new_for_testing(0),
        WeakSharedWorkerRuntimeService::default(),
        key.script_url().to_owned(),
        "loader".to_owned(),
        target_output,
    ))
}

pub(super) fn loading_host_with_runtime_service(
    instance_id: SharedWorkerInstanceId,
    key: &SharedWorkerKey,
    runtime_service: &SharedWorkerRuntimeService,
) -> Arc<RendererSharedWorkerHost> {
    runtime_service.ensure_target_output_streams_for_test();
    Arc::new(RendererSharedWorkerHost::new_loading(
        instance_id,
        runtime_service
            .owner_local_host_id()
            .unwrap_or_else(|| crate::runtime::RendererOwnerLocalHostId::new_for_testing(0)),
        runtime_service.downgrade(),
        key.script_url().to_owned(),
        "loader".to_owned(),
        runtime_service.open_target_output_stream(instance_id),
    ))
}

pub(super) fn runtime_service() -> SharedWorkerRuntimeService {
    let service = SharedWorkerRuntimeService::default();
    service.set_owner_local_host_id(crate::runtime::RendererOwnerLocalHostId::new_for_testing(0));
    service
}

pub(super) fn store_loading_host(
    runtime_service: &SharedWorkerRuntimeService,
    instance_id: SharedWorkerInstanceId,
    host: Arc<RendererSharedWorkerHost>,
) {
    runtime_service.insert_loading_host(instance_id, host);
}

pub(super) fn stored_loading_host(
    runtime_service: &SharedWorkerRuntimeService,
    instance_id: SharedWorkerInstanceId,
) -> Option<Arc<RendererSharedWorkerHost>> {
    runtime_service.loading_host(instance_id)
}

pub(super) fn loading_hosts_empty(runtime_service: &SharedWorkerRuntimeService) -> bool {
    runtime_service.loading_hosts_empty()
}

pub(super) fn connect_matching(
    runtime_service: &SharedWorkerRuntimeService,
    key: SharedWorkerKey,
    descriptor: SharedWorkerDescriptor,
) -> SharedWorkerConnectAction<SharedRendererSharedWorkerHost> {
    runtime_service.connect_matching(key, descriptor, runtime_service.next_client_owner_id())
}

pub(super) fn finish_loading_matching(
    runtime_service: &SharedWorkerRuntimeService,
    key: &SharedWorkerKey,
    instance_id: SharedWorkerInstanceId,
    host: SharedRendererSharedWorkerHost,
) -> SharedWorkerLoadReady<SharedRendererSharedWorkerHost> {
    runtime_service.finish_loading_matching(key, instance_id, host)
}

pub(super) fn remove_all_instances_matching(
    runtime_service: &SharedWorkerRuntimeService,
) -> Vec<SharedWorkerInstanceRemoval<SharedRendererSharedWorkerHost>> {
    runtime_service.remove_all_matching_instances()
}

pub(super) fn matching_clients_for_instance(
    runtime_service: &SharedWorkerRuntimeService,
    instance_id: SharedWorkerInstanceId,
) -> Vec<SharedWorkerClientId> {
    runtime_service.matching_clients_for_instance(instance_id)
}

pub(super) fn matching_is_empty(runtime_service: &SharedWorkerRuntimeService) -> bool {
    runtime_service.matching_is_empty()
}

pub(super) fn active_owner_ids_for_instance(
    runtime_service: &SharedWorkerRuntimeService,
    instance_id: SharedWorkerInstanceId,
) -> Vec<moli_shared_worker::SharedWorkerClientOwnerId> {
    runtime_service.active_owner_ids_for_instance(instance_id)
}

pub(super) fn owner_lifecycle_is_empty(runtime_service: &SharedWorkerRuntimeService) -> bool {
    runtime_service.owner_lifecycle_is_empty()
}

pub(super) fn install_owner_wake_sender(
    runtime_service: &SharedWorkerRuntimeService,
) -> tokio::sync::mpsc::UnboundedReceiver<SharedWorkerRuntimeOwnerWake> {
    let (sender, receiver) = shared_worker_owner_wake_channel();
    runtime_service.add_owner_wake_sender(sender);
    receiver
}
