use anyhow::{Context, Result, anyhow};
use std::pin::pin;
use std::time::{Duration, Instant};

use super::ScriptVm;
use crate::context_bootstrap::{
    construct_original_event, construct_original_page_transition_event,
    record_performance_dom_content_loaded_event_end,
    record_performance_dom_content_loaded_event_start, record_performance_load_event_end,
    record_performance_load_event_start,
};
use crate::document_runtime::{
    DeferredPageTask, DeferredPageTaskLane, DocumentRuntime, FollowupPageTaskDisposition,
};
use crate::frame_owner_model::MainDocumentScriptLoadDelayLease;
use crate::host::{EventTargetHandle, HostTimeoutRunResult};
use crate::native_bridge::{JsContextHost, element::queue_revealed_lazy_media_loads};
use crate::page_task_queue::{PostParseLifecycleWork, PostParsePageOwnedWork};
use crate::planning::PreparedScript;
use crate::script_vm::ImmediateRuntimeScriptWorkSignal;
use crate::util::v8str;
use crate::v8_execution_watchdog::{
    V8ExecutionWatchdog, V8ExecutionWatchdogKind, V8ExecutionWatchdogOutcome,
};

#[cfg(not(test))]
const LIFECYCLE_EVENT_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(test)]
const LIFECYCLE_EVENT_WATCHDOG_TIMEOUT: Duration = Duration::from_millis(500);

