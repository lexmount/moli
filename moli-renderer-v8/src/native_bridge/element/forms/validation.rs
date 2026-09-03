use v8::RegExpCreationFlags;

use crate::custom_elements::is_form_associated_custom_element_handle;
use crate::webidl;
use moli_dom::forms::{
    FormControlValidity, form_control_type_supports_intrinsic_validation, input_range_overflow,
    input_range_underflow, input_step as form_input_step, input_step_base as form_input_step_base,
    input_type_value_mismatch, normalize_custom_validation_message, number_aligns_to_step,
    number_step_mismatch, parse_input_numeric_value as parse_input_numeric_value_for_type,
    parse_non_negative_integer_prefix,
    text_control_suffers_too_long as form_text_control_suffers_too_long,
    text_control_suffers_too_short as form_text_control_suffers_too_short,
};
use moli_webapi_declare::WebApiObject;

use super::super::{
    element_internals_validation_message_for_target_handle,
    element_internals_validity_for_target_handle, element_internals_will_validate_for_handle,
};
use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLFormControl.setCustomValidity")]
struct SetCustomValidityArgs {
    #[webidl(required)]
    message: String,
}

type ControlValidity = FormControlValidity;

#[derive(WebApiObject)]
#[webapi(interface = "ValidityState", data_properties, enumerable)]
struct ValidityStateDeclaration {
    value_missing: bool,
    type_mismatch: bool,
    pattern_mismatch: bool,
    too_long: bool,
    too_short: bool,
    range_underflow: bool,
    range_overflow: bool,
    step_mismatch: bool,
    bad_input: bool,
    custom_error: bool,
    valid: bool,
}

pub(in crate::native_bridge) fn form_validate_for_submission(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
) -> bool {
    form_check_validity(scope, runtime_ptr, form_handle)
}

pub(in crate::native_bridge) fn form_check_validity_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, form_handle)) =
        node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_bool(true);
        return;
    };
    rv.set_bool(form_check_validity(scope, runtime_ptr, form_handle));
}

pub(in crate::native_bridge) fn form_report_validity_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, form_handle)) =
        node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_bool(true);
        return;
    };
    rv.set_bool(form_check_validity(scope, runtime_ptr, form_handle));
}

fn form_check_validity(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
) -> bool {
    let invalid_controls = {
        let runtime = unsafe { &*runtime_ptr };
        if runtime
            .dom_host()
            .node(form_handle)
            .and_then(Node::as_element)
            .is_none_or(|element| !element.is_html_form())
        {
            return true;
        }
        invalid_form_control_handles(scope, runtime_ptr, form_handle)
    };

    for handle in &invalid_controls {
        dispatch_invalid_event(scope, runtime_ptr, *handle);
    }
    invalid_controls.is_empty()
}

fn invalid_form_control_handles(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
) -> Vec<DomHandle> {
    let controls = {
        let runtime = unsafe { &*runtime_ptr };
        form_control_elements(runtime, form_handle)
    };
    controls
        .into_iter()
        .filter(|handle| !control_satisfies_constraints(scope, runtime_ptr, *handle))
        .collect()
}

pub(in crate::native_bridge) fn control_check_validity_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_bool(true);
        return;
    };
    rv.set_bool(control_check_validity(scope, runtime_ptr, handle));
}

pub(in crate::native_bridge) fn control_report_validity_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_bool(true);
        return;
    };
    rv.set_bool(control_check_validity(scope, runtime_ptr, handle));
}

fn control_check_validity(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    let is_valid = control_satisfies_constraints(scope, runtime_ptr, handle);
    if !is_valid {
        dispatch_invalid_event(scope, runtime_ptr, handle);
    }
    is_valid
}

pub(in crate::native_bridge) fn control_set_custom_validity_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SetCustomValidityArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let message = normalize_custom_validation_message(&parsed.message);
    let runtime = unsafe { &mut *runtime_ptr };
    let old_state = runtime.retained_current_element_state(handle);
    let changed = runtime
        .dom_host_mut()
        .set_custom_validation_message(handle, &message);
    if changed {
        runtime.note_element_state_style_activity_with_old_state(
            handle,
            dom::ElementState::VALIDITY_STATES,
            old_state,
        );
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn control_will_validate_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_bool(false);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    rv.set_bool(control_will_validate(runtime, handle));
}

