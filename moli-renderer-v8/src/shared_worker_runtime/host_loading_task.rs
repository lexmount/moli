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
    cancel_wait: tokio::sync::oneshot::Sender<()>,
}

impl SharedWorkerLoadingTask {
    pub(super) fn pending(
        cancel_handle: FetchCancelHandle,
        cancel_wait: tokio::sync::oneshot::Sender<()>,
    ) -> Self {
        Self {
            cancel_handle,
            cancel_wait,
        }
    }

    pub(super) fn cancel(self) {
        // Let the physical collector settle with its exact received prefix.
        // Only the controller's pending response needs a separate wake.
        self.cancel_handle.cancel();
        let _ = self.cancel_wait.send(());
    }
}

pub(super) fn spawn_shared_worker_loading_task(
    host: Arc<RendererSharedWorkerHost>,
    params: SharedWorkerLaunchParams,
    fetch: SharedWorkerScriptFetch,
    cancel_handle: FetchCancelHandle,
    cancel_wait: tokio::sync::oneshot::Receiver<()>,
    network: Arc<crate::network::ResourceTransfer>,
) {
    let task_runner = params
        .launch_context
        .execution_policy
        .worker_context_runtime
        .resource_task_runner()
        .expect("BrowserContext must select a resource executor before loading a SharedWorker");
    task_runner.clone().spawn(async move {
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
            cancel_handle,
            cancel_wait,
            &network,
        )
        .await;
        if let Err(message) = &result {
            network.failed(&crate::network::ResourceResponseFailure::Request(
                message.clone(),
            ));
        }
        host.enqueue_loading_completion(params, result);
    })
}
