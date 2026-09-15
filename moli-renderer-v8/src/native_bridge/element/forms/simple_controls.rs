use super::*;
use crate::native_bridge::document::{
    detached_element_local_name, detached_form_owner_object, detached_native_handle_for_runtime,
    detached_native_object_for_handle, detached_parent_node_object,
};
use crate::native_bridge::element::{html_element_getter_receiver, html_element_setter_receiver};
use moli_dom::forms::{
    MeterElementValues, ProgressElementValues, meter_element_values, progress_element_values,
};

fn button_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_getter_receiver(scope, receiver, "HTMLButtonElement", member, "button")
}

fn button_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_setter_receiver(scope, receiver, "HTMLButtonElement", member, "button")
}

fn button_string_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    property: &'static str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = button_getter_receiver(scope, receiver, property) else {
        rv.set_empty_string();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, attribute).unwrap_or_default();
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn set_button_dom_string_attribute_on_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    property: &'static str,
) {
    let Some((runtime_ptr, handle)) = button_setter_receiver(scope, receiver, property) else {
        return;
    };
    let Some(value) =
        form_dom_string_property_value(scope, value, "HTMLButtonElement", property, false)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value);
}

fn button_boolean_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    property: &'static str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = button_getter_receiver(scope, receiver, property) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        attribute,
    ));
}

fn set_button_boolean_attribute_on_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    property: &'static str,
) {
    let Some((runtime_ptr, handle)) = button_setter_receiver(scope, receiver, property) else {
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

fn fieldset_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_getter_receiver(scope, receiver, "HTMLFieldSetElement", member, "fieldset")
}

fn fieldset_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_setter_receiver(scope, receiver, "HTMLFieldSetElement", member, "fieldset")
}

fn meter_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_getter_receiver(scope, receiver, "HTMLMeterElement", member, "meter")
}

fn progress_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_getter_receiver(scope, receiver, "HTMLProgressElement", member, "progress")
}

pub(in crate::native_bridge) fn datalist_options_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let collection = collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        handle,
        CollectionKind::HtmlCollection,
        LiveCollectionQueryKind::TagName,
        Some("option".to_owned()),
        false,
    );
    rv.set(collection.into());
}

pub(in crate::native_bridge) fn legend_form_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(fieldset) = detached_legend_fieldset_object(scope, args.this()) {
        match detached_form_owner_object(scope, fieldset) {
            Some(form) => rv.set(form.into()),
            None => rv.set_null(),
        }
        return;
    }
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        set_wrapped_node_or_null(scope, &mut rv, std::ptr::null_mut(), None);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let form = legend_fieldset_ancestor(runtime, handle)
        .and_then(|fieldset| runtime.dom_host().form_control_owner(fieldset));
    set_wrapped_node_or_null(scope, &mut rv, runtime_ptr, form);
}

fn detached_legend_fieldset_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    legend: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if detached_element_local_name(scope, legend).as_deref() != Some("legend") {
        return None;
    }
    let mut current = detached_parent_node_object(scope, legend);
    while let Some(candidate) = current {
        if detached_element_local_name(scope, candidate).as_deref() == Some("fieldset") {
            return Some(candidate);
        }
        current = detached_parent_node_object(scope, candidate);
    }
    None
}

pub(in crate::native_bridge) fn fieldset_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = fieldset_getter_receiver(scope, args.this(), "disabled")
    else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        "disabled",
    ));
}

pub(in crate::native_bridge) fn fieldset_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = fieldset_setter_receiver(scope, args.this(), "disabled")
    else {
        rv.set_undefined();
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        "disabled",
        args.get(0).boolean_value(scope),
    );
    let runtime = unsafe { &*runtime_ptr };
    if runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.has_attribute("disabled"))
        && runtime
            .active_element_handle()
            .is_some_and(|active| node_contains(runtime, handle, active))
    {
        update_focus(scope, runtime_ptr, None);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn fieldset_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if fieldset_getter_receiver(scope, args.this(), "type").is_none() {
        rv.set_empty_string();
        return;
    }
    match v8_string(scope, "fieldset") {
        Some(value) => rv.set(value.into()),
        None => rv.set_empty_string(),
    }
}

pub(in crate::native_bridge) fn output_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match v8_string(scope, "output") {
        Some(value) => rv.set(value.into()),
        None => rv.set_empty_string(),
    }
}

