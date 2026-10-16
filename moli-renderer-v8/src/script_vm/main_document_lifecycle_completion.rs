//! Main-document lifecycle checkpoint continuation and turn-exit completion.
//!
//! Body execution and completion are deliberately separate modules. The body
//! can only return a one-shot, stage-specific checkpoint value; this component
//! consumes that value after the coordinator has run the checkpoint. It never
//! selects work, publishes DCL/load authority, or executes a generic runtime
//! drain before the lifecycle journal is reconciled.

#[cfg(test)]
use super::MainDocumentLifecycleBody;
use super::main_document_lifecycle::{
    MainDocumentLifecycleCheckpointContinuation, MainDocumentLifecycleDomContentLoadedEventEnd,
};
use super::{
    MainDocumentLifecycleCallbackEffect, MainDocumentLifecycleCheckpoint,
    MainDocumentLifecycleEventDispatch, MainDocumentLifecycleExecution,
    MainDocumentLifecycleFollowup, MainDocumentLifecycleStep, ScriptVm,
};
use crate::style_engine::StyleInvalidationTurnExitBoundary;

impl ScriptVm {
    /// Compatibility completion used only by low-level ScriptVm execution.
    ///
    /// Checkpoints remain inside this call. Production PageVm lifecycle and
    /// parser-completion work must instead drive
    /// `begin_main_document_lifecycle_body()` and every returned
    /// checkpoint through the dedicated lifecycle coordinator.
    #[cfg(test)]
    pub(crate) fn execute_main_document_lifecycle_body(
        &mut self,
        body: MainDocumentLifecycleBody,
    ) -> MainDocumentLifecycleExecution {
        let execution = self.execute_main_document_lifecycle_body_inner(body);
        self.finish_main_document_lifecycle_turn(execution)
    }

    #[cfg(test)]
    pub(super) fn execute_main_document_lifecycle_body_inner(
        &mut self,
        body: MainDocumentLifecycleBody,
    ) -> MainDocumentLifecycleExecution {
        let mut step = self.begin_main_document_lifecycle_body(body);
        loop {
            match step {
                MainDocumentLifecycleStep::Completed(execution) => return execution,
                MainDocumentLifecycleStep::Checkpoint(checkpoint) => {
                    if let Err(error) = self.perform_owner_lane_task_microtask_checkpoints() {
                        let (mut execution, _) = checkpoint.into_parts();
                        execution.fail(format!(
                            "main-document lifecycle compatibility checkpoint failed: {error:#}"
                        ));
                        return execution;
                    }
                    step = self.resume_main_document_lifecycle_after_checkpoint(checkpoint);
                }
            }
        }
    }

