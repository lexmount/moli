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
        if task.kind() == RendererPageRenderingUpdateTaskKind::LayoutResourceAdmission {
            let css_images = self
                ._context_host
                .borrow_mut()
                .take_authorized_layout_resources(task.task_id(), owner.target())
                .ok_or_else(|| anyhow!("authorized layout resource task lost its exact payload"))?;
            self.reconcile_document_web_fonts_for_layout();
            self.start_css_images_discovered_by_layout(css_images);
            let document = self._context_host.borrow().document_handle();
            return self.with_default_context_scope(|scope, host_ptr| {
                crate::native_bridge::element::queue_revealed_lazy_image_loads(
                    scope, host_ptr, document,
                );
                Ok(true)
            });
        }
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

    pub(crate) fn discard_stale_rendering_update_task(
        &mut self,
        task_id: RendererPageRenderingUpdateTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_rendering_update_task(task_id)
    }
}
