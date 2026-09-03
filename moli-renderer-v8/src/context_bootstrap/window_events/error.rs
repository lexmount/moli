use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ErrorEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    cancelable: bool,
    #[webapi(data_property, enumerable)]
    bubbles: bool,
    #[webapi(data_property, enumerable)]
    message: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    filename: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    lineno: u32,
    #[webapi(data_property, enumerable)]
    colno: u32,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct ErrorEventDetailsDeclaration<'scope> {
    message: v8::Local<'scope, v8::Value>,
    filename: v8::Local<'scope, v8::Value>,
    lineno: u32,
    colno: u32,
    error: v8::Local<'scope, v8::Value>,
}

pub(super) fn ensure_window_reflecting_body_onerror_handler(scope: &mut v8::PinScope<'_, '_>) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let _ = crate::native_bridge::element::resolve_window_event_handler_content_attribute(
        scope, host_ptr, "error",
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
    host_ptr: *mut JsContextHost,
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
    host_ptr: *mut JsContextHost,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error_value: Option<v8::Local<'s, v8::Value>>,
) -> std::result::Result<(), String> {
    let current_context = scope.get_current_context();
    let reporting_owner = unsafe { &*host_ptr }
        .window_execution_context_identity_for_v8_context(scope, current_context)
        .map(|identity| identity.owner());
    let _error_reporting_scope = if let Some(owner) = reporting_owner {
        let Some(reporting_scope) =
            (unsafe { &mut *host_ptr }).enter_window_error_reporting_scope(owner)
        else {
            return Ok(());
        };
        Some(reporting_scope)
    } else {
        None
    };

    let global = scope.get_current_context().global(scope);
    ensure_window_reflecting_body_onerror_handler(scope);
    let error_value = error_value.unwrap_or_else(|| v8::null(scope).into());

    let message = v8_string(scope, message)
        .map(|s| s.into())
        .unwrap_or_else(|| v8::null(scope).into());
    let filename = v8_string(scope, filename)
        .map(|s| s.into())
        .unwrap_or_else(|| v8::null(scope).into());
    let init =
        ErrorEventInitDeclaration::new(true, false, message, filename, lineno, colno, error_value)
            .bind(scope)
            .expect("ErrorEvent init declaration should bind");

    let event_type = v8str(scope, "error");
    let event = global
        .get(scope, v8str(scope, "ErrorEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .and_then(|ctor| ctor.new_instance(scope, &[event_type.into(), init.into()]))
        .or_else(|| {
            let event_ctor = global
                .get(scope, v8str(scope, "Event").into())
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
            let event = event_ctor.new_instance(scope, &[event_type.into(), init.into()])?;
            let details =
                ErrorEventDetailsDeclaration::new(message, filename, lineno, colno, error_value);
            if details.initialize(scope, event).is_err() {
                return None;
            }
            Some(event)
        });
    let Some(event) = event else {
        return Ok(());
    };
    mark_event_trusted(scope, event);

    let runtime = unsafe { &mut *host_ptr };
    if let Some(child_handle) =
        crate::context_bootstrap::child_browsing_context_handle_for_current_realm_scope(scope)
    {
        runtime.dispatch_child_window_event(scope, child_handle, "error", event);
        return Ok(());
    }
    runtime.dispatch_public_event(scope, host_ptr, EventTargetHandle::Window, event)?;

    let global_value: v8::Local<'_, v8::Value> = global.into();
    let _ = event.set(scope, v8str(scope, "target").into(), global_value);
    let _ = event.set(scope, v8str(scope, "currentTarget").into(), global_value);
    Ok(())
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