pub(in crate::native_bridge) fn meter_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = meter_values_for_receiver(scope, args.this(), "value")
        .map(|values| values.value)
        .unwrap_or(0.0);
    set_number_return(scope, rv, value);
}

pub(in crate::native_bridge) fn meter_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_finite_double_attribute_on_receiver(
        scope,
        args.this(),
        args.get(0),
        "value",
        "HTMLMeterElement",
        "value",
        "meter",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn meter_min_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = meter_values_for_receiver(scope, args.this(), "min")
        .map(|values| values.min)
        .unwrap_or(0.0);
    set_number_return(scope, rv, value);
}

pub(in crate::native_bridge) fn meter_min_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_finite_double_attribute_on_receiver(
        scope,
        args.this(),
        args.get(0),
        "min",
        "HTMLMeterElement",
        "min",
        "meter",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn meter_max_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = meter_values_for_receiver(scope, args.this(), "max")
        .map(|values| values.max)
        .unwrap_or(1.0);
    set_number_return(scope, rv, value);
}

pub(in crate::native_bridge) fn meter_max_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_finite_double_attribute_on_receiver(
        scope,
        args.this(),
        args.get(0),
        "max",
        "HTMLMeterElement",
        "max",
        "meter",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn meter_low_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = meter_values_for_receiver(scope, args.this(), "low")
        .map(|values| values.low)
        .unwrap_or(0.0);
    set_number_return(scope, rv, value);
}

pub(in crate::native_bridge) fn meter_low_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_finite_double_attribute_on_receiver(
        scope,
        args.this(),
        args.get(0),
        "low",
        "HTMLMeterElement",
        "low",
        "meter",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn meter_high_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = meter_values_for_receiver(scope, args.this(), "high")
        .map(|values| values.high)
        .unwrap_or(1.0);
    set_number_return(scope, rv, value);
}

pub(in crate::native_bridge) fn meter_high_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_finite_double_attribute_on_receiver(
        scope,
        args.this(),
        args.get(0),
        "high",
        "HTMLMeterElement",
        "high",
        "meter",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn meter_optimum_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = meter_values_for_receiver(scope, args.this(), "optimum")
        .map(|values| values.optimum)
        .unwrap_or(0.5);
    set_number_return(scope, rv, value);
}

pub(in crate::native_bridge) fn meter_optimum_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_finite_double_attribute_on_receiver(
        scope,
        args.this(),
        args.get(0),
        "optimum",
        "HTMLMeterElement",
        "optimum",
        "meter",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn progress_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = progress_values_for_receiver(scope, args.this(), "value")
        .map(|values| values.value)
        .unwrap_or(0.0);
    set_number_return(scope, rv, value);
}

pub(in crate::native_bridge) fn progress_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_finite_double_attribute_on_receiver(
        scope,
        args.this(),
        args.get(0),
        "value",
        "HTMLProgressElement",
        "value",
        "progress",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn progress_max_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = progress_values_for_receiver(scope, args.this(), "max")
        .map(|values| values.max)
        .unwrap_or(1.0);
    set_number_return(scope, rv, value);
}

pub(in crate::native_bridge) fn progress_max_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, args.this(), "HTMLProgressElement", "max", "progress")
    else {
        rv.set_undefined();
        return;
    };
    let Some(number) = finite_double_from_value(scope, args.get(0), "HTMLProgressElement", "max")
    else {
        return;
    };
    if number > 0.0 {
        set_numeric_attribute(scope, runtime_ptr, handle, "max", number);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn progress_position_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = progress_values_for_receiver(scope, args.this(), "position")
        .map(|values| values.position)
        .unwrap_or(-1.0);
    set_number_return(scope, rv, value);
}

pub(in crate::native_bridge) fn output_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_empty_string();
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .text_content(handle)
        .unwrap_or_default();
    match v8_string(scope, &value) {
        Some(value) => rv.set(value.into()),
        None => rv.set_empty_string(),
    }
}

