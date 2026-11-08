//! Synchronous exception reporting that completes classic-script evaluation.
//!
//! This checkpoint is part of the classic-script evaluation algorithm. It is
//! neither the script element's later load/error terminal nor the enclosing
//! selected Page task completion.

use anyhow::Result;

use super::ScriptVm;
use crate::context_bootstrap::dispatch_window_error_event_with_details;
use crate::exception_reporting::{V8ExceptionReport, log_uncaught_script_exception};

impl ScriptVm {
    pub(super) fn report_classic_script_exception_and_finish_evaluation_best_effort(
        &mut self,
        report: &V8ExceptionReport,
    ) {
        log_uncaught_script_exception(report);
        if let Err(error) = self.dispatch_classic_script_exception_and_finish_evaluation(report) {
            self.record_runtime_warning(format_args!(
                "classic script exception reporting failed: {error}"
            ));
        }
    }

    fn dispatch_classic_script_exception_and_finish_evaluation(
        &mut self,
        report: &V8ExceptionReport,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            let error_value = report
                .exception
                .as_ref()
                .map(|exception| v8::Local::new(scope, exception));
            let dispatch_result = dispatch_window_error_event_with_details(
                scope,
                host_ptr,
                &report.summary,
                report.source.as_deref().unwrap_or(""),
                report.line.unwrap_or(0) as u32,
                report.column.unwrap_or(0) as u32,
                error_value,
            )
            .map_err(anyhow::Error::msg);
            let checkpoint_result = Self::perform_microtask_checkpoints(scope, None);
            dispatch_result?;
            checkpoint_result
        })
    }
}
