//! Body-only execution for generic main-Document post-parse work.
//!
//! These bodies used to share a generic pre-task checkpoint and three callback
//! helpers that performed another checkpoint internally. This module performs
//! only the selected work and returns a concrete execution fact. PageVm owns
//! the single task-end checkpoint, report/journal publication, and lifecycle
//! priming in its sibling completion coordinator.

use super::ScriptVm;
use super::security_policy::ContentSecurityPolicyViolationBodyExecution;
use crate::{
    dom::native::DocumentReadyState,
    page_task_queue::{
        MainDocumentCompletionRecheckEffect, MainDocumentPostParseCallbackExecution,
        MainDocumentPostParseCallbackSettlement, MainDocumentPostParseExecution,
        MainDocumentPostParseOwner, MainDocumentPostParseStateExecution,
        MainDocumentPostParseTargetEffect, MainDocumentPostParseWork,
        MainDocumentScriptLoadDelayEffect,
    },
};

impl ScriptVm {
    pub(crate) fn current_main_document_post_parse_owner(
        &self,
    ) -> Option<MainDocumentPostParseOwner> {
        self.current_main_document_task_owner()
            .map(MainDocumentPostParseOwner::new)
    }

    fn applied_main_document_post_parse_target(
        &self,
        selected_owner: MainDocumentPostParseOwner,
    ) -> MainDocumentPostParseTargetEffect {
        MainDocumentPostParseTargetEffect::applied(
            selected_owner,
            self.current_main_document_post_parse_owner(),
        )
    }

