//! DedicatedWorker error-event dispatch and propagation.
//!
//! This is deliberately separate from ordinary Worker message dispatch.
//! `dispatch_worker_error_event_with_*` retains the established inner
//! checkpoint used to settle listener cancellation before deciding whether an
//! uncanceled Worker error propagates to the owning Window. That semantic
//! checkpoint is not the HTML task-end checkpoint: the selected Page-task
//! dispatcher still submits the ordinary completion after this body returns.

use anyhow::Result;

pub(super) fn dispatch_script_load_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    error_message: &str,
    script_url: &str,
) {
    let worker_error = v8::null(scope).into();
    crate::context_bootstrap::dispatch_worker_error_event_with_error(
        scope,
        worker,
        error_message,
        script_url,
        0,
        0,
        worker_error,
    );
}

pub(super) struct DedicatedWorkerError<'a> {
    pub(super) message: &'a str,
    pub(super) filename: &'a str,
    pub(super) lineno: u32,
    pub(super) colno: u32,
    pub(super) event_kind: crate::worker::WorkerParentErrorEventKind,
    pub(super) phase: crate::worker::WorkerErrorPhase,
}

pub(super) fn dispatch_error_and_maybe_propagate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    worker: v8::Local<'s, v8::Object>,
    error: DedicatedWorkerError<'_>,
) -> Result<()> {
    let worker_error = v8::null(scope).into();
    let unhandled = crate::context_bootstrap::dispatch_worker_error_event_with_kind(
        scope,
        worker,
        error.message,
        error.filename,
        error.lineno,
        error.colno,
        worker_error,
        error.event_kind,
    );
    // A failure while creating the worker's initial script only fires an
    // `error` event at the Worker object. Runtime script errors additionally
    // propagate to the owning Window when that Worker event is not canceled.
    if unhandled && error.phase == crate::worker::WorkerErrorPhase::Runtime {
        crate::context_bootstrap::dispatch_window_error_event_with_details(
            scope,
            host_ptr,
            error.message,
            error.filename,
            error.lineno,
            error.colno,
            None,
        )
        .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}
