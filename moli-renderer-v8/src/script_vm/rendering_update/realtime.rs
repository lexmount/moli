use anyhow::{Result, anyhow, ensure};

use super::ScriptVm;
use crate::{
    context_bootstrap::{
        current_window_performance_time_origin_seed, dispatch_window_error_event_with_details,
    },
    native_bridge::{JsContextHost, WindowDocumentTaskTarget, WindowExecutionContextBinding},
    page_task_queue::RendererPageRenderingUpdateTaskId,
    v8_execution_watchdog::{
        SCRIPT_TURN_WATCHDOG_TIMEOUT, V8ExecutionWatchdog, V8ExecutionWatchdogKind,
        V8ExecutionWatchdogOutcome,
    },
    window_webidl_callback::{
        PreparedWindowWebIdlCallbackFunction, WindowWebIdlCallbackFunctionOutcome,
        invoke_window_webidl_callback_function,
    },
};

impl ScriptVm {
    pub(super) fn apply_realtime_rendering_opportunity(
        &mut self,
        id: RendererPageRenderingUpdateTaskId,
        root: WindowDocumentTaskTarget,
    ) -> Result<bool> {
        self.with_default_context_scope(|scope, host_ptr| {
            if !unsafe { &*host_ptr }.begin_realtime_rendering_opportunity(id, root)? {
                return Ok(false);
            }
            // Only the owning Rc/raw pointer crosses callbacks. In particular,
            // no registry, Host, style or layout RefCell guard survives JS.
            let result = Self::run_realtime_rendering_phases(scope, host_ptr, id, root);
            unsafe { &*host_ptr }.finish_realtime_rendering_opportunity(id, root, result.is_ok());
            result
        })
    }

    fn run_realtime_rendering_phases(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        id: RendererPageRenderingUpdateTaskId,
        root: WindowDocumentTaskTarget,
    ) -> Result<bool> {
        // One monotonic sample per opportunity, then each target Window's
        // internal time origin. Neither performance nor Date is read from JS.
        let epoch_ms = moli_time::monotonic_timestamp_seconds() * 1000.0;
        let mut invoked = Self::run_realtime_environment_phases(scope, host_ptr, id, root)?;
        let mut next_document_index = 0;
        while let Some(target) =
            unsafe { &*host_ptr }.realtime_rendering_document(id, root, next_document_index)
        {
            let document_index = next_document_index;
            next_document_index += 1;
            if !unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
                break;
            }
            let handles = unsafe { &*host_ptr }.realtime_animation_frame_handles(target);
            if handles.is_empty() {
                continue;
            }
            let binding = {
                let host = unsafe { &*host_ptr };
                host.current_window_execution_context_owner(target.dispatch_scope())
                    .and_then(|owner| {
                        host.clone_window_execution_context_binding(
                            scope,
                            owner,
                            target.dispatch_scope(),
                        )
                    })
            };
            let Some(binding) = binding else {
                continue;
            };
            let origin = binding
                .with_current_scope(scope, host_ptr, |scope, _| {
                    current_window_performance_time_origin_seed(scope)
                })
                .flatten()
                .ok_or_else(|| anyhow!("animation frame target has no native time origin"))?;
            let timestamp = moli_time::coarsened_dom_time_millis((epoch_ms - origin).max(0.0));

            for handle in handles {
                if !unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
                    break;
                }
                // document.open keeps the same callback map and current keys.
                // Resolve only the target explicitly transferred by that
                // transaction; navigation must not select a replacement map.
                let Some(target) =
                    unsafe { &*host_ptr }.realtime_rendering_document(id, root, document_index)
                else {
                    break;
                };
                let Some(callback) =
                    unsafe { &*host_ptr }.take_realtime_animation_frame(target, handle)
                else {
                    continue;
                };
                let callback = callback.prepare(scope);
                if !callback.is_current(unsafe { &*host_ptr })
                    || !callback.relevant_identity().is_some_and(|identity| {
                        unsafe { &*host_ptr }.owner_scripting_enabled(identity.dispatch_scope())
                    })
                {
                    continue;
                }
                invoked = true;
                invoke_animation_frame(scope, host_ptr, &callback, timestamp)?;
                // Explicit microtask policy requires embedder cleanup after
                // each callback, including a reported exception. This is not
                // the Page task completion and does not pump other tasks.
                Self::perform_microtask_checkpoints(scope, None)?;
            }
        }
        unsafe { &*host_ptr }.run_realtime_layout_phase(id, root)?;
        let resize_invoked = Self::run_realtime_resize_observer_phases(scope, host_ptr, id, root)?;
        if resize_invoked {
            // RO and its loop-error handler can change geometry after the last
            // gather. Refresh later consumers without restarting this batch's
            // depth loop or consuming other Documents' pending RO checks.
            unsafe { &*host_ptr }.run_realtime_layout_phase(id, root)?;
        }
        invoked |= resize_invoked;
        if unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
            crate::observer_runtime::sample_realtime_intersections(scope, host_ptr)?;
        }
        if unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
            // Layout has published the actual child viewports, and rAF/RO may
            // have changed CSS font sources. Admit those resources through the
            // existing owner-checked loader after sampling this frame. A ready
            // font invalidates geometry and requests its own next opportunity;
            // synchronous getters never start resource or callback work.
            crate::document_runtime::DocumentRuntime::reconcile_document_web_fonts_in_current_scope(
                scope, host_ptr,
            );
        }
        Ok(invoked)
    }
}

fn invoke_animation_frame(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    callback: &PreparedWindowWebIdlCallbackFunction,
    timestamp: f64,
) -> Result<()> {
    let watchdog = V8ExecutionWatchdog::arm(
        V8ExecutionWatchdogKind::LifecycleEvent,
        scope.thread_safe_handle(),
        SCRIPT_TURN_WATCHDOG_TIMEOUT,
    );
    let timeout = watchdog.timeout();
    let arguments = [v8::Number::new(scope, timestamp).into()];
    let receiver = v8::undefined(scope).into();
    let outcome = invoke_window_webidl_callback_function(
        scope,
        host_ptr,
        "animation frame",
        "animation frame callback threw",
        "requestAnimationFrame",
        callback,
        receiver,
        &arguments,
    );
    if let WindowWebIdlCallbackFunctionOutcome::Threw(report) = outcome
        && callback.is_current(unsafe { &*host_ptr })
        && let Some(identity) = callback.relevant_identity()
        && let Some(context) = callback.relevant_context(scope)
    {
        // Report to the callback's relevant global, which need not be the
        // Window whose animation-frame list supplied this callback.
        let binding = WindowExecutionContextBinding::new(
            identity.owner(),
            identity.dispatch_scope(),
            identity.realm_token(),
            v8::Global::new(scope, context),
        );
        binding.with_current_scope(scope, host_ptr, |scope, _| {
            let error = report
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
                error,
            );
        });
    }
    ensure!(
        watchdog.disarm() != V8ExecutionWatchdogOutcome::TimedOut,
        "animation frame callback exceeded {timeout:?} and was terminated"
    );
    Ok(())
}
