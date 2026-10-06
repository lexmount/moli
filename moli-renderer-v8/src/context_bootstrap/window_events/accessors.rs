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
    let host = unsafe { &*host_ptr };
    match window_child_context_handle(scope, receiver) {
        Some(handle) => {
            host.child_window_event_handler_property_value(scope, handle, property_name)
        }
        None => host.registered_event_handler_property_value(
            scope,
            EventTargetHandle::Window,
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

fn window_event_value_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    // Real globals keep the current event in their concrete callback realm.
    // Resolving only the Document owner would read the default world's slot
    // even when this receiver is an isolated world's Window.
    if let Some(context) = receiver.get_creation_context(scope)
        && receiver.strict_equals(context.global(scope).into())
    {
        return get_private_value(scope, receiver, WINDOW_EVENT_SLOT);
    }
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
            // An empty target Window current event is still authoritative;
            // do not fall back to the caller Window's active event.
            Some(
                get_private_value(scope, global, WINDOW_EVENT_SLOT)
                    .unwrap_or_else(|| v8::undefined(scope).into()),
            )
        });
    target_event.or_else(|| {
        let global = scope.get_current_context().global(scope);
        get_private_value(scope, global, WINDOW_EVENT_SLOT)
    })
}

pub(in crate::context_bootstrap) fn window_event_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    super::super::runtime_state::define_replaceable_window_property(
        scope,
        args.this(),
        "event",
        args.get(0),
    );
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
    rv.set(window_event_handler_slot_value(scope, WINDOW_ONERROR_SLOT));
}

pub(in crate::context_bootstrap) fn window_onerror_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    if window_child_context_handle(scope, args.this()).is_some() {
        set_window_event_handler_value(scope, args.this(), "onerror", args.get(0));
        rv.set_undefined();
        return;
    }
    set_window_body_onerror_handler_compiled(scope, true);
    set_window_onerror_handler_value(scope, args.get(0));
    rv.set_undefined();
}

pub(crate) fn set_window_onerror_handler_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) {
    set_window_event_handler_slot(scope, WINDOW_ONERROR_SLOT, value);
}

pub(crate) fn window_body_onerror_handler_is_compiled(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, WINDOW_BODY_ONERROR_COMPILED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn set_window_body_onerror_handler_compiled(
    scope: &mut v8::PinScope<'_, '_>,
    compiled: bool,
) {
    let global = scope.get_current_context().global(scope);
    let compiled = v8::Boolean::new(scope, compiled);
    set_private_value(
        scope,
        global,
        WINDOW_BODY_ONERROR_COMPILED_SLOT,
        compiled.into(),
    );
}

pub(in crate::context_bootstrap) fn window_onunhandledrejection_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    if window_child_context_handle(scope, args.this()).is_some() {
        rv.set(
            window_event_handler_value(scope, args.this(), "onunhandledrejection")
                .unwrap_or_else(|| v8::null(scope).into()),
        );
        return;
    }
    rv.set(window_event_handler_slot_value(
        scope,
        WINDOW_ONUNHANDLEDREJECTION_SLOT,
    ));
}

pub(in crate::context_bootstrap) fn window_onunhandledrejection_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    if window_child_context_handle(scope, args.this()).is_some() {
        set_window_event_handler_value(scope, args.this(), "onunhandledrejection", args.get(0));
        rv.set_undefined();
        return;
    }
    set_window_event_handler_slot(scope, WINDOW_ONUNHANDLEDREJECTION_SLOT, args.get(0));
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
    if window_child_context_handle(scope, args.this()).is_some() {
        rv.set(
            window_event_handler_value(scope, args.this(), "onrejectionhandled")
                .unwrap_or_else(|| v8::null(scope).into()),
        );
        return;
    }
    rv.set(window_event_handler_slot_value(
        scope,
        WINDOW_ONREJECTIONHANDLED_SLOT,
    ));
}

pub(in crate::context_bootstrap) fn window_onrejectionhandled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_window_receiver(scope, &args) {
        return;
    }
    if window_child_context_handle(scope, args.this()).is_some() {
        set_window_event_handler_value(scope, args.this(), "onrejectionhandled", args.get(0));
        rv.set_undefined();
        return;
    }
    set_window_event_handler_slot(scope, WINDOW_ONREJECTIONHANDLED_SLOT, args.get(0));
    rv.set_undefined();
}

fn set_window_event_handler_slot(
    scope: &mut v8::PinScope<'_, '_>,
    slot_name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    let key = v8str(scope, slot_name);

    if value.is_null_or_undefined() {
        let _ = global.set(scope, key.into(), v8::null(scope).into());
        return;
    }

    if value.is_function() {
        let _ = global.set(scope, key.into(), value);
        return;
    }

    let _ = global.set(scope, key.into(), v8::null(scope).into());
}

fn window_event_handler_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    slot_name: &'static str,
) -> v8::Local<'s, v8::Value> {
    match global_hidden_value(scope, slot_name) {
        Some(value) if !value.is_undefined() => value,
        _ => v8::null(scope).into(),
    }
}
