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
        let resize = task.kind() == RendererPageRenderingUpdateTaskKind::ResizeObserver;
        let (applied, mut minimum_depth) = self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .apply_authorized_rendering_update(
                    scope,
                    host_ptr,
                    task.task_id(),
                    owner.target(),
                    task.kind(),
                )
                .ok_or_else(|| anyhow!("authorized rendering update lost its exact payload"))
        })?;
        if resize && applied {
            let previous_target = self
                ._context_host
                .borrow_mut()
                .resize_observer_rendering_update_target
                .replace(owner.target());
            let broadcast: Result<(bool, bool)> = (|| {
                let mut undelivered = false;
                for _ in 0..32 {
                    let viewport = self
                        ._context_host
                        .borrow()
                        .layout_viewport_for_document(self.document_runtime.document_handle());
                    self.with_fresh_layout_pass(
                        moli_layout::LayoutPassRequest::new(
                            viewport,
                            moli_layout::LayoutFlushReason::Explicit,
                        ),
                        |_| Ok(()),
                    )?;
                    let (depth, skipped) = self.with_default_context_scope(|scope, host_ptr| {
                        let host = unsafe { &mut *host_ptr };
                        Ok(host.broadcast_resize_observers_for_target(
                            scope,
                            owner.target(),
                            minimum_depth,
                        ))
                    })?;
                    undelivered = skipped;
                    let Some(depth) = depth else {
                        return Ok((undelivered, false));
                    };
                    minimum_depth = depth;
                }
                // Yield the owner without dropping legitimate deeper work.
                // The budget itself is not evidence of a resize feedback loop.
                Ok((undelivered, true))
            })();
            self._context_host
                .borrow_mut()
                .resize_observer_rendering_update_target = previous_target;
            let (undelivered, resume) = broadcast?;
            if resume || undelivered {
                self._context_host
                    .borrow_mut()
                    .queue_resize_observer_rendering_update_for_target(
                        owner.target(),
                        if resume { minimum_depth } else { 0 },
                    );
            }
            if undelivered && !resume {
                self.record_runtime_warning(format_args!(
                    "ResizeObserver loop completed with undelivered notifications."
                ));
                self.with_default_context_scope(|scope, host_ptr| {
                    unsafe { &mut *host_ptr }
                        .report_resize_observer_loop_error_for_target(
                            scope,
                            host_ptr,
                            owner.target(),
                        )
                        .map_err(anyhow::Error::msg)
                })?;
            }
        }
        Ok(applied)
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
