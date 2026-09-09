use anyhow::{Result, anyhow};

use super::{ScriptVm, runtime_bindings::dispatch_promise_rejection_task};
use crate::{
    page_task_queue::{
        PagePromiseRejectionTargetEffect, RendererPagePromiseRejectionOwner,
        RendererPagePromiseRejectionTaskId, RendererPagePromiseRejectionTaskKind,
    },
    runtime::AuthorizedCurrentPagePromiseRejection,
};

impl ScriptVm {
    pub(crate) fn current_pending_promise_rejection_owner(
        &self,
        task_id: RendererPagePromiseRejectionTaskId,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<(
        RendererPagePromiseRejectionOwner,
        RendererPagePromiseRejectionTaskKind,
    )> {
        let (target, kind) = self
            ._context_host
            .borrow()
            .current_pending_promise_rejection_target(task_id)?;
        Some((
            RendererPagePromiseRejectionOwner::new(root_document, target),
            kind,
        ))
    }

    pub(crate) fn apply_current_promise_rejection_body(
        &mut self,
        authorization: AuthorizedCurrentPagePromiseRejection,
    ) -> Result<PagePromiseRejectionTargetEffect> {
        let task = authorization.into_task();
        let payload = self
            ._context_host
            .borrow_mut()
            .take_pending_promise_rejection_task(task.task_id(), task.owner().target(), task.kind())
            .ok_or_else(|| anyhow!("authorized promise rejection lost its exact payload"))?;
        self.with_default_context_scope(|scope, _| {
            Ok(
                if dispatch_promise_rejection_task(
                    scope,
                    task.owner().target(),
                    task.kind(),
                    payload,
                ) {
                    PagePromiseRejectionTargetEffect::DispatchedToCurrentOwner
                } else {
                    PagePromiseRejectionTargetEffect::CurrentOwnerHadNoEventTarget
                },
            )
        })
    }

    pub(crate) fn discard_stale_promise_rejection_task(
        &mut self,
        task_id: RendererPagePromiseRejectionTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_promise_rejection_task(task_id)
    }
}
