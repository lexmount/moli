use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::{
    page_task_queue::{
        PageServiceWorkerQueryTargetEffect, RendererPageServiceWorkerQueryOwner,
        RendererPageServiceWorkerQueryTaskId, RendererPageServiceWorkerQueryTaskKind,
    },
    runtime::AuthorizedCurrentPageServiceWorkerQuery,
};

impl ScriptVm {
    pub(crate) fn current_pending_service_worker_query_owner(
        &self,
        task_id: RendererPageServiceWorkerQueryTaskId,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<(
        RendererPageServiceWorkerQueryOwner,
        RendererPageServiceWorkerQueryTaskKind,
    )> {
        let (target, kind) = self
            ._context_host
            .borrow()
            .current_pending_service_worker_query_task(task_id)?;
        Some((
            RendererPageServiceWorkerQueryOwner::new(root_document, target),
            kind,
        ))
    }

    /// Apply one already-authorized registration query body.
    ///
    /// The selected DOM-manipulation dispatcher owns the task checkpoint,
    /// child-record synchronization, and runtime/style follow-up.
    pub(crate) fn apply_current_service_worker_query_body(
        &mut self,
        authorization: AuthorizedCurrentPageServiceWorkerQuery,
    ) -> Result<PageServiceWorkerQueryTargetEffect> {
        let task = authorization.into_task();
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let Some(query) = self
            ._context_host
            .borrow_mut()
            .take_pending_service_worker_query_for_exact_target(task_id, owner.target(), kind)
        else {
            return Err(anyhow!(
                "authorized ServiceWorkerContainer registration query task lost its exact pending query"
            ));
        };
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(
                unsafe { &mut *host_ptr }.dispatch_authorized_service_worker_query(
                    scope,
                    owner.target(),
                    query,
                ),
            )
        })
    }

    pub(crate) fn discard_stale_service_worker_query_task(
        &mut self,
        task_id: RendererPageServiceWorkerQueryTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_service_worker_query_task(task_id)
    }
}
