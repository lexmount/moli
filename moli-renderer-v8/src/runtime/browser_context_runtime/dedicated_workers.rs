use crate::runtime::WorkerInspectorError;
use anyhow::{Context as _, Result};
use tokio::sync::oneshot;

use crate::runtime::{RendererRuntimeInspectorMessage, RendererRuntimeInspectorResponseSender};
use crate::worker::{WorkerDevToolsHandle, WorkerHandle, WorkerSpawnOptions, WorkerStartup};

use super::{DedicatedWorkerDevToolsTarget, RendererBrowserContextRuntime};

impl RendererBrowserContextRuntime {
    /// Allocates a browser-context-unique identity for one DedicatedWorker
    /// lifetime. The JavaScript-facing worker id is Page-local and therefore
    /// cannot safely key protocol targets after Page replacement.
    pub(crate) fn allocate_dedicated_worker_instance_id(&self) -> u64 {
        self.inner
            .next_dedicated_worker_instance_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .saturating_add(1)
    }

    pub(crate) fn register_loading_dedicated_worker_devtools_target(
        &self,
        instance_id: u64,
        output_journal: Option<crate::runtime::RendererTurnOutputJournal>,
    ) {
        let startup = WorkerStartup::new();
        self.inner.dedicated_worker_devtools_targets.lock().insert(
            instance_id,
            DedicatedWorkerDevToolsTarget {
                handle: startup.devtools_handle(),
                output_journal,
                startup: Some(startup),
            },
        );
    }

    pub(crate) fn register_running_dedicated_worker_devtools_target(
        &self,
        instance_id: u64,
        handle: &WorkerHandle,
        output_journal: Option<crate::runtime::RendererTurnOutputJournal>,
    ) {
        self.inner.dedicated_worker_devtools_targets.lock().insert(
            instance_id,
            DedicatedWorkerDevToolsTarget {
                handle: handle.devtools_handle(),
                output_journal,
                startup: None,
            },
        );
    }

    pub(crate) fn start_dedicated_worker(
        &self,
        instance_id: u64,
        options: WorkerSpawnOptions,
    ) -> Option<WorkerHandle> {
        let mut targets = self.inner.dedicated_worker_devtools_targets.lock();
        let Some(startup) = targets
            .get_mut(&instance_id)
            .and_then(|target| target.startup.take())
        else {
            drop(targets);
            if let Some(client_id) = options.reserved_service_worker_client_id {
                self.unregister_service_worker_client(client_id);
            }
            return None;
        };
        // A retired target or a second start cannot recreate the endpoint.
        Some(startup.spawn(options))
    }

    pub(crate) fn unregister_dedicated_worker_devtools_handle(&self, instance_id: u64) {
        let target = self
            .inner
            .dedicated_worker_devtools_targets
            .lock()
            .remove(&instance_id);
        if let Some(target) = target {
            target.handle.dispose(WorkerInspectorError::Closed);
        }
    }

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

    pub(crate) fn dedicated_worker_pause_on_start_for_devtools(&self) -> bool {
        self.inner
            .dedicated_worker_pause_on_start_for_devtools
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn dedicated_worker_devtools_handle(&self, instance_id: u64) -> Option<WorkerDevToolsHandle> {
        self.inner
            .dedicated_worker_devtools_targets
            .lock()
            .get(&instance_id)
            .map(|target| target.handle.clone())
    }

    fn dedicated_worker_devtools_target(
        &self,
        instance_id: u64,
    ) -> Option<(
        WorkerDevToolsHandle,
        Option<crate::runtime::RendererTurnOutputJournal>,
    )> {
        self.inner
            .dedicated_worker_devtools_targets
            .lock()
            .get(&instance_id)
            .map(|target| (target.handle.clone(), target.output_journal.clone()))
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
        let Some((handle, output_journal)) = self.dedicated_worker_devtools_target(instance_id)
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
        let Some(handle) = self.dedicated_worker_devtools_handle(instance_id) else {
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
        inspector_session_id: String,
    ) -> bool {
        self.dedicated_worker_devtools_handle(instance_id)
            .is_some_and(|handle| {
                handle.attach_runtime_inspector_session(Some(inspector_session_id))
            })
    }

    pub fn detach_dedicated_worker_runtime_inspector_session(
        &self,
        instance_id: u64,
        inspector_session_id: String,
    ) -> Option<oneshot::Receiver<Result<()>>> {
        self.dedicated_worker_devtools_handle(instance_id)
            .map(|handle| handle.detach_runtime_inspector_session(Some(inspector_session_id)))
    }

    pub fn run_dedicated_worker_if_waiting_for_debugger_for_devtools(
        &self,
        instance_id: u64,
    ) -> bool {
        self.running_dedicated_worker_devtools_handle(instance_id)
            .is_some_and(|handle| handle.run_if_waiting_for_debugger())
    }

    pub fn close_dedicated_worker_for_devtools(&self, instance_id: u64) -> bool {
        self.running_dedicated_worker_devtools_handle(instance_id)
            .is_some_and(|handle| handle.terminate_for_devtools())
    }

    fn running_dedicated_worker_devtools_handle(
        &self,
        instance_id: u64,
    ) -> Option<WorkerDevToolsHandle> {
        self.inner
            .dedicated_worker_devtools_targets
            .lock()
            .get(&instance_id)
            .filter(|target| target.startup.is_none())
            .map(|target| target.handle.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loading_worker_retirement_removes_pending_inspector_attachments() {
        let runtime = RendererBrowserContextRuntime::new();
        let instance = runtime.allocate_dedicated_worker_instance_id();
        runtime.register_loading_dedicated_worker_devtools_target(instance, None);
        assert!(
            runtime.attach_dedicated_worker_runtime_inspector_session(instance, "retired".into())
        );
        assert!(runtime.attach_dedicated_worker_runtime_inspector_session(instance, "live".into()));
        assert!(runtime.attach_dedicated_worker_runtime_inspector_session(instance, "live".into()));
        runtime
            .detach_dedicated_worker_runtime_inspector_session(instance, "retired".into())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        {
            let targets = runtime.inner.dedicated_worker_devtools_targets.lock();
            let target = &targets[&instance];
            assert!(target.startup.is_some(), "worker is still loading");
            assert_eq!(target.handle.inspector_tasks().session_count(), 1);
        }
        let pending = runtime.dispatch_dedicated_worker_runtime_protocol_message(
            instance,
            Some("live".into()),
            r#"{"id":1,"method":"Runtime.enable"}"#.into(),
        );
        let mut pending = std::pin::pin!(pending);
        assert!(futures_util::poll!(&mut pending).is_pending());
        runtime.unregister_dedicated_worker_devtools_handle(instance);
        assert_eq!(
            pending
                .await
                .unwrap_err()
                .downcast::<WorkerInspectorError>()
                .unwrap(),
            WorkerInspectorError::Closed
        );
        assert!(
            !runtime.attach_dedicated_worker_runtime_inspector_session(instance, "late".into())
        );
        assert!(
            runtime
                .inner
                .dedicated_worker_devtools_targets
                .lock()
                .is_empty()
        );
    }
}
