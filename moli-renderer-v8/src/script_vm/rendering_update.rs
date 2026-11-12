use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::page_task_queue::{
    RendererPageRenderingUpdateOwner, RendererPageRenderingUpdateTaskId,
    RendererPageRenderingUpdateTaskKind,
};
use crate::runtime::AuthorizedCurrentPageRenderingUpdate;

impl ScriptVm {
    pub(super) fn queue_main_document_post_parse_autofocus_best_effort(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) {
        use crate::native_bridge::PostParseAutofocusAdmission;

        let admission = self
            ._context_host
            .borrow_mut()
            .queue_main_document_post_parse_autofocus(owner);
        match admission {
            PostParseAutofocusAdmission::Published
            | PostParseAutofocusAdmission::NotNeeded
            | PostParseAutofocusAdmission::StaleOwner => {}
            PostParseAutofocusAdmission::RouteClosed => self.record_runtime_warning(format_args!(
                "post-parse autofocus rendering route closed for {owner:?}"
            )),
        }
    }

    pub(crate) fn current_pending_rendering_update_owner(
        &self,
        task_id: RendererPageRenderingUpdateTaskId,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<(
        RendererPageRenderingUpdateOwner,
        RendererPageRenderingUpdateTaskKind,
    )> {
        let (target, kind) = self
            ._context_host
            .borrow()
            .current_pending_rendering_update_task(task_id)?;
        Some((
            RendererPageRenderingUpdateOwner::new(root_document, target),
            kind,
        ))
    }

    /// Apply only one authorized rendering update's callback-visible body.
    ///
    /// The selected Page-task dispatcher owns the task-end checkpoint, child
    /// synchronization, and runtime follow-up. Rendering-update payload
    /// settlement remains here so reentrant scroll or animation work receives
    /// a distinct later source entry before any callback runs.
    pub(crate) fn apply_current_rendering_update_body(
        &mut self,
        authorization: AuthorizedCurrentPageRenderingUpdate,
    ) -> Result<bool> {
        let task = authorization.into_task();
        let owner = task.owner();
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .apply_authorized_rendering_update(
                    scope,
                    host_ptr,
                    task.task_id(),
                    owner.target(),
                    task.kind(),
                )
                .ok_or_else(|| anyhow!("authorized rendering update lost its exact payload"))
        })
    }

    /// Runs a ResizeObserver delivery that is waiting for the next animation
    /// frame now, once, so a snapshot of rendered output (a dump, screenshot
    /// or PDF) includes what the observers write, as a browser's frame before
    /// capture would. Pages without queued observations do nothing.
    pub(crate) fn run_pending_resize_observer_frame(&mut self) {
        // Inside a debugger pause the frame timer delivers after resume.
        if crate::runtime::nested_main_dispatch_active() {
            return;
        }
        let _ = self.with_current_main_world_scope(|scope, _| {
            crate::context_bootstrap::deliver_resize_observer_frame(scope);
            Ok(())
        });
    }

    pub(crate) fn discard_stale_rendering_update_task(
        &mut self,
        task_id: RendererPageRenderingUpdateTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_rendering_update_task(task_id)
    }
}
