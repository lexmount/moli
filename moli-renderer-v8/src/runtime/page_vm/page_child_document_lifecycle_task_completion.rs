//! Task-end boundary for an exact child Document lifecycle action.
//!
//! DCL and complete use the shared DOM source. Interactive readiness belongs
//! to parser stop; an unmaterialized realm may need a child owner continuation.
//! The selected dispatcher completes the event's checkpoint and follow-up.

use crate::page_task_queue::{
    PageChildDocumentLifecycleTargetEffect, PageChildDocumentLifecycleTurnAction,
};

use super::{IntoPageTaskCompletion, PageTaskCompletion};

impl IntoPageTaskCompletion for PageChildDocumentLifecycleTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageChildDocumentLifecycleTargetEffect::EventDispatchedToCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageChildDocumentLifecycleTargetEffect::ConsumedCurrentOwnerWithoutEvent
            | PageChildDocumentLifecycleTargetEffect::FailedForCurrentOwner => {
                // The exact current task entered its child Window realm. The
                // previous realm-scope helper always completed an ordinary
                // checkpoint even if no event wrapper survived or execution
                // failed, but there is no callback follow-up to reconcile.
                PageTaskCompletion::CheckpointOnly
            }
            PageChildDocumentLifecycleTargetEffect::DiscardedStaleOwner { .. } => {
                // A stale action never entered the replacement realm and must
                // not manufacture a checkpoint there.
                PageTaskCompletion::NoCompletion
            }
        }
    }
}
