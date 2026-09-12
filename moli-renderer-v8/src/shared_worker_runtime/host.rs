use std::{collections::HashMap, sync::Arc};

use moli_shared_worker::{SharedWorkerClientId, SharedWorkerInstanceId};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::worker::{WorkerHandle, WorkerMessage, WorkerToParentMessage};

use super::{
    client::RendererSharedWorkerClient, host_loading_task::SharedWorkerLoadingTask,
    service::WeakSharedWorkerRuntimeService,
};

pub(super) enum SharedWorkerOutputPublicationState {
    Active,
    Closing(Vec<crate::runtime::RendererRuntimeInspectorResponsePublication>),
    Retired {
        terminal_predecessor: Option<crate::runtime::RendererOutputFence>,
    },
}

pub(super) struct RendererSharedWorkerHost {
    instance_id: SharedWorkerInstanceId,
    owner_local_host_id: crate::runtime::RendererOwnerLocalHostId,
    script_url: Mutex<String>,
    name: String,
    runtime_service: WeakSharedWorkerRuntimeService,
    pub(super) state: Mutex<RendererSharedWorkerHostState>,
    pub(super) clients: Mutex<HashMap<SharedWorkerClientId, RendererSharedWorkerClient>>,
    target_output: crate::runtime::RendererTurnOutputJournal,
    pub(super) worker_lifecycle: crate::runtime::RendererWorkerLifecycleReporter,
    pub(super) output_publications: Mutex<SharedWorkerOutputPublicationState>,
}

pub(super) enum RendererSharedWorkerHostState {
    Loading {
        task: Option<SharedWorkerLoadingTask>,
    },
    Running {
        tx: mpsc::UnboundedSender<WorkerMessage>,
        handle: Option<WorkerHandle>,
        parent_rx: Option<mpsc::UnboundedReceiver<WorkerToParentMessage>>,
    },
    Closed,
}

pub(super) type SharedRendererSharedWorkerHost = Arc<RendererSharedWorkerHost>;

impl RendererSharedWorkerHost {
    pub(super) fn new_loading(
        instance_id: SharedWorkerInstanceId,
        owner_local_host_id: crate::runtime::RendererOwnerLocalHostId,
        runtime_service: WeakSharedWorkerRuntimeService,
        initial_script_url: String,
        name: String,
        target_output: crate::runtime::RendererTurnOutputJournal,
        worker_lifecycle: crate::runtime::RendererWorkerLifecycleReporter,
    ) -> Self {
        Self {
            instance_id,
            owner_local_host_id,
            script_url: Mutex::new(initial_script_url),
            name,
            runtime_service,
            state: Mutex::new(RendererSharedWorkerHostState::Loading { task: None }),
            clients: Mutex::new(HashMap::new()),
            target_output,
            worker_lifecycle,
            output_publications: Mutex::new(SharedWorkerOutputPublicationState::Active),
        }
    }

    pub(super) fn instance_id(&self) -> SharedWorkerInstanceId {
        self.instance_id
    }

    pub(super) fn owner_local_host_id(&self) -> crate::runtime::RendererOwnerLocalHostId {
        self.owner_local_host_id
    }

    pub(super) fn runtime_service(&self) -> &WeakSharedWorkerRuntimeService {
        &self.runtime_service
    }

    pub(super) fn current_script_url(&self) -> String {
        self.script_url.lock().clone()
    }

    pub(super) fn set_current_script_url(&self, script_url: String) {
        *self.script_url.lock() = script_url;
    }

    pub(super) fn worker_name(&self) -> String {
        self.name.clone()
    }

    pub(super) fn target_output(&self) -> &crate::runtime::RendererTurnOutputJournal {
        &self.target_output
    }

    #[cfg(test)]
    pub(super) fn target_output_retired(&self) -> bool {
        matches!(
            *self.output_publications.lock(),
            SharedWorkerOutputPublicationState::Retired { .. }
        )
    }
}
