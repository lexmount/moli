use anyhow::{Result, ensure};

use super::ScriptVm;
use crate::{
    context_bootstrap::{
        broadcast_realtime_resize_observer, dispatch_window_error_event_with_details,
        gather_realtime_resize_observers,
    },
    native_bridge::{JsContextHost, WindowDocumentTaskTarget},
    observer_runtime,
    page_task_queue::RendererPageRenderingUpdateTaskId,
    v8_execution_watchdog::{
        SCRIPT_TURN_WATCHDOG_TIMEOUT, V8ExecutionWatchdog, V8ExecutionWatchdogKind,
        V8ExecutionWatchdogOutcome,
    },
};

impl ScriptVm {
    pub(super) fn run_realtime_resize_observer_phases(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        id: RendererPageRenderingUpdateTaskId,
        root: WindowDocumentTaskTarget,
    ) -> Result<bool> {
        if !observer_runtime::has_pending_realtime_resize_observers(unsafe { &*host_ptr }, None) {
            return Ok(false);
        }
        // A callback can keep appending deeper descendants. Depth convergence
        // must follow the algorithm, with an explicit failure budget for the
        // whole lifecycle rather than an arbitrary number of iterations.
        let watchdog = V8ExecutionWatchdog::arm(
            V8ExecutionWatchdogKind::LifecycleEvent,
            scope.thread_safe_handle(),
            SCRIPT_TURN_WATCHDOG_TIMEOUT,
        );
        let timeout = watchdog.timeout();
        let result = Self::run_resize_observer_documents(scope, host_ptr, id, root, &watchdog);
        ensure!(
            watchdog.disarm() != V8ExecutionWatchdogOutcome::TimedOut,
            "ResizeObserver lifecycle exceeded {timeout:?} and was terminated"
        );
        result
    }

    fn run_resize_observer_documents(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        id: RendererPageRenderingUpdateTaskId,
        root: WindowDocumentTaskTarget,
        watchdog: &V8ExecutionWatchdog,
    ) -> Result<bool> {
        let mut invoked = false;
        let mut index = 0;
        while let Some(target) = unsafe { &*host_ptr }.realtime_rendering_document(id, root, index)
        {
            let document_index = index;
            index += 1;
            if !unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
                break;
            }
            if !observer_runtime::has_pending_realtime_resize_observers(
                unsafe { &*host_ptr },
                Some(target),
            ) {
                continue;
            }
            let mut depth = 0;
            loop {
                ensure_resize_lifecycle_running(scope, watchdog)?;
                if !unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
                    break;
                }
                let Some(target) =
                    unsafe { &*host_ptr }.realtime_rendering_document(id, root, document_index)
                else {
                    break;
                };
                if !unsafe { &*host_ptr }.realtime_rendering_target_is_runnable(target) {
                    break;
                }
                unsafe { &*host_ptr }.run_realtime_layout_phase(id, root)?;
                ensure_resize_lifecycle_running(scope, watchdog)?;
                let batch = gather_realtime_resize_observers(scope, host_ptr, target, depth)?;
                if batch.observers.is_empty() {
                    if batch.skipped {
                        observer_runtime::request_realtime_resize_observer_document(
                            unsafe { &*host_ptr },
                            target,
                        );
                        report_resize_loop_error(scope, host_ptr, target);
                        invoked = true;
                        ensure_resize_lifecycle_running(scope, watchdog)?;
                        Self::perform_microtask_checkpoints(scope, None)?;
                        ensure_resize_lifecycle_running(scope, watchdog)?;
                    }
                    break;
                }
                let mut shallowest = usize::MAX;
                for observer in batch.observers {
                    ensure_resize_lifecycle_running(scope, watchdog)?;
                    if !unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
                        break;
                    }
                    let Some(target) =
                        unsafe { &*host_ptr }.realtime_rendering_document(id, root, document_index)
                    else {
                        break;
                    };
                    if !unsafe { &*host_ptr }.realtime_rendering_target_is_runnable(target) {
                        break;
                    }
                    if let Some(reported_depth) =
                        broadcast_realtime_resize_observer(scope, host_ptr, observer)?
                    {
                        invoked = true;
                        shallowest = shallowest.min(reported_depth);
                        ensure_resize_lifecycle_running(scope, watchdog)?;
                        Self::perform_microtask_checkpoints(scope, None)?;
                    }
                    ensure_resize_lifecycle_running(scope, watchdog)?;
                }
                depth = shallowest;
            }
        }
        Ok(invoked)
    }
}

fn ensure_resize_lifecycle_running(
    scope: &mut v8::PinScope<'_, '_>,
    watchdog: &V8ExecutionWatchdog,
) -> Result<()> {
    ensure!(
        !watchdog.has_timed_out() && !scope.is_execution_terminating(),
        "ResizeObserver lifecycle execution was terminated"
    );
    Ok(())
}

fn report_resize_loop_error(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    target: WindowDocumentTaskTarget,
) {
    let binding = {
        let host = unsafe { &*host_ptr };
        host.current_window_execution_context_owner(target.dispatch_scope())
            .and_then(|owner| {
                host.clone_window_execution_context_binding(scope, owner, target.dispatch_scope())
            })
    };
    if let Some(binding) = binding {
        binding.with_current_scope(scope, host_ptr, |scope, _| {
            let _ = dispatch_window_error_event_with_details(
                scope,
                host_ptr,
                "ResizeObserver loop completed with undelivered notifications.",
                "",
                0,
                0,
                None,
            );
        });
    }
}
