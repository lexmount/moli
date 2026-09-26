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
        let (mut invoked, animation_owner) =
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
            })?;
        let Some(animation_owner) = animation_owner else {
            return Ok(invoked);
        };
        let target = owner.target();
        if crate::observer_runtime::document_has_rendering_observers(
            &mut self._context_host.borrow_mut(),
            animation_owner,
        ) {
            // Keep a single deadline across callbacks, their microtasks, error
            // reporting and repeated layout. A callback can keep adding deeper
            // targets indefinitely even though each individual broadcast ends.
            let watchdog = self.with_default_context_scope(|scope, _| {
                Ok(crate::v8_execution_watchdog::V8ExecutionWatchdog::arm(
                    crate::v8_execution_watchdog::V8ExecutionWatchdogKind::RenderingObservers,
                    scope.thread_safe_handle(),
                    crate::v8_execution_watchdog::SCRIPT_TURN_WATCHDOG_TIMEOUT,
                ))
            })?;
            let mut depth = 0;
            loop {
                let current = self.with_default_context_scope(|scope, host_ptr| {
                    if scope.is_execution_terminating() {
                        return Ok(None);
                    }
                    let host = unsafe { &mut *host_ptr };
                    Ok(host
                        .resolve_current_rendering_update_context(scope, target)
                        .map(|(document, _)| {
                            host.top_level_document_for_document(document)
                                .unwrap_or(document)
                        }))
                })?;
                let Some(document) = current else {
                    break;
                };
                let request = {
                    let host = self._context_host.borrow();
                    host.layout_policy().uses_real_layout().then(|| {
                        moli_layout::LayoutPassRequest::new(
                            host.layout_viewport_for_document(document),
                            moli_layout::LayoutFlushReason::RenderingUpdate,
                        )
                    })
                };
                if let Some(request) = request {
                    self.with_fresh_document_layout_pass(document, request, |_| Ok(()))?;
                }
                let broadcast = self.with_default_context_scope(|scope, host_ptr| {
                    Ok(
                        crate::context_bootstrap::broadcast_document_resize_observers(
                            scope,
                            host_ptr,
                            target,
                            animation_owner,
                            depth,
                        )?,
                    )
                })?;
                invoked |= broadcast.invoked;
                if let Some(next_depth) = broadcast.shallowest_depth {
                    depth = next_depth;
                } else {
                    if broadcast.skipped {
                        self.with_default_context_scope(|scope, host_ptr| {
                            if !scope.is_execution_terminating() {
                                crate::context_bootstrap::report_document_resize_observer_loop_error(scope, host_ptr, target);
                            }
                            Ok(())
                        })?;
                        invoked = true;
                    }
                    break;
                }
            }
            if watchdog.disarm()
                == crate::v8_execution_watchdog::V8ExecutionWatchdogOutcome::TimedOut
            {
                tracing::warn!("rendering observers exceeded their execution deadline");
            }
        }
        self.with_default_context_scope(|scope, host_ptr| {
            invoked |= unsafe { &mut *host_ptr }.finish_document_animation_frame(
                scope,
                host_ptr,
                target,
                animation_owner,
            );
            crate::observer_runtime::update_document_intersections(
                scope,
                host_ptr,
                target,
                animation_owner,
            )?;
            Ok(invoked)
        })
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
