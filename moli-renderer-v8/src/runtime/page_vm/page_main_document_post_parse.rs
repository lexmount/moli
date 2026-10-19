//! Shared task-completion coordinator for main-Document post-parse work.
//!
//! Lifecycle residence, parse-time execution, and the main-Document runtime
//! source keep their own scheduling authority. Once one of the eight generic
//! bodies has executed, every carrier submits the same typed fact here. This
//! component owns exactly one task-end, Page report/journal publication, and
//! lifecycle priming; it never selects another Page task.

use anyhow::Result;

use crate::{
    frame_owner_model::MainDocumentLoadCompletionState,
    page_task_queue::MainDocumentPostParseExecution,
    runtime::{RendererDocumentLifecycleMilestone, RendererDocumentLifecycleTransition},
};

use super::PageVm;

impl PageVm {
    pub(super) fn finish_main_document_post_parse_execution(
        &mut self,
        execution: MainDocumentPostParseExecution,
    ) -> Result<()> {
        let kind = execution.kind();
        let target = execution.target();
        let callback = execution.callback();
        let task_end = execution.task_end();
        let applied = target.applied_to_selected_owner();
        let lifecycle_identity = self.document_lifecycle.identity();

        self.vm_mut()
            .finish_main_document_post_parse_task_end(task_end)?;

        match execution {
            MainDocumentPostParseExecution::FinishNavigationImageDocument(_) => {
                tracing::debug!("completed navigation image response task");
            }
            MainDocumentPostParseExecution::SeedDocumentOwnedBlockingStylesheets(execution) => {
                let (_, accepted_count) = execution.into_parts();
                tracing::debug!(accepted_count, "completed blocking stylesheet seed task");
            }
            MainDocumentPostParseExecution::RecordDocumentScriptRun(execution) => {
                let (target, run) = execution.into_parts();
                if target.applied_to_selected_owner() {
                    self.report.runs.push(run);
                }
            }
            MainDocumentPostParseExecution::DispatchContentSecurityPolicyViolation(execution)
            | MainDocumentPostParseExecution::DispatchScriptEvent(execution)
            | MainDocumentPostParseExecution::ReportWindowScriptFailure(execution) => {
                tracing::debug!(
                    callback = ?execution.callback(),
                    settlement = ?execution.settlement(),
                    "completed post-parse callback task"
                );
            }
            MainDocumentPostParseExecution::SettleMainDocumentScriptLoadDelay(execution) => {
                let (target, effect) = execution.into_parts();
                tracing::debug!(
                    ?target,
                    owner = ?effect.owner(),
                    kind = ?effect.kind(),
                    release = ?effect.release(),
                    "completed main-document script load-delay settlement task"
                );
            }
            MainDocumentPostParseExecution::CheckMainDocumentCompletion(execution) => {
                let (target, effect) = execution.into_parts();
                tracing::debug!(
                    ?target,
                    owner = ?effect.owner(),
                    readiness = ?effect.readiness(),
                    load_completion = ?effect.load_completion(),
                    "completed main document completion recheck task"
                );
                if target.applied_to_selected_owner()
                    && effect.load_completion() == Some(MainDocumentLoadCompletionState::Completed)
                    && self.document_lifecycle.identity() == lifecycle_identity
                    && self.vm().current_main_document_task_owner() == Some(effect.owner())
                {
                    let transition = self.document_lifecycle.complete_pending_milestone(
                        lifecycle_identity,
                        RendererDocumentLifecycleMilestone::Load,
                    );
                    if !matches!(transition, RendererDocumentLifecycleTransition::Recorded(_)) {
                        tracing::debug!(
                            owner = ?effect.owner(),
                            ?lifecycle_identity,
                            ?transition,
                            "renderer lifecycle journal rejected descendant-completed load milestone"
                        );
                    }
                }
            }
            MainDocumentPostParseExecution::RecordDetachedPostParseRuns(execution) => {
                let (target, runs) = execution.into_parts();
                if target.applied_to_selected_owner() {
                    self.report.runs.extend(runs);
                }
            }
        }

        if applied {
            self.vm_mut()
                .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        }
        tracing::debug!(
            kind,
            ?target,
            ?callback,
            ?task_end,
            "completed typed post-parse task"
        );
        Ok(())
    }

    pub(super) fn discard_stale_main_document_post_parse_execution(
        &self,
        work: crate::page_task_queue::MainDocumentPostParseWork,
    ) -> MainDocumentPostParseExecution {
        work.discarded_stale(self.vm().current_main_document_post_parse_owner())
    }
}
