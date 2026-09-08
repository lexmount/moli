//! Named completion boundaries for synchronous script terminal algorithms.
//!
//! Most script terminals are selected Page tasks and therefore use only the
//! body APIs from `script_event_body`; their dispatcher owns task completion.
//! The methods in this module cover the remaining synchronous algorithms whose
//! observable contract already includes an immediate checkpoint. Keeping each
//! public entry tied to its real carrier prevents a generic event helper from
//! silently acquiring checkpoint authority again.

use anyhow::{Context, Result};

use super::{ScriptTerminalBodyActivity, ScriptVm};
use crate::host::ScriptEventTask;
use crate::page_task_queue::{PostParseLifecycleWork, WindowScriptFailureReportTask};
use crate::planning::PreparedScript;

impl ScriptVm {
    fn dispatch_script_event_and_finish_synchronous_checkpoint(
        &mut self,
        task: &ScriptEventTask,
        boundary: &'static str,
    ) -> Result<()> {
        self.dispatch_script_event_body(task)?;
        self.perform_owner_lane_task_microtask_checkpoints()
            .with_context(|| format!("{boundary} checkpoint failed"))
    }

    fn report_window_error_and_finish_synchronous_checkpoint(
        &mut self,
        message: &str,
        filename: Option<&str>,
        error_value: Option<crate::types::ScriptErrorValue>,
        boundary: &'static str,
    ) -> Result<()> {
        self.report_window_error_body(message, filename, error_value)?;
        self.perform_owner_lane_task_microtask_checkpoints()
            .with_context(|| format!("{boundary} checkpoint failed"))
    }

    fn dispatch_planned_script_failure_and_finish_each_synchronous_checkpoint(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_value: Option<crate::types::ScriptErrorValue>,
        boundary: &'static str,
    ) {
        let planned_failure_work = self.document_runtime.plan_script_failure_lifecycle_work(
            script,
            message,
            module_failure_policy,
            error_value,
        );
        for work in planned_failure_work {
            let result = match work {
                PostParseLifecycleWork::DispatchScriptEvent(task) => {
                    self.dispatch_script_event_and_finish_synchronous_checkpoint(&task, boundary)
                }
                PostParseLifecycleWork::ReportWindowScriptFailure(task) => self
                    .report_window_error_and_finish_synchronous_checkpoint(
                        &task.message,
                        task.filename.as_deref(),
                        task.error_value,
                        boundary,
                    ),
                _ => continue,
            };
            if let Err(error) = result {
                self.record_runtime_warning(format_args!(
                    "{boundary} failed for `{}`: {error}",
                    script.url
                ));
            }
        }
    }

    /// Report an invalid parser-owned import map and complete that synchronous
    /// parser algorithm step. This is not a script-element terminal task.
    pub(crate) fn report_parser_import_map_registration_failure_and_finish_algorithm_best_effort(
        &mut self,
        message: &str,
        filename: Option<&str>,
    ) {
        if let Err(error) = self.report_window_error_and_finish_synchronous_checkpoint(
            message,
            filename,
            None,
            "parser import-map failure reporting",
        ) {
            self.record_runtime_warning(format_args!(
                "parser import-map failure reporting failed for `{}`: {error}",
                filename.unwrap_or("")
            ));
        }
    }

    /// Complete a parser-owned module failure that is still settled inside the
    /// module algorithm rather than returned to a selected parser task.
    pub(crate) fn dispatch_parser_owned_module_failure_and_finish_settlement_best_effort(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_value: Option<crate::types::ScriptErrorValue>,
    ) {
        self.dispatch_planned_script_failure_and_finish_each_synchronous_checkpoint(
            script,
            message,
            module_failure_policy,
            error_value,
            "parser-owned module failure settlement",
        );
    }

    /// Complete an unclaimed runtime-script terminal that is still executed by
    /// the legacy synchronous runtime algorithm. Typed DocumentScript actions
    /// use the body-only path and the selected-task dispatcher instead.
    pub(super) fn dispatch_unclaimed_runtime_script_failure_and_finish_terminal_best_effort(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_value: Option<crate::types::ScriptErrorValue>,
    ) {
        self.dispatch_planned_script_failure_and_finish_each_synchronous_checkpoint(
            script,
            message,
            module_failure_policy,
            error_value,
            "unclaimed runtime-script terminal",
        );
    }

