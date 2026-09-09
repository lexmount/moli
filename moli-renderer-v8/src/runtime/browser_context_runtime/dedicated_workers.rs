use crate::runtime::RendererWorkerInspectionEndpoint;
use crate::worker::{WorkerDevToolsHandle, WorkerHandle};

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

    pub(crate) fn attach_dedicated_worker_devtools_handle(
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
            },
        );
    }

    pub(crate) fn unregister_dedicated_worker_devtools_handle(&self, instance_id: u64) {
        self.inner
            .dedicated_worker_devtools_targets
            .lock()
            .remove(&instance_id);
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

    pub(super) fn dedicated_worker_inspection_endpoint(
        &self,
        instance_id: u64,
    ) -> Option<RendererWorkerInspectionEndpoint> {
        let targets = self.inner.dedicated_worker_devtools_targets.lock();
        let target = targets.get(&instance_id)?;
        Some(RendererWorkerInspectionEndpoint::new(
            target.handle.clone(),
            target.output_journal.clone(),
            "DedicatedWorkerRuntimeUnavailable",
        ))
    }

    pub fn run_dedicated_worker_if_waiting_for_debugger_for_devtools(
        &self,
        instance_id: u64,
    ) -> bool {
        self.dedicated_worker_devtools_handle(instance_id)
            .is_some_and(|handle| handle.run_if_waiting_for_debugger())
    }

    pub fn close_dedicated_worker_for_devtools(&self, instance_id: u64) -> bool {
        self.dedicated_worker_devtools_handle(instance_id)
            .is_some_and(|handle| handle.terminate_for_devtools())
    }
}
