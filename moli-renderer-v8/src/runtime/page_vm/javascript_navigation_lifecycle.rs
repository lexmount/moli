//! Exact lifecycle reconciliation for `javascript:` location navigation.
//!
//! A normal cross-Document URL commits to replacing the current Document, so
//! its old lifecycle resident can retire before the network navigation is
//! followed. A `javascript:` URL is conditional: a non-string completion
//! keeps the same Document, while a string completion replaces it through
//! `document.open()`. The lifecycle resident therefore remains suspended
//! until URL execution reveals which transition actually happened.
//!
//! This module is the only boundary that classifies that result. Callers must
//! not reconstruct it from `Completed`, the current Document identity and an
//! `Option<PendingDocumentLifecycleTurn>`.

use anyhow::Result;

use crate::PageVmInitStage;
use crate::runtime::{
    PageVmFollowNavigationTurnOutcome, RendererDocumentLifecycleIdentity,
    RendererDocumentLifecycleWaitOutcome,
};

use super::super::document_lifecycle_turn::{
    DocumentLifecycleNavigationTiming, DocumentLifecycleTurnAction, DocumentLifecycleTurnOutcome,
    PendingDocumentLifecycleTurn,
};
use super::{PageVm, renderer_document_lifecycle_milestone_for_stage};
use crate::runtime::DocumentReplacementLifecycleActionSnapshot;

/// The durable lifecycle fact left after one `javascript:` URL finishes.
///
/// Follow-up location navigation is intentionally absent. It is an execution
/// result (`PageVmFollowNavigationTurnOutcome::TriggeredNavigation`), not a
/// lifecycle reconciliation fact, and must be followed before either resident
/// is scheduled again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum JavascriptNavigationLifecycleReconciliation {
    /// No exact lifecycle continuation is runnable after reconciliation.
    ///
    /// This is valid when the observed milestone was already reached, or when
    /// a replacement Document exists but its parser admission is still
    /// blocked. It never authorizes a lifecycle scan or a synthetic wake.
    NoLifecycleContinuation,

    /// The URL did not replace the Document; resume the resident that was
    /// suspended when the pending navigation was surfaced to the owner.
    ResumeExisting {
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
    },

    /// A string completion committed a replacement Document and reconciliation
    /// installed that Document's exact lifecycle resident.
    StartedReplacement {
        previous: RendererDocumentLifecycleIdentity,
        current: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
    },
}

impl JavascriptNavigationLifecycleReconciliation {
    /// Map a completed URL execution onto the existing owner-loop contract.
    ///
    /// Resuming D1 and starting D2 are different lifecycle facts, but both are
    /// consumed by the same stable `PostParseLifecycle` owner path. Keeping the
    /// distinction here preserves diagnostics without adding parallel outer
    /// scheduler variants.
    pub(super) fn into_follow_outcome_after_completed_javascript_url(
        self,
    ) -> PageVmFollowNavigationTurnOutcome {
        match self {
            Self::NoLifecycleContinuation => PageVmFollowNavigationTurnOutcome::Completed,
            Self::ResumeExisting {
                document,
                target_stage,
            } => PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                target_stage,
                outcome: DocumentLifecycleTurnOutcome::runnable(
                    DocumentLifecycleTurnAction::None,
                    document,
                ),
            },
            Self::StartedReplacement {
                previous,
                current,
                target_stage,
            } => PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                target_stage,
                outcome: DocumentLifecycleTurnOutcome::runnable(
                    DocumentLifecycleTurnAction::DocumentReplaced { previous, current },
                    current,
                ),
            },
        }
    }
}

impl PageVm {
    /// Surface one pending top-level navigation while preserving only the
    /// lifecycle state that can legally survive the follow operation.
    ///
    /// A same-Document `javascript:` URL suspends its exact resident because
    /// execution has not yet decided whether replacement occurs. Every normal
    /// cross-Document URL retires the old resident immediately. The returned
    /// action only transfers control to the navigation owner; it is not a
    /// lifecycle wake.
    pub(super) fn transition_lifecycle_for_pending_top_level_navigation(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        source_document: RendererDocumentLifecycleIdentity,
        stage: PageVmInitStage,
    ) -> Option<DocumentLifecycleTurnOutcome> {
        if !self.vm().has_pending_location_navigation() {
            return None;
        }
        let current_document = self.document_lifecycle.identity();
        let suspends_exact_lifecycle = self.vm().has_pending_javascript_location_navigation()
            && pending_document_lifecycle_turn
                .as_ref()
                .is_some_and(|pending| pending.document == current_document);
        if !suspends_exact_lifecycle {
            *pending_document_lifecycle_turn = None;
        }
        Some(DocumentLifecycleTurnOutcome::idle(
            DocumentLifecycleTurnAction::RequestedTopLevelNavigation {
                source_document,
                stage,
                timing: self.pending_navigation_timing_for_lifecycle_stage(stage),
            },
        ))
    }

    fn pending_navigation_timing_for_lifecycle_stage(
        &self,
        stage: PageVmInitStage,
    ) -> DocumentLifecycleNavigationTiming {
        let milestone = renderer_document_lifecycle_milestone_for_stage(stage);
        if matches!(
            self.document_lifecycle_wait_outcome(milestone),
            RendererDocumentLifecycleWaitOutcome::Reached(_)
        ) {
            DocumentLifecycleNavigationTiming::AfterMilestone
        } else {
            DocumentLifecycleNavigationTiming::BeforeMilestone
        }
    }

    /// Commit any replacement admission produced by the URL, then classify
    /// the exact resident that remains. The returned enum is the authority;
    /// callers must not inspect lifecycle storage to refine it.
    pub(super) async fn reconcile_javascript_navigation_lifecycle_after_owner_action(
        &mut self,
        snapshot: DocumentReplacementLifecycleActionSnapshot,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        source_document: RendererDocumentLifecycleIdentity,
    ) -> Result<JavascriptNavigationLifecycleReconciliation> {
        let _ = self
            .reconcile_document_replacement_lifecycle_after_owner_action(
                snapshot,
                pending_document_lifecycle_turn,
            )
            .await?;

        let current = self.document_lifecycle.identity();
        let Some(resident) = pending_document_lifecycle_turn.as_ref() else {
            return Ok(JavascriptNavigationLifecycleReconciliation::NoLifecycleContinuation);
        };
        anyhow::ensure!(
            resident.document == current,
            "javascript: navigation reconciliation left a stale lifecycle resident"
        );
        if current == source_document {
            Ok(
                JavascriptNavigationLifecycleReconciliation::ResumeExisting {
                    document: current,
                    target_stage: resident.stage,
                },
            )
        } else {
            Ok(
                JavascriptNavigationLifecycleReconciliation::StartedReplacement {
                    previous: source_document,
                    current,
                    target_stage: resident.stage,
                },
            )
        }
    }
}
