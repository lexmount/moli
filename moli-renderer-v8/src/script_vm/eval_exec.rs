use std::pin::pin;

use anyhow::{Result, anyhow};
use tracing::debug;
use url::Url;

use super::ScriptVm;
use super::native_module::RuntimeModuleScriptGraphStart;
use super::post_parse::dynamic_script_execute_is_runnable_before_dom_content_loaded;
use super::runtime_bindings::perform_microtask_checkpoint_and_report_pending_promise_rejections;
use crate::context_bootstrap::dispatch_window_error_event_with_details;
use crate::dynamic_script_owner::{DynamicScriptOwnerPoll, DynamicScriptRunnable};
use crate::exception_reporting::{
    V8ExceptionReport, build_event_handler_exception_report, uncaught_script_error,
};
use crate::network::ResourceRequestClient;
use crate::script_provenance::CompiledStringProvenance;
use crate::style_engine::StyleInvalidationTurnExitBoundary;
use crate::util::{
    context_host_ptr_from_global_bridge, create_script_origin_with_base_url_and_nonce,
    script_base_url_continuation_data, v8_string,
};
use crate::v8_execution_watchdog::{
    SCRIPT_TURN_WATCHDOG_TIMEOUT, V8ExecutionWatchdog, V8ExecutionWatchdogKind,
    V8ExecutionWatchdogOutcome,
};

/// Source-neutral result of one bounded runtime-script owner flush.
///
/// `WaitingForSource` means the exact runtime owner still retains work but no
/// executable item exists. The owner has already armed its stable completion
/// route before this value is returned; callers must yield to the Page
/// scheduler instead of awaiting network input inside the executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuntimePendingWorkFlushOutcome {
    Complete,
    WaitingForSource,
}

#[must_use = "raw script execution must be completed through an explicit turn boundary"]
pub(super) struct PendingScriptTurn<T> {
    result: T,
}

pub(super) enum RawScriptExecutionError {
    Exception {
        report: Box<V8ExceptionReport>,
        phase: &'static str,
    },
    Internal(anyhow::Error),
}

type RawScriptExecutionResult<T> = std::result::Result<T, RawScriptExecutionError>;

impl RawScriptExecutionError {
    pub(super) fn into_anyhow(self) -> anyhow::Error {
        match self {
            Self::Exception { report, phase } => uncaught_script_error(*report, phase),
            Self::Internal(error) => error,
        }
    }
}

impl From<anyhow::Error> for RawScriptExecutionError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

