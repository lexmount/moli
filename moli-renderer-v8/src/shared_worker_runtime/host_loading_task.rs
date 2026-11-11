use std::sync::Arc;

use moli_fetch::FetchCancelHandle;

use super::{
    host::RendererSharedWorkerHost,
    loading::{
        SharedWorkerLaunchParams, SharedWorkerScriptFetch, fetch_shared_worker_script_source_async,
    },
};

pub(super) struct SharedWorkerLoadingTask {
    cancel_handle: FetchCancelHandle,
    join_handle: Option<tokio::task::JoinHandle<()>>,
}

impl SharedWorkerLoadingTask {
    pub(super) fn pending(cancel_handle: FetchCancelHandle) -> Self {
        Self {
            cancel_handle,
            join_handle: None,
        }
    }

    pub(super) fn set_join_handle(&mut self, join_handle: tokio::task::JoinHandle<()>) {
        self.join_handle = Some(join_handle);
    }

    pub(super) fn cancel(self) {
        self.cancel_handle.cancel();
        if let Some(join_handle) = self.join_handle {
            join_handle.abort();
        }
    }
}

pub(super) fn spawn_shared_worker_loading_task(
    host: Arc<RendererSharedWorkerHost>,
    params: SharedWorkerLaunchParams,
    fetch: SharedWorkerScriptFetch,
    cancel_handle: FetchCancelHandle,
) -> tokio::task::JoinHandle<()> {
    let task_runner = fetch.task_runner.clone();
    fetch.task_runner.spawn_abortable(async move {
        let result = fetch_shared_worker_script_source_async(
            &fetch.request_client,
            task_runner,
            &fetch.script_url,
            &fetch.initiator_url,
            fetch.request_policy,
            params
                .launch_context
                .execution_policy
                .service_worker_runtime
                .clone(),
            params.reserved_service_worker_client_id,
            params.creator_service_worker_client_id,
            cancel_handle,
        )
        .await;
        host.enqueue_loading_completion(params, result);
    })
}