pub(in crate::native_bridge) fn control_validity_getter_function<'s>(
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
    let validity = control_validity(scope, runtime_ptr, handle);
    let object = build_validity_state_object(scope, validity);
    rv.set(object.into());
}

pub(in crate::native_bridge) fn control_matches_validity_pseudo(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    selector: &str,
) -> Option<bool> {
    let selector = selector.trim();
    let runtime = unsafe { &*runtime_ptr };
    if matches!(selector, ":disabled" | ":enabled")
        && is_form_associated_custom_element_handle(runtime, handle)
    {
        let disabled = form_control_is_effectively_disabled(runtime, handle);
        return Some(if selector == ":disabled" {
            disabled
        } else {
            !disabled
        });
    }
    if !matches!(selector, ":valid" | ":invalid") {
        return None;
    }
    let invalid = control_validity_pseudo_state(scope, runtime_ptr, handle)?;
    Some(if selector == ":valid" {
        !invalid
    } else {
        invalid
    })
}

pub(in crate::native_bridge) fn control_validity_pseudo_state(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<bool> {
    let runtime = unsafe { &*runtime_ptr };
    if runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| matches!(element.local_name(), "form" | "fieldset"))
    {
        let controls = form_control_elements(runtime, handle);
        let valid = controls
            .into_iter()
            .all(|control| control_satisfies_constraints(scope, runtime_ptr, control));
        return Some(!valid);
    }
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    if is_form_associated_custom_element_handle(runtime, handle) {
        if !control_will_validate(runtime, handle) {
            return None;
        }
        return Some(!control_validity(scope, runtime_ptr, handle).valid());
    }
    if !element_matches_validity_pseudo(runtime, handle, element) {
        return None;
    }
    let valid = control_is_readonly_barred_from_constraint_validation(element)
        || control_validity(scope, runtime_ptr, handle).valid();
    Some(!valid)
}

pub(in crate::native_bridge) fn control_validation_message_getter_function<'s>(
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
    let message = control_validation_message(scope, runtime_ptr, handle);
    if let Some(value) = v8_string(scope, &message) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn build_validity_state_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    validity: ControlValidity,
) -> v8::Local<'s, v8::Object> {
    ValidityStateDeclaration::new(
        validity.value_missing,
        validity.type_mismatch,
        validity.pattern_mismatch,
        validity.too_long,
        validity.too_short,
        validity.range_underflow,
        validity.range_overflow,
        validity.step_mismatch,
        validity.bad_input,
        validity.custom_error,
        validity.valid(),
    )
    .bind(scope)
    .expect("ValidityState declaration should bind")
}

fn control_validation_message(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> String {
    let will_validate = {
        let runtime = unsafe { &*runtime_ptr };
        control_will_validate(runtime, handle)
    };
    if !will_validate {
        return String::new();
    }
    let is_form_associated_custom_element = {
        let runtime = unsafe { &*runtime_ptr };
        is_form_associated_custom_element_handle(runtime, handle)
    };
    if is_form_associated_custom_element {
        return element_internals_validation_message_for_target_handle(scope, runtime_ptr, handle);
    }
    let runtime = unsafe { &*runtime_ptr };
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return String::new();
    };
    let validity = control_validity(scope, runtime_ptr, handle);
    validity.validation_message(element.custom_validation_message())
}

fn control_satisfies_constraints(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    let will_validate = {
        let runtime = unsafe { &*runtime_ptr };
        control_will_validate(runtime, handle)
    };
    !will_validate || control_validity(scope, runtime_ptr, handle).valid()
}