impl ScriptVm {
    fn dispatch_document_event_body(&mut self, event_type: &str) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                // SAFETY: as_ptr() — V8 callbacks are re-entrant; borrow_mut() panics. See util.rs.
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                document_runtime
                    .dispatch_document_event(scope, host_ptr, event_type)
                    .map_err(anyhow::Error::msg)
            })
    }

    pub(super) fn perform_owner_lane_task_microtask_checkpoints(&mut self) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                // SAFETY: `context_ptr` points at `self.page_default_context`, and this closure runs with
                // the document isolate entered. `ScriptVm` owns both values, so the context belongs
                // to the entered isolate for the full checkpoint scope.
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                Self::perform_microtask_checkpoints(scope, None)?;
                Ok(())
            })
            .context("owner-lane microtask checkpoint failed")
    }

    /// Execute the body of one V8 foreground continuation.
    ///
    /// The selected Page-task dispatcher owns the task-end checkpoint. Keeping
    /// this method body-only prevents a V8 helper from creating an intermediate
    /// checkpoint before the scheduler task has actually completed.
    pub(crate) fn run_v8_foreground_task_body(
        &mut self,
        task: moli_v8_platform::V8ForegroundTask,
    ) -> bool {
        self.run_v8_foreground_task(task)
    }

    /// Perform one selected Page task's agent checkpoint and turn-exit cleanup.
    ///
    /// Callers must invoke this only after a task family has removed every
    /// helper-local checkpoint from the selected task body. The runtime
    /// `page_task_checkpoint` component is the production authority deciding
    /// when this primitive runs.
    pub(crate) fn finish_selected_page_task_checkpoint(
        &mut self,
        boundary: crate::style_engine::StyleInvalidationTurnExitBoundary,
    ) -> Result<()> {
        let result = self.perform_owner_lane_task_microtask_checkpoints();
        self.finish_runtime_turn_with_style_drain(boundary, result)
    }

    /// Finish one selected callback task with its established post-checkpoint
    /// reconciliation.
    ///
    /// This sequence is intentionally distinct from
    /// `finish_selected_page_task_checkpoint()`: child browsing contexts and
    /// runtime scripts produced by Promise reactions do not exist until the
    /// checkpoint has run, while `finish_host_task_turn()` owns the final
    /// runtime/style turn-exit cleanup. No borrow or V8 scope crosses the
    /// asynchronous runtime follow-up.
    pub(crate) async fn finish_selected_page_callback_task(
        &mut self,
        loader: &crate::network::ResourceRequestClient,
    ) -> Result<()> {
        self.perform_owner_lane_task_microtask_checkpoints()?;
        self.sync_child_browsing_context_records();
        self.finish_host_task_turn(loader, false).await
    }

    /// Dispatch one Document lifecycle event body without ending its HTML task.
    ///
    /// The main-document lifecycle coordinator owns the ordinary task-end
    /// checkpoint. Keeping this primitive body-only prevents DCL and
    /// `readystatechange` helpers from silently becoming a second completion
    /// authority. Parser-finish DCL temporarily uses the explicit compatibility
    /// wrapper below until P5-A3 moves that direct successor as one unit.
    pub(super) fn dispatch_document_lifecycle_event_body(
        &mut self,
        event_type: &str,
    ) -> Result<()> {
        let watchdog = self
            .renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                V8ExecutionWatchdog::arm(
                    V8ExecutionWatchdogKind::LifecycleEvent,
                    scope.thread_safe_handle(),
                    LIFECYCLE_EVENT_WATCHDOG_TIMEOUT,
                )
            });
        let result = {
            let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
            self.renderer_document_isolate
                .with_renderer_document_isolate_mut(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    if event_type == "DOMContentLoaded" {
                        record_performance_dom_content_loaded_event_start(scope);
                    }
                    Ok(())
                })
        }
        .and_then(|()| self.dispatch_document_event_body(event_type));
        let timed_out = watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut;
        result?;
        if timed_out {
            anyhow::bail!(
                "document lifecycle event `{event_type}` exceeded {:?} and was terminated",
                LIFECYCLE_EVENT_WATCHDOG_TIMEOUT
            );
        }
        Ok(())
    }

    /// Publish the event-end timing that is intentionally observable only
    /// after the owning lifecycle task's checkpoint.
    pub(super) fn record_document_lifecycle_event_end(&mut self, event_type: &str) {
        if event_type == "DOMContentLoaded" {
            let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
            self.renderer_document_isolate
                .with_renderer_document_isolate_mut(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    record_performance_dom_content_loaded_event_end(scope);
                });
        }
    }

    /// Compatibility boundary for direct ScriptVm fixtures. Production
    /// lifecycle work uses the body-only primitive and a typed coordinator.
    #[cfg(test)]
    pub(super) fn dispatch_document_lifecycle_event(&mut self, event_type: &str) -> Result<()> {
        self.dispatch_document_lifecycle_event_body(event_type)?;
        self.perform_owner_lane_task_microtask_checkpoints()
            .with_context(|| {
                format!("document lifecycle event `{event_type}` microtask checkpoint failed")
            })?;
        self.record_document_lifecycle_event_end(event_type);
        Ok(())
    }

    pub(super) fn dispatch_document_lifecycle_event_body_best_effort(
        &mut self,
        event_type: &str,
    ) -> super::MainDocumentLifecycleEventDispatch {
        if let Err(error) = self.dispatch_document_lifecycle_event_body(event_type) {
            self.record_runtime_warning(format_args!(
                "document lifecycle event dispatch failed for `{event_type}`: {error}"
            ));
            return super::MainDocumentLifecycleEventDispatch::FailedBestEffort;
        }
        super::MainDocumentLifecycleEventDispatch::Completed
    }

    pub(super) fn queue_current_main_document_image_load_events(&mut self) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                // SAFETY: as_ptr() - V8 callbacks are re-entrant; borrow_mut() panics. See util.rs.
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                document_runtime.queue_current_main_document_image_load_events(scope, host_ptr);
                Ok(())
            })
    }

    pub(super) fn queue_current_main_document_media_loads(&mut self) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                // SAFETY: as_ptr() - V8 callbacks are re-entrant; borrow_mut() panics. See util.rs.
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                document_runtime.queue_current_main_document_media_loads(scope, host_ptr);
                Ok(())
            })
    }

    /// Dispatch the Window `load` body without crossing the load-to-pageshow
    /// lifecycle checkpoint.
    pub(super) fn dispatch_window_load_event_body(&mut self) -> Result<()> {
        let timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let task_started = timing_enabled.then(Instant::now);
        self.prune_stale_child_default_execution_contexts();
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                // SAFETY: as_ptr() — V8 callbacks are re-entrant; borrow_mut() panics. See util.rs.
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                let global = scope.get_current_context().global(scope);
                let step_started = timing_enabled.then(Instant::now);
                if let Some(document_value) = global.get(scope, v8str(scope, "document").into())
                    && let Ok(document) = v8::Local::<v8::Object>::try_from(document_value)
                    && let Some(body_value) = document.get(scope, v8str(scope, "body").into())
                    && let Ok(body) = v8::Local::<v8::Object>::try_from(body_value)
                {
                    let _ = body.get(scope, v8str(scope, "onload").into());
                }
                if let Some(step_started) = step_started {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        stage = "window_load_event_step_completed",
                        step = "body_onload_getter",
                        step_elapsed_ms = step_started.elapsed().as_millis(),
                        total_elapsed_ms = task_started
                            .as_ref()
                            .map(|started| started.elapsed().as_millis())
                            .unwrap_or_default(),
                    );
                }

                let step_started = timing_enabled.then(Instant::now);
                let event = construct_original_event(scope, "load")
                    .ok_or_else(|| anyhow!("failed to construct window load event"))?;
                if let Some(step_started) = step_started {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        stage = "window_load_event_step_completed",
                        step = "construct_load_event",
                        step_elapsed_ms = step_started.elapsed().as_millis(),
                        total_elapsed_ms = task_started
                            .as_ref()
                            .map(|started| started.elapsed().as_millis())
                            .unwrap_or_default(),
                    );
                }
                let document_target = EventTargetHandle::Node(document_runtime.document_handle());
                record_performance_load_event_start(scope);
                let step_started = timing_enabled.then(Instant::now);
                document_runtime
                    .dispatch_public_event_with_original_target(
                        scope,
                        host_ptr,
                        EventTargetHandle::Window,
                        document_target,
                        event,
                    )
                    .map_err(anyhow::Error::msg)?;
                if let Some(step_started) = step_started {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        stage = "window_load_event_step_completed",
                        step = "dispatch_public_load_event",
                        step_elapsed_ms = step_started.elapsed().as_millis(),
                        total_elapsed_ms = task_started
                            .as_ref()
                            .map(|started| started.elapsed().as_millis())
                            .unwrap_or_default(),
                    );
                }
                let step_started = timing_enabled.then(Instant::now);
                queue_revealed_lazy_media_loads(scope, host_ptr);
                if let Some(step_started) = step_started {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        stage = "window_load_event_step_completed",
                        step = "queue_revealed_lazy_media_loads",
                        step_elapsed_ms = step_started.elapsed().as_millis(),
                        total_elapsed_ms = task_started
                            .as_ref()
                            .map(|started| started.elapsed().as_millis())
                            .unwrap_or_default(),
                    );
                }
                Ok(())
            })
    }

    /// Publish load event-end and dispatch the non-persisted `pageshow` body.
    ///
    /// The caller must have completed the explicit load-to-pageshow lifecycle
    /// checkpoint. This method deliberately does not perform the final task-end
    /// checkpoint after pageshow.
    pub(super) fn dispatch_window_pageshow_event_body(&mut self) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                // SAFETY: as_ptr() — V8 callbacks are re-entrant; borrow_mut() panics. See util.rs.
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                let document_target = EventTargetHandle::Node(document_runtime.document_handle());
                record_performance_load_event_end(scope);
                let pageshow =
                    construct_original_page_transition_event(scope, "pageshow", false)
                        .ok_or_else(|| anyhow!("failed to construct window pageshow event"))?;
                document_runtime
                    .dispatch_public_event_with_original_target(
                        scope,
                        host_ptr,
                        EventTargetHandle::Window,
                        document_target,
                        pageshow,
                    )
                    .map_err(anyhow::Error::msg)?;
                Ok(())
            })
    }

    /// Compatibility wrapper for direct ScriptVm fixtures. Production
    /// ordinary lifecycle work uses `dispatch_window_load_event_body()` and
    /// receives its final task-end from the lifecycle coordinator.
    #[cfg(test)]
    pub(super) fn dispatch_window_load_event(&mut self) -> Result<()> {
        self.dispatch_window_load_event_body()?;
        self.perform_owner_lane_task_microtask_checkpoints()?;
        self.dispatch_window_pageshow_event_body()?;
        self.perform_owner_lane_task_microtask_checkpoints()
    }

    pub(super) fn dispatch_window_load_event_body_best_effort(
        &mut self,
    ) -> super::MainDocumentLifecycleEventDispatch {
        if let Err(error) = self.dispatch_window_load_event_body() {
            self.record_runtime_warning(format_args!("window load dispatch failed: {error}"));
            return super::MainDocumentLifecycleEventDispatch::FailedBestEffort;
        }
        super::MainDocumentLifecycleEventDispatch::Completed
    }

    pub(super) fn dispatch_window_pageshow_event_body_best_effort(
        &mut self,
    ) -> super::MainDocumentLifecycleEventDispatch {
        if let Err(error) = self.dispatch_window_pageshow_event_body() {
            self.record_runtime_warning(format_args!("window pageshow dispatch failed: {error}"));
            return super::MainDocumentLifecycleEventDispatch::FailedBestEffort;
        }
        super::MainDocumentLifecycleEventDispatch::Completed
    }

    pub(super) fn merge_followup_task_disposition(
        disposition: &mut FollowupPageTaskDisposition,
        next: FollowupPageTaskDisposition,
    ) {
        match next {
            FollowupPageTaskDisposition::Skipped => {}
            FollowupPageTaskDisposition::Deferred => {
                *disposition = FollowupPageTaskDisposition::Deferred;
            }
            FollowupPageTaskDisposition::Enqueued => {
                if *disposition == FollowupPageTaskDisposition::Skipped {
                    *disposition = FollowupPageTaskDisposition::Enqueued;
                }
            }
        }
    }

    fn deferred_lifecycle_work_for_lane(
        work: PostParseLifecycleWork,
        lane: DeferredPageTaskLane,
    ) -> Result<DeferredPageTask> {
        DeferredPageTask::page_owned_work(PostParsePageOwnedWork::lifecycle_work(work), lane)
            .ok_or_else(|| anyhow!("parser-boundary lifecycle work must execute at its concrete parser task boundary"))
    }

    #[cfg(test)]
    fn deferred_page_owned_work_for_lane(
        work: PostParsePageOwnedWork,
        lane: DeferredPageTaskLane,
    ) -> Result<DeferredPageTask> {
        DeferredPageTask::page_owned_work(work, lane).ok_or_else(|| {
            anyhow!(
                "parser-boundary page-owned work must execute at its concrete parser task boundary"
            )
        })
    }

    fn enqueue_deferred_page_task(&mut self, task: DeferredPageTask) {
        let (lane, work) = task.into_parts();
        match lane {
            DeferredPageTaskLane::ParserBoundary => {
                unreachable!(
                    "DeferredPageTask constructor rejects parser-boundary work before storage"
                );
            }
            DeferredPageTaskLane::PreDomContentLoaded => {
                self.document_runtime
                    .enqueue_parser_owned_pre_domcontentloaded_page_owned_work(work);
            }
            DeferredPageTaskLane::PostDomContentLoaded => {
                self.enqueue_main_document_runtime_work(work);
            }
        }
    }

    pub(super) fn enqueue_main_document_runtime_work(&mut self, work: PostParsePageOwnedWork) {
        let Some(document_owner) = self.current_main_document_task_owner() else {
            self.record_runtime_warning(format_args!(
                "dropping post-DCL runtime work without a current main Document owner"
            ));
            return;
        };
        let producer = self
            .post_domcontentloaded_page_task_tx
            .bind_main_document_runtime_producer(document_owner);
        if producer.send_post_parse_work_when_ready(work).is_err() {
            self.record_runtime_warning(format_args!(
                "main-Document runtime work could not bind its ready-source admission"
            ));
        }
    }

    pub(crate) fn enqueue_runtime_script_work_continuation(&mut self) {
        let Some(document_owner) = self.current_main_document_task_owner() else {
            self.record_runtime_warning(format_args!(
                "dropping runtime continuation without a current main Document owner"
            ));
            return;
        };
        if self.queued_main_document_runtime_continuation_owner == Some(document_owner) {
            return;
        }
        let continuation = self
            .page_runtime_wake_tx
            .bind_main_document_runtime_continuation(document_owner);
        if continuation.send_runtime_script_continuation().is_ok() {
            self.queued_main_document_runtime_continuation_owner = Some(document_owner);
        } else {
            self.record_runtime_warning(format_args!(
                "main-Document runtime route closed before its continuation was accepted"
            ));
        }
    }

    /// Publish one durable scheduler token for the current main Document's
    /// parser-owned module queue.
    ///
    /// The underlying scheduler store may contain several ready actions. One
    /// token authorizes one action. When a ready tail remains, the body
    /// executor may publish its next token before the current selected-task
    /// checkpoint; the scheduler cannot select that token until the current
    /// dispatcher returns. The exact-owner guard prevents repeated readiness
    /// observations from duplicating the same head.
    pub(crate) fn enqueue_parser_owned_module_continuation(&mut self) -> bool {
        let Some(document_owner) = self.current_main_document_task_owner() else {
            self.record_runtime_warning(format_args!(
                "parser-owned module continuation became ready without a current main Document owner"
            ));
            return false;
        };
        if self.queued_main_document_parser_module_continuation_owner == Some(document_owner) {
            return true;
        }
        let queued = self
            .page_runtime_wake_tx
            .bind_main_document_runtime_continuation(document_owner)
            .send_parser_owned_module_continuation()
            .is_ok();
        if queued {
            self.queued_main_document_parser_module_continuation_owner = Some(document_owner);
        } else {
            self.record_runtime_warning(format_args!(
                "main-Document runtime route closed before parser module continuation admission"
            ));
        }
        queued
    }

    pub(crate) fn begin_parser_owned_module_continuation_turn(
        &mut self,
        document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) {
        if self.queued_main_document_parser_module_continuation_owner == Some(document_owner) {
            self.queued_main_document_parser_module_continuation_owner = None;
        }
    }

    pub(super) fn enqueue_immediate_runtime_script_work_signal(
        &mut self,
        signal: ImmediateRuntimeScriptWorkSignal,
    ) {
        match signal {
            ImmediateRuntimeScriptWorkSignal::StablePageTurnContinuation => {
                self.enqueue_runtime_script_work_continuation();
            }
        }
    }

    pub(super) fn enqueue_runtime_script_work_continuation_if_ready(&mut self) {
        self.enqueue_immediate_runtime_script_work_signal(
            ImmediateRuntimeScriptWorkSignal::StablePageTurnContinuation,
        );
    }

    #[cfg(test)]
    pub(super) fn enqueue_page_owned_work_or_defer(
        &mut self,
        work: PostParsePageOwnedWork,
        lane: DeferredPageTaskLane,
    ) -> Result<FollowupPageTaskDisposition> {
        let mut ready_task = None;
        let deferred_task = Self::deferred_page_owned_work_for_lane(work, lane)?;
        let disposition = self
            .document_runtime
            .deferred_page_tasks_mut()
            .enqueue_or_defer(deferred_task, |task| ready_task = Some(task));
        if let Some(task) = ready_task {
            self.enqueue_deferred_page_task(task);
        }
        Ok(disposition)
    }

    pub(super) fn enqueue_lifecycle_work_or_defer(
        &mut self,
        work: PostParseLifecycleWork,
        lane: DeferredPageTaskLane,
    ) -> Result<FollowupPageTaskDisposition> {
        let mut ready_task = None;
        let deferred_task = Self::deferred_lifecycle_work_for_lane(work, lane)?;
        let disposition = self
            .document_runtime
            .deferred_page_tasks_mut()
            .enqueue_or_defer(deferred_task, |task| ready_task = Some(task));
        if let Some(task) = ready_task {
            self.enqueue_deferred_page_task(task);
        }
        Ok(disposition)
    }

    pub(super) fn enqueue_lifecycle_work_items_or_defer(
        &mut self,
        work_items: Vec<PostParseLifecycleWork>,
        lane: DeferredPageTaskLane,
    ) -> Result<FollowupPageTaskDisposition> {
        let mut disposition = FollowupPageTaskDisposition::Skipped;

        for work in work_items {
            Self::merge_followup_task_disposition(
                &mut disposition,
                self.enqueue_lifecycle_work_or_defer(work, lane)?,
            );
        }

        Ok(disposition)
    }

    pub(super) fn drain_deferred_page_tasks(&mut self) -> Result<()> {
        let mut drained = Vec::new();
        self.document_runtime
            .deferred_page_tasks_mut()
            .drain_into(|deferred| drained.push(deferred));
        for deferred in drained {
            self.enqueue_deferred_page_task(deferred);
        }
        if self
            .document_runtime
            .runtime_script_work_mut()
            .is_paused_for_deferred_page_tasks()
            && !self.document_runtime.runtime_script_work_mut().is_idle()
        {
            if self.runtime_script_work_should_signal_immediate_progress() {
                self.enqueue_runtime_script_work_continuation_if_ready();
            }
        } else {
            self.document_runtime
                .runtime_script_work_mut()
                .resume_after_deferred_page_tasks();
        }
        Ok(())
    }

    pub(super) fn refresh_script_vm_local_document_state(&mut self) {
        self.apply_pending_main_document_owner_transitions();
        self.document_runtime.accept_ready_runtime_script_events();
        self.sync_live_document_style_sources_if_pending();
    }

    pub(crate) fn drain_deferred_page_tasks_best_effort(&mut self) {
        if let Err(error) = self.drain_deferred_page_tasks() {
            self.record_runtime_warning(format_args!("deferred page-task drain failed: {error}"));
        }
    }

    pub(super) fn enqueue_script_failure_lifecycle_work_for_prepared_script(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
    ) -> Result<FollowupPageTaskDisposition> {
        self.enqueue_script_failure_lifecycle_work_with_load_delay_binding(
            script,
            message,
            module_failure_policy,
            error_constructor,
            None,
        )
    }

    pub(super) fn enqueue_script_failure_lifecycle_work_with_load_delay_binding(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Result<FollowupPageTaskDisposition> {
        let mut planned_failure_work = self.document_runtime.plan_script_failure_lifecycle_work(
            script,
            message,
            module_failure_policy,
            error_constructor,
        );
        if let Some(binding) = load_delay_binding {
            planned_failure_work.push(PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(
                binding,
            ));
        }
        self.enqueue_lifecycle_work_items_or_defer(
            planned_failure_work,
            self.prepared_script_followup_lane(script),
        )
    }

    pub(crate) fn enqueue_script_failure_lifecycle_work_best_effort(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
    ) {
        match self.enqueue_script_failure_lifecycle_work_for_prepared_script(
            script,
            message,
            module_failure_policy,
            error_constructor,
        ) {
            Ok(FollowupPageTaskDisposition::Skipped) => {}
            Ok(FollowupPageTaskDisposition::Deferred | FollowupPageTaskDisposition::Enqueued) => {}
            Err(error) => {
                self.record_runtime_warning(format_args!(
                    "script failure page-task enqueue failed for `{}`: {error}",
                    script.url
                ));
            }
        }
    }

    pub(super) fn enqueue_script_load_lifecycle_work_for_prepared_script_best_effort(
        &mut self,
        script: &PreparedScript,
    ) -> FollowupPageTaskDisposition {
        self.enqueue_script_load_lifecycle_work_with_load_delay_binding_best_effort(script, None)
    }

    pub(super) fn enqueue_script_load_lifecycle_work_with_load_delay_binding_best_effort(
        &mut self,
        script: &PreparedScript,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> FollowupPageTaskDisposition {
        let mut work_items = Vec::new();
        if let Some(work) = self.plan_script_load_lifecycle_work_for_prepared_script(script) {
            work_items.push(work);
        }
        if let Some(binding) = load_delay_binding {
            work_items.push(PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(
                binding,
            ));
        }
        if work_items.is_empty() {
            return FollowupPageTaskDisposition::Skipped;
        }
        let enqueue_result = self.enqueue_lifecycle_work_items_or_defer(
            work_items,
            self.prepared_script_followup_lane(script),
        );
        enqueue_result.unwrap_or_else(|error| {
            panic!(
                "validated script terminal work failed to enter its lifecycle lane for `{}`: {error}",
                script.url
            )
        })
    }

    pub(crate) fn enqueue_main_document_script_load_delay_settlement_best_effort(
        &mut self,
        script: &PreparedScript,
        binding: MainDocumentScriptLoadDelayLease,
    ) -> FollowupPageTaskDisposition {
        let owner = binding.owner();
        let kind = binding.kind();
        let load_delay_token = binding.load_delay_token();
        match self.enqueue_lifecycle_work_or_defer(
            PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(binding),
            self.prepared_script_followup_lane(script),
        ) {
            Ok(disposition) => disposition,
            Err(error) => {
                panic!(
                    "exact script load-delay lease {owner:?}/{kind:?}/{load_delay_token:?} failed to enter its validated lifecycle lane for `{}`: {error}",
                    script.url
                )
            }
        }
    }

    pub(super) fn pause_runtime_script_work_at_followup_task_boundary(
        &mut self,
        disposition: FollowupPageTaskDisposition,
    ) -> bool {
        match disposition {
            FollowupPageTaskDisposition::Skipped => false,
            FollowupPageTaskDisposition::Deferred => {
                self.handle_runtime_script_work_at_explicit_boundary(false);
                true
            }
            FollowupPageTaskDisposition::Enqueued => {
                self.handle_runtime_script_work_at_explicit_boundary(true);
                true
            }
        }
    }

    /// Execute one due timer heap head without checkpointing or callback
    /// follow-up.
    ///
    /// Production callers must enter through the selected Page timer task,
    /// which commits the single task-end callback completion.
    pub(super) fn run_next_timeout_body(&mut self) -> Result<HostTimeoutRunResult> {
        #[cfg(test)]
        if let Some(message) = self.test_next_timeout_failure.take() {
            return Err(anyhow!("{message}"));
        }

        let document_runtime: *mut DocumentRuntime = &mut *self.document_runtime;
        self.with_default_context_scope(|scope, _host_ptr| {
            // SAFETY: the raw pointer is derived from `self.document_runtime` immediately before
            // entering this non-escaping closure.
            // `with_default_context_scope` only
            // creates the V8 context scope and does not access or move
            // `document_runtime`; the render owner lane runs this
            // synchronously on one thread.
            Ok(unsafe { &mut *document_runtime }.run_next_timeout_body(scope))
        })
    }

    /// Complete one timer in a low-level ScriptVm fixture without a Page owner
    /// slot.
    ///
    /// This preserves the body -> one checkpoint -> child-sync boundary for
    /// domain tests. PageVm behavior tests must use the production selected
    /// dispatcher so runtime follow-up and replacement reconciliation are not
    /// bypassed.
    #[cfg(test)]
    pub(crate) fn run_next_timeout_for_test(&mut self) -> Result<HostTimeoutRunResult> {
        let result = self.run_next_timeout_body()?;
        if result.consumed_heap_head() {
            self.perform_owner_lane_task_microtask_checkpoints()?;
            self.sync_child_browsing_context_records();
        }
        Ok(result)
    }

    #[cfg(test)]
    pub(super) fn fail_next_timeout_for_testing(&mut self, message: impl Into<String>) {
        self.test_next_timeout_failure = Some(message.into());
    }

    pub(crate) fn ms_to_next_timeout(&self) -> Option<u64> {
        self.document_runtime.ms_to_next_timeout()
    }

    pub(crate) fn has_ready_timeout(&self) -> bool {
        self.document_runtime.has_ready_timeout()
    }

    pub(crate) fn next_timeout_deadline(&self) -> Option<Instant> {
        self.document_runtime.next_timeout_deadline()
    }

    pub(crate) fn record_runtime_warning(&mut self, message: std::fmt::Arguments<'_>) {
        let text = message.to_string();
        let execution_context_id = self.runtime_observable_default_execution_context_id();
        self.runtime_observable_source_queue
            .record_lifecycle_error(text.clone());
        // The queue above is the durable CLI/diagnostic history. Protocol
        // delivery is a separate concrete fact: bind it to the turn and realm
        // that produced the warning instead of waiting for another activity
        // to diff the cumulative queue.
        self._context_host.borrow().append_live_turn_observation(
            crate::runtime::RendererProtocolObservation::RuntimeLifecycleError {
                text,
                execution_context_id,
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn runtime_observable_lifecycle_errors_for_testing(&mut self) -> Vec<String> {
        let _ = self.sync_runtime_observable_source_events();
        self.runtime_observable_source_queue
            .lifecycle_error_messages_for_testing()
    }
}
