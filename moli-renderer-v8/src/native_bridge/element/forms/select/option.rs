use super::helpers::{
    effective_option_selected, element_option_value, select_is_multiple, select_option_handles,
};
use super::*;
use crate::dom::forms::{OptionNearestSelectStep, OptionNearestSelectTraversal};
use crate::native_bridge::{
    document::{
        detached_element_local_name, detached_element_namespace_uri, detached_form_owner_object,
        detached_parent_node_object, set_detached_text_replacement_value,
    },
    element::{html_element_getter_receiver, html_element_setter_receiver},
};

fn option_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_getter_receiver(scope, receiver, "HTMLOptionElement", member, "option")
}

fn option_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_setter_receiver(scope, receiver, "HTMLOptionElement", member, "option")
}

fn option_boolean_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &'static str,
    property: &'static str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_getter_receiver(scope, receiver, property) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        attribute,
    ));
}

fn set_option_boolean_attribute_on_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &'static str,
    property: &'static str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_setter_receiver(scope, receiver, property) else {
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        attribute,
        value.boolean_value(scope),
    );
}

fn set_option_dom_string_attribute_on_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &'static str,
    property: &'static str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_setter_receiver(scope, receiver, property) else {
        return;
    };
    let Some(value) =
        form_dom_string_property_value(scope, value, "HTMLOptionElement", property, false)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value);
}

fn detached_option_form_owner_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    option: v8::Local<'s, v8::Object>,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    let mut current = detached_parent_node_object(scope, option)?;
    let mut traversal = OptionNearestSelectTraversal::default();
    loop {
        if let (Some(namespace), Some(local_name)) = (
            detached_element_namespace_uri(scope, current),
            detached_element_local_name(scope, current),
        ) {
            match traversal.visit_ancestor(&namespace, &local_name) {
                OptionNearestSelectStep::Continue => {}
                OptionNearestSelectStep::Select => {
                    return Some(detached_form_owner_object(scope, current));
                }
                OptionNearestSelectStep::Blocked => return Some(None),
            }
        }
        let Some(parent) = detached_parent_node_object(scope, current) else {
            return Some(None);
        };
        current = parent;
    }
}

pub(in crate::native_bridge) fn option_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_getter_receiver(scope, args.this(), "value") else {
        rv.set_null();
        return;
    };
    let value = element_option_value(unsafe { &*runtime_ptr }, handle).unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn option_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_setter_receiver(scope, args.this(), "value") else {
        rv.set_undefined();
        return;
    };
    let Some(value) =
        form_dom_string_property_value(scope, args.get(0), "HTMLOptionElement", "value", false)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "value", &value);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn option_text_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_getter_receiver(scope, args.this(), "text") else {
        rv.set_empty_string();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| element.option_text(runtime.dom_host().dom(), handle))
        .unwrap_or_default();
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn option_text_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_setter_receiver(scope, args.this(), "text") else {
        rv.set_undefined();
        return;
    };
    let Some(value) =
        form_dom_string_property_value(scope, args.get(0), "HTMLOptionElement", "text", false)
    else {
        return;
    };
    if set_detached_text_replacement_value(scope, args.this(), &value).is_some() {
        rv.set_undefined();
        return;
    }
    let _ = set_text_content_in_reaction_scope(scope, runtime_ptr, handle, &value);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn option_selected_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_getter_receiver(scope, args.this(), "selected") else {
        rv.set_bool(false);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    rv.set_bool(effective_option_selected(runtime, handle));
}

pub(in crate::native_bridge) fn option_selected_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_setter_receiver(scope, args.this(), "selected") else {
        rv.set_undefined();
        return;
    };
    let selected = args.get(0).boolean_value(scope);
    let runtime = unsafe { &mut *runtime_ptr };
    let owner_select = runtime.dom_host().option_nearest_ancestor_select(handle);
    if let Some(select_handle) = owner_select
        && selected
        && !select_is_multiple(runtime, select_handle)
    {
        for option in select_option_handles(runtime, select_handle) {
            let _ = runtime.set_selected_state(scope, runtime_ptr, option, option == handle);
        }
        let _ = runtime.set_select_explicit_none(scope, runtime_ptr, select_handle, false);
    } else {
        let _ = runtime.set_selected_state(scope, runtime_ptr, handle, selected);
        if let Some(select_handle) = owner_select
            && !select_is_multiple(runtime, select_handle)
        {
            let _ = runtime.set_select_explicit_none(scope, runtime_ptr, select_handle, false);
        }
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn option_form_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_getter_receiver(scope, args.this(), "form") else {
        rv.set_null();
        return;
    };
    if let Some(form) = detached_option_form_owner_object(scope, args.this()) {
        match form {
            Some(form) => rv.set(form.into()),
            None => rv.set_null(),
        }
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let form = runtime
        .dom_host()
        .option_nearest_ancestor_select(handle)
        .and_then(|select| form_associated_form_owner(runtime, select));
    set_wrapped_node_or_null(scope, &mut rv, runtime_ptr, form);
}

pub(in crate::native_bridge) fn option_index_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_getter_receiver(scope, args.this(), "index") else {
        rv.set_int32(0);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let index = runtime
        .dom_host()
        .option_nearest_ancestor_select(handle)
        .and_then(|select| {
            select_option_handles(runtime, select)
                .into_iter()
                .position(|option| option == handle)
        })
        .unwrap_or(0) as i32;
    rv.set_int32(index);
}

pub(in crate::native_bridge) fn option_label_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = option_getter_receiver(scope, args.this(), "label") else {
        rv.set_empty_string();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let label = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| element.option_label(runtime.dom_host().dom(), handle))
        .unwrap_or_default();
    if let Some(label) = v8_string(scope, &label) {
        rv.set(label.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn option_label_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_option_dom_string_attribute_on_receiver(scope, args.this(), "label", "label", args.get(0));
    rv.set_undefined();
}

pub(in crate::native_bridge) fn option_default_selected_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    option_boolean_attribute_getter(scope, args.this(), "selected", "defaultSelected", rv);
}

pub(in crate::native_bridge) fn option_default_selected_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_option_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "selected",
        "defaultSelected",
        args.get(0),
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn option_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    option_boolean_attribute_getter(scope, args.this(), "disabled", "disabled", rv);
}

pub(in crate::native_bridge) fn option_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_option_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "disabled",
        "disabled",
        args.get(0),
    );
    rv.set_undefined();
}