fn control_validity(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> ControlValidity {
    let is_form_associated_custom_element = {
        let runtime = unsafe { &*runtime_ptr };
        is_form_associated_custom_element_handle(runtime, handle)
    };
    if is_form_associated_custom_element {
        return element_internals_validity_for_target_handle(scope, runtime_ptr, handle);
    }
    let runtime = unsafe { &*runtime_ptr };
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return ControlValidity::default();
    };
    ControlValidity {
        value_missing: control_suffers_required_value_missing(runtime, handle, element),
        type_mismatch: control_suffers_type_mismatch(runtime, handle, element),
        pattern_mismatch: control_suffers_pattern_mismatch(scope, runtime, handle, element),
        too_long: control_suffers_too_long(runtime, handle, element),
        too_short: control_suffers_too_short(runtime, handle, element),
        range_underflow: control_suffers_range_underflow(runtime, handle, element),
        range_overflow: control_suffers_range_overflow(runtime, handle, element),
        step_mismatch: control_suffers_step_mismatch(runtime, handle, element),
        bad_input: control_suffers_bad_input(runtime, handle, element),
        custom_error: !element.custom_validation_message().is_empty(),
    }
}

fn control_suffers_bad_input(
    _runtime: &JsContextHost,
    _handle: DomHandle,
    element: &Element,
) -> bool {
    element.is_html_input()
        && element.input_type() == InputType::Number
        && element_type_supports_intrinsic_validation(element)
        && element.input_bad_input()
}

fn control_will_validate(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    if is_form_associated_custom_element_handle(runtime, handle) {
        return element_internals_will_validate_for_handle(runtime, handle);
    }
    element_is_constraint_validation_candidate(runtime, handle, element)
}

fn control_suffers_required_value_missing(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    if !element_type_supports_intrinsic_validation(element) {
        return false;
    }

    if element.is_html_input() && element.input_type() == InputType::Radio {
        return radio_group_suffers_required_value_missing(runtime, handle);
    }

    if !element.has_attribute("required")
        || required_value_missing_is_suppressed_for_immutable_control(runtime, handle, element)
    {
        return false;
    }

    match element.local_name() {
        "input" => input_suffers_required_value_missing(runtime, handle, element),
        "select" => select_suffers_required_value_missing(runtime, handle),
        "textarea" => text_control_value(runtime, handle).is_empty(),
        _ => false,
    }
}

fn element_is_constraint_validation_candidate(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    if form_control_is_effectively_disabled(runtime, handle) {
        return false;
    }
    if control_has_datalist_ancestor(runtime, handle) {
        return false;
    }
    if control_is_readonly_barred_from_constraint_validation(element) {
        return false;
    }
    element_type_supports_intrinsic_validation_with_tree(runtime, handle, element)
}

fn element_matches_validity_pseudo(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    if form_control_is_effectively_disabled(runtime, handle) {
        return false;
    }
    if control_has_datalist_ancestor(runtime, handle) {
        return false;
    }
    element_type_supports_intrinsic_validation_with_tree(runtime, handle, element)
}

fn element_type_supports_intrinsic_validation(element: &Element) -> bool {
    let input_type = element.is_html_input().then(|| element.input_type());
    form_control_type_supports_intrinsic_validation(element.local_name(), input_type, false)
}

fn element_type_supports_intrinsic_validation_with_tree(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    let input_type = element.is_html_input().then(|| element.input_type());
    form_control_type_supports_intrinsic_validation(
        element.local_name(),
        input_type,
        runtime.dom_host().button_is_submit_button(handle),
    )
}

fn control_is_readonly_barred_from_constraint_validation(element: &Element) -> bool {
    if !element.has_attribute("readonly") {
        return false;
    }
    element.is_html_textarea() || element.is_html_input()
}

fn required_value_missing_is_suppressed_for_immutable_control(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    let immutable =
        element.has_attribute("readonly") || form_control_is_effectively_disabled(runtime, handle);
    immutable
        && (element.is_html_textarea()
            || (element.is_html_input() && element.input_type().supports_readonly()))
}

fn control_has_datalist_ancestor(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let mut current = runtime.dom_host().parent_node(handle);
    while let Some(parent) = current {
        let Some(parent_element) = runtime.dom_host().node(parent).and_then(Node::as_element)
        else {
            current = runtime.dom_host().parent_node(parent);
            continue;
        };
        if parent_element.is_html_element("datalist") {
            return true;
        }
        current = runtime.dom_host().parent_node(parent);
    }
    false
}

