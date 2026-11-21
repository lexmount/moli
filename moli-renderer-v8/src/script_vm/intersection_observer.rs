use anyhow::{Result, ensure};

use super::ScriptVm;
use crate::{
    page_task_queue::{
        PageIntersectionObserverTargetEffect, RendererPageIntersectionObserverOwner,
        RendererPageIntersectionObserverTaskId, RendererPageIntersectionObserverTaskKind,
    },
    runtime::{AuthorizedCurrentPageIntersectionObserverTask, RendererDocumentToken},
    v8_execution_watchdog::V8ExecutionWatchdog,
};

impl ScriptVm {
    pub(crate) fn current_pending_intersection_observer_owner(
        &self,
        id: RendererPageIntersectionObserverTaskId,
        root: RendererDocumentToken,
    ) -> Option<(
        RendererPageIntersectionObserverOwner,
        RendererPageIntersectionObserverTaskKind,
    )> {
        let (target, kind) = self
            ._context_host
            .borrow()
            .current_pending_intersection_observer_task(id)?;
        Some((
            RendererPageIntersectionObserverOwner::new(root, target),
            kind,
        ))
    }

    pub(crate) fn apply_current_intersection_observer_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageIntersectionObserverTask,
    ) -> Result<PageIntersectionObserverTargetEffect> {
        let task = authorization.into_task();
        let target = task.owner().target();
        // Release the Document's queued flag before callbacks can enqueue more work.
        ensure!(
            self._context_host
                .borrow_mut()
                .take_pending_intersection_observer_task(task.task_id(), target, task.kind()),
            "authorized IO notification lost its exact payload"
        );
        self.with_default_context_scope(|scope, host_ptr| {
            let ids = crate::observer_runtime::realtime_intersection_observer_ids(
                unsafe { &*host_ptr },
                target,
            );
            let mut invoked = false;
            for id in ids {
                let delivered = V8ExecutionWatchdog::with_lifecycle_budget(
                    scope,
                    "IntersectionObserver callback",
                    |scope| {
                        Ok(
                            crate::observer_runtime::deliver_realtime_intersection_observer(
                                scope, host_ptr, target, id,
                            ),
                        )
                    },
                )?;
                if delivered {
                    invoked = true;
                    Self::perform_microtask_checkpoints(scope, None)?;
                }
            }
            Ok(if invoked {
                PageIntersectionObserverTargetEffect::CallbackInvokedForCurrentOwner
            } else {
                PageIntersectionObserverTargetEffect::CurrentOwnerCallbackRetired
            })
        })
    }

    pub(crate) fn discard_stale_intersection_observer_task(
        &mut self,
        id: RendererPageIntersectionObserverTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_intersection_observer_task(id)
    }
}
