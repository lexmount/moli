use super::*;
use crate::{document_runtime::DomHandle, exception_reporting::V8ExceptionReport};
use moli_webapi::events::subclasses::error::construct_original_error_event;

const BODY_ONERROR_RESOLUTION_GUARD_SLOT: &str = "__moliBodyOnerrorResolutionGuard";

pub(super) fn ensure_window_reflecting_body_onerror_handler(scope: &mut v8::PinScope<'_, '_>) {
    let global = scope.get_current_context().global(scope);
    if global_hidden_value(scope, WINDOW_ONERROR_SLOT).is_some_and(|handler| handler.is_function())
    {
        return;
    }
    if get_private_value(scope, global, BODY_ONERROR_RESOLUTION_GUARD_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        return;
    }
    set_private_value(
        scope,
        global,
        BODY_ONERROR_RESOLUTION_GUARD_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    let body = global
        .get(scope, v8str(scope, "document").into())
        .and_then(|document| v8::Local::<v8::Object>::try_from(document).ok())
        .and_then(|document| document.get(scope, v8str(scope, "body").into()))
        .and_then(|body| v8::Local::<v8::Object>::try_from(body).ok());
    if let Some(body) = body {
        let _ = body.get(scope, v8str(scope, "onerror").into());
    }
    set_private_value(
        scope,
        global,
        BODY_ONERROR_RESOLUTION_GUARD_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
}

struct ReportedExceptionDetails {
    message: String,
    filename: String,
    lineno: u32,
    colno: u32,
    error: v8::Global<v8::Value>,
}

fn reported_exception_details<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reason: v8::Local<'s, v8::Value>,
) -> ReportedExceptionDetails {
    // `reportError()` feeds the exception directly into HTML's exception
    // reporting algorithm. V8's Message preserves the engine's internal
    // exception metadata without observing author-defined `name`, `message`,
    // or location getters on the value.
    let exception_message = v8::Exception::create_message(scope, reason);
    let message = exception_message.get(scope).to_rust_string_lossy(scope);
    let filename = exception_message
        .get_script_resource_name(scope)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let lineno = exception_message
        .get_line_number(scope)
        .and_then(|line| u32::try_from(line).ok())
        .unwrap_or(0);
    let colno = exception_message
        .get_start_column()
        .checked_add(1)
        .and_then(|column| u32::try_from(column).ok())
        .unwrap_or(0);

    ReportedExceptionDetails {
        message,
        filename,
        lineno,
        colno,
        error: v8::Global::new(scope, reason),
    }
}

pub(crate) fn dispatch_window_report_error_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    message: &str,
    filename: Option<&str>,
) -> std::result::Result<(), String> {
    let message_value = v8_string(scope, message)
        .ok_or_else(|| "failed to allocate reportError message".to_owned())?;
    let reason = v8::Exception::error(scope, message_value);
    if let Some(filename) = filename
        && let Some(filename_value) = v8_string(scope, filename)
        && let Ok(error_object) = v8::Local::<v8::Object>::try_from(reason)
    {
        let _ = error_object.set(
            scope,
            v8str(scope, "fileName").into(),
            filename_value.into(),
        );
    }
    dispatch_window_error_event_with_details(
        scope,
        host_ptr,
        message,
        filename.unwrap_or(""),
        0,
        0,
        Some(reason),
    )
}

pub(crate) fn dispatch_window_error_event_with_details<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error_value: Option<v8::Local<'s, v8::Value>>,
) -> std::result::Result<(), String> {
    let event = dispatch_window_error_event(
        scope,
        host_ptr,
        message,
        filename,
        lineno,
        colno,
        error_value,
    )?;
    if let Some(event) = event {
        emit_window_exception_diagnostic(
            scope,
            unsafe { &*host_ptr },
            event,
            message,
            filename,
            (lineno, colno),
            error_value,
        );
    }
    Ok(())
}

pub(crate) fn dispatch_window_callback_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    child_event: Option<(DomHandle, v8::Local<'s, v8::Object>)>,
    report: &V8ExceptionReport,
) {
    let filename = report.source.as_deref().unwrap_or("");
    let lineno = report.line.unwrap_or(0) as u32;
    let colno = report.column.unwrap_or(0) as u32;
    if let Some((handle, event)) = child_event {
        unsafe { &mut *host_ptr }.dispatch_child_window_event(scope, handle, "error", event);
        emit_window_exception_diagnostic(
            scope,
            unsafe { &*host_ptr },
            event,
            &report.summary,
            filename,
            (lineno, colno),
            report.exception.as_ref(),
        );
    } else {
        let exception = report
            .exception
            .as_ref()
            .map(|exception| v8::Local::new(scope, exception));
        let _ = dispatch_window_error_event_with_details(
            scope,
            host_ptr,
            &report.summary,
            filename,
            lineno,
            colno,
            exception,
        );
    }
}

