use super::super::*;
use crate::util::utf16_len;
use crate::webidl;
use moli_dom::forms::{
    InputStepDirection, InputStepError, InputStepOutcome, InputStepState, date_input_milliseconds,
    date_input_value_from_milliseconds, month_input_milliseconds,
    month_input_value_from_milliseconds, step_input_value, time_input_milliseconds,
    time_input_value_from_milliseconds, week_input_milliseconds,
    week_input_value_from_milliseconds,
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLInputElement.stepUp")]
struct InputStepUpArgs {
    #[webidl(default = 1)]
    n: i32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLInputElement.stepDown")]
struct InputStepDownArgs {
    #[webidl(default = 1)]
    n: i32,
}

pub(in crate::native_bridge) fn input_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_type_getter_from_object(scope, args.this(), &mut rv);
}

fn input_type_getter_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_null();
        return;
    };
    let input_type = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::input_type)
        .unwrap_or_default();
    let Some(value) = v8_string(scope, input_type.as_ref()) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn input_type_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    input_type_setter_on_object(scope, args.this(), args.get(0));
    rv.set_undefined();
}

fn input_type_setter_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(value) =
        form_dom_string_property_value(scope, value, "HTMLInputElement", "type", false)
    else {
        return;
    };
    let canonical = InputType::from_attribute_value(Some(&value));
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    let previous_state: Option<(InputType, String)> = {
        let runtime = unsafe { &*runtime_ptr };
        runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .map(|element| (element.input_type(), element.input_value().to_owned()))
    };

    set_reflected_attribute(scope, runtime_ptr, handle, "type", &value);

    if let Some((previous_type, previous_value)) = previous_state {
        let was_selectable = previous_type.supports_variable_length_selection();
        let is_selectable = canonical.supports_variable_length_selection();
        if !was_selectable && is_selectable {
            let _ = unsafe { &mut *runtime_ptr }.set_selection_range(handle, 0, 0);
        } else if was_selectable && is_selectable {
            let current_value = unsafe { &*runtime_ptr }
                .dom_host()
                .node(handle)
                .and_then(Node::as_element)
                .map(Element::input_value)
                .unwrap_or_default();
            if current_value != previous_value {
                reset_input_selection_to_end(unsafe { &mut *runtime_ptr }, handle);
            }
        }
    }

    if canonical == InputType::Radio {
        let checked = unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(Element::checked);
        if checked {
            let runtime = unsafe { &mut *runtime_ptr };
            let _ = runtime.set_checked_state(scope, runtime_ptr, handle, true);
        }
    }
}

pub(in crate::native_bridge) fn input_value_getter_function<'s>(
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
    let runtime = unsafe { &*runtime_ptr };
    let value = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::input_value)
        .unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn input_value_setter_function<'s>(
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
    let Some(next_value) =
        form_dom_string_property_value(scope, args.get(0), "HTMLInputElement", "value", true)
    else {
        return;
    };
    let input_type = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::input_type)
        .unwrap_or_default();
    if input_type.is_checkable() {
        // Checkbox and radio inputs use the HTML default/on value mode. Their
        // IDL setter reflects the content attribute instead of creating a
        // dirty, non-attribute value.
        set_reflected_attribute(scope, runtime_ptr, handle, "value", &next_value);
        rv.set_undefined();
        return;
    }
    if input_type == InputType::File && !next_value.is_empty() {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "File input value may only be set to the empty string.",
        );
        return;
    }
    let previous_value: String = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::input_value)
        .unwrap_or_default();
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_input_value(handle, &next_value);
    let current_value = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::input_value)
        .unwrap_or_default();
    if current_value != previous_value {
        reset_input_selection_to_end(runtime, handle);
    }
    rv.set_undefined();
}

fn reset_input_selection_to_end(runtime: &mut JsContextHost, handle: DomHandle) {
    let Some(end) = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .filter(|element| element.input_type().supports_variable_length_selection())
        .map(|element| utf16_len(&element.input_value()) as u32)
    else {
        return;
    };
    let _ = runtime.set_selection_range(handle, end, end);
}

pub(in crate::native_bridge) fn input_value_as_number_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_value_as_number_getter_from_object(scope, args.this(), &mut rv);
}

fn input_value_as_number_getter_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set(v8::Number::new(scope, f64::NAN).into());
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .filter(|element| element.is_html_input())
        .and_then(input_value_as_number)
        .unwrap_or(f64::NAN);
    rv.set(v8::Number::new(scope, value).into());
}

pub(in crate::native_bridge) fn input_value_as_number_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    input_value_as_number_setter_on_object(scope, args.this(), args.get(0));
    rv.set_undefined();
}

fn input_value_as_number_setter_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    let number = match webidl::convert::<webidl::UnrestrictedDouble>(
        scope,
        value,
        webidl::Context::member("HTMLInputElement", "valueAsNumber"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if number.is_infinite() {
        let Some(message) = v8::String::new(scope, "valueAsNumber must be a finite number.") else {
            return;
        };
        scope.throw_exception(v8::Exception::type_error(scope, message));
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let input_type = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .filter(|element| element.is_html_input())
        .map(Element::input_type);
    let Some(input_type) = input_type.filter(|input_type| input_type.supports_value_as_number())
    else {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "This input type does not support valueAsNumber.",
        );
        return;
    };
    let next_value = if number.is_nan() {
        String::new()
    } else {
        moli_dom::forms::input_number_to_value_string(input_type, number).unwrap_or_default()
    };
    let _ = runtime.set_input_value(handle, &next_value);
}

pub(in crate::native_bridge) fn input_value_as_date_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_value_as_date_getter_from_object(scope, args.this(), &mut rv);
}

