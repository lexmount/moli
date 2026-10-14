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

#[derive(Default)]
struct ResizeObserverDocumentPhase {
    depth: usize,
    loop_error_reported: bool,
    skipped: bool,
}

impl ScriptVm {
    pub(super) fn run_realtime_resize_observer_phases(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        id: RendererPageRenderingUpdateTaskId,
        root: WindowDocumentTaskTarget,
    ) -> Result<bool> {
        if !observer_runtime::has_pending_realtime_resize_observers(unsafe { &*host_ptr }, None) {
            finish_resize_observer_controllers(unsafe { &*host_ptr }, id, root);
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
        let outcome = watchdog.disarm();
        if (result.is_err() || outcome == V8ExecutionWatchdogOutcome::TimedOut)
            && unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root)
        {
            // Gather consumed check demand before broadcast. Retain unfinished
            // observations for a later explicitly requested opportunity, while
            // failed-frame completion suppresses automatic renewal. Do not
            // transfer failed work into a replacement Document's lifecycle.
            observer_runtime::request_realtime_resize_observer_checks(unsafe { &*host_ptr });
        }
        ensure!(
            outcome != V8ExecutionWatchdogOutcome::TimedOut,
            "ResizeObserver lifecycle exceeded {timeout:?} and was terminated"
        );
        if result.is_ok() {
            finish_resize_observer_controllers(unsafe { &*host_ptr }, id, root);
        }
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
        // Indices belong to this running frame's fixed Document list. Only
        // document.open may explicitly rebind a slot, preserving its Window
        // and this opportunity's depth/error state. Navigation never does.
        let mut documents: Vec<ResizeObserverDocumentPhase> = Vec::new();
        'cycles: loop {
            ensure_resize_lifecycle_running(scope, watchdog)?;
            if !unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
                break;
            }
            let mut repeat = false;
            let mut index = 0;
            while let Some(target) =
                unsafe { &*host_ptr }.realtime_rendering_document(id, root, index)
            {
                let document_index = index;
                index += 1;
                if !unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
                    break 'cycles;
                }
                if documents.len() <= document_index {
                    documents.push(ResizeObserverDocumentPhase::default());
                }
                let phase = &mut documents[document_index];
                if !unsafe { &*host_ptr }.realtime_rendering_target_is_runnable(target) {
                    continue;
                }
                ensure_resize_lifecycle_running(scope, watchdog)?;
                unsafe { &*host_ptr }.run_realtime_layout_phase(id, root)?;
                ensure_resize_lifecycle_running(scope, watchdog)?;
                let batch = gather_realtime_resize_observers(scope, host_ptr, target, phase.depth)?;
                phase.skipped = batch.skipped;
                if batch.observers.is_empty() {
                    // Like an empty broadcast, an empty gather has no deeper
                    // active observation. Do not reopen this Document at depth
                    // zero if a later Document or error handler changes it.
                    if batch.has_controller {
                        phase.depth = usize::MAX;
                    }
                    if batch.skipped && !phase.loop_error_reported {
                        phase.loop_error_reported = true;
                        report_resize_loop_error(scope, host_ptr, target);
                        invoked = true;
                        repeat = true;
                        ensure_resize_lifecycle_running(scope, watchdog)?;
                        Self::perform_microtask_checkpoints(scope, None)?;
                        ensure_resize_lifecycle_running(scope, watchdog)?;
                    }
                    continue;
                }
                let mut shallowest = usize::MAX;
                let _sampling = observer_runtime::begin_realtime_resize_observer_sampling(
                    unsafe { &*host_ptr },
                    batch.sources,
                )?;
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
                        repeat = true;
                        shallowest = shallowest.min(reported_depth);
                        ensure_resize_lifecycle_running(scope, watchdog)?;
                        Self::perform_microtask_checkpoints(scope, None)?;
                    }
                    ensure_resize_lifecycle_running(scope, watchdog)?;
                }
                phase.depth = shallowest;
            }
            // Repeat only style/layout and RO work, preserving every Document's
            // depth. Later Documents can dirty earlier ones at the same depth
            // (skipped) or at a deeper depth (still eligible in this opportunity).
            if !repeat {
                break;
            }
        }
        if unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
            // Each gather consumes requested checks. Restore deferred demand
            // only after the last cycle, using a document.open-rebound target
            // when present, so another gather cannot consume the next frame.
            for (index, phase) in documents.iter().enumerate() {
                if phase.skipped
                    && let Some(target) =
                        unsafe { &*host_ptr }.realtime_rendering_document(id, root, index)
                    && unsafe { &*host_ptr }.realtime_rendering_target_is_runnable(target)
                {
                    observer_runtime::request_realtime_resize_observer_document(
                        unsafe { &*host_ptr },
                        target,
                    );
                }
            }
        }
        Ok(invoked)
    }
}

fn finish_resize_observer_controllers(
    host: &JsContextHost,
    id: RendererPageRenderingUpdateTaskId,
    root: WindowDocumentTaskTarget,
) {
    if !host.realtime_rendering_opportunity_is_current(id, root) {
        return;
    }
    // Blink's ClearResizeObserverLimit creates an empty Window controller
    // after a successful RO phase, even if no constructor ran. Keep only its
    // owner metadata here; depth and loop-error state are pass-local above.
    let mut index = 0;
    while let Some(target) = host.realtime_rendering_document(id, root, index) {
        observer_runtime::ensure_realtime_resize_observer_controller(host, target);
        index += 1;
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
