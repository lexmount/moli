//! Durable executable state for one exact renderer Document lifecycle.
//!
//! The stable owner-local page residence owns this state. `PageVm` borrows it
//! only while executing one bounded lifecycle action; protocol projections and
//! the authoritative lifecycle journal remain separate owners.

use std::time::Instant;

use crate::PageVmInitStage;
use crate::script_vm::{
    PostParseLifecycleCompletionAction, PostParseLifecycleDriver, PostParsePageOwnedTask,
};

use super::{RendererDocumentLifecycleIdentity, RendererLifecycleTerminationStamp};

/// The exact effect performed by one bounded lifecycle owner turn.
///
/// `None` is a real outcome: a stale wake or a readiness probe may discover
/// that no action is authorized for the requested Document. It must never be
/// treated as permission to bind the turn to the page's current Document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DocumentLifecycleTurnAction {
    None,
    Progressed,
    ReachedStage(PageVmInitStage),
    DocumentReplaced {
        previous: RendererDocumentLifecycleIdentity,
        current: RendererDocumentLifecycleIdentity,
    },
    RequestedTopLevelNavigation {
        source_document: RendererDocumentLifecycleIdentity,
        stage: PageVmInitStage,
        timing: DocumentLifecycleNavigationTiming,
    },
}

/// Orders a navigation produced by a lifecycle action against that action's
/// target milestone. A load-stage task may navigate before `load`; a load
/// handler navigates after the authoritative `load` fact. Those cases have
/// different protocol ordering and cannot be inferred from `stage` alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DocumentLifecycleNavigationTiming {
    BeforeMilestone,
    AfterMilestone,
}

impl DocumentLifecycleTurnAction {
    /// Freeze lifecycle-relative publication ordering before the exact
    /// Document turn is restored to the Page residence.
    ///
    /// A navigation requested by a load handler is produced after the
    /// authoritative load fact. Protocol may therefore have to retain this
    /// batch behind that exact Document's pending `Page.loadEventFired`.
    pub(super) const fn renderer_output_ordering(self) -> super::RendererOutputPublicationOrdering {
        match self {
            Self::RequestedTopLevelNavigation {
                source_document,
                timing: DocumentLifecycleNavigationTiming::AfterMilestone,
                ..
            } => super::RendererOutputPublicationOrdering::AfterPendingPageLoad { source_document },
            _ => super::RendererOutputPublicationOrdering::Unconstrained,
        }
    }
}

/// Whether the stable page residence should admit another lifecycle turn.
///
/// Readiness is deliberately separate from action. A turn can make progress
/// and then block, or reach DOMContentLoaded and leave the same exact
/// Document immediately runnable for its load stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DocumentLifecycleTurnReadiness {
    Runnable {
        document: RendererDocumentLifecycleIdentity,
    },
    Blocked {
        document: RendererDocumentLifecycleIdentity,
    },
    Idle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DocumentLifecycleTurnOutcome {
    pub(super) action: DocumentLifecycleTurnAction,
    pub(super) readiness: DocumentLifecycleTurnReadiness,
}

/// Command/navigation-side observation of an exact Document milestone.
///
/// This is intentionally not scheduler readiness. The page owner progresses
/// lifecycle work whether or not an observer exists; an observer only decides
/// when its own DCL/load request may complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DocumentLifecycleObserverOutcome {
    Pending,
    Reached,
    DocumentReplaced {
        document: RendererDocumentLifecycleIdentity,
    },
    Interrupted(RendererLifecycleTerminationStamp),
    NavigationPending,
    /// The exact Document journal is still pending, but its stable Page
    /// residence no longer contains an executable continuation. This is an
    /// ownership invariant violation, not successful observation.
    MissingResident,
}

impl DocumentLifecycleTurnOutcome {
    pub(super) const fn idle(action: DocumentLifecycleTurnAction) -> Self {
        Self {
            action,
            readiness: DocumentLifecycleTurnReadiness::Idle,
        }
    }

    pub(super) const fn runnable(
        action: DocumentLifecycleTurnAction,
        document: RendererDocumentLifecycleIdentity,
    ) -> Self {
        Self {
            action,
            readiness: DocumentLifecycleTurnReadiness::Runnable { document },
        }
    }

    pub(super) const fn blocked(
        action: DocumentLifecycleTurnAction,
        document: RendererDocumentLifecycleIdentity,
    ) -> Self {
        Self {
            action,
            readiness: DocumentLifecycleTurnReadiness::Blocked { document },
        }
    }
}

pub(super) struct PendingDocumentLifecycleTurn {
    pub(super) document: RendererDocumentLifecycleIdentity,
    pub(super) stage: PageVmInitStage,
    /// Whether the previous bounded action proved that this exact lifecycle
    /// can immediately take another owner turn without ordinary Page work.
    ///
    /// This is stable residence state, not a wake hint. It keeps a runnable
    /// parser preparation and lifecycle task admission contiguous. Once an
    /// event is admitted to the DOM source, the resident waits for its receipt
    /// and ordinary task arbitration owns event delivery.
    pub(super) owner_turn_is_runnable: bool,
    pub(super) driver: PostParseLifecycleDriver,
    pub(super) completed_task: Option<PostParsePageOwnedTask>,
    pub(super) awaiting_dom_task: Option<PendingDocumentLifecycleDomTask>,
    pub(super) completion_action: Option<PostParseLifecycleCompletionAction>,
    /// Whether this exact resident owns a sealed main-parser defer/module
    /// continuation.
    ///
    /// Chromium schedules this work as
    /// `TaskType::kInternalContinueScriptLoading`, ahead of ordinary DOM
    /// tasks. Keeping the fact here avoids reconstructing that priority by
    /// inspecting unrelated stylesheet or event queues.
    pub(super) has_sealed_main_parser_script_queue: bool,
    pub(super) started: Instant,
}

pub(super) struct PendingDocumentLifecycleDomTask {
    pub(super) task: PostParsePageOwnedTask,
    pub(super) completion: tokio::sync::oneshot::Receiver<
        crate::page_task_queue::RendererPageMainDocumentLifecycleCompletion,
    >,
}
