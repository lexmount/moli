use super::reactions::enter_custom_element_reaction;

use super::super::{
    document_runtime::DomHandle,
    exception_reporting::{CallbackExceptionLogLevel, invoke_callback_with_report},
    host::report_event_listener_exception,
    native_bridge::{PageScriptHost, wrapped_handle_value},
};

pub(super) fn invoke_custom_element_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    callback_name: &str,
    callback: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
) {
    if let Err(report) = invoke_callback_with_report(
        scope,
        "custom element callback",
        "custom element callback threw",
        CallbackExceptionLogLevel::Debug,
        callback_name,
        callback,
        receiver,
        args,
    ) {
        report_event_listener_exception(
            scope,
            host_ptr,
            "custom element callback",
            callback,
            &report,
        );
    }
}

pub(crate) fn call_lifecycle_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    handle: DomHandle,
    callback_name: &str,
) {
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle))
    {
        return;
    }
    let Some(wrapper) = custom_element_callback_receiver(scope, host_ptr, handle) else {
        return;
    };
    let Some(callback) = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| {
            store.lifecycle_callback_for_handle(scope, host_ptr, handle, callback_name)
        })
    else {
        return;
    };
    let _reaction = enter_custom_element_reaction(host_ptr);
    invoke_custom_element_callback(
        scope,
        host_ptr,
        &format!("custom element {callback_name}"),
        callback,
        wrapper.into(),
        &[],
    );
}

pub(super) fn custom_element_callback_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    wrapped_handle_value(scope, host_ptr, handle)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}
