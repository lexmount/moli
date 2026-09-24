//! Planned form navigation dispatches callbacks and reconciles their effects.
use super::{IntoPageTaskCompletion, PageTaskCompletion};
use crate::page_task_queue::{PageFormNavigationTargetEffect, PageFormNavigationTurnAction};

impl IntoPageTaskCompletion for PageFormNavigationTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageFormNavigationTargetEffect::AppliedToCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageFormNavigationTargetEffect::CurrentOwnerNoLongerEligible => {
                PageTaskCompletion::CheckpointOnly
            }
            PageFormNavigationTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}
