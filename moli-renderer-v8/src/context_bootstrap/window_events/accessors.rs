use super::super::window_accessors::window_child_context_handle;
use super::*;
use crate::{
    document_runtime::EventTargetHandle,
    util::{context_host_ptr_from_global_bridge, context_host_ptr_from_window_object},
    webidl,
};

fn require_window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> bool {
    if context_host_ptr_from_window_object(scope, args.this()).is_some() {
        return true;
    }
    webidl::throw_type_error(
        scope,
        "Window event handler called on incompatible receiver.",
    );
    false
}

fn window_event_handler_name_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<String> {
    v8::Local::<v8::String>::try_from(data)
        .ok()
        .map(|name| name.to_rust_string_lossy(scope))
}

fn window_event_handler_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    property_name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let host_ptr = context_host_ptr_from_window_object(scope, receiver)
        .or_else(|| context_host_ptr_from_global_bridge(scope))?;
    match window_child_context_handle(scope, receiver) {
        Some(handle) => unsafe { &*host_ptr }.child_window_event_handler_property_value(
            scope,
            handle,
            property_name,
        ),
        None => crate::native_bridge::element::resolve_window_event_handler_content_attribute(
            scope,
            host_ptr,
            property_name.strip_prefix("on").unwrap_or(property_name),
        ),
    }
}

fn set_window_event_handler_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    property_name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_window_object(scope, receiver)
        .or_else(|| context_host_ptr_from_global_bridge(scope))
    else {
        return;
    };
    let handler = v8::Local::<v8::Function>::try_from(value).ok();
    let host = unsafe { &mut *host_ptr };
    match window_child_context_handle(scope, receiver) {
        Some(handle) => {
            let relevant_context = handler
                .and_then(|handler| handler.get_creation_context(scope))
                .unwrap_or_else(|| scope.get_current_context());
            host.set_child_window_event_handler_property(
                scope,
                handle,
                property_name,
                handler,
                relevant_context,
            );
        }
        None => host.set_registered_event_handler_property(
            scope,
            EventTargetHandle::Window,
            property_name.strip_prefix("on").unwrap_or(property_name),
            handler,
        ),
    }
}

pub(in crate::context_bootstrap) fn window_event_handler_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    let Some(property_name) = window_event_handler_name_from_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    rv.set(
        window_event_handler_value(scope, args.this(), &property_name)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(in crate::context_bootstrap) fn window_event_handler_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    if let Some(property_name) = window_event_handler_name_from_data(scope, args.data()) {
        set_window_event_handler_value(scope, args.this(), &property_name, args.get(0));
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn window_console_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    let global = scope.get_current_context().global(scope);
    match get_private_value(scope, global, WINDOW_CONSOLE_SLOT)
        .filter(|value| !value.is_undefined())
    {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::context_bootstrap) fn window_event_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    match window_event_value_for_receiver(scope, args.this()) {
        Some(value) => rv.set(value),
        None => rv.set(v8::undefined(scope).into()),
    }
}

pub(in crate::context_bootstrap) fn window_event_value_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let target_event = context_host_ptr_from_window_object(scope, receiver)
        .or_else(|| context_host_ptr_from_global_bridge(scope))
        .and_then(|host_ptr| {
            let dispatch_scope = if let Some(popup_id) =
                crate::native_bridge::lightweight_popup_id_from_window(scope, receiver)
            {
                crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id)
            } else if let Some(handle) = window_child_context_handle(scope, receiver) {
                crate::native_bridge::OwnerDispatchScope::Child(handle)
            } else {
                crate::native_bridge::OwnerDispatchScope::Top
            };
            let host = unsafe { &*host_ptr };
            let owner = host.current_window_execution_context_owner(dispatch_scope)?;
            let (_, context) = host.window_execution_context(scope, owner, dispatch_scope)?;
            let global = context.global(scope);
            object_own_hidden_value(scope, global, WINDOW_EVENT_SLOT)
        });
    target_event.or_else(|| global_hidden_value(scope, WINDOW_EVENT_SLOT))
}

pub(in crate::context_bootstrap) fn window_event_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    let global = scope.get_current_context().global(scope);
    let key = v8str(scope, WINDOW_EVENT_SLOT);
    let _ = global.set(scope, key.into(), args.get(0));
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn window_onmessageerror_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    rv.set(
        window_event_handler_value(scope, args.this(), "onmessageerror")
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(in crate::context_bootstrap) fn window_onmessageerror_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    set_window_event_handler_value(scope, args.this(), "onmessageerror", args.get(0));
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn window_onerror_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    if window_child_context_handle(scope, args.this()).is_some() {
        rv.set(
            window_event_handler_value(scope, args.this(), "onerror")
                .unwrap_or_else(|| v8::null(scope).into()),
        );
        return;
    }
    super::error::ensure_window_reflecting_body_onerror_handler(scope);
    rv.set(
        window_event_handler_value(scope, args.this(), "onerror")
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(in crate::context_bootstrap) fn window_onerror_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    set_window_event_handler_value(scope, args.this(), "onerror", args.get(0));
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn window_onunhandledrejection_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    rv.set(
        window_event_handler_value(scope, args.this(), "onunhandledrejection")
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(in crate::context_bootstrap) fn window_onunhandledrejection_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    set_window_event_handler_value(scope, args.this(), "onunhandledrejection", args.get(0));
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn window_onrejectionhandled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    rv.set(
        window_event_handler_value(scope, args.this(), "onrejectionhandled")
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(in crate::context_bootstrap) fn window_onrejectionhandled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    set_window_event_handler_value(scope, args.this(), "onrejectionhandled", args.get(0));
    rv.set_undefined();
}