pub(in crate::native_bridge) fn output_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) =
        form_dom_string_property_value(scope, args.get(0), "HTMLOutputElement", "value", false)
    else {
        return;
    };
    let default_value = {
        let runtime = unsafe { &*runtime_ptr };
        runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .and_then(Element::output_default_value)
            .map(str::to_owned)
            .unwrap_or_else(|| runtime.dom_host().text_content(handle).unwrap_or_default())
    };
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_output_default_value_state(handle, Some(default_value));
    let _ = set_text_content_in_reaction_scope(scope, runtime_ptr, handle, &value);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn output_default_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_empty_string();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(Element::output_default_value)
        .map(str::to_owned)
        .unwrap_or_else(|| runtime.dom_host().text_content(handle).unwrap_or_default());
    match v8_string(scope, &value) {
        Some(value) => rv.set(value.into()),
        None => rv.set_empty_string(),
    }
}

pub(in crate::native_bridge) fn output_default_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = form_dom_string_property_value(
        scope,
        args.get(0),
        "HTMLOutputElement",
        "defaultValue",
        false,
    ) else {
        return;
    };
    let value_mode = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(Element::output_default_value)
        .is_some();
    if value_mode {
        let _ = unsafe { &mut *runtime_ptr }
            .dom_host_mut()
            .set_output_default_value_state(handle, Some(value));
    } else {
        let _ = set_text_content_in_reaction_scope(scope, runtime_ptr, handle, &value);
    }
    rv.set_undefined();
}

fn node_contains(runtime: &JsContextHost, ancestor: DomHandle, node: DomHandle) -> bool {
    let mut current = Some(node);
    while let Some(handle) = current {
        if handle == ancestor {
            return true;
        }
        current = runtime.dom_host().parent_node(handle);
    }
    false
}

fn legend_fieldset_ancestor(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    let mut current = runtime.dom_host().parent_node(handle);
    while let Some(parent) = current {
        if runtime.dom_host().is_html_element_named(parent, "fieldset") {
            return Some(parent);
        }
        current = runtime.dom_host().parent_node(parent);
    }
    None
}

fn meter_values_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<MeterElementValues> {
    let (runtime_ptr, handle) = meter_getter_receiver(scope, object, member)?;
    Some(meter_values(unsafe { &*runtime_ptr }, handle))
}

fn meter_values(runtime: &JsContextHost, handle: DomHandle) -> MeterElementValues {
    let value = element_attribute(runtime, handle, "value");
    let min = element_attribute(runtime, handle, "min");
    let max = element_attribute(runtime, handle, "max");
    let low = element_attribute(runtime, handle, "low");
    let high = element_attribute(runtime, handle, "high");
    let optimum = element_attribute(runtime, handle, "optimum");
    meter_element_values(
        value.as_deref(),
        min.as_deref(),
        max.as_deref(),
        low.as_deref(),
        high.as_deref(),
        optimum.as_deref(),
    )
}

fn progress_values_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<ProgressElementValues> {
    let (runtime_ptr, handle) = progress_getter_receiver(scope, object, member)?;
    Some(progress_values(unsafe { &*runtime_ptr }, handle))
}

fn progress_values(runtime: &JsContextHost, handle: DomHandle) -> ProgressElementValues {
    let value = element_attribute(runtime, handle, "value");
    let max = element_attribute(runtime, handle, "max");
    progress_element_values(value.as_deref(), max.as_deref())
}

fn set_finite_double_attribute_on_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    name: &str,
    owner: &'static str,
    property: &'static str,
    local_name: &'static str,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, receiver, owner, property, local_name)
    else {
        return;
    };
    let Some(number) = finite_double_from_value(scope, value, owner, property) else {
        return;
    };
    set_numeric_attribute(scope, runtime_ptr, handle, name, number);
}

fn finite_double_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) -> Option<f64> {
    match webidl::convert::<webidl::Double>(scope, value, webidl::Context::member(owner, property))
    {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn set_numeric_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    number: f64,
) {
    let Some(serialized) = v8::Number::new(scope, number)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, name, &serialized);
}

fn set_number_return(
    scope: &mut v8::PinScope<'_, '_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    value: f64,
) {
    rv.set(v8::Number::new(scope, value).into());
}

pub(in crate::native_bridge) fn button_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    button_boolean_attribute_getter(scope, args.this(), "disabled", "disabled", rv);
}

pub(in crate::native_bridge) fn button_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "disabled",
        args.get(0),
        "disabled",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn button_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    button_string_attribute_getter(scope, args.this(), "value", "value", rv);
}

