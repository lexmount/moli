use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::page_task_queue::{
    RendererPageWebRtcOwner, RendererPageWebRtcTaskId, RendererPageWebRtcTaskKind,
};
use crate::runtime::AuthorizedCurrentPageWebRtc;

impl ScriptVm {
    pub(crate) fn current_pending_webrtc_owner(
        &self,
        task_id: RendererPageWebRtcTaskId,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<(RendererPageWebRtcOwner, RendererPageWebRtcTaskKind)> {
        let (target, kind) = self
            ._context_host
            .borrow()
            .current_pending_webrtc_task(task_id)?;
        Some((RendererPageWebRtcOwner::new(root_document, target), kind))
    }

    /// Apply only the callback-visible body of one authorized WebRTC task.
    ///
    /// The selected Page-task dispatcher owns the task-end checkpoint, child
    /// synchronization, and runtime follow-up. Keeping those operations out of
    /// this helper prevents low-level semantic fixtures or nested callers from
    /// manufacturing an extra HTML task boundary.
    pub(crate) fn apply_current_webrtc_body(
        &mut self,
        authorization: AuthorizedCurrentPageWebRtc,
    ) -> Result<bool> {
        let task = authorization.into_task();
        let owner = task.owner();
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .apply_authorized_webrtc(
                    scope,
                    host_ptr,
                    task.task_id(),
                    owner.target(),
                    task.kind(),
                )
                .ok_or_else(|| anyhow!("authorized WebRTC task lost its exact payload"))
        })
    }

    pub(crate) fn discard_stale_webrtc_task(&mut self, task_id: RendererPageWebRtcTaskId) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_webrtc_task(task_id)
    }
}
