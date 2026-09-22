use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::runtime::{PageOwnerTurnOutcome, RendererOwnerResourceActivitySource};

use super::RendererPageResourceCompletionOwner;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageResourceCompletionDocumentEffect {
    AppliedToCurrentOwner,
    /// Exact-owner authorization succeeded, but the inner domain payload had
    /// already been consumed or retired before application entered page code.
    /// This remains a current selected task, rather than being mislabeled as
    /// work belonging to a stale Document.
    CurrentOwnerHadNoApplicablePayload,
    /// Application entered page code or event dispatch before a callback
    /// replaced its target. The old target may no longer be current, but the
    /// enclosing task still owes completion for the code that already ran.
    SupersededDuringApplication {
        current_owner: Option<RendererPageResourceCompletionOwner>,
    },
    DiscardedStaleOwner {
        current_owner: Option<RendererPageResourceCompletionOwner>,
    },
}

/// Whether applying the already-selected terminal entered page code or an
/// event-dispatch algorithm.
///
/// This fact is produced by the body. It is not queue metadata and cannot be
/// used to prioritize the resource terminal. The distinction only tells the
/// task-end coordinator whether child-record reconciliation is required after
/// the checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageResourceCompletionBodyActivity {
    NoPageCodeOrEventDispatch,
    PageCodeOrEventDispatchAttempted,
}

/// Follow-up that becomes legal only after this selected task's checkpoint.
///
/// Parser completion and release of the final main-Document load-delay lease
/// must wait for the producing script or error callback's reactions. The body
/// returns the exact owner and bounded obligation instead of rediscovering
/// lifecycle work after the checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageResourceCompletionPostCheckpointEffect {
    None,
    CompletePopupDocumentParser {
        completion: crate::native_bridge::PopupDocumentParserCompletion,
    },
    /// The selected resource terminal released the final load-delay lease for
    /// this exact main Document. The resource target itself is normally
    /// consumed by the body, so the post-checkpoint coordinator must validate
    /// the durable Document owner rather than trying to rediscover the spent
    /// fetch owner.
    PrimeMainDocumentLifecycle {
        owner: FrameDocumentTaskOwner,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageResourceCompletionOutputEffect {
    None,
    CaptureRequired,
}

impl PageResourceCompletionOutputEffect {
    pub(crate) const fn capture_if(required: bool) -> Self {
        if required {
            Self::CaptureRequired
        } else {
            Self::None
        }
    }
}

/// What one page-owned resource-completion turn actually consumed.
///
/// A native terminal can have two independently observable effects: publish
/// network/protocol facts that remain valid after Document replacement, and
/// offer Document-owned terminal semantics to the exact Document that
/// initiated the work. The latter may still be a legitimate no-op, so it is
/// not described as a guaranteed DOM mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageResourceCompletionTurnAction {
    pub(crate) source: RendererOwnerResourceActivitySource,
    pub(crate) owner: RendererPageResourceCompletionOwner,
    pub(crate) document_effect: PageResourceCompletionDocumentEffect,
    pub(crate) body_activity: PageResourceCompletionBodyActivity,
    pub(crate) post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect,
    pub(crate) output_effect: PageResourceCompletionOutputEffect,
}

impl PageResourceCompletionTurnAction {
    pub(crate) const fn applied(
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        output_effect: PageResourceCompletionOutputEffect,
    ) -> Self {
        Self {
            source,
            owner,
            document_effect: PageResourceCompletionDocumentEffect::AppliedToCurrentOwner,
            body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
            post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
            output_effect,
        }
    }

    pub(crate) const fn applied_after_page_code(
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        output_effect: PageResourceCompletionOutputEffect,
    ) -> Self {
        Self {
            source,
            owner,
            document_effect: PageResourceCompletionDocumentEffect::AppliedToCurrentOwner,
            body_activity: PageResourceCompletionBodyActivity::PageCodeOrEventDispatchAttempted,
            post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
            output_effect,
        }
    }

    pub(crate) const fn current_owner_without_payload(
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        output_effect: PageResourceCompletionOutputEffect,
    ) -> Self {
        Self {
            source,
            owner,
            document_effect:
                PageResourceCompletionDocumentEffect::CurrentOwnerHadNoApplicablePayload,
            body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
            post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
            output_effect,
        }
    }

    pub(crate) const fn superseded_after_page_code(
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        current_owner: Option<RendererPageResourceCompletionOwner>,
        output_effect: PageResourceCompletionOutputEffect,
    ) -> Self {
        Self {
            source,
            owner,
            document_effect: PageResourceCompletionDocumentEffect::SupersededDuringApplication {
                current_owner,
            },
            body_activity: PageResourceCompletionBodyActivity::PageCodeOrEventDispatchAttempted,
            post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
            output_effect,
        }
    }

    pub(crate) const fn superseded_without_page_code(
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        current_owner: Option<RendererPageResourceCompletionOwner>,
        output_effect: PageResourceCompletionOutputEffect,
    ) -> Self {
        Self {
            source,
            owner,
            document_effect: PageResourceCompletionDocumentEffect::SupersededDuringApplication {
                current_owner,
            },
            body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
            post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
            output_effect,
        }
    }

    pub(crate) const fn discarded_stale(
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        current_owner: Option<RendererPageResourceCompletionOwner>,
        output_effect: PageResourceCompletionOutputEffect,
    ) -> Self {
        Self {
            source,
            owner,
            document_effect: PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner,
            },
            body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
            post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
            output_effect,
        }
    }

    pub(crate) const fn with_post_checkpoint_effect(
        mut self,
        effect: PageResourceCompletionPostCheckpointEffect,
    ) -> Self {
        self.post_checkpoint_effect = effect;
        self
    }

    #[cfg(test)]
    pub(crate) fn source(self) -> RendererOwnerResourceActivitySource {
        self.source
    }
}

pub(crate) type PageResourceCompletionTurnOutcome =
    PageOwnerTurnOutcome<PageResourceCompletionTurnAction>;
