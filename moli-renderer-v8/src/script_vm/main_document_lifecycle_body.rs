//! Main-document lifecycle callback bodies.
//!
//! This module only applies exact-Document transitions and dispatches the next
//! callback body. It returns a typed checkpoint continuation rather than
//! performing checkpoints itself. The sibling completion module owns every
//! resume and turn-exit step.

use super::main_document_lifecycle::{
    MainDocumentLifecycleCheckpointContinuation, MainDocumentLifecycleDomContentLoadedEventEnd,
    MainDocumentLifecycleSettlement,
};
use super::{
    MainDocumentLifecycleBody, MainDocumentLifecycleCallbackEffect,
    MainDocumentLifecycleEventDispatch, MainDocumentLifecycleExecution,
    MainDocumentLifecycleFollowup, MainDocumentLifecycleStep, MainDocumentLifecycleTargetRejection,
    ScriptVm,
};
use crate::dom::native::DocumentReadyState;
use crate::frame_owner_model::{FrameDocumentTaskOwner, MainDocumentInteractiveLifecycleAction};

impl ScriptVm {
    /// Apply Blink's `CancelParsing()` readiness boundary for Page.stopLoading.
    ///
    /// This is deliberately separate from the ordinary Window-load body: the
    /// Document remains current and receives `readystatechange`, while the
    /// frame-owner lifecycle suppresses all later DCL/load delivery.
    pub(crate) fn stop_current_main_document_loading(&mut self) -> anyhow::Result<()> {
        let Some(owner) = self.current_main_document_task_owner() else {
            return Ok(());
        };
        let ready_state_changed = self
            ._context_host
            .borrow_mut()
            .stop_current_main_document_loading(owner)
            .unwrap_or(false);

        if ready_state_changed {
            self.set_document_ready_state(DocumentReadyState::Complete)?;
            let _dispatch =
                self.dispatch_document_lifecycle_event_body_best_effort("readystatechange");
        }

        let checkpoint = self.finish_main_document_lifecycle_checkpoint();
        self.finish_main_document_lifecycle_turn(checkpoint)
    }

    /// Start one lifecycle body without performing any task-end or internal
    /// lifecycle checkpoint.
    pub(crate) fn begin_main_document_lifecycle_body(
        &mut self,
        body: MainDocumentLifecycleBody,
    ) -> MainDocumentLifecycleStep {
        match body {
            MainDocumentLifecycleBody::Interactive(action) => {
                self.begin_main_document_interactive_body(body, action)
            }
            MainDocumentLifecycleBody::DomContentLoaded { owner } => {
                self.begin_main_document_domcontentloaded_body(body, owner)
            }
            MainDocumentLifecycleBody::WindowLoad { owner } => {
                self.begin_main_document_window_load_body(body, owner)
            }
        }
    }

    fn begin_main_document_interactive_body(
        &mut self,
        body: MainDocumentLifecycleBody,
        action: MainDocumentInteractiveLifecycleAction,
    ) -> MainDocumentLifecycleStep {
        if !self
            ._context_host
            .borrow_mut()
            .apply_current_main_document_interactive_transition(action)
        {
            tracing::debug!(
                owner = ?action.owner(),
                "ignored stale main document interactive transition"
            );
            return self.not_applied_ordinary_step(
                body,
                MainDocumentLifecycleTargetRejection::TransitionRejected,
            );
        }

        tracing::debug!(
            owner = ?action.owner(),
            "applying document-owned main interactive transition"
        );
        if let Err(error) = self.set_document_ready_state(DocumentReadyState::Interactive) {
            return MainDocumentLifecycleExecution::applied(
                body,
                self.current_main_document_task_owner(),
                MainDocumentLifecycleCallbackEffect::NotEntered,
                MainDocumentLifecycleSettlement::Failed(error.to_string()),
                MainDocumentLifecycleFollowup::None,
            )
            .checkpoint(
                MainDocumentLifecycleCheckpointContinuation::FinishCurrentTaskWithoutCallback,
            );
        }
        // Dispatch failure is warning-only, as before A2. It does not erase
        // the selected task's checkpoint or block image/media admission.
        let _dispatch = self.dispatch_document_lifecycle_event_body_best_effort("readystatechange");
        MainDocumentLifecycleExecution::applied(
            body,
            self.current_main_document_task_owner(),
            MainDocumentLifecycleCallbackEffect::InteractiveReadystatechangeAttempted,
            MainDocumentLifecycleSettlement::Completed,
            MainDocumentLifecycleFollowup::None,
        )
        .checkpoint(
            MainDocumentLifecycleCheckpointContinuation::FinishInteractive {
                owner: action.owner(),
            },
        )
    }