    /// Resume exactly the lifecycle phase authorized by a completed
    /// checkpoint. The returned step either requests the next named checkpoint
    /// or contains the final execution fact.
    pub(crate) fn resume_main_document_lifecycle_after_checkpoint(
        &mut self,
        checkpoint: MainDocumentLifecycleCheckpoint,
    ) -> MainDocumentLifecycleStep {
        let (mut execution, continuation) = checkpoint.into_parts();
        execution.observe_current_owner_after_execution(self.current_main_document_task_owner());
        match continuation {
            MainDocumentLifecycleCheckpointContinuation::FinishInteractive { owner } => {
                if self.main_document_lifecycle_owner_is_current(owner) {
                    if let Err(error) = self.queue_current_main_document_image_load_events() {
                        execution.fail(error.to_string());
                    } else if let Err(error) = self.queue_current_main_document_media_loads() {
                        execution.fail(error.to_string());
                    }
                } else {
                    tracing::debug!(
                        ?owner,
                        "main document was replaced during interactive readystatechange"
                    );
                }
                execution.completed()
            }
            MainDocumentLifecycleCheckpointContinuation::FinishDomContentLoaded {
                owner,
                event_end,
            } => {
                if self.main_document_lifecycle_owner_is_current(owner) {
                    if event_end == MainDocumentLifecycleDomContentLoadedEventEnd::Record {
                        self.record_document_lifecycle_event_end("DOMContentLoaded");
                    }
                    self.queue_main_document_post_parse_autofocus_best_effort(owner);
                } else {
                    tracing::debug!(
                        ?owner,
                        "main document was replaced during DOMContentLoaded dispatch"
                    );
                }
                execution.completed()
            }
            MainDocumentLifecycleCheckpointContinuation::ContinueWindowLoad { owner } => {
                self.continue_main_document_window_load(execution, owner)
            }
            MainDocumentLifecycleCheckpointContinuation::ContinueWindowPageshow { owner } => {
                if !self.main_document_lifecycle_owner_is_current(owner) {
                    tracing::debug!(?owner, "main document was replaced during load dispatch");
                    return execution.completed();
                }
                // A failed best-effort pageshow body still owes the final
                // selected-task checkpoint and exact load settlement.
                let _dispatch = self.dispatch_window_pageshow_event_body_best_effort();
                execution.checkpoint(
                    MainDocumentLifecycleCheckpointContinuation::FinishWindowLoad { owner },
                )
            }
            MainDocumentLifecycleCheckpointContinuation::FinishWindowLoad { owner } => {
                if self.main_document_lifecycle_owner_is_current(owner) {
                    let completion = self
                        ._context_host
                        .borrow_mut()
                        .finish_current_main_document_load_dispatch(owner);
                    if completion.is_none() {
                        execution.fail(
                            "current main load dispatch did not settle exactly once".to_owned(),
                        );
                    } else {
                        let followup = self
                            .prepare_top_level_meta_refresh_navigation(owner)
                            .unwrap_or(MainDocumentLifecycleFollowup::None);
                        execution.set_followup(followup);
                    }
                } else {
                    tracing::debug!(
                        ?owner,
                        "main document was replaced during pageshow dispatch"
                    );
                }
                execution.completed()
            }
            MainDocumentLifecycleCheckpointContinuation::FinishCurrentTaskWithoutCallback => {
                execution.completed()
            }
        }
    }

    /// Continue the load body after readiness, or resume it once a retained
    /// media resource releases its Window load barrier. Readystatechange is
    /// owned by the complete transition and is never repeated here.
    pub(super) fn continue_main_document_window_load(
        &mut self,
        mut execution: MainDocumentLifecycleExecution,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> MainDocumentLifecycleStep {
        if !self.main_document_lifecycle_owner_is_current(owner)
            || !self
                ._context_host
                .borrow_mut()
                .begin_current_main_document_load_dispatch(owner)
        {
            return execution.completed();
        }
        execution.set_callback(MainDocumentLifecycleCallbackEffect::WindowLoadCompoundAttempted);
        let continuation = match self.dispatch_window_load_event_body_best_effort() {
            MainDocumentLifecycleEventDispatch::Completed => {
                MainDocumentLifecycleCheckpointContinuation::ContinueWindowPageshow { owner }
            }
            MainDocumentLifecycleEventDispatch::FailedBestEffort => {
                MainDocumentLifecycleCheckpointContinuation::FinishWindowLoad { owner }
            }
        };
        execution.checkpoint(continuation)
    }

    /// Complete one main-document lifecycle checkpoint and synchronize
    /// child-frame records created by callback reactions. No runtime work is
    /// executed here; milestone publication still belongs to the PageVm
    /// coordinator. Parser exact-DCL reaches this boundary only after its
    /// parser predecessor has completed; this checkpoint belongs to DCL's own
    /// lifecycle task, just like the ordinary lifecycle continuations.
    pub(crate) fn finish_main_document_lifecycle_checkpoint(&mut self) -> anyhow::Result<()> {
        self.perform_owner_lane_task_microtask_checkpoints()?;
        self.sync_child_browsing_context_records();
        Ok(())
    }

    /// Discharge owner transitions, style invalidations, and typed runtime wake
    /// publication once after the lifecycle state machine reaches completion.
    pub(crate) fn finish_main_document_lifecycle_turn<T>(&mut self, result: T) -> T {
        self.finish_runtime_turn_with_style_drain(
            StyleInvalidationTurnExitBoundary::NonScriptPageTask,
            result,
        )
    }
}
