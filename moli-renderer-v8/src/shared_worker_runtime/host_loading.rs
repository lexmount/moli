use moli_fetch::FetchCancelHandle;
use std::sync::Arc;

use super::{
    host::{RendererSharedWorkerHost, RendererSharedWorkerHostState},
    host_loading_task::{SharedWorkerLoadingTask, spawn_shared_worker_loading_task},
    load_completion::SharedWorkerRuntimeCompletion,
    loading::{SharedWorkerLaunchParams, SharedWorkerScriptFetch},
};

impl RendererSharedWorkerHost {
    pub(super) fn begin_loading_task(&self) -> FetchCancelHandle {
        let cancel_handle = FetchCancelHandle::new();
        let mut state = self.state.lock();
        if let RendererSharedWorkerHostState::Loading { task } = &mut *state {
            *task = Some(SharedWorkerLoadingTask::pending(cancel_handle.clone()));
        }
        cancel_handle
    }

    pub(super) fn record_loading_task_handle(&self, handle: tokio::task::JoinHandle<()>) {
        let mut handle = Some(handle);
        {
            let mut state = self.state.lock();
            if let RendererSharedWorkerHostState::Loading { task: Some(task) } = &mut *state {
                task.set_join_handle(handle.take().expect("handle not moved"));
            }
        }
        if let Some(handle) = handle {
            handle.abort();
        }
    }

    pub(super) fn start_script_fetch(
        self: &Arc<Self>,
        params: SharedWorkerLaunchParams,
        fetch: SharedWorkerScriptFetch,
    ) -> Result<(), String> {
        let cancel_handle = self.begin_loading_task();
        let handle =
            spawn_shared_worker_loading_task(Arc::clone(self), params, fetch, cancel_handle);
        self.record_loading_task_handle(handle);
        Ok(())
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
            self.retire_unstarted_output();
        }
    }

    pub(super) fn close_completed_loading(&self) {
        let mut state = self.state.lock();
        let retired_loading = if matches!(*state, RendererSharedWorkerHostState::Loading { .. }) {
            // The loader already produced this completion; do not
            // abort/join the loading task from its own completion path.
            *state = RendererSharedWorkerHostState::Closed;
            true
        } else {
            false
        };
        drop(state);
        if retired_loading {
            self.retire_unstarted_output();
        }
    }
}