fn input_suffers_required_value_missing(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    match element.input_type() {
        InputType::Checkbox => !element.checked(),
        InputType::Radio => {
            !element.attribute("name").unwrap_or_default().is_empty()
                && !radio_group_has_checked_control(runtime, handle)
        }
        _ => element.input_value().is_empty(),
    }
}

fn control_suffers_type_mismatch(
    _runtime: &JsContextHost,
    _handle: DomHandle,
    element: &Element,
) -> bool {
    if !element.is_html_input() || !element_type_supports_intrinsic_validation(element) {
        return false;
    }
    let value = element.input_value();
    if value.is_empty() {
        return false;
    }
    input_type_value_mismatch(
        element.input_type(),
        &value,
        element.has_attribute("multiple"),
    )
}

fn control_suffers_pattern_mismatch(
    scope: &mut v8::PinScope<'_, '_>,
    _runtime: &JsContextHost,
    _handle: DomHandle,
    element: &Element,
) -> bool {
    if !element.is_html_input()
        || !element_type_supports_intrinsic_validation(element)
        || !element.input_type().supports_pattern()
    {
        return false;
    }
    let Some(pattern) = element.attribute("pattern") else {
        return false;
    };
    let value = element.input_value();
    if value.is_empty() {
        return false;
    }
    if element.input_type() == InputType::Email && element.has_attribute("multiple") {
        return v8_pattern_is_usable(scope, pattern).is_some_and(|()| {
            value
                .split(',')
                .map(str::trim)
                .any(|part| !v8_pattern_matches_entire_value(scope, pattern, part).unwrap_or(true))
        });
    }
    v8_pattern_is_usable(scope, pattern).is_some_and(|()| {
        v8_pattern_matches_entire_value(scope, pattern, &value).is_some_and(|matched| !matched)
    })
}

fn v8_pattern_is_usable(scope: &mut v8::PinScope<'_, '_>, pattern: &str) -> Option<()> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let pattern = v8_string(&scope, pattern)?;
    if v8::RegExp::new(&scope, pattern, RegExpCreationFlags::UNICODE_SETS).is_none() {
        scope.reset();
        return None;
    }
    Some(())
}

fn v8_pattern_matches_entire_value(
    scope: &mut v8::PinScope<'_, '_>,
    pattern: &str,
    value: &str,
) -> Option<bool> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();

    // HTML pattern validation is implicitly anchored; run it through V8 so
    // backreferences, lookaheads, and other ECMAScript regexp syntax match the
    // JavaScript engine instead of a Rust-side approximation.
    let anchored_pattern = format!("^(?:{pattern})$");
    let pattern = v8_string(&scope, &anchored_pattern)?;
    let subject = v8_string(&scope, value)?;
    let Some(regex) = v8::RegExp::new(&scope, pattern, RegExpCreationFlags::UNICODE_SETS) else {
        // Invalid HTML pattern values are ignored by constraint validation, so
        // swallow V8's SyntaxError instead of leaking it through input.validity.
        scope.reset();
        return None;
    };

    let Some(result) = regex.exec(&scope, subject) else {
        // Exec should only be empty for exceptional cases; constraint
        // validation treats those like an unusable pattern and continues.
        scope.reset();
        return None;
    };
    Some(!result.is_null())
}

fn control_suffers_too_long(runtime: &JsContextHost, handle: DomHandle, element: &Element) -> bool {
    if !element.input_value_user_edited()
        || !text_control_length_validation_applies(runtime, handle, element)
    {
        return false;
    }
    form_text_control_suffers_too_long(
        &text_control_value(runtime, handle),
        element.attribute("maxlength"),
    )
}

fn control_suffers_too_short(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    if !element.input_value_user_edited()
        || !text_control_length_validation_applies(runtime, handle, element)
    {
        return false;
    }
    form_text_control_suffers_too_short(
        &text_control_value(runtime, handle),
        element.attribute("minlength"),
    )
}

fn text_control_length_validation_applies(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    element_is_constraint_validation_candidate(runtime, handle, element)
        && (element.is_html_textarea()
            || (element.is_html_input() && element.input_type().supports_text_length_validation()))
}

