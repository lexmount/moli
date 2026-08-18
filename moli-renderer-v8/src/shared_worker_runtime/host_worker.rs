use std::sync::Arc;

use tokio::sync::mpsc;

use crate::worker::{
    WorkerDevToolsHandle, WorkerGlobalKind, WorkerMessage, WorkerSpawnOptions,
    spawn_worker_with_options,
};

use super::{
    host::{RendererSharedWorkerHost, RendererSharedWorkerHostState},
    loading::{SharedWorkerLaunchParams, SharedWorkerLoadedScript},
    pump::drain_shared_worker_parent_messages,
};

impl RendererSharedWorkerHost {
    pub(super) fn start_running(
        &self,
        script: SharedWorkerLoadedScript,
        params: SharedWorkerLaunchParams,
    ) -> bool {
        let script_source = script.source;
        let script_url = script.script_url;
        let launch_context = params.launch_context;
        let execution_policy = launch_context.execution_policy;
        let name = launch_context.name.clone();
        let policy_context = script
            .response_policy_context
            .unwrap_or(execution_policy.policy_context);
        let reserved_service_worker_client_id = params.reserved_service_worker_client_id;
        let options = WorkerSpawnOptions::new_with_request_client(
            script_source,
            script_url.clone(),
            launch_context.request_client,
        )
        .with_script_kind(execution_policy.script_kind)
        .with_module_static_import_initiator_url(
            execution_policy.module_static_import_initiator_url,
        )
        .with_module_static_import_content_security_policies(
            execution_policy.module_static_import_content_security_policies,
        )
        .with_referrer_policy(script.response_referrer_policy)
        .with_content_security_policies(script.response_content_security_policies)
        .with_content_security_report_only_policies(
            script.response_content_security_report_only_policies,
        )
        .with_content_security_reporting_endpoints(
            script.response_content_security_reporting_endpoints,
        )
        .with_module_credentials_mode(execution_policy.module_credentials_mode)
        .with_network_policy(execution_policy.network_policy)
        .with_policy_context(policy_context)
        .with_worker_context_runtime(execution_policy.worker_context_runtime.clone())
        .with_global_kind(WorkerGlobalKind::Shared {
            name: name.clone(),
            storage_key: params.key.storage_key().clone(),
        })
        .with_storage_key_top_level_site(execution_policy.storage_key_top_level_site)
        .with_creator_storage_key(params.key.storage_key().clone())
        .with_indexed_db_manager(execution_policy.indexed_db_manager)
        .with_storage_bucket_store(execution_policy.storage_bucket_store);
        let options = if let Some(runtime) = execution_policy.service_worker_runtime {
            options.with_service_worker_runtime(runtime)
        } else {
            options
        };
        let options = if let Some(client_id) = reserved_service_worker_client_id {
            options.with_reserved_service_worker_client_id(client_id)
        } else {
            options
        };
        let mut handle = spawn_worker_with_options(options);
        let tx = handle.tx.clone();
        let parent_rx = handle.take_receiver();
        let mut state = self.state.lock();
        if !matches!(*state, RendererSharedWorkerHostState::Loading { .. }) {
            drop(state);
            handle.terminate_and_join();
            return false;
        }
        self.set_current_script_url(script_url);
        *state = RendererSharedWorkerHostState::Running {
            tx,
            handle: Some(handle),
            parent_rx,
        };
        true
    }

    #[cfg(test)]
    pub(super) fn is_closed(&self) -> bool {
        matches!(*self.state.lock(), RendererSharedWorkerHostState::Closed)
    }

    pub(super) fn start_parent_message_pump(self: &Arc<Self>) {
        let rx = {
            let mut state = self.state.lock();
            match &mut *state {
                RendererSharedWorkerHostState::Running { parent_rx, .. } => parent_rx.take(),
                RendererSharedWorkerHostState::Loading { .. }
                | RendererSharedWorkerHostState::Closed => None,
            }
        };
        drain_shared_worker_parent_messages(Arc::clone(self), self.current_script_url(), rx);
    }

    fn running_tx(&self) -> Option<mpsc::UnboundedSender<WorkerMessage>> {
        let state = self.state.lock();
        match &*state {
            RendererSharedWorkerHostState::Running { tx, .. } => Some(tx.clone()),
            RendererSharedWorkerHostState::Loading { .. }
            | RendererSharedWorkerHostState::Closed => None,
        }
    }

    pub(super) fn running_devtools_handle(&self) -> Option<WorkerDevToolsHandle> {
        let state = self.state.lock();
        match &*state {
            RendererSharedWorkerHostState::Running {
                handle: Some(handle),
                ..
            } => Some(handle.devtools_handle()),
            RendererSharedWorkerHostState::Loading { .. }
            | RendererSharedWorkerHostState::Running { handle: None, .. }
            | RendererSharedWorkerHostState::Closed => None,
        }
    }

    pub(super) fn send_worker_message(&self, message: WorkerMessage) -> bool {
        let Some(tx) = self.running_tx() else {
            return false;
        };
        tx.send(message).is_ok()
    }

    pub(super) fn terminate_and_join(&self) {
        let (task, handle) = {
            let mut state = self.state.lock();
            match &mut *state {
                RendererSharedWorkerHostState::Loading { task } => {
                    let task = task.take();
                    *state = RendererSharedWorkerHostState::Closed;
                    (task, None)
                }
                RendererSharedWorkerHostState::Running { handle, .. } => {
                    let handle = handle.take();
                    *state = RendererSharedWorkerHostState::Closed;
                    (None, handle)
                }
                RendererSharedWorkerHostState::Closed => (None, None),
            }
        };
        if let Some(task) = task {
            task.cancel();
        }
        if let Some(handle) = handle {
            handle.terminate_and_join();
        }
    }

    pub(super) fn terminate_without_join(&self) {
        let (task, handle) = {
            let mut state = self.state.lock();
            match &mut *state {
                RendererSharedWorkerHostState::Loading { task } => {
                    let task = task.take();
                    *state = RendererSharedWorkerHostState::Closed;
                    (task, None)
                }
                RendererSharedWorkerHostState::Running { handle, .. } => {
                    let handle = handle.take();
                    *state = RendererSharedWorkerHostState::Closed;
                    (None, handle)
                }
                RendererSharedWorkerHostState::Closed => (None, None),
            }
        };
        if let Some(task) = task {
            task.cancel();
        }
        if let Some(handle) = handle {
            handle.terminate();
        }
    }
}
