use crate::page_task_queue::{
    PageWebRtcTargetEffect, PageWebRtcTurnAction, PageWebRtcTurnOutcome, RendererPageWebRtcOwner,
    RendererPageWebRtcTask, RendererPageWebRtcTaskId, RendererPageWebRtcTaskKind,
};

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PageWebRtcTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageWebRtcTargetEffect::DispatchedToCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageWebRtcTargetEffect::CurrentOwnerHadNoEventTarget => {
                PageTaskCompletion::CheckpointOnly
            }
            PageWebRtcTargetEffect::DiscardedStaleOwner { .. } => PageTaskCompletion::NoCompletion,
        }
    }
}

/// Proof that the Page arbiter matched the PageVm namespace, exact Document,
/// Host-local payload id, and WebRTC operation before V8 application.
pub(crate) type AuthorizedCurrentPageWebRtc =
    AuthorizedCurrentWindowDocumentTask<RendererPageWebRtcTask>;

impl PageVm {
    fn current_page_webrtc_owner(
        &self,
        task_id: RendererPageWebRtcTaskId,
    ) -> Option<(RendererPageWebRtcOwner, RendererPageWebRtcTaskKind)> {
        self.vm()
            .current_pending_webrtc_owner(task_id, self.document_lifecycle.identity().document)
    }

    pub(in crate::runtime) fn apply_selected_page_webrtc_turn(
        &mut self,
        task: RendererPageWebRtcTask,
    ) -> anyhow::Result<PageWebRtcTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.current_page_webrtc_owner(task_id);
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, kind, current) {
                Ok(authorization) => {
                    if self.vm_mut().apply_current_webrtc_body(authorization)? {
                        PageWebRtcTargetEffect::DispatchedToCurrentOwner
                    } else {
                        PageWebRtcTargetEffect::CurrentOwnerHadNoEventTarget
                    }
                }
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut().discard_stale_webrtc_task(task_id);
                    }
                    let current_owner = stale.current_owner();
                    tracing::debug!(
                        ?owner,
                        ?current_owner,
                        ?task_id,
                        ?kind,
                        "discarded stale exact-Document WebRTC task"
                    );
                    PageWebRtcTargetEffect::DiscardedStaleOwner { current_owner }
                }
            };
        let action = PageWebRtcTurnAction {
            owner,
            task_id,
            kind,
            target_effect,
        };
        Ok(PageWebRtcTurnOutcome::new(action))
    }
}