struct MainFrameScriptJob<'a> {
    source: &'a str,
    provenance: Option<CompiledStringProvenance>,
    line_offset: i32,
    script_nonce: Option<&'a str>,
    drain_microtasks: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UncaughtScriptReportTarget {
    LogOnly,
    CurrentWindow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextStringEvaluationKind {
    PageScript,
    InspectorInternal,
}

impl ContextStringEvaluationKind {
    fn compile<'s>(
        self,
        scope: &v8::PinScope<'s, '_>,
        source: v8::Local<'s, v8::String>,
    ) -> Option<v8::Local<'s, v8::Script>> {
        match self {
            Self::PageScript => v8::Script::compile(scope, source, None),
            Self::InspectorInternal => v8::inspector::compile_inspector_script(scope, source)
                .map(|script| script.bind_to_current_context(scope)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceTextScriptCompletionMode {
    Ignore,
    ValueTypeAware,
}

#[derive(Clone, Copy)]
enum EvalStringMicrotaskCheckpoint {
    Perform,
    #[cfg(test)]
    SkipForSelectedTaskBoundaryObservation,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SourceTextScriptCompletion {
    Ignored,
    String(String),
    NonString,
}

fn script_exception_error(
    scope: &mut v8::PinScope<'_, '_>,
    report: V8ExceptionReport,
    phase: &'static str,
    target: UncaughtScriptReportTarget,
) -> RawScriptExecutionError {
    if target == UncaughtScriptReportTarget::CurrentWindow {
        report_script_exception_to_current_window(scope, &report);
    }
    RawScriptExecutionError::Exception {
        report: Box::new(report),
        phase,
    }
}

fn report_script_exception_to_current_window(
    scope: &mut v8::PinScope<'_, '_>,
    report: &V8ExceptionReport,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let error_value = report
        .exception
        .as_ref()
        .map(|exception| v8::Local::new(scope, exception));
    let _ = dispatch_window_error_event_with_details(
        scope,
        host_ptr,
        &report.summary,
        report.source.as_deref().unwrap_or(""),
        report.line.unwrap_or(0) as u32,
        report.column.unwrap_or(0) as u32,
        error_value,
    );
}

pub(crate) fn execute_source_text_on_current_stack(
    scope: &mut v8::PinScope<'_, '_>,
    source: &str,
    script_url: Option<&Url>,
    script_base_url: Option<&Url>,
    line_offset: i32,
    script_nonce: Option<&str>,
    drain_microtasks: bool,
) -> Result<()> {
    let provenance = script_url.cloned().map(|source_url| {
        let module_base_url = script_base_url
            .cloned()
            .unwrap_or_else(|| source_url.clone());
        CompiledStringProvenance::new(source_url, module_base_url)
    });
    execute_source_text_on_current_stack_with_completion(
        scope,
        source,
        provenance.as_ref(),
        line_offset,
        script_nonce,
        drain_microtasks,
        UncaughtScriptReportTarget::CurrentWindow,
        SourceTextScriptCompletionMode::Ignore,
    )
    .map(|_| ())
    .map_err(RawScriptExecutionError::into_anyhow)
}

#[allow(clippy::too_many_arguments)]
fn execute_source_text_on_current_stack_with_completion(
    scope: &mut v8::PinScope<'_, '_>,
    source: &str,
    provenance: Option<&CompiledStringProvenance>,
    line_offset: i32,
    script_nonce: Option<&str>,
    drain_microtasks: bool,
    report_target: UncaughtScriptReportTarget,
    completion_mode: SourceTextScriptCompletionMode,
) -> RawScriptExecutionResult<SourceTextScriptCompletion> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    if let Some(provenance) = provenance
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(&mut scope)
    {
        unsafe { &mut *host_ptr }.register_compiled_string_provenance(provenance);
    }
    let source =
        v8_string(&scope, source).ok_or_else(|| anyhow!("failed to allocate v8 source string"))?;
    let origin = provenance.map(|provenance| {
        create_script_origin_with_base_url_and_nonce(
            &mut scope,
            provenance.source_url().as_str(),
            line_offset,
            Some(provenance.module_base_url()),
            script_nonce,
        )
    });
    let script = v8::Script::compile(&scope, source, origin.as_ref()).ok_or_else(|| {
        let exception = scope.exception();
        let message = scope.message();
        let stack_trace = scope.stack_trace();
        let report =
            build_event_handler_exception_report(&mut scope, exception, message, stack_trace);
        script_exception_error(&mut scope, report, "compile", report_target)
    })?;
    let previous_continuation_data = scope.get_continuation_preserved_embedder_data();
    if let Some(provenance) = provenance
        && let Some(value) =
            script_base_url_continuation_data(&mut scope, provenance.module_base_url())
    {
        scope.set_continuation_preserved_embedder_data(value);
    }
    let watchdog = V8ExecutionWatchdog::arm(
        V8ExecutionWatchdogKind::ScriptTurn,
        scope.thread_safe_handle(),
        SCRIPT_TURN_WATCHDOG_TIMEOUT,
    );
    let run_result = script.run(&scope);
    let watchdog_timed_out = watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut;
    scope.set_continuation_preserved_embedder_data(previous_continuation_data);
    let value = run_result.ok_or_else(|| {
        if watchdog_timed_out {
            return RawScriptExecutionError::Internal(anyhow!(
                "script execution exceeded {:?} and was terminated",
                SCRIPT_TURN_WATCHDOG_TIMEOUT
            ));
        }
        let exception = scope.exception();
        let message = scope.message();
        let stack_trace = scope.stack_trace();
        let report =
            build_event_handler_exception_report(&mut scope, exception, message, stack_trace);
        script_exception_error(&mut scope, report, "execute", report_target)
    })?;
    let completion = match completion_mode {
        SourceTextScriptCompletionMode::Ignore => SourceTextScriptCompletion::Ignored,
        SourceTextScriptCompletionMode::ValueTypeAware if value.is_string() => {
            let text = value
                .to_string(&scope)
                .ok_or_else(|| anyhow!("v8 string completion did not stringify"))?;
            SourceTextScriptCompletion::String(text.to_rust_string_lossy(&scope))
        }
        SourceTextScriptCompletionMode::ValueTypeAware => SourceTextScriptCompletion::NonString,
    };
    if drain_microtasks {
        ScriptVm::perform_microtask_checkpoints(
            &mut scope,
            provenance.map(CompiledStringProvenance::source_url),
        )?;
    }
    Ok(completion)
}

impl<T> PendingScriptTurn<T> {
    fn new(result: T) -> Self {
        Self { result }
    }

    fn map<U>(self, map: impl FnOnce(T) -> U) -> PendingScriptTurn<U> {
        // Transform the result without discharging the required turn-exit boundary.
        PendingScriptTurn {
            result: map(self.result),
        }
    }

    pub(super) fn finish_with_style_drain(
        self,
        vm: &mut ScriptVm,
        boundary: StyleInvalidationTurnExitBoundary,
    ) -> T {
        vm.finish_runtime_turn_with_style_drain(boundary, self.result)
    }

    pub(super) fn finish_with_child_record_sync_and_style_drain(
        self,
        vm: &mut ScriptVm,
        boundary: StyleInvalidationTurnExitBoundary,
    ) -> T {
        vm.finish_runtime_turn_with_child_record_sync_and_style_drain(boundary, self.result)
    }

    fn into_inner_for_enclosing_script_turn(self) -> T {
        self.result
    }

    pub(super) fn into_inner_for_internal_snapshot(self) -> T {
        self.result
    }

    #[cfg(test)]
    pub(super) fn into_inner_for_test(self) -> T {
        self.result
    }
}

impl ScriptVm {
    // Test-only raw script execution. Production owner-level standalone turns
    // must use `exec_runtime_turn` so style invalidation drain stays explicit at
    // the turn boundary.
    #[cfg(test)]
    pub(crate) fn exec(&mut self, source: &str, script_url: Option<&Url>) -> Result<()> {
        self.exec_without_turn_drain_with_options(source, script_url, 0, None, true)
            .into_inner_for_test()
    }

    pub(crate) fn exec_runtime_turn(
        &mut self,
        source: &str,
        script_url: Option<&Url>,
    ) -> Result<()> {
        let pending = self.exec_without_turn_drain_with_options(source, script_url, 0, None, true);
        pending.finish_with_style_drain(self, StyleInvalidationTurnExitBoundary::RuntimeEvaluate)
    }

    // Executes script work that is already covered by an outer runtime-work
    // flush. Standalone owner turns must call `exec_runtime_turn` instead.
    fn exec_without_turn_drain_with_options(
        &mut self,
        source: &str,
        script_url: Option<&Url>,
        line_offset: i32,
        script_nonce: Option<&str>,
        drain_microtasks: bool,
    ) -> PendingScriptTurn<Result<()>> {
        let job = MainFrameScriptJob {
            source,
            provenance: script_url.cloned().map(CompiledStringProvenance::at_url),
            line_offset,
            script_nonce,
            drain_microtasks,
        };
        self.execute_main_frame_script_job_without_turn_drain(job)
            .map(|result| {
                let result = result.map_err(RawScriptExecutionError::into_anyhow);
                if result.is_ok() {
                    self.sync_child_browsing_context_records();
                }
                result
            })
    }

    fn execute_main_frame_script_job_without_turn_drain(
        &mut self,
        job: MainFrameScriptJob<'_>,
    ) -> PendingScriptTurn<RawScriptExecutionResult<()>> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context as *const _;
        self.exec_in_context_ptr_without_turn_drain(
            context_ptr,
            job.source,
            job.provenance,
            job.line_offset,
            job.script_nonce,
            job.drain_microtasks,
            UncaughtScriptReportTarget::LogOnly,
        )
    }

    pub(super) fn exec_in_enclosing_script_turn_with_provenance(
        &mut self,
        source: &str,
        provenance: &CompiledStringProvenance,
        line_offset: i32,
        script_nonce: Option<&str>,
        drain_microtasks: bool,
    ) -> std::result::Result<(), RawScriptExecutionError> {
        self.execute_main_frame_script_job_without_turn_drain(MainFrameScriptJob {
            source,
            provenance: Some(provenance.clone()),
            line_offset,
            script_nonce,
            drain_microtasks,
        })
        .into_inner_for_enclosing_script_turn()
    }

    pub(super) fn exec_in_context_ptr_runtime_turn(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        source: &str,
        script_url: Option<&Url>,
        line_offset: i32,
        drain_microtasks: bool,
        sync_child_records: bool,
    ) -> Result<()> {
        let provenance = script_url.cloned().map(CompiledStringProvenance::at_url);
        let pending = self
            .exec_in_context_ptr_without_turn_drain(
                context_ptr,
                source,
                provenance,
                line_offset,
                None,
                drain_microtasks,
                UncaughtScriptReportTarget::LogOnly,
            )
            .map(|result| result.map_err(RawScriptExecutionError::into_anyhow));
        if sync_child_records {
            return pending.finish_with_child_record_sync_and_style_drain(
                self,
                StyleInvalidationTurnExitBoundary::RuntimeEvaluate,
            );
        }
        pending.finish_with_style_drain(self, StyleInvalidationTurnExitBoundary::RuntimeEvaluate)
    }

    fn exec_in_context_ptr_without_turn_drain(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        source: &str,
        provenance: Option<CompiledStringProvenance>,
        line_offset: i32,
        script_nonce: Option<&str>,
        drain_microtasks: bool,
        report_target: UncaughtScriptReportTarget,
    ) -> PendingScriptTurn<RawScriptExecutionResult<()>> {
        self.execute_source_text_in_context_ptr_without_turn_drain(
            context_ptr,
            source,
            provenance,
            line_offset,
            script_nonce,
            drain_microtasks,
            report_target,
            SourceTextScriptCompletionMode::Ignore,
        )
        .map(|result| result.map(|_completion| ()))
    }

    pub(super) fn execute_source_text_in_context_ptr_runtime_turn_with_base_url_and_current_window_error_report(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        source: &str,
        script_url: Option<&Url>,
        script_base_url: Option<&Url>,
        line_offset: i32,
        script_nonce: Option<&str>,
        drain_microtasks: bool,
        sync_child_records: bool,
        completion_mode: SourceTextScriptCompletionMode,
    ) -> Result<SourceTextScriptCompletion> {
        let provenance = script_url.cloned().map(|source_url| {
            let module_base_url = script_base_url
                .cloned()
                .unwrap_or_else(|| source_url.clone());
            CompiledStringProvenance::new(source_url, module_base_url)
        });
        let pending = self
            .execute_source_text_in_context_ptr_without_turn_drain(
                context_ptr,
                source,
                provenance,
                line_offset,
                script_nonce,
                drain_microtasks,
                UncaughtScriptReportTarget::CurrentWindow,
                completion_mode,
            )
            .map(|result| result.map_err(RawScriptExecutionError::into_anyhow));
        if sync_child_records {
            return pending.finish_with_child_record_sync_and_style_drain(
                self,
                StyleInvalidationTurnExitBoundary::RuntimeEvaluate,
            );
        }
        pending.finish_with_style_drain(self, StyleInvalidationTurnExitBoundary::RuntimeEvaluate)
    }

    /// Execute source text as the body of an already-selected Page task.
    ///
    /// The caller must return an execution-produced completion fact to the
    /// unique selected-task dispatcher. This primitive therefore performs no
    /// microtask checkpoint, child-record synchronization, runtime follow-up,
    /// or turn-exit style drain of its own.
    pub(super) fn execute_source_text_in_context_ptr_selected_page_task_body(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        source: &str,
        script_url: Option<&Url>,
        script_base_url: Option<&Url>,
        line_offset: i32,
        script_nonce: Option<&str>,
        completion_mode: SourceTextScriptCompletionMode,
    ) -> Result<SourceTextScriptCompletion> {
        let provenance = script_url.cloned().map(|source_url| {
            let module_base_url = script_base_url
                .cloned()
                .unwrap_or_else(|| source_url.clone());
            CompiledStringProvenance::new(source_url, module_base_url)
        });
        self.execute_source_text_in_context_ptr_without_turn_drain(
            context_ptr,
            source,
            provenance,
            line_offset,
            script_nonce,
            false,
            UncaughtScriptReportTarget::CurrentWindow,
            completion_mode,
        )
        .map(|result| result.map_err(RawScriptExecutionError::into_anyhow))
        .into_inner_for_enclosing_script_turn()
    }

    fn execute_source_text_in_context_ptr_without_turn_drain(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        source: &str,
        provenance: Option<CompiledStringProvenance>,
        line_offset: i32,
        script_nonce: Option<&str>,
        drain_microtasks: bool,
        report_target: UncaughtScriptReportTarget,
        completion_mode: SourceTextScriptCompletionMode,
    ) -> PendingScriptTurn<RawScriptExecutionResult<SourceTextScriptCompletion>> {
        let source = source.to_owned();
        let script_nonce = script_nonce.map(str::to_owned);
        PendingScriptTurn::new(
            self.renderer_document_isolate
                .with_renderer_document_isolate_mut::<
                    RawScriptExecutionResult<SourceTextScriptCompletion>,
                >(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    execute_source_text_on_current_stack_with_completion(
                        scope,
                        &source,
                        provenance.as_ref(),
                        line_offset,
                        script_nonce.as_deref(),
                        drain_microtasks,
                        report_target,
                        completion_mode,
                    )
                }),
        )
    }

    pub(super) fn perform_microtask_checkpoints(
        scope: &mut v8::PinScope<'_, '_>,
        script_url: Option<&Url>,
    ) -> Result<()> {
        // V8's checkpoint drains the current microtask queue. Running it once
        // matches browser event-loop semantics; follow-up tasks get their own
        // checkpoints at later task boundaries.
        if let Some(script_url) = script_url {
            debug!(url = %script_url, "starting microtask checkpoint");
        }
        let watchdog = V8ExecutionWatchdog::arm(
            V8ExecutionWatchdogKind::ScriptTurn,
            scope.thread_safe_handle(),
            SCRIPT_TURN_WATCHDOG_TIMEOUT,
        );
        perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
        let watchdog_timed_out = watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut;
        if watchdog_timed_out {
            return Err(anyhow!(
                "microtask checkpoint exceeded {:?} and was terminated",
                SCRIPT_TURN_WATCHDOG_TIMEOUT
            ));
        }
        if let Some(script_url) = script_url {
            debug!(url = %script_url, "finished microtask checkpoint");
        }
        Ok(())
    }

    pub(super) fn reset_dom_binding_trace_window() {
        if moli_trace::dom_binding_timing_enabled() {
            let _ = moli_trace::take_dom_binding_operation_stats();
            let _ = moli_trace::take_promise_hook_stats();
        }
    }

    pub(super) fn emit_dom_binding_trace_window(
        stage: &'static str,
        phase: &'static str,
        script_url: Option<&Url>,
        elapsed: std::time::Duration,
    ) {
        if !moli_trace::dom_binding_timing_enabled() {
            return;
        }
        let stats = moli_trace::take_dom_binding_operation_stats();
        let promise_stats = moli_trace::take_promise_hook_stats();
        if stats.is_empty() && promise_stats == moli_trace::PromiseHookStats::default() {
            return;
        }
        let operation_count: u64 = stats.iter().map(|stat| stat.count).sum();
        let total_us: u128 = stats.iter().map(|stat| stat.total_us).sum();
        let max = stats
            .iter()
            .max_by_key(|stat| stat.max_us)
            .map(|stat| (stat.op, stat.max_us))
            .unwrap_or(("<none>", 0));
        let operations = stats
            .iter()
            .map(|stat| {
                format!(
                    "{}:count={},total_us={},max_us={}",
                    stat.op, stat.count, stat.total_us, stat.max_us
                )
            })
            .collect::<Vec<_>>()
            .join(";");
        tracing::info!(
            target: "moli_cdp_nav_timing",
            stage,
            phase,
            url = script_url.map(|url| url.as_str()).unwrap_or("<none>"),
            elapsed_ms = elapsed.as_millis(),
            elapsed_us = elapsed.as_micros(),
            operation_count,
            total_us,
            max_op = max.0,
            max_op_us = max.1,
            promise_init_count = promise_stats.init_count,
            promise_resolve_count = promise_stats.resolve_count,
            promise_reaction_before_count = promise_stats.reaction_before_count,
            promise_reaction_after_count = promise_stats.reaction_after_count,
            operations = %operations,
        );
    }

    pub(crate) fn eval(&mut self, source: &str) -> Result<String> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context as *const _;
        self.eval_string_in_context_ptr_runtime_turn(context_ptr, source, false)
    }

    pub(crate) fn eval_javascript_url_runtime_turn(
        &mut self,
        source: &str,
    ) -> Result<Option<String>> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context as *const _;
        let completion = self
            .execute_source_text_in_context_ptr_runtime_turn_with_base_url_and_current_window_error_report(
                context_ptr,
                source,
                None,
                None,
                0,
                None,
                true,
                false,
                SourceTextScriptCompletionMode::ValueTypeAware,
            )?;
        Ok(match completion {
            SourceTextScriptCompletion::String(value) => Some(value),
            SourceTextScriptCompletion::NonString => None,
            SourceTextScriptCompletion::Ignored => {
                unreachable!("javascript URL requested value-aware completion")
            }
        })
    }

    #[cfg(test)]
    pub(crate) fn eval_with_child_record_sync(&mut self, source: &str) -> Result<String> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context as *const _;
        self.eval_string_in_context_ptr_runtime_turn(context_ptr, source, true)
    }

    pub(super) fn eval_string_in_context_ptr_runtime_turn(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        source: &str,
        sync_child_records: bool,
    ) -> Result<String> {
        let pending = self.eval_string_in_context_ptr_without_turn_drain(
            context_ptr,
            source,
            EvalStringMicrotaskCheckpoint::Perform,
        );
        if sync_child_records {
            return pending.finish_with_child_record_sync_and_style_drain(
                self,
                StyleInvalidationTurnExitBoundary::RuntimeEvaluate,
            );
        }
        pending.finish_with_style_drain(self, StyleInvalidationTurnExitBoundary::RuntimeEvaluate)
    }

    // Evaluates a trusted internal expression while taking a VM snapshot or
    // reconciling VM-owned state. This is not an owner-visible script turn; new
    // page/automation entrypoints must use `eval_string_in_context_ptr_runtime_turn`.
    pub(super) fn eval_string_in_context_ptr_internal_snapshot(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        source: &str,
    ) -> Result<String> {
        self.eval_string_in_context_ptr_without_turn_drain_with_kind(
            context_ptr,
            source,
            ContextStringEvaluationKind::InspectorInternal,
            EvalStringMicrotaskCheckpoint::Perform,
        )
        .into_inner_for_internal_snapshot()
    }

    /// Read a test probe without manufacturing an intervening task checkpoint.
    ///
    /// P5 body/completion witnesses use this only between an already-selected
    /// task body and its explicit completion. Ordinary fixtures must use
    /// `eval()`, whose runtime turn owns a checkpoint.
    #[cfg(test)]
    pub(crate) fn eval_without_microtask_checkpoint_for_test(
        &mut self,
        source: &str,
    ) -> Result<String> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context as *const _;
        self.eval_string_in_context_ptr_without_turn_drain(
            context_ptr,
            source,
            EvalStringMicrotaskCheckpoint::SkipForSelectedTaskBoundaryObservation,
        )
        .into_inner_for_test()
    }

    // Raw string evaluation in a specific context. Keep this private so callers
    // must choose either the runtime-turn facade or the internal snapshot facade.
    fn eval_string_in_context_ptr_without_turn_drain(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        source: &str,
        checkpoint: EvalStringMicrotaskCheckpoint,
    ) -> PendingScriptTurn<Result<String>> {
        self.eval_string_in_context_ptr_without_turn_drain_with_kind(
            context_ptr,
            source,
            ContextStringEvaluationKind::PageScript,
            checkpoint,
        )
    }

    fn eval_string_in_context_ptr_without_turn_drain_with_kind(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        source: &str,
        kind: ContextStringEvaluationKind,
        checkpoint: EvalStringMicrotaskCheckpoint,
    ) -> PendingScriptTurn<Result<String>> {
        PendingScriptTurn::new(
            self.renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
                    let mut scope = try_catch.init();
                    let source = v8_string(&scope, source)
                        .ok_or_else(|| anyhow!("failed to allocate v8 source string"))?;
                    let script = kind.compile(&scope, source).ok_or_else(|| {
                        let exception = scope.exception();
                        let message = scope.message();
                        let stack_trace = scope.stack_trace();
                        let report = build_event_handler_exception_report(
                            &mut scope,
                            exception,
                            message,
                            stack_trace,
                        );
                        uncaught_script_error(report, "compile")
                    })?;
                    let value = script.run(&scope).ok_or_else(|| {
                        let exception = scope.exception();
                        let message = scope.message();
                        let stack_trace = scope.stack_trace();
                        let report = build_event_handler_exception_report(
                            &mut scope,
                            exception,
                            message,
                            stack_trace,
                        );
                        uncaught_script_error(report, "execute")
                    })?;
                    if kind == ContextStringEvaluationKind::PageScript {
                        match checkpoint {
                            EvalStringMicrotaskCheckpoint::Perform => {
                                Self::perform_microtask_checkpoints(&mut scope, None)?;
                            }
                            #[cfg(test)]
                            EvalStringMicrotaskCheckpoint::SkipForSelectedTaskBoundaryObservation => {
                            }
                        }
                    }
                    let text = value
                        .to_string(&scope)
                        .ok_or_else(|| anyhow!("v8 script did not return a string"))?;
                    Ok(text.to_rust_string_lossy(&scope))
                }),
        )
    }

    pub(super) async fn flush_pending_work(
        &mut self,
        loader: &ResourceRequestClient,
        wait_for_dynamic_loads: bool,
    ) -> std::result::Result<(), String> {
        let _ = self
            .flush_pending_work_with_turn_budget(loader, wait_for_dynamic_loads, false)
            .await?;
        Ok(())
    }

    pub(super) async fn flush_pending_work_with_turn_budget(
        &mut self,
        loader: &ResourceRequestClient,
        wait_for_dynamic_loads: bool,
        yield_after_one_runnable: bool,
    ) -> std::result::Result<RuntimePendingWorkFlushOutcome, String> {
        let result = self
            .flush_pending_work_with_turn_budget_inner(
                loader,
                wait_for_dynamic_loads,
                yield_after_one_runnable,
            )
            .await;
        self.finish_runtime_turn_with_style_drain(
            crate::style_engine::StyleInvalidationTurnExitBoundary::RuntimePendingWorkFlush,
            result,
        )
    }

    async fn flush_pending_work_with_turn_budget_inner(
        &mut self,
        loader: &ResourceRequestClient,
        wait_for_dynamic_loads: bool,
        yield_after_one_runnable: bool,
    ) -> std::result::Result<RuntimePendingWorkFlushOutcome, String> {
        self.refresh_script_vm_local_document_state();
        if self
            .document_runtime
            .runtime_script_work_mut()
            .is_paused_for_deferred_page_tasks()
        {
            return Ok(RuntimePendingWorkFlushOutcome::Complete);
        }
        let runtime_script_work = self.document_runtime.runtime_script_work_handle();
        let mut processed_runnable_this_turn = false;

        loop {
            self.document_runtime.accept_ready_runtime_script_events();
            let only_domcontentloaded_gated_scripts = wait_for_dynamic_loads && {
                let mut work = runtime_script_work.borrow_mut();
                !work.dynamic_scripts.is_idle()
                    && work.dynamic_scripts.has_only_scripts_matching(|script| {
                        !dynamic_script_execute_is_runnable_before_dom_content_loaded(
                            &self.document_runtime,
                            script,
                        )
                    })
            };
            if only_domcontentloaded_gated_scripts {
                break;
            }

            let poll = {
                let mut work = runtime_script_work.borrow_mut();
                work.dynamic_scripts.poll_nonblocking()
            };
            match poll {
                DynamicScriptOwnerPoll::Work(work) => match *work {
                    DynamicScriptRunnable::Execute {
                        id,
                        script,
                        source_network_result,
                    } => {
                        if yield_after_one_runnable && processed_runnable_this_turn {
                            self.document_runtime
                                .runtime_script_work_mut()
                                .dynamic_scripts
                                .requeue_ready_script_front(id, script, source_network_result);
                            return Ok(RuntimePendingWorkFlushOutcome::Complete);
                        }
                        if !dynamic_script_execute_is_runnable_before_dom_content_loaded(
                            &self.document_runtime,
                            &script,
                        ) {
                            self.document_runtime
                                .runtime_script_work_mut()
                                .dynamic_scripts
                                .requeue_ready_script_front(id, script, source_network_result);
                            return Ok(RuntimePendingWorkFlushOutcome::Complete);
                        }
                        if self.prepared_script_uses_runtime_owned_page_task_execution(&script) {
                            self.document_runtime
                                .runtime_script_work_mut()
                                .dynamic_scripts
                                .requeue_ready_script_front(id, script, source_network_result);
                            self.enqueue_immediate_runtime_script_work_if_needed();
                            return Ok(RuntimePendingWorkFlushOutcome::Complete);
                        }
                        if script.kind == crate::types::ScriptKind::Module {
                            if let Some(network_result) = source_network_result.as_deref() {
                                self.record_script_subresource_network_result(
                                    script.initiator_url.clone(),
                                    script.url.clone(),
                                    network_result,
                                );
                            }
                            match self.start_runtime_module_script_graph_for_owner(&script, id) {
                                RuntimeModuleScriptGraphStart::Started(actions) => {
                                    self.commit_runtime_module_graph_start_actions(actions);
                                    processed_runnable_this_turn = true;
                                    continue;
                                }
                                RuntimeModuleScriptGraphStart::NotModuleScript => {}
                            }
                        }
                        debug!(
                            url = %script.url,
                            mode = ?script.mode,
                            kind = ?script.kind,
                            "flush_pending_work executing ready dynamic script"
                        );
                        if let Some(network_result) = source_network_result.as_deref() {
                            self.record_script_subresource_network_result(
                                script.initiator_url.clone(),
                                script.url.clone(),
                                network_result,
                            );
                        }
                        let document_owner_before_run =
                            self.current_main_document_task_owner().expect(
                                "runtime script execution requires a current main Document owner",
                            );
                        match self.execute_prepared_script_once(loader, &script).await {
                            Ok(true) => {
                                if self.script_run_replaced_document(
                                    document_owner_before_run,
                                    &script,
                                ) {
                                    return Ok(RuntimePendingWorkFlushOutcome::Complete);
                                }
                                let lease = self
                                    .document_runtime
                                    .runtime_script_work_mut()
                                    .dynamic_scripts
                                    .finish_script_terminal(id);
                                if let Some(lease) = self
                                    .exact_runtime_script_terminal_lease_or_warn(id, &script, lease)
                                {
                                    self.apply_runtime_script_success_terminal(&script, lease);
                                }
                                processed_runnable_this_turn = true;
                            }
                            Ok(false) => {
                                let lease = self
                                    .document_runtime
                                    .runtime_script_work_mut()
                                    .dynamic_scripts
                                    .finish_script_terminal(id);
                                if let Some(lease) = self
                                    .exact_runtime_script_terminal_lease_or_warn(id, &script, lease)
                                {
                                    self.release_runtime_script_load_delay_lease(&script, lease);
                                }
                                processed_runnable_this_turn = true;
                            }
                            Err(error) => {
                                let module_failure_policy = error.module_failure_policy();
                                let failure_kind = error.module_load_stage().map_or_else(
                                    || {
                                        crate::dynamic_script_owner::DynamicScriptOwner::legacy_message_failure_kind(
                                            &script,
                                            error.message(),
                                        )
                                    },
                                    |stage| {
                                        crate::dynamic_script_owner::DynamicScriptOwner::module_load_failure_kind(
                                            &script, stage,
                                        )
                                    },
                                );
                                let error_constructor = error.error_constructor();
                                let error = error.into_message();
                                if failure_kind
                                    == crate::dynamic_script_owner::DynamicScriptFailureKind::Immediate
                                {
                                    let lease = self
                                        .document_runtime
                                        .runtime_script_work_mut()
                                        .dynamic_scripts
                                        .finish_script_terminal(id);
                                    if let Some(lease) = self
                                        .exact_runtime_script_terminal_lease_or_warn(
                                            id, &script, lease,
                                        )
                                    {
                                        self.apply_runtime_script_failure_terminal(&script,
                                            &error,
                                            module_failure_policy,
                                            error_constructor,
                                            lease,
                                        );
                                    }
                                } else {
                                    self.document_runtime
                                        .runtime_script_work_mut()
                                        .dynamic_scripts
                                        .note_script_failed_with_kind_and_error_constructor(
                                            id,
                                            &script,
                                            error,
                                            failure_kind,
                                            module_failure_policy,
                                            error_constructor,
                                        );
                                }
                                processed_runnable_this_turn = true;
                            }
                        }
                    }
                    DynamicScriptRunnable::ContinueModuleScriptGraph { id, continuation } => {
                        self.document_runtime
                            .runtime_script_work_mut()
                            .dynamic_scripts
                            .requeue_module_script_graph_ready_front(id, continuation);
                        return Ok(RuntimePendingWorkFlushOutcome::Complete);
                    }
                    DynamicScriptRunnable::ContinueModuleScriptEvaluation { id, evaluation } => {
                        self.document_runtime
                            .runtime_script_work_mut()
                            .dynamic_scripts
                            .requeue_module_script_evaluation_ready_front(id, evaluation);
                        return Ok(RuntimePendingWorkFlushOutcome::Complete);
                    }
                    DynamicScriptRunnable::DispatchError {
                        id,
                        script,
                        message,
                        kind,
                        module_failure_policy,
                        source_network_result,
                        error_constructor,
                    } => {
                        if yield_after_one_runnable && processed_runnable_this_turn {
                            self.document_runtime
                                .runtime_script_work_mut()
                                .dynamic_scripts
                                .requeue_failed_script_front_with_error_constructor(
                                    id,
                                    script,
                                    message,
                                    kind,
                                    module_failure_policy,
                                    source_network_result,
                                    error_constructor,
                                );
                            return Ok(RuntimePendingWorkFlushOutcome::Complete);
                        }
                        if self
                            .document_runtime
                            .prepared_script_waits_until_dom_content_loaded(&script)
                            && !dynamic_script_execute_is_runnable_before_dom_content_loaded(
                                &self.document_runtime,
                                &script,
                            )
                        {
                            self.document_runtime
                                .runtime_script_work_mut()
                                .dynamic_scripts
                                .requeue_failed_script_front_with_error_constructor(
                                    id,
                                    script,
                                    message,
                                    kind,
                                    module_failure_policy,
                                    source_network_result,
                                    error_constructor,
                                );
                            return Ok(RuntimePendingWorkFlushOutcome::Complete);
                        }
                        if let Some(network_result) = source_network_result.as_deref() {
                            self.record_script_subresource_network_result(
                                script.initiator_url.clone(),
                                script.url.clone(),
                                network_result,
                            );
                        }
                        self.record_runtime_warning(format_args!(
                            "dynamic script load failed for `{}`: {message}",
                            script.url
                        ));
                        processed_runnable_this_turn = true;
                        let lease = self
                            .document_runtime
                            .runtime_script_work_mut()
                            .dynamic_scripts
                            .finish_script_terminal(id);
                        if let Some(lease) =
                            self.exact_runtime_script_terminal_lease_or_warn(id, &script, lease)
                        {
                            self.apply_runtime_script_failure_terminal(
                                &script,
                                &message,
                                module_failure_policy,
                                error_constructor,
                                lease,
                            );
                        }
                    }
                },
                DynamicScriptOwnerPoll::Idle => {
                    if wait_for_dynamic_loads {
                        let dynamic_scripts_are_idle = self
                            .document_runtime
                            .runtime_script_work_mut()
                            .dynamic_scripts
                            .is_idle();
                        if !dynamic_scripts_are_idle {
                            assert!(
                                self.arm_runtime_script_work_continuation_if_needed(),
                                "pending runtime-script source work must accept a stable continuation"
                            );
                            return Ok(RuntimePendingWorkFlushOutcome::WaitingForSource);
                        }
                    }
                    break;
                }
                DynamicScriptOwnerPoll::StalledWithoutInflightLoads => {
                    return Err("dynamic script drain stalled without in-flight loads".to_owned());
                }
            }
        }

        debug!("flush_pending_work completed");
        Ok(RuntimePendingWorkFlushOutcome::Complete)
    }

    pub(super) fn commit_runtime_module_graph_start_actions(
        &mut self,
        actions: crate::module_script_continuation::NativeModuleOwnerActions,
    ) {
        let (ready_scripts, ready_evaluations, runtime_failures) = actions.into_parts();
        for continuation in ready_scripts {
            let owner_id = continuation
                .dynamic_script_owner_id()
                .expect("runtime-owned ready module continuation should carry dynamic owner id");
            self.document_runtime
                .runtime_script_work_mut()
                .dynamic_scripts
                .requeue_module_script_graph_ready_front(owner_id, Box::new(continuation));
        }
        for evaluation in ready_evaluations {
            let owner_id = evaluation
                .script_continuation
                .dynamic_script_owner_id()
                .expect("runtime-owned ready module evaluation should carry dynamic owner id");
            self.document_runtime
                .runtime_script_work_mut()
                .dynamic_scripts
                .requeue_module_script_evaluation_ready_front(owner_id, Box::new(evaluation));
        }
        for (continuation, error) in runtime_failures {
            let owner_id = continuation
                .dynamic_script_owner_id()
                .expect("runtime-owned module graph failure should carry dynamic owner id");
            let kind = crate::dynamic_script_owner::DynamicScriptOwner::module_load_failure_kind(
                &continuation.script,
                error.stage(),
            );
            self.document_runtime
                .runtime_script_work_mut()
                .dynamic_scripts
                .requeue_failed_script_front_with_error_constructor(
                    owner_id,
                    continuation.script,
                    error.message().to_owned(),
                    kind,
                    Some(crate::host::ModuleFailurePolicy::for_module_load_error(
                        &error,
                    )),
                    None,
                    error.error_constructor(),
                );
        }
    }

    fn drain_pending_style_invalidations_for_turn_exit(
        &mut self,
        boundary: crate::style_engine::StyleInvalidationTurnExitBoundary,
    ) {
        self._context_host
            .borrow()
            .drain_pending_style_invalidations_for_turn_exit(boundary);
    }

    pub(super) fn finish_runtime_turn_with_style_drain<T>(
        &mut self,
        boundary: crate::style_engine::StyleInvalidationTurnExitBoundary,
        result: T,
    ) -> T {
        self.apply_pending_main_document_owner_transitions();
        self.apply_pending_child_document_owner_retirements();
        self.drain_pending_style_invalidations_for_turn_exit(boundary);
        let runtime_continuation_is_ready =
            self.runtime_script_work_should_signal_immediate_progress();
        if runtime_continuation_is_ready {
            self.enqueue_runtime_script_work_continuation_if_ready();
        }
        result
    }

    pub(super) fn finish_runtime_turn_with_child_record_sync_and_style_drain<T>(
        &mut self,
        boundary: crate::style_engine::StyleInvalidationTurnExitBoundary,
        result: T,
    ) -> T {
        self.sync_child_browsing_context_records();
        self.finish_runtime_turn_with_style_drain(boundary, result)
    }
}
