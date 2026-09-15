use crate::dom::native::Node;
use crate::util::v8_string;

use super::super::super::{
    node::node_runtime_and_handle_from_object_or_detached, set_wrapped_handle_or_null_for_receiver,
};
use super::super::{
    element_attribute, property_string_value, set_reflected_attribute,
    set_reflected_boolean_attribute,
};

fn template_shadow_root_mode_value(value: &str) -> Option<&'static str> {
    if value.eq_ignore_ascii_case("open") {
        Some("open")
    } else if value.eq_ignore_ascii_case("closed") {
        Some("closed")
    } else {
        None
    }
}

fn template_shadow_root_mode_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = element_attribute(runtime, handle, "shadowrootmode")
        .as_deref()
        .and_then(template_shadow_root_mode_value)
        .unwrap_or("");
    let Some(value) = v8_string(scope, value) else {
        rv.set_undefined();
        return;
    };
    rv.set(value.into());
}

fn template_shadow_root_slot_assignment_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = element_attribute(runtime, handle, "shadowrootslotassignment")
        .filter(|value| value.eq_ignore_ascii_case("manual"))
        .map(|_| "manual")
        .unwrap_or("named");
    let Some(value) = v8_string(scope, value) else {
        rv.set_undefined();
        return;
    };
    rv.set(value.into());
}

fn template_shadow_root_adopted_style_sheets_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value =
        element_attribute(runtime, handle, "shadowrootadoptedstylesheets").unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_undefined();
        return;
    };
    rv.set(value.into());
}

fn template_shadow_root_custom_element_registry_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value =
        element_attribute(runtime, handle, "shadowrootcustomelementregistry").unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_undefined();
        return;
    };
    rv.set(value.into());
}

fn template_shadow_root_boolean_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_undefined();
        return;
    };
    rv.set_bool(element_attribute(unsafe { &*runtime_ptr }, handle, name).is_some());
}

fn set_template_shadow_root_string_attribute_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    let Some(value) = property_string_value(scope, value) else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, name, &value);
}

fn set_template_shadow_root_boolean_attribute_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    set_reflected_boolean_attribute(scope, runtime_ptr, handle, name, value.boolean_value(scope));
}

fn template_content_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(contents_handle) = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(|element| element.template_contents())
    else {
        rv.set_null();
        return;
    };
    set_wrapped_handle_or_null_for_receiver(
        scope,
        &mut rv,
        runtime_ptr,
        object,
        Some(contents_handle),
    );
}

pub(in crate::native_bridge) fn template_content_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    template_content_for_object(scope, args.this(), rv);
}

pub(in crate::native_bridge) fn template_shadow_root_mode_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    template_shadow_root_mode_for_object(scope, args.this(), rv);
}

pub(in crate::native_bridge) fn template_shadow_root_mode_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_template_shadow_root_string_attribute_for_object(
        scope,
        args.this(),
        "shadowrootmode",
        args.get(0),
    );
}

pub(in crate::native_bridge) fn template_shadow_root_delegates_focus_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    template_shadow_root_boolean_for_object(scope, args.this(), "shadowrootdelegatesfocus", rv);
}

pub(in crate::native_bridge) fn template_shadow_root_delegates_focus_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_template_shadow_root_boolean_attribute_for_object(
        scope,
        args.this(),
        "shadowrootdelegatesfocus",
        args.get(0),
    );
}

pub(in crate::native_bridge) fn template_shadow_root_clonable_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    template_shadow_root_boolean_for_object(scope, args.this(), "shadowrootclonable", rv);
}

pub(in crate::native_bridge) fn template_shadow_root_clonable_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_template_shadow_root_boolean_attribute_for_object(
        scope,
        args.this(),
        "shadowrootclonable",
        args.get(0),
    );
}

pub(in crate::native_bridge) fn template_shadow_root_serializable_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    template_shadow_root_boolean_for_object(scope, args.this(), "shadowrootserializable", rv);
}

pub(in crate::native_bridge) fn template_shadow_root_serializable_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_template_shadow_root_boolean_attribute_for_object(
        scope,
        args.this(),
        "shadowrootserializable",
        args.get(0),
    );
}

pub(in crate::native_bridge) fn template_shadow_root_custom_element_registry_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    template_shadow_root_custom_element_registry_for_object(scope, args.this(), rv);
}

pub(in crate::native_bridge) fn template_shadow_root_custom_element_registry_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_template_shadow_root_string_attribute_for_object(
        scope,
        args.this(),
        "shadowrootcustomelementregistry",
        args.get(0),
    );
}

pub(in crate::native_bridge) fn template_shadow_root_slot_assignment_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    template_shadow_root_slot_assignment_for_object(scope, args.this(), rv);
}

pub(in crate::native_bridge) fn template_shadow_root_slot_assignment_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_template_shadow_root_string_attribute_for_object(
        scope,
        args.this(),
        "shadowrootslotassignment",
        args.get(0),
    );
}

pub(in crate::native_bridge) fn template_shadow_root_adopted_style_sheets_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    template_shadow_root_adopted_style_sheets_for_object(scope, args.this(), rv);
}

pub(in crate::native_bridge) fn template_shadow_root_adopted_style_sheets_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_template_shadow_root_string_attribute_for_object(
        scope,
        args.this(),
        "shadowrootadoptedstylesheets",
        args.get(0),
    );
}