pub(in crate::native_bridge) fn button_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_dom_string_attribute_on_receiver(scope, args.this(), "value", args.get(0), "value");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn button_form_action_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = button_getter_receiver(scope, args.this(), "formAction")
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = element_attribute(runtime, handle, "formaction")
        .filter(|value| !value.is_empty())
        .map(|_| resolve_url_like_attribute(runtime, handle, "formaction"))
        .unwrap_or_else(|| runtime.host_document().url().to_string());
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_null();
    }
}

pub(in crate::native_bridge) fn button_form_action_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "formaction",
        args.get(0),
        "formAction",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn button_form_enctype_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = button_getter_receiver(scope, args.this(), "formEnctype")
    else {
        rv.set_empty_string();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "formenctype")
        .map(|value| normalized_form_enctype(&value))
        .unwrap_or("");
    if let Some(value) = v8_string(scope, value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn button_form_enctype_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "formenctype",
        args.get(0),
        "formEnctype",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn button_form_method_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = button_getter_receiver(scope, args.this(), "formMethod")
    else {
        rv.set_empty_string();
        return;
    };
    let method = element_attribute(unsafe { &*runtime_ptr }, handle, "formmethod")
        .map(|value| normalized_form_method(&value))
        .unwrap_or("");
    if let Some(value) = v8_string(scope, method) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn button_form_method_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "formmethod",
        args.get(0),
        "formMethod",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn button_form_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = button_getter_receiver(scope, args.this(), "formTarget")
    else {
        rv.set_empty_string();
        return;
    };
    let value =
        element_attribute(unsafe { &*runtime_ptr }, handle, "formtarget").unwrap_or_default();
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn button_form_target_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "formtarget",
        args.get(0),
        "formTarget",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn button_form_no_validate_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = button_getter_receiver(scope, args.this(), "formNoValidate")
    else {
        rv.set_undefined();
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        "formnovalidate",
    ));
}

pub(in crate::native_bridge) fn button_form_no_validate_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "formnovalidate",
        args.get(0),
        "formNoValidate",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn button_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = button_getter_receiver(scope, args.this(), "type") else {
        rv.set_empty_string();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        rv.set_empty_string();
        return;
    };
    let value = element
        .button_type_state()
        .reflected_keyword(runtime.dom_host().button_is_submit_button(handle));
    if let Some(value) = v8_string(scope, value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn button_type_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = button_setter_receiver(scope, args.this(), "type") else {
        rv.set_undefined();
        return;
    };
    let Some(value) =
        form_dom_string_property_value(scope, args.get(0), "HTMLButtonElement", "type", false)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "type", &value);
    rv.set_undefined();
}

struct ElementReflectionValue<'s> {
    object: v8::Local<'s, v8::Object>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
}

impl<'s> webidl::WebIdlConverter<'s> for ElementReflectionValue<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        _context: webidl::Context,
        _options: &Self::Options,
    ) -> std::result::Result<Self, webidl::WebIdlError> {
        let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| {
            webidl::WebIdlError::custom_message("Element reflection values must be Elements.")
        })?;
        let (runtime_ptr, handle) = node_runtime_and_handle_from_object_or_detached(scope, object)
            .map_err(|_| {
                webidl::WebIdlError::custom_message("Element reflection values must be Elements.")
            })?;
        if !node_is_element(unsafe { &*runtime_ptr }, handle) {
            return Err(webidl::WebIdlError::custom_message(
                "Element reflection values must be Elements.",
            ));
        }
        Ok(Self {
            object,
            runtime_ptr,
            handle,
        })
    }
}

fn element_reflection_handle_for_owner(
    scope: &mut v8::PinScope<'_, '_>,
    owner_runtime_ptr: *mut JsContextHost,
    reference: ElementReflectionValue<'_>,
) -> Option<DomHandle> {
    current_or_live_delegate_node_arg_handle(scope, owner_runtime_ptr, reference.object.into())
        .filter(|handle| node_is_element(unsafe { &*owner_runtime_ptr }, *handle))
        .or_else(|| (reference.runtime_ptr == owner_runtime_ptr).then_some(reference.handle))
}

