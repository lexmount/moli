use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::{
    page_task_queue::{
        RendererPageElementToggleEventKind, RendererPageElementToggleEventOwner,
        RendererPageElementToggleEventTaskId,
    },
    runtime::AuthorizedCurrentPageElementToggleEvent,
};

impl ScriptVm {
    pub(crate) fn current_pending_element_toggle_event_owner(
        &self,
        task_id: RendererPageElementToggleEventTaskId,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<(
        RendererPageElementToggleEventOwner,
        RendererPageElementToggleEventKind,
    )> {
        let (target, kind) = self
            ._context_host
            .borrow()
            .current_pending_element_toggle_event_task(task_id)?;
        Some((
            RendererPageElementToggleEventOwner::new(root_document, target),
            kind,
        ))
    }

    /// Dispatch one already-authorized toggle event body.
    ///
    /// The selected DOM-manipulation task owns the task-end checkpoint,
    /// child-record synchronization, and runtime follow-up. Keeping those
    /// boundaries out of this helper also means a cancelled coalescing entry,
    /// which never becomes a selected task, cannot manufacture a checkpoint.
    pub(crate) fn apply_current_element_toggle_event_body(
        &mut self,
        authorization: AuthorizedCurrentPageElementToggleEvent,
    ) -> Result<bool> {
        let task = authorization.into_task();
        let (owner, task_id, kind, data) = task.into_parts();
        if !self
            ._context_host
            .borrow_mut()
            .take_pending_element_toggle_event_for_exact_target(task_id, owner.target(), kind)
        {
            return Err(anyhow!(
                "authorized element toggle event lost its exact pending payload"
            ));
        }
        if matches!(
            owner.target().dispatch_scope(),
            crate::native_bridge::OwnerDispatchScope::Top
        ) {
            let target = crate::host::EventTargetHandle::Node(data.element());
            if self
                .prepare_main_document_event(target, target, "toggle")
                .is_none()
            {
                // The native task was dispatched to its exact owner. Preserve
                // ordinary task completion even when no JS wrapper is needed.
                return Ok(true);
            }
        }
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(
                unsafe { &mut *host_ptr }.dispatch_authorized_element_toggle_event(
                    scope,
                    host_ptr,
                    owner.target(),
                    data,
                ),
            )
        })
    }

    pub(crate) fn discard_stale_element_toggle_event_task(
        &mut self,
        task_id: RendererPageElementToggleEventTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_element_toggle_event_task(task_id)
    }
}