    fn begin_main_document_domcontentloaded_body(
        &mut self,
        body: MainDocumentLifecycleBody,
        owner: FrameDocumentTaskOwner,
    ) -> MainDocumentLifecycleStep {
        let Some(action) = self
            ._context_host
            .borrow_mut()
            .prepare_current_main_document_domcontentloaded_transition(owner)
        else {
            tracing::debug!(
                ?owner,
                "ignored stale or blocked main DOMContentLoaded transition"
            );
            return self.not_applied_ordinary_step(
                body,
                MainDocumentLifecycleTargetRejection::TransitionRejected,
            );
        };
        if !self
            ._context_host
            .borrow_mut()
            .apply_current_main_document_domcontentloaded_transition(action)
        {
            tracing::debug!(
                ?owner,
                "main document was replaced before DOMContentLoaded dispatch"
            );
            return self.not_applied_ordinary_step(
                body,
                MainDocumentLifecycleTargetRejection::TransitionRejected,
            );
        }

        tracing::debug!(
            ?owner,
            "dispatching document-owned main DOMContentLoaded transition"
        );
        self.document_runtime.note_dom_content_loaded_dispatched();
        self.document_runtime
            .record_quirks_mode_inspector_issue_at_dom_content_loaded();
        let event_end =
            match self.dispatch_document_lifecycle_event_body_best_effort("DOMContentLoaded") {
                MainDocumentLifecycleEventDispatch::Completed => {
                    MainDocumentLifecycleDomContentLoadedEventEnd::Record
                }
                MainDocumentLifecycleEventDispatch::FailedBestEffort => {
                    MainDocumentLifecycleDomContentLoadedEventEnd::DispatchFailed
                }
            };
        MainDocumentLifecycleExecution::applied(
            body,
            self.current_main_document_task_owner(),
            MainDocumentLifecycleCallbackEffect::DomContentLoadedAttempted,
            MainDocumentLifecycleSettlement::Completed,
            MainDocumentLifecycleFollowup::None,
        )
        .checkpoint(
            MainDocumentLifecycleCheckpointContinuation::FinishDomContentLoaded {
                owner,
                event_end,
            },
        )
    }

    fn begin_main_document_window_load_body(
        &mut self,
        body: MainDocumentLifecycleBody,
        owner: FrameDocumentTaskOwner,
    ) -> MainDocumentLifecycleStep {
        let Some(action) = self
            ._context_host
            .borrow_mut()
            .prepare_current_main_document_complete_transition(owner)
        else {
            tracing::debug!(?owner, "ignored stale or blocked main complete transition");
            return self.not_applied_ordinary_step(
                body,
                MainDocumentLifecycleTargetRejection::TransitionRejected,
            );
        };
        if !self
            ._context_host
            .borrow_mut()
            .apply_current_main_document_complete_transition(action)
        {
            tracing::debug!(?owner, "ignored stale main complete transition");
            return self.not_applied_ordinary_step(
                body,
                MainDocumentLifecycleTargetRejection::TransitionRejected,
            );
        }

        tracing::debug!(?owner, "applying document-owned main complete transition");
        if let Err(error) = self.set_document_ready_state(DocumentReadyState::Complete) {
            return MainDocumentLifecycleExecution::applied(
                body,
                self.current_main_document_task_owner(),
                MainDocumentLifecycleCallbackEffect::NotEntered,
                MainDocumentLifecycleSettlement::Failed(error.to_string()),
                MainDocumentLifecycleFollowup::None,
            )
            .checkpoint(
                MainDocumentLifecycleCheckpointContinuation::FinishCurrentTaskWithoutCallback,
            );
        }
        // A failed best-effort readystatechange body still owns this selected
        // task and must cross the complete-to-load checkpoint.
        let _dispatch = self.dispatch_document_lifecycle_event_body_best_effort("readystatechange");
        MainDocumentLifecycleExecution::applied(
            body,
            self.current_main_document_task_owner(),
            MainDocumentLifecycleCallbackEffect::CompleteReadystatechangeAttempted,
            MainDocumentLifecycleSettlement::Completed,
            MainDocumentLifecycleFollowup::None,
        )
        .checkpoint(MainDocumentLifecycleCheckpointContinuation::ContinueWindowLoad { owner })
    }

    fn not_applied_ordinary_step(
        &self,
        body: MainDocumentLifecycleBody,
        reason: MainDocumentLifecycleTargetRejection,
    ) -> MainDocumentLifecycleStep {
        let current_owner = self.current_main_document_task_owner();
        let execution =
            MainDocumentLifecycleExecution::completed_not_applied(body, reason, current_owner);
        if current_owner == Some(body.owner()) {
            execution.checkpoint(
                MainDocumentLifecycleCheckpointContinuation::FinishCurrentTaskWithoutCallback,
            )
        } else {
            execution.completed()
        }
    }

    pub(super) fn main_document_lifecycle_owner_is_current(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self._context_host
            .borrow()
            .main_document_task_owner_is_current(owner)
    }
}