fn set_button_element_reflection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    property: &'static str,
    attribute: &'static str,
) {
    let Some((runtime_ptr, owner)) = button_setter_receiver(scope, source, property) else {
        return;
    };
    if value.is_null_or_undefined() {
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let _ = unsafe { &mut *runtime_ptr }
                .remove_attribute_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    owner,
                    attribute,
                );
        });
        return;
    }
    let reference = match webidl::convert::<ElementReflectionValue<'_>>(
        scope,
        value,
        webidl::Context::member("HTMLButtonElement", property),
    ) {
        Ok(reference) => reference,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let target = element_reflection_handle_for_owner(scope, runtime_ptr, reference);
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.set_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            owner,
            attribute,
            "",
        );
        let _ = runtime.dom_host_mut().set_explicit_element_references(
            owner,
            attribute,
            target.into_iter().collect(),
        );
    });
}

fn set_wrapped_button_element_or_null<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    runtime_ptr: *mut JsContextHost,
    source: v8::Local<'s, v8::Object>,
    target: Option<DomHandle>,
) {
    let Some(target) = target else {
        rv.set_null();
        return;
    };
    if detached_native_handle_for_runtime(scope, runtime_ptr, source).is_some()
        && let Some(object) = detached_native_object_for_handle(scope, runtime_ptr, target)
    {
        rv.set(object.into());
        return;
    }
    match unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, target)
    {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn button_interest_for_element_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.this();
    let Some((runtime_ptr, source_handle)) =
        button_getter_receiver(scope, source, "interestForElement")
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let target = reflected_element_attribute_handle(runtime, source_handle, "interestfor");
    set_wrapped_button_element_or_null(scope, &mut rv, runtime_ptr, source, target);
}

pub(in crate::native_bridge) fn button_interest_for_element_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_element_reflection(
        scope,
        args.this(),
        args.get(0),
        "interestForElement",
        "interestfor",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn button_popover_target_element_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.this();
    let Some((runtime_ptr, source_handle)) =
        button_getter_receiver(scope, source, "popoverTargetElement")
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let target = reflected_element_attribute_handle(runtime, source_handle, "popovertarget");
    set_wrapped_button_element_or_null(scope, &mut rv, runtime_ptr, source, target);
}

pub(in crate::native_bridge) fn button_popover_target_element_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_element_reflection(
        scope,
        args.this(),
        args.get(0),
        "popoverTargetElement",
        "popovertarget",
    );
    rv.set_undefined();
}

fn canonical_popover_target_action(value: Option<String>) -> &'static str {
    match value.as_deref() {
        Some(value) if value.eq_ignore_ascii_case("show") => "show",
        Some(value) if value.eq_ignore_ascii_case("hide") => "hide",
        _ => "toggle",
    }
}

pub(in crate::native_bridge) fn button_popover_target_action_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let action = canonical_popover_target_action(element_attribute(
        unsafe { &*runtime_ptr },
        handle,
        "popovertargetaction",
    ));
    let Some(action) = v8_string(scope, action) else {
        rv.set_null();
        return;
    };
    rv.set(action.into());
}

pub(in crate::native_bridge) fn button_popover_target_action_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_attribute_property_on_object_or_detached(
        scope,
        args.this(),
        "popovertargetaction",
        args.get(0),
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn button_command_for_element_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.this();
    let Some((runtime_ptr, source_handle)) =
        button_getter_receiver(scope, source, "commandForElement")
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let target = reflected_element_attribute_handle(runtime, source_handle, "commandfor");
    set_wrapped_button_element_or_null(scope, &mut rv, runtime_ptr, source, target);
}

pub(in crate::native_bridge::element) fn normalized_button_command(
    value: Option<String>,
) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if value.starts_with("--") {
        return value;
    }
    for keyword in [
        "toggle-popover",
        "show-popover",
        "hide-popover",
        "close",
        "request-close",
        "show-modal",
    ] {
        if value.eq_ignore_ascii_case(keyword) {
            return keyword.to_owned();
        }
    }
    String::new()
}

pub(in crate::native_bridge) fn button_command_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = button_getter_receiver(scope, args.this(), "command") else {
        rv.set_empty_string();
        return;
    };
    let command = normalized_button_command(element_attribute(
        unsafe { &*runtime_ptr },
        handle,
        "command",
    ));
    let Some(command) = v8_string(scope, &command) else {
        rv.set_empty_string();
        return;
    };
    rv.set(command.into());
}

pub(in crate::native_bridge) fn button_command_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "command",
        args.get(0),
        "command",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn button_command_for_element_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_button_element_reflection(
        scope,
        args.this(),
        args.get(0),
        "commandForElement",
        "commandfor",
    );
    rv.set_undefined();
}
