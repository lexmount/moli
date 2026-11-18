use anyhow::{Context as _, Result};
use tokio::sync::oneshot;

use super::{DedicatedWorkerTarget, RendererBrowserContextRuntime};
use crate::runtime::{
    RendererRuntimeInspectorMessage, RendererRuntimeInspectorResponseSender, WorkerInspectorError,
};
use crate::worker::WorkerControlHandle;

impl RendererBrowserContextRuntime {
    pub fn set_shared_worker_pause_on_start_for_devtools(&self, pause: bool) {
        self.inner
            .shared_worker_pause_on_start_for_devtools
            .store(pause, std::sync::atomic::Ordering::Release);
    }
    pub fn set_dedicated_worker_pause_on_start_for_devtools(&self, pause: bool) {
        self.inner
            .dedicated_worker_pause_on_start_for_devtools
            .store(pause, std::sync::atomic::Ordering::Relaxed);
    }

    fn dedicated_worker_control(&self, instance_id: u64) -> Option<WorkerControlHandle> {
        self.inner
            .dedicated_worker_targets
            .lock()
            .get(&instance_id)
            .and_then(|target| match target {
                DedicatedWorkerTarget::Running { handle, .. } => Some(handle.clone()),
                DedicatedWorkerTarget::Loading { .. } => None,
            })
    }

    fn dedicated_worker_target(&self, instance_id: u64) -> Option<DedicatedWorkerTarget> {
        self.inner
            .dedicated_worker_targets
            .lock()
            .get(&instance_id)
            .cloned()
    }

    pub async fn dispatch_dedicated_worker_runtime_protocol_message(
        &self,
        instance_id: u64,
        inspector_session_id: Option<String>,
        raw_json: String,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        self.dispatch_dedicated_worker_runtime_protocol_message_with_optional_deferred_response(
            instance_id,
            inspector_session_id,
            raw_json,
            None,
        )
        .await
    }

    pub async fn dispatch_dedicated_worker_runtime_protocol_message_with_deferred_response(
        &self,
        instance_id: u64,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        self.dispatch_dedicated_worker_runtime_protocol_message_with_optional_deferred_response(
            instance_id,
            inspector_session_id,
            raw_json,
            Some(deferred_response),
        )
        .await
    }

    pub async fn dispatch_dedicated_worker_runtime_protocol_message_with_devtools_session_response(
        &self,
        instance_id: u64,
        inspector_session_id: String,
        raw_json: String,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Result<crate::runtime::CompletedWorkerRuntimeInspectorCommandDispatch> {
        let Some(DedicatedWorkerTarget::Running {
            handle,
            output_journal,
        }) = self.dedicated_worker_target(instance_id)
        else {
            return Err(WorkerInspectorError::DedicatedUnavailable.into());
        };
        let Some(output_journal) = output_journal else {
            return Err(WorkerInspectorError::DedicatedUnavailable.into());
        };
        let response = response
            .route_to_worker_devtools_session_output(inspector_session_id.clone(), output_journal);
        let error_response = response.clone();
        let settlement = response
            .take_session_response_settlement_receiver()
            .expect("a Worker DevTools response must own one settlement receiver");
        let (response_tx, response_rx) = oneshot::channel();
        let dispatched = handle.dispatch_runtime_protocol_message(
            Some(inspector_session_id),
            raw_json,
            Some(response),
            response_tx,
        );
        let dispatch = if dispatched {
            response_rx
                .await
                .unwrap_or_else(|_| Err(WorkerInspectorError::DedicatedUnavailable.into()))
        } else {
            Err(WorkerInspectorError::DedicatedUnavailable.into())
        };
        Ok(
            crate::runtime::CompletedWorkerRuntimeInspectorCommandDispatch::finish(
                dispatch,
                settlement,
                error_response,
            ),
        )
    }

    async fn dispatch_dedicated_worker_runtime_protocol_message_with_optional_deferred_response(
        &self,
        instance_id: u64,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let Some(handle) = self.dedicated_worker_control(instance_id) else {
            return Err(WorkerInspectorError::DedicatedUnavailable.into());
        };
        let (response_tx, response_rx) = oneshot::channel();
        if !handle.dispatch_runtime_protocol_message(
            inspector_session_id,
            raw_json,
            deferred_response,
            response_tx,
        ) {
            return Err(WorkerInspectorError::DedicatedUnavailable.into());
        }
        response_rx
            .await
            .context(WorkerInspectorError::DedicatedUnavailable)?
    }

    pub fn attach_dedicated_worker_runtime_inspector_session(
        &self,
        instance_id: u64,
        inspector_session_id: Option<String>,
    ) -> bool {
        let mut targets = self.inner.dedicated_worker_targets.lock();
        match targets.get_mut(&instance_id) {
            Some(DedicatedWorkerTarget::Loading { inspector_sessions }) => {
                if !inspector_sessions.contains(&inspector_session_id) {
                    inspector_sessions.push(inspector_session_id);
                }
                true
            }
            Some(DedicatedWorkerTarget::Running { handle, .. }) => {
                handle.attach_runtime_inspector_session(inspector_session_id)
            }
            None => false,
        }
    }

    pub fn detach_dedicated_worker_runtime_inspector_session(
        &self,
        instance_id: u64,
        inspector_session_id: Option<String>,
    ) -> Option<tokio::sync::oneshot::Receiver<anyhow::Result<()>>> {
        let mut targets = self.inner.dedicated_worker_targets.lock();
        match targets.get_mut(&instance_id)? {
            DedicatedWorkerTarget::Loading { inspector_sessions } => {
                inspector_sessions.retain(|session| session != &inspector_session_id);
                let (tx, rx) = oneshot::channel();
                let _ = tx.send(Ok(()));
                Some(rx)
            }
            DedicatedWorkerTarget::Running { handle, .. } => {
                Some(handle.detach_runtime_inspector_session(inspector_session_id))
            }
        }
    }

    pub fn run_dedicated_worker_if_waiting_for_debugger_for_devtools(
        &self,
        instance_id: u64,
    ) -> bool {
        self.dedicated_worker_control(instance_id)
            .is_some_and(|handle| handle.run_if_waiting_for_debugger())
    }

    pub fn close_dedicated_worker_for_devtools(&self, instance_id: u64) -> bool {
        self.dedicated_worker_control(instance_id)
            .is_some_and(|handle| handle.terminate())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loading_worker_retirement_removes_pending_inspector_attachments() {
        let runtime = RendererBrowserContextRuntime::new();
        let instance = runtime.allocate_dedicated_worker_instance_id();
        assert!(
            runtime.attach_dedicated_worker_runtime_inspector_session(
                instance,
                Some("retired".into())
            )
        );
        assert!(
            runtime
                .attach_dedicated_worker_runtime_inspector_session(instance, Some("live".into()))
        );
        assert!(
            runtime
                .attach_dedicated_worker_runtime_inspector_session(instance, Some("live".into()))
        );
        runtime
            .detach_dedicated_worker_runtime_inspector_session(instance, Some("retired".into()))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        {
            let targets = runtime.inner.dedicated_worker_targets.lock();
            let DedicatedWorkerTarget::Loading { inspector_sessions } = &targets[&instance] else {
                panic!("worker is still loading");
            };
            assert_eq!(inspector_sessions, &[Some("live".into())]);
        }
        runtime.unregister_dedicated_worker(instance);
        assert!(
            !runtime
                .attach_dedicated_worker_runtime_inspector_session(instance, Some("late".into()))
        );
        assert!(runtime.inner.dedicated_worker_targets.lock().is_empty());
    }
}
