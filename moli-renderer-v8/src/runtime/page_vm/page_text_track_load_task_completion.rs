//! Completion classification for the text-track loading algorithm.
//!
//! The algorithm crosses two HTML task sources, but source membership does not
//! decide completion. `Start` only performs stable-state/fetch-start work;
//! terminal kinds can dispatch `load`/`error` and therefore require callback
//! reconciliation.

use crate::page_task_queue::{
    PageTextTrackLoadStalePayloadEffect, PageTextTrackLoadTargetEffect,
    PageTextTrackLoadTurnAction, RendererPageTextTrackLoadTaskKind,
};

use super::{IntoPageTaskCompletion, PageTaskCompletion};

impl IntoPageTaskCompletion for PageTextTrackLoadTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match (self.kind, self.target_effect) {
            (
                RendererPageTextTrackLoadTaskKind::NetworkTerminal
                | RendererPageTextTrackLoadTaskKind::FailureTerminal,
                PageTextTrackLoadTargetEffect::AppliedToCurrentOwner,
            ) => PageTaskCompletion::CallbackCompletion,
            (
                _,
                PageTextTrackLoadTargetEffect::AppliedToCurrentOwner
                | PageTextTrackLoadTargetEffect::CurrentOwnerNoLongerEligible,
            )
            | (
                _,
                PageTextTrackLoadTargetEffect::DiscardedStaleOwner {
                    stale_payload_effect: PageTextTrackLoadStalePayloadEffect::DiscardedExactPayload,
                    ..
                },
            ) => PageTaskCompletion::CheckpointOnly,
            (
                _,
                PageTextTrackLoadTargetEffect::DiscardedStaleOwner {
                    stale_payload_effect:
                        PageTextTrackLoadStalePayloadEffect::ForeignPageVmStatePreserved
                        | PageTextTrackLoadStalePayloadEffect::NoDiscardedExactPayload,
                    ..
                },
            ) => PageTaskCompletion::NoCompletion,
        }
    }
}