    /// Execute one already-selected post-parse body without checkpointing,
    /// draining deferred work, reconciling owner transitions, or priming
    /// lifecycle successors.
    pub(crate) fn execute_main_document_post_parse_body(
        &mut self,
        selected_owner: MainDocumentPostParseOwner,
        work: MainDocumentPostParseWork,
    ) -> MainDocumentPostParseExecution {
        let current_owner = self.current_main_document_post_parse_owner();
        if current_owner != Some(selected_owner) {
            return work.discarded_stale(current_owner);
        }

        match work {
            MainDocumentPostParseWork::SeedDocumentOwnedBlockingStylesheets(inputs) => {
                let accepted = self.accept_main_document_blocking_stylesheet_inputs(
                    selected_owner.document_owner(),
                    &inputs,
                );
                MainDocumentPostParseExecution::SeedDocumentOwnedBlockingStylesheets(
                    MainDocumentPostParseStateExecution::new(
                        self.applied_main_document_post_parse_target(selected_owner),
                        accepted,
                    ),
                )
            }
            MainDocumentPostParseWork::RecordDocumentScriptRun(run) => {
                self.document_runtime
                    .set_document_ready_state(DocumentReadyState::Loading);
                MainDocumentPostParseExecution::RecordDocumentScriptRun(
                    MainDocumentPostParseStateExecution::new(
                        self.applied_main_document_post_parse_target(selected_owner),
                        run,
                    ),
                )
            }
            MainDocumentPostParseWork::DispatchContentSecurityPolicyViolation(task) => {
                if task.owner() != selected_owner.document_owner() {
                    return MainDocumentPostParseWork::DispatchContentSecurityPolicyViolation(task)
                        .discarded_stale(self.current_main_document_post_parse_owner());
                }
                let settlement = match self
                    .dispatch_content_security_policy_violation_event_body(&task)
                {
                    ContentSecurityPolicyViolationBodyExecution::DiscardedStaleDocument => {
                        return MainDocumentPostParseWork::DispatchContentSecurityPolicyViolation(
                            task,
                        )
                        .discarded_stale(self.current_main_document_post_parse_owner());
                    }
                    ContentSecurityPolicyViolationBodyExecution::DispatchAttempted(Ok(())) => {
                        MainDocumentPostParseCallbackSettlement::Completed
                    }
                    ContentSecurityPolicyViolationBodyExecution::DispatchAttempted(Err(error)) => {
                        self.record_runtime_warning(format_args!(
                            "securitypolicyviolation page-task body failed for `{}`: {error}",
                            task.violation().blocked_uri
                        ));
                        MainDocumentPostParseCallbackSettlement::FailedBestEffort
                    }
                };
                MainDocumentPostParseExecution::DispatchContentSecurityPolicyViolation(
                    MainDocumentPostParseCallbackExecution::dispatch_attempted(
                        self.applied_main_document_post_parse_target(selected_owner),
                        settlement,
                    ),
                )
            }
            MainDocumentPostParseWork::DispatchScriptEvent(task) => {
                let settlement = match self.dispatch_script_event_body(&task) {
                    Ok(()) => MainDocumentPostParseCallbackSettlement::Completed,
                    Err(error) => {
                        self.record_runtime_warning(format_args!(
                            "script {} body dispatch failed for `{}`: {error}",
                            task.event_name(),
                            task.handle
                        ));
                        MainDocumentPostParseCallbackSettlement::FailedBestEffort
                    }
                };
                MainDocumentPostParseExecution::DispatchScriptEvent(
                    MainDocumentPostParseCallbackExecution::dispatch_attempted(
                        self.applied_main_document_post_parse_target(selected_owner),
                        settlement,
                    ),
                )
            }
            MainDocumentPostParseWork::ReportWindowScriptFailure(task) => {
                let settlement = match self.report_window_error_body(
                    &task.message,
                    task.filename.as_deref(),
                    task.error_constructor,
                ) {
                    Ok(()) => MainDocumentPostParseCallbackSettlement::Completed,
                    Err(error) => {
                        self.record_runtime_warning(format_args!(
                            "window script failure body failed for `{}`: {error}",
                            task.filename.as_deref().unwrap_or("")
                        ));
                        MainDocumentPostParseCallbackSettlement::FailedBestEffort
                    }
                };
                MainDocumentPostParseExecution::ReportWindowScriptFailure(
                    MainDocumentPostParseCallbackExecution::dispatch_attempted(
                        self.applied_main_document_post_parse_target(selected_owner),
                        settlement,
                    ),
                )
            }
            MainDocumentPostParseWork::SettleMainDocumentScriptLoadDelay(binding) => {
                if binding.owner() != selected_owner.document_owner() {
                    return MainDocumentPostParseWork::SettleMainDocumentScriptLoadDelay(binding)
                        .discarded_stale(self.current_main_document_post_parse_owner());
                }
                let owner = binding.owner();
                let kind = binding.kind();
                let load_delay_token = binding.load_delay_token();
                let release = self
                    ._context_host
                    .borrow_mut()
                    .release_main_document_script_load_delay(binding);
                if !release.released() {
                    tracing::warn!(
                        ?owner,
                        ?kind,
                        ?load_delay_token,
                        "current exact script load-delay lease was not owned at settlement"
                    );
                }
                tracing::debug!(
                    ?owner,
                    ?kind,
                    ?load_delay_token,
                    ?release,
                    "settled main async script lifecycle binding inside selected body"
                );
                MainDocumentPostParseExecution::SettleMainDocumentScriptLoadDelay(
                    MainDocumentPostParseStateExecution::new(
                        self.applied_main_document_post_parse_target(selected_owner),
                        MainDocumentScriptLoadDelayEffect::released(owner, kind, release),
                    ),
                )
            }
            MainDocumentPostParseWork::CheckMainDocumentCompletion { owner } => {
                if owner != selected_owner.document_owner() {
                    return MainDocumentPostParseWork::CheckMainDocumentCompletion { owner }
                        .discarded_stale(self.current_main_document_post_parse_owner());
                }
                let load_completion =
                    self.finish_main_document_load_after_descendant_completion(owner);
                let readiness = self.check_main_document_completion(owner);
                MainDocumentPostParseExecution::CheckMainDocumentCompletion(
                    MainDocumentPostParseStateExecution::new(
                        self.applied_main_document_post_parse_target(selected_owner),
                        MainDocumentCompletionRecheckEffect::applied(
                            owner,
                            load_completion,
                            readiness,
                        ),
                    ),
                )
            }
            MainDocumentPostParseWork::RecordDetachedPostParseRuns(runs) => {
                MainDocumentPostParseExecution::RecordDetachedPostParseRuns(
                    MainDocumentPostParseStateExecution::new(
                        self.applied_main_document_post_parse_target(selected_owner),
                        runs,
                    ),
                )
            }
        }
    }
}
