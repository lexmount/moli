mod registration;

use crate::structured_clone::V8StructuredClonePayload;
use crate::worker::WorkerHandle;
use moli_storage_key::MoliStorageKey;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerOwnerScope {
    Top,
    Child(crate::document_runtime::DomHandle),
    LightweightPopup(u64),
}

pub(super) enum WorkerExecutionState {
    Loading {
        pending_messages: Vec<V8StructuredClonePayload>,
        load_task: Option<tokio::task::JoinHandle<()>>,
        terminated: bool,
        /// Outside-settings load captured from the exact creator Document
        /// when `new Worker()` starts.
        ///
        /// Besides freezing the request client, the lease binds cancellation
        /// and task execution to that exact Document. Completion may run
        /// after navigation, but can never rebind to the then-current main
        /// Document.
        outside_settings_load: crate::network::loads::ResourceLoadLease,
        name: String,
        module_credentials_mode: moli_fetch::RequestCredentialsMode,
        storage_key_top_level_site: String,
        creator_storage_key: MoliStorageKey,
        reserved_service_worker_client_id:
            Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    },
    Running {
        handle: WorkerHandle,
    },
}

#[derive(Default)]
pub(super) struct WorkerRelayTerminalState {
    client_source_drained: bool,
    host_bridge_drained: bool,
}

impl WorkerRelayTerminalState {
    pub(super) fn mark_client_source_drained(&mut self) {
        self.client_source_drained = true;
    }

    pub(super) fn mark_host_bridge_drained(&mut self) {
        self.host_bridge_drained = true;
    }

    pub(super) const fn is_fully_drained(&self) -> bool {
        self.client_source_drained && self.host_bridge_drained
    }
}

pub(super) struct WorkerConnectionState {
    pub(super) renderer_instance_id: u64,
    pub(super) target_created: bool,
    pub(super) wrapper: v8::Global<v8::Object>,
    pub(super) owner: super::WindowExecutionContextBinding,
    pub(super) client_event_producer:
        crate::page_task_queue::RendererPageDedicatedWorkerClientEventProducer,
    pub(super) relay_terminal: WorkerRelayTerminalState,
    pub(super) execution: WorkerExecutionState,
}