    /// Report a TLA rejection whose module settlement owns the reaction
    /// checkpoint. Selected parser continuations use the body-only branch.
    pub(crate) fn report_module_tla_rejection_and_finish_reaction_best_effort(
        &mut self,
        message: &str,
        filename: Option<&str>,
        error_value: Option<crate::types::ScriptErrorValue>,
    ) {
        if let Err(error) = self.report_window_error_and_finish_synchronous_checkpoint(
            message,
            filename,
            error_value,
            "module TLA rejection reaction",
        ) {
            self.record_runtime_warning(format_args!(
                "module TLA rejection reporting failed for `{}`: {error}",
                filename.unwrap_or("")
            ));
        }
    }

    /// Complete a runtime-owned script-element event still carried by the
    /// synchronous runtime terminal algorithm.
    pub(super) fn dispatch_runtime_script_terminal_event_and_finish_checkpoint_best_effort(
        &mut self,
        task: &ScriptEventTask,
    ) {
        if let Err(error) = self.dispatch_script_event_and_finish_synchronous_checkpoint(
            task,
            "runtime script terminal",
        ) {
            self.record_runtime_warning(format_args!(
                "runtime script {} terminal failed for `{}`: {error}",
                task.event_name(),
                task.handle
            ));
        }
    }

    /// Complete a runtime-owned Window error still carried by the synchronous
    /// runtime terminal algorithm.
    pub(super) fn report_runtime_script_failure_terminal_and_finish_checkpoint_best_effort(
        &mut self,
        task: &WindowScriptFailureReportTask,
    ) {
        if let Err(error) = self.report_window_error_and_finish_synchronous_checkpoint(
            &task.message,
            task.filename.as_deref(),
            task.error_value,
            "runtime script failure terminal",
        ) {
            self.record_runtime_warning(format_args!(
                "runtime script failure terminal failed for `{}`: {error}",
                task.filename.as_deref().unwrap_or("")
            ));
        }
    }

    /// Plan and dispatch a terminal body without completing its selected task.
    pub(crate) fn dispatch_current_prepared_script_error_body_best_effort(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_value: Option<crate::types::ScriptErrorValue>,
    ) -> ScriptTerminalBodyActivity {
        let planned_failure_work = self.document_runtime.plan_script_failure_lifecycle_work(
            script,
            message,
            module_failure_policy,
            error_value,
        );
        let mut activity = ScriptTerminalBodyActivity::NoEventDispatch;
        for work in planned_failure_work {
            match work {
                PostParseLifecycleWork::DispatchScriptEvent(task) => {
                    self.dispatch_script_event_body_best_effort(&task);
                    activity = ScriptTerminalBodyActivity::EventDispatchAttempted;
                }
                PostParseLifecycleWork::ReportWindowScriptFailure(task) => {
                    self.report_window_error_body_best_effort(
                        &task.message,
                        task.filename.as_deref(),
                        task.error_value,
                    );
                    activity = ScriptTerminalBodyActivity::EventDispatchAttempted;
                }
                _ => {}
            }
        }
        activity
    }

    #[cfg(test)]
    pub(crate) fn dispatch_script_event_and_checkpoint_for_test(&mut self, task: &ScriptEventTask) {
        if let Err(error) = self
            .dispatch_script_event_and_finish_synchronous_checkpoint(task, "test-only script event")
        {
            self.record_runtime_warning(format_args!(
                "test-only script {} dispatch failed for `{}`: {error}",
                task.event_name(),
                task.handle
            ));
        }
    }

    #[cfg(test)]
    pub(crate) fn report_window_script_failure_task_and_checkpoint_for_test(
        &mut self,
        task: &WindowScriptFailureReportTask,
    ) {
        self.report_window_script_failure_and_checkpoint_for_test(
            &task.message,
            task.filename.as_deref(),
            task.error_value,
        );
    }

    #[cfg(test)]
    pub(crate) fn report_window_script_failure_and_checkpoint_for_test(
        &mut self,
        message: &str,
        filename: Option<&str>,
        error_value: Option<crate::types::ScriptErrorValue>,
    ) {
        if let Err(error) = self.report_window_error_and_finish_synchronous_checkpoint(
            message,
            filename,
            error_value,
            "test-only Window error",
        ) {
            self.record_runtime_warning(format_args!(
                "test-only Window error dispatch failed for `{}`: {error}",
                filename.unwrap_or("")
            ));
        }
    }
}
