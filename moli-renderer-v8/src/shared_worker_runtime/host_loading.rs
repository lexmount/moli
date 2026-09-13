use moli_fetch::FetchCancelHandle;
use std::sync::Arc;

use super::{
    host::{RendererSharedWorkerHost, RendererSharedWorkerHostState},
    host_loading_task::{SharedWorkerLoadingTask, spawn_shared_worker_loading_task},
    load_completion::SharedWorkerRuntimeCompletion,
    loading::{SharedWorkerLaunchParams, SharedWorkerScriptFetch},
};

impl RendererSharedWorkerHost {
    pub(super) fn begin_loading_task(
        &self,
    ) -> (FetchCancelHandle, tokio::sync::oneshot::Receiver<()>) {
        let cancel_handle = FetchCancelHandle::new();
        let (cancel_wait, cancelled) = tokio::sync::oneshot::channel();
        let mut state = self.state.lock();
        if let RendererSharedWorkerHostState::Loading { task } = &mut *state {
            *task = Some(SharedWorkerLoadingTask::pending(
                cancel_handle.clone(),
                cancel_wait,
            ));
        } else {
            cancel_handle.cancel();
        }
        (cancel_handle, cancelled)
    }

    pub(super) fn start_script_fetch(
        self: &Arc<Self>,
        params: SharedWorkerLaunchParams,
        fetch: SharedWorkerScriptFetch,
        network: Arc<crate::worker::WorkerResourceTransfer>,
    ) {
        let (cancel_handle, cancel_wait) = self.begin_loading_task();
        spawn_shared_worker_loading_task(
            Arc::clone(self),
            params,
            fetch,
            cancel_handle,
            cancel_wait,
            network,
        );
    }

    pub(super) fn enqueue_loading_completion(
        self: &Arc<Self>,
        params: SharedWorkerLaunchParams,
        result: Result<super::loading::SharedWorkerLoadedScript, String>,
    ) {
        let runtime_service = self.runtime_service().clone();
        let event = SharedWorkerRuntimeCompletion::script_load_finished(
            runtime_service.clone(),
            self.instance_id(),
            params,
            result,
        );
        if !runtime_service.enqueue_service_lane_completion(event) {
            return;
        }
        runtime_service.signal_service_lane_wake();
    }

    pub(super) fn cancel_loading(&self) {
        let (task, retired_loading) = {
            let mut state = self.state.lock();
            let mut task_to_cancel = None;
            let mut retired_loading = false;
            if let RendererSharedWorkerHostState::Loading { task } = &mut *state {
                task_to_cancel = task.take();
                *state = RendererSharedWorkerHostState::Closed;
                retired_loading = true;
            }
            (task_to_cancel, retired_loading)
        };
        if let Some(task) = task {
            task.cancel();
        }
        if retired_loading {
            self.publish_destroyed_target_event();
        }
    }

    pub(super) fn close_completed_loading(&self) {
        let mut state = self.state.lock();
        let retired_loading = if matches!(*state, RendererSharedWorkerHostState::Loading { .. }) {
            // The loader has already settled its request before this completion.
            *state = RendererSharedWorkerHostState::Closed;
            true
        } else {
            false
        };
        drop(state);
        if retired_loading {
            self.publish_destroyed_target_event();
        }
    }
}
