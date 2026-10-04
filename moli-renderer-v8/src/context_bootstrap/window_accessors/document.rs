use super::helpers::{window_child_context_handle, window_host_ptr, window_receiver};
use crate::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, WindowEnvironmentSettings},
    util::{get_private_object, set_private_value},
};

pub(in crate::context_bootstrap) const WINDOW_DOCUMENT_SLOT: &str =
    "__moliWindowAssociatedDocument";

pub(in crate::context_bootstrap) fn window_document_getter_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    cache_property: Option<v8::Local<'s, v8::Private>>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let template = match cache_property {
        Some(cache_property) => {
            v8::FunctionTemplate::new_with_cache(scope, window_document_getter, cache_property)
        }
        None => v8::FunctionTemplate::new(scope, window_document_getter),
    };
    template.remove_prototype();
    // V8 checks the receiver against the actual access Context before
    // invoking the callback, including when the getter is borrowed.
    template.set_accept_any_receiver(false);
    template
}

pub(crate) fn retain_window_document_in_retired_realm(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<()> {
    let context = scope.get_current_context();
    let host_ptr = crate::util::context_host_ptr_from_context_slot(context)?;
    let _ = WindowEnvironmentSettings::bind_current_main_document_for_retirement(scope, unsafe {
        &*host_ptr
    });
    let window = context.global(scope);
    if get_private_object(scope, window, WINDOW_DOCUMENT_SLOT).is_some() {
        return Some(());
    }
    let handle = WindowEnvironmentSettings::for_current_realm(scope)
        .map(|settings| settings.document_handle())
        .unwrap_or_else(|| unsafe { &*host_ptr }.document_handle());
    let document = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)?;
    // Fix the outgoing global's Document before its WindowProxy is reused.
    // V8 resolves a cached accessor on this global, so a saved JS function
    // keeps reading its own Document after navigation.
    set_private_value(scope, window, WINDOW_DOCUMENT_SLOT, document.into());
    Some(())
}

pub(crate) fn bind_current_child_window_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    child_handle: DomHandle,
    document: v8::Local<'s, v8::Object>,
) -> Option<()> {
    WindowEnvironmentSettings::bind_current_child_document(scope, host, child_handle)?;
    let window = scope.get_current_context().global(scope);
    // Realm retirement clears the shared wrapper cache. Keep the original
    // Document as a private V8 edge so Window.document retains object identity
    // without a Rust Global keeping this Context alive.
    set_private_value(scope, window, WINDOW_DOCUMENT_SLOT, document.into());
    Some(())
}

pub(in crate::context_bootstrap) fn window_document_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let Some(context) = receiver.get_creation_context(scope) else {
        rv.set_null();
        return;
    };
    let Some(host_ptr) = crate::util::context_host_ptr_from_context_slot(context)
        .or_else(|| window_host_ptr(scope, receiver))
    else {
        rv.set_null();
        return;
    };
    // A borrowed getter runs in its function's realm. Resolve both settings
    // and wrappers in the receiver's realm, including a retained old Window.
    let scope = &mut v8::ContextScope::new(scope, context);
    let window = context.global(scope);
    if let Some(document) = get_private_object(scope, window, WINDOW_DOCUMENT_SLOT) {
        rv.set(document.into());
        return;
    }
    let handle = if let Some(settings) = WindowEnvironmentSettings::for_current_realm(scope) {
        settings.document_handle()
    } else if let Some(handle) = window_child_context_handle(scope, receiver) {
        // During bootstrap the Document wrapper can precede settings binding.
        // Only this exact live Window may follow the child's current route.
        let host = unsafe { &*host_ptr };
        if host
            .window_execution_context_identity_for_access_check(context)
            .is_none_or(|identity| !host.window_execution_context_identity_is_current(identity))
        {
            rv.set_null();
            return;
        }
        match unsafe { &mut *host_ptr }.child_browsing_context_document_wrapper(scope, handle) {
            Some(document) => rv.set(document.into()),
            None => rv.set_null(),
        }
        return;
    } else {
        unsafe { &*host_ptr }.document_handle()
    };
    match unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)
    {
        Some(document) => rv.set(document.into()),
        None => rv.set_null(),
    }
}