fn input_value_as_date_getter_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(element) = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .filter(|element| element.is_html_input())
    else {
        rv.set_null();
        return;
    };
    let Some(millis) =
        input_value_as_date_milliseconds(element.input_type(), &element.input_value())
    else {
        rv.set_null();
        return;
    };
    let Some(date) = v8::Date::new(scope, millis) else {
        rv.set_null();
        return;
    };
    rv.set(date.into());
}

pub(in crate::native_bridge) fn input_value_as_date_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    input_value_as_date_setter_on_object(scope, args.this(), args.get(0));
    rv.set_undefined();
}

fn input_value_as_date_setter_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    let input_type = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .filter(|element| element.is_html_input())
        .map(Element::input_type)
        .unwrap_or_default();
    if !input_type.supports_value_as_date() {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "This input type does not support valueAsDate.",
        );
        return;
    }

    let next_value = if value.is_null() {
        String::new()
    } else {
        let Ok(date) = v8::Local::<v8::Date>::try_from(value) else {
            throw_type_error(
                scope,
                "Failed to set the 'valueAsDate' property on 'HTMLInputElement': The provided value is not a Date.",
            );
            return;
        };
        input_date_value_from_milliseconds(input_type, date.value_of()).unwrap_or_default()
    };
    let _ = unsafe { &mut *runtime_ptr }.set_input_value(handle, &next_value);
}

pub(in crate::native_bridge) fn input_step_up_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<InputStepUpArgs>(scope, &args) else {
        return;
    };
    input_step_by(
        scope,
        args,
        &mut rv,
        InputStepDirection::Up,
        f64::from(parsed.n),
    );
}

pub(in crate::native_bridge) fn input_step_down_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<InputStepDownArgs>(scope, &args) else {
        return;
    };
    input_step_by(
        scope,
        args,
        &mut rv,
        InputStepDirection::Down,
        f64::from(parsed.n),
    );
}

fn input_step_by(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    direction: InputStepDirection,
    n: f64,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };

    let outcome = {
        let runtime = unsafe { &*runtime_ptr };
        let element = runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .filter(|element| element.is_html_input());
        match element {
            Some(element) => {
                let input_type = element.input_type();
                let input_value = element.input_value();
                step_input_value(
                    InputStepState {
                        input_type,
                        value: &input_value,
                        min: element.attribute("min"),
                        max: element.attribute("max"),
                        step: element.attribute("step"),
                        value_attribute: element.attribute("value"),
                    },
                    direction,
                    n,
                )
            }
            None => Err(InputStepError::Unsupported),
        }
    };

    match outcome {
        Ok(InputStepOutcome::Set(value)) => {
            let runtime = unsafe { &mut *runtime_ptr };
            let _ = runtime.set_input_value(handle, &value);
        }
        Ok(InputStepOutcome::NoChange) => {}
        Err(InputStepError::Unsupported) => {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "This input type does not support stepUp() or stepDown().",
            );
            return;
        }
        Err(InputStepError::NoAllowedStep) => {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "This input type has no allowed value step.",
            );
            return;
        }
    }
    rv.set_undefined();
}

fn input_value_as_number(element: &Element) -> Option<f64> {
    moli_dom::forms::parse_input_numeric_value(element.input_type(), &element.input_value())
}

fn input_value_as_date_milliseconds(input_type: InputType, value: &str) -> Option<f64> {
    match input_type {
        InputType::Date => date_input_milliseconds(value),
        InputType::Month => month_input_milliseconds(value),
        InputType::Week => week_input_milliseconds(value),
        InputType::Time => time_input_milliseconds(value),
        _ => None,
    }
}

fn input_date_value_from_milliseconds(input_type: InputType, value: f64) -> Option<String> {
    if !value.is_finite() {
        return Some(String::new());
    }
    match input_type {
        InputType::Date => date_input_value_from_milliseconds(value),
        InputType::Month => month_input_value_from_milliseconds(value),
        InputType::Week => week_input_value_from_milliseconds(value),
        InputType::Time => time_input_value_from_milliseconds(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webidl_long_conversion_handles_special_and_wrapping_values() {
        assert_eq!(webidl_long_from_number(f64::NAN), 0);
        assert_eq!(webidl_long_from_number(f64::INFINITY), 0);
        assert_eq!(webidl_long_from_number(f64::NEG_INFINITY), 0);
        assert_eq!(webidl_long_from_number(2.75), 2);
        assert_eq!(webidl_long_from_number(-2.75), -2);
        assert_eq!(webidl_long_from_number(4_294_967_297.0), 1);
        assert_eq!(webidl_long_from_number(2_147_483_648.0), -2_147_483_648);
    }
}
