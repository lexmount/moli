use crate::native_bridge::JsContextHost;

use super::super::{
    context_bootstrap::{
        dispatch_window_error_event_with_details, new_most_derived_dom_exception_value,
    },
    document_runtime::DomHandle,
    exception_reporting::V8ExceptionReport,
    host::report_event_listener_exception,
    util::{set_private_value, v8_string, v8str},
};

pub(super) const CUSTOM_ELEMENT_ALREADY_CONSTRUCTED_HANDLE_SLOT: &str =
    "__moliCustomElementAlreadyConstructedHandle";
const CUSTOM_ELEMENT_ALREADY_CONSTRUCTED_MESSAGE: &str =
    "Custom element constructor already consumed this element";

pub(super) enum ConstructionFailure<'s> {
    TypeError(&'static str),
    NotSupported(&'static str),
    Exception(v8::Local<'s, v8::Value>),
}

pub(super) fn report_custom_element_construction_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    constructor: Option<v8::Local<'s, v8::Function>>,
    failure: ConstructionFailure<'s>,
) {
    let (message, error_value) = match failure {
        ConstructionFailure::TypeError(message) => {
            let message_value =
                v8_string(scope, message).unwrap_or_else(|| v8::String::empty(scope));
            (
                message.to_owned(),
                v8::Exception::type_error(scope, message_value),
            )
        }
        ConstructionFailure::NotSupported(message) => (
            message.to_owned(),
            new_most_derived_dom_exception_value(scope, message, "NotSupportedError"),
        ),
        ConstructionFailure::Exception(exception) => {
            (error_message_for_value(scope, exception), exception)
        }
    };
    if let Some(constructor) = constructor {
        let report = V8ExceptionReport {
            muted_errors: false,
            summary: message,
            source: None,
            line: None,
            column: None,
            source_line: None,
            stack: None,
            callback_context: None,
            exception: Some(v8::Global::new(scope, error_value)),
        };
        report_event_listener_exception(
            scope,
            host_ptr,
            "custom element construction",
            constructor,
            &report,
        );
        return;
    }
    let _ = dispatch_window_error_event_with_details(
        scope,
        host_ptr,
        &message,
        "",
        0,
        0,
        Some(error_value),
    );
}

fn error_message_for_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> String {
    if value.is_object()
        && let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Some(message) = object.get(scope, v8str(scope, "message").into())
        && !message.is_null_or_undefined()
        && let Some(message) = message.to_string(scope)
    {
        return message.to_rust_string_lossy(scope);
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

pub(crate) fn throw_already_constructed_custom_element_error(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) {
    let message = v8_string(scope, CUSTOM_ELEMENT_ALREADY_CONSTRUCTED_MESSAGE)
        .unwrap_or_else(|| v8::String::empty(scope));
    let exception = v8::Exception::type_error(scope, message);
    if let Ok(object) = v8::Local::<v8::Object>::try_from(exception) {
        let handle_value = v8::BigInt::new_from_u64(scope, handle.index() as u64);
        set_private_value(
            scope,
            object,
            CUSTOM_ELEMENT_ALREADY_CONSTRUCTED_HANDLE_SLOT,
            handle_value.into(),
        );
    }
    scope.throw_exception(exception);
}
