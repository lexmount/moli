//! Synchronous exception reporting that completes classic-script evaluation.
//!
//! This checkpoint is part of the classic-script evaluation algorithm. It is
//! neither the script element's later load/error terminal nor the enclosing
//! selected Page task completion.

use std::pin::pin;

use anyhow::Result;

use super::ScriptVm;
use crate::context_bootstrap::dispatch_window_error_event_with_details;
use crate::exception_reporting::{V8ExceptionReport, log_uncaught_script_exception};
use crate::native_bridge::JsContextHost;

impl ScriptVm {
    pub(super) fn report_classic_script_exception_and_finish_evaluation_best_effort(
        &mut self,
        report: &V8ExceptionReport,
        muted_errors: bool,
    ) {
        log_uncaught_script_exception(report);
        if let Err(error) =
            self.dispatch_classic_script_exception_and_finish_evaluation(report, muted_errors)
        {
            self.record_runtime_warning(format_args!(
                "classic script exception reporting failed: {error}"
            ));
        }
    }

    fn dispatch_classic_script_exception_and_finish_evaluation(
        &mut self,
        report: &V8ExceptionReport,
        muted_errors: bool,
    ) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let error_value = if muted_errors {
                    None
                } else {
                    report
                        .exception
                        .as_ref()
                        .map(|exception| v8::Local::new(scope, exception))
                };
                let (summary, source, line, column) = if muted_errors {
                    ("Script error.", "", 0, 0)
                } else {
                    (
                        report.summary.as_str(),
                        report.source.as_deref().unwrap_or(""),
                        report.line.unwrap_or(0) as u32,
                        report.column.unwrap_or(0) as u32,
                    )
                };
                // SAFETY: as_ptr() — V8 callbacks are re-entrant; borrow_mut() panics. See util.rs.
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                let execution_scope = crate::script_cleanup::ScriptExecutionScope::enter(scope);
                let dispatch_result = dispatch_window_error_event_with_details(
                    scope,
                    host_ptr,
                    summary,
                    source,
                    line,
                    column,
                    error_value,
                )
                .map_err(anyhow::Error::msg);
                drop(execution_scope);
                let checkpoint_result = Self::perform_microtask_checkpoints(scope, None);
                dispatch_result?;
                checkpoint_result
            })
    }
}
