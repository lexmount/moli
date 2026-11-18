use crate::worker::WorkerHandle;

use super::{DedicatedWorkerTarget, RendererBrowserContextRuntime};

#[cfg(feature = "devtools")]
mod devtools;

impl RendererBrowserContextRuntime {
    /// Allocates a browser-context-unique identity for one DedicatedWorker
    /// lifetime. The JavaScript-facing worker id is Page-local and therefore
    /// cannot safely key protocol targets after Page replacement.
    pub(crate) fn allocate_dedicated_worker_instance_id(&self) -> u64 {
        let instance_id = self
            .inner
            .next_dedicated_worker_instance_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .saturating_add(1);
        self.inner.dedicated_worker_targets.lock().insert(
            instance_id,
            DedicatedWorkerTarget::Loading {
                #[cfg(feature = "devtools")]
                inspector_sessions: Vec::new(),
            },
        );
        instance_id
    }

    pub(crate) fn attach_dedicated_worker_control(
        &self,
        instance_id: u64,
        handle: &WorkerHandle,
        output_journal: Option<crate::runtime::RendererTurnOutputJournal>,
    ) {
        let mut targets = self.inner.dedicated_worker_targets.lock();
        let Some(target) = targets.get_mut(&instance_id) else {
            return;
        };
        let DedicatedWorkerTarget::Loading {
            #[cfg(feature = "devtools")]
            inspector_sessions,
        } = target
        else {
            return;
        };
        #[cfg(not(feature = "devtools"))]
        let _ = output_journal;
        let handle = handle.control_handle();
        // Keep attachment updates and handle publication under one lock. A
        // concurrent detach must either remove a pending id or retire its token.
        #[cfg(feature = "devtools")]
        for session in inspector_sessions.drain(..) {
            let _ = handle.attach_runtime_inspector_session(session);
        }
        *target = DedicatedWorkerTarget::Running {
            handle,
            #[cfg(feature = "devtools")]
            output_journal,
        };
    }

    pub(crate) fn unregister_dedicated_worker(&self, instance_id: u64) {
        self.inner
            .dedicated_worker_targets
            .lock()
            .remove(&instance_id);
    }

    pub(crate) fn dedicated_worker_waits_for_debugger(&self) -> bool {
        #[cfg(feature = "devtools")]
        {
            self.inner
                .dedicated_worker_pause_on_start_for_devtools
                .load(std::sync::atomic::Ordering::Relaxed)
        }
        #[cfg(not(feature = "devtools"))]
        {
            false
        }
    }
}