fn control_suffers_range_underflow(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    let Some(value) = input_value_for_numeric_validity(runtime, handle, element) else {
        return false;
    };
    input_range_underflow(
        element.input_type(),
        value,
        element.attribute("min"),
        element.attribute("max"),
    )
}

fn control_suffers_range_overflow(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    let Some(value) = input_value_for_numeric_validity(runtime, handle, element) else {
        return false;
    };
    input_range_overflow(
        element.input_type(),
        value,
        element.attribute("min"),
        element.attribute("max"),
    )
}

fn control_suffers_step_mismatch(
    runtime: &JsContextHost,
    handle: DomHandle,
    element: &Element,
) -> bool {
    let Some(value) = input_value_for_numeric_validity(runtime, handle, element) else {
        return false;
    };
    if let Some(mismatch) = control_suffers_number_step_mismatch(element) {
        return mismatch;
    }
    let Some(step) = input_step(element) else {
        return false;
    };
    let base = input_step_base(element);
    !number_aligns_to_step(value, base, step)
}

fn control_suffers_number_step_mismatch(element: &Element) -> Option<bool> {
    if !element.is_html_input() || element.input_type() != InputType::Number {
        return None;
    }
    number_step_mismatch(
        &element.input_value(),
        element.attribute("step"),
        element.attribute("min"),
        element.attribute("value"),
    )
}

fn input_value_for_numeric_validity(
    _runtime: &JsContextHost,
    _handle: DomHandle,
    element: &Element,
) -> Option<f64> {
    if !element.is_html_input() || !element_type_supports_intrinsic_validation(element) {
        return None;
    }
    parse_input_numeric_value(element, &element.input_value())
}

pub(in crate::native_bridge::element::forms) fn parse_input_numeric_value(
    element: &Element,
    value: &str,
) -> Option<f64> {
    parse_input_numeric_value_for_type(element.input_type(), value)
}

pub(in crate::native_bridge::element::forms) fn input_step(element: &Element) -> Option<f64> {
    form_input_step(element.input_type(), element.attribute("step"))
}

pub(in crate::native_bridge::element::forms) fn input_step_base(element: &Element) -> f64 {
    form_input_step_base(
        element.input_type(),
        element.attribute("min"),
        element.attribute("value"),
    )
}

fn radio_group_has_checked_control(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .radio_group_members(handle)
        .into_iter()
        .any(|candidate| {
            runtime
                .dom_host()
                .node(candidate)
                .and_then(Node::as_element)
                .is_some_and(Element::checked)
        })
}

fn radio_group_suffers_required_value_missing(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let members = runtime.dom_host().radio_group_members(handle);
    let has_required = members.iter().any(|candidate| {
        runtime
            .dom_host()
            .node(*candidate)
            .and_then(Node::as_element)
            .is_some_and(|element| element.has_attribute("required"))
    });
    has_required
        && !members.into_iter().any(|candidate| {
            runtime
                .dom_host()
                .node(candidate)
                .and_then(Node::as_element)
                .is_some_and(Element::checked)
        })
}

fn select_suffers_required_value_missing(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let selected_options = runtime.dom_host().select_selected_option_elements(handle);
    if selected_options.is_empty() {
        return true;
    }

    let placeholder = select_placeholder_label_option(runtime, handle);
    selected_options.len() == 1 && Some(selected_options[0]) == placeholder
}

fn select_placeholder_label_option(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let select = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    if select.has_attribute("multiple") || select_display_size_for_validation(select) != 1 {
        return None;
    }
    let first = runtime
        .dom_host()
        .select_option_elements(handle)
        .first()
        .copied()?;
    if runtime.dom_host().parent_node(first) != Some(handle) {
        return None;
    }
    let element = runtime.dom_host().node(first).and_then(Node::as_element)?;
    if element
        .option_value(runtime.dom_host().dom(), first)
        .is_empty()
    {
        Some(first)
    } else {
        None
    }
}

fn select_display_size_for_validation(select: &Element) -> i32 {
    select
        .attribute("size")
        .map(parse_non_negative_integer_prefix)
        .unwrap_or(0)
        .max(1)
}

pub(in crate::native_bridge) fn dispatch_invalid_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    if let Some(event) = construct_simple_event(scope, "invalid", false, true, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
}
