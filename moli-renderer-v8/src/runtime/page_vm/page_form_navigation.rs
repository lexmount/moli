use crate::page_task_queue::{
    PageFormNavigationTargetEffect, PageFormNavigationTurnAction, PageFormNavigationTurnOutcome,
    RendererPageFormNavigationOwner, RendererPageFormNavigationTask,
    RendererPageFormNavigationTaskId, RendererPageFormNavigationTaskKind,
};

use super::{AuthorizedCurrentWindowDocumentTask, PageVm};

pub(crate) type AuthorizedCurrentPageFormNavigation =
    AuthorizedCurrentWindowDocumentTask<RendererPageFormNavigationTask>;

impl PageVm {
    fn current_page_form_navigation_owner(
        &self,
        task_id: RendererPageFormNavigationTaskId,
    ) -> Option<(
        RendererPageFormNavigationOwner,
        RendererPageFormNavigationTaskKind,
    )> {
        self.vm().current_pending_form_navigation_owner(
            task_id,
            self.document_lifecycle.identity().document,
        )
    }

    pub(in crate::runtime) fn apply_selected_page_form_navigation_turn(
        &mut self,
        task: RendererPageFormNavigationTask,
    ) -> anyhow::Result<PageFormNavigationTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.current_page_form_navigation_owner(task_id);
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, kind, current) {
                Ok(authorization) => {
                    if self
                        .vm_mut()
                        .apply_current_form_navigation_body(authorization)?
                    {
                        PageFormNavigationTargetEffect::AppliedToCurrentOwner
                    } else {
                        PageFormNavigationTargetEffect::CurrentOwnerNoLongerEligible
                    }
                }
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut().discard_stale_form_navigation_task(task_id);
                    }
                    let current_owner = stale.current_owner();
                    tracing::debug!(
                        ?owner,
                        ?current_owner,
                        ?task_id,
                        ?kind,
                        "discarded stale exact-Document planned form navigation task"
                    );
                    PageFormNavigationTargetEffect::DiscardedStaleOwner { current_owner }
                }
            };
        let action = PageFormNavigationTurnAction {
            owner,
            task_id,
            kind,
            target_effect,
        };
        Ok(PageFormNavigationTurnOutcome::new(action))
    }
}