fn emit_window_exception_diagnostic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &PageScriptHost,
    event: v8::Local<'s, v8::Object>,
    message: &str,
    filename: &str,
    location: (u32, u32),
    exception: Option<impl v8::Handle<Data = v8::Value>>,
) {
    host.emit_diagnostic(|source| {
        let default_prevented =
            crate::context_bootstrap::event_bool_attribute(scope, event, "defaultPrevented");
        let stack = if default_prevented {
            None
        } else {
            exception.and_then(|exception| {
                let value = v8::Local::new(scope, exception);
                crate::exception_reporting::captured_exception_stack(scope, value)
            })
        };
        Some(crate::DiagnosticEvent::runtime_exception(
            source,
            crate::ExceptionOrigin::UncaughtException,
            message,
            filename,
            location,
            stack,
            crate::ErrorReportOutcome::from_default_prevented(default_prevented),
        ))
    });
}

pub(crate) fn dispatch_window_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error_value: Option<v8::Local<'s, v8::Value>>,
) -> std::result::Result<Option<v8::Local<'s, v8::Object>>, String> {
    let current_context = scope.get_current_context();
    let reporting_owner = unsafe { &*host_ptr }
        .window_execution_context_identity_for_v8_context(scope, current_context)
        .map(|identity| identity.owner());
    let _error_reporting_scope = if let Some(owner) = reporting_owner {
        let Some(reporting_scope) =
            (unsafe { &mut *host_ptr }).enter_window_error_reporting_scope(owner)
        else {
            return Ok(None);
        };
        Some(reporting_scope)
    } else {
        None
    };
    let popup_id = crate::native_bridge::active_lightweight_popup_id(scope);
    if popup_id.is_none() {
        ensure_window_reflecting_body_onerror_handler(scope);
    }
    let error_value = error_value.unwrap_or_else(|| v8::null(scope).into());

    let Some(event) =
        construct_original_error_event(scope, message, filename, lineno, colno, error_value)
    else {
        return Ok(None);
    };
    mark_event_trusted(scope, event);

    let runtime = unsafe { &mut *host_ptr };
    if let Some(popup_id) = popup_id {
        runtime.dispatch_lightweight_popup_window_event(scope, popup_id, "error", event);
        return Ok(Some(event));
    }
    if let Some(child_handle) =
        crate::context_bootstrap::child_browsing_context_handle_for_current_realm_scope(scope)
    {
        runtime.dispatch_child_window_event(scope, child_handle, "error", event);
        return Ok(Some(event));
    }
    runtime.dispatch_public_event(scope, host_ptr, EventTargetHandle::Window, event)?;

    let global = scope.get_current_context().global(scope);
    let global_value: v8::Local<'_, v8::Value> = global.into();
    let _ = crate::context_bootstrap::event_backing(scope, event).set(
        scope,
        v8str(scope, "target").into(),
        global_value,
    );
    let _ = crate::context_bootstrap::event_backing(scope, event).set(
        scope,
        v8str(scope, "currentTarget").into(),
        global_value,
    );
    Ok(Some(event))
}

pub(in crate::context_bootstrap) fn window_report_error_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };

    let receiver = match crate::native_bridge::WindowOperationReceiver::capture_and_authorize(
        scope,
        args.this(),
        unsafe { &*host_ptr },
    ) {
        Ok(receiver) => receiver,
        Err(crate::native_bridge::WindowOperationReceiverCaptureError::IllegalInvocation) => {
            throw_type_error(scope, "Illegal invocation");
            return;
        }
        Err(crate::native_bridge::WindowOperationReceiverCaptureError::CrossOrigin) => {
            crate::native_bridge::throw_cross_origin_location_security_error(scope);
            return;
        }
    };

    if args.length() < 1 {
        throw_type_error(
            scope,
            &crate::webidl::WebIdlError::missing_required(crate::webidl::Context::argument(
                "Window.reportError",
                1,
            ))
            .to_string(),
        );
        return;
    }

    let details = reported_exception_details(scope, args.get(0));
    let Some(binding) = receiver.resolve_live_binding(unsafe { &*host_ptr }) else {
        // A discarded Window remains a valid receiver, but there is no live
        // global left on which to report the exception.
        return;
    };
    let dispatch_result = binding.with_current_scope(scope, host_ptr, |scope, _dispatch_scope| {
        let error = v8::Local::new(scope, &details.error);
        dispatch_window_error_event_with_details(
            scope,
            host_ptr,
            &details.message,
            &details.filename,
            details.lineno,
            details.colno,
            Some(error),
        )
    });
    if let Some(Err(message)) = dispatch_result {
        throw_type_error(scope, &message);
    }
}
