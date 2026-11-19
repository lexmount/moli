//! DedicatedWorker error-event dispatch and propagation.
//!
//! This is deliberately separate from ordinary Worker message dispatch.
//! `dispatch_worker_error_event_with_kind` retains the established inner
//! checkpoint used to settle listener cancellation before deciding whether an
//! uncanceled Worker error propagates to the owning Window. That semantic
//! checkpoint is not the HTML task-end checkpoint: the selected Page-task
//! dispatcher still submits the ordinary completion after this body returns.

use crate::{
    diagnostics::ErrorReportOutcome,
    native_bridge::{PageScriptHost, WindowExecutionContextIdentity},
    worker::{WorkerErrorPhase, WorkerErrorReport},
};
use anyhow::Result;

pub(super) fn dispatch_script_load_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    error_message: &str,
    script_url: &str,
) {
    let worker_error = v8::null(scope).into();
    let _ = crate::context_bootstrap::dispatch_worker_error_event_with_kind(
        scope,
        worker,
        error_message,
        script_url,
        0,
        0,
        worker_error,
        crate::worker::WorkerParentErrorEventKind::Event,
    );
}

pub(super) fn dispatch_error_and_maybe_propagate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    owner_window: WindowExecutionContextIdentity,
    worker: Option<v8::Local<'s, v8::Object>>,
    mut error: WorkerErrorReport,
) -> Result<()> {
    let unhandled = if let Some(worker) = worker {
        let worker_error = v8::null(scope).into();
        let Some(unhandled) = crate::context_bootstrap::dispatch_worker_error_event_with_kind(
            scope,
            worker,
            &error.message,
            &error.filename,
            error.lineno,
            error.colno,
            worker_error,
            error.event_kind,
        ) else {
            return Ok(());
        };
        unhandled
    } else {
        true
    };
    // A failure while creating the worker's initial script only fires an
    // `error` event at the Worker object. Runtime script errors additionally
    // propagate to the owning Window when that Worker event is not canceled.
    if unhandled && error.phase == WorkerErrorPhase::Runtime {
        // The Worker listener may have retired its Window. Dropping the report
        // records the interruption.
        if !unsafe { &*host_ptr }.window_execution_context_identity_is_current(owner_window) {
            return Ok(());
        }
        let event = crate::context_bootstrap::dispatch_window_error_event(
            scope,
            host_ptr,
            &error.message,
            &error.filename,
            error.lineno,
            error.colno,
            None,
        )
        .map_err(anyhow::Error::msg)?;
        error.complete(|| {
            event.map_or(ErrorReportOutcome::Interrupted, |event| {
                ErrorReportOutcome::from_default_prevented(
                    crate::context_bootstrap::event_bool_attribute(
                        scope,
                        event,
                        "defaultPrevented",
                    ),
                )
            })
        });
    } else {
        error.complete(|| ErrorReportOutcome::from_default_prevented(!unhandled));
    }
    Ok(())
}
