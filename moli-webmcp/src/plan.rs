//! Validate all input and prepare native control updates before applying any.

use std::collections::HashSet;
use std::sync::Arc;

use moli_dom::{
    forms::{InputType, sanitize_input_value_for_type_with_multiple},
    native::{DomHost, NativeNodeId},
};
use serde_json::Value;

use crate::{ParameterControls, number, schema};

/// Native updates to apply in order after the complete input has been validated.
pub enum Fill {
    Value(NativeNodeId, String),
    Checked(NativeNodeId, bool),
    Select(NativeNodeId, Vec<String>),
    Checkable {
        handle: NativeNodeId,
        selected: Arc<HashSet<String>>,
        radio: bool,
    },
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(number::stringify(value)),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn boolean(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Number(value) => Some(number::integer(value)? != 0),
        Value::String(value) if value.eq_ignore_ascii_case("true") || value == "1" => Some(true),
        Value::String(value) if value.eq_ignore_ascii_case("false") || value == "0" => Some(false),
        _ => None,
    }
}

fn values(value: &Value) -> Option<Vec<String>> {
    let values = value
        .as_array()?
        .iter()
        .map(scalar)
        .collect::<Option<Vec<_>>>()?;
    let unique = values.iter().collect::<HashSet<_>>();
    (unique.len() == values.len()).then_some(values)
}

/// Validate all arguments without mutating the DOM.
///
/// Unknown parameters and invalid choices reject the entire fill plan.
pub fn prepare_fill(
    dom: &DomHost,
    parameters: ParameterControls,
    input: &str,
) -> Option<Vec<Fill>> {
    let input: indexmap::IndexMap<String, Value> = serde_json::from_str(input).ok()?;
    let mut plan = Vec::new();
    for (name, value) in &input {
        let group = parameters.get(name)?;
        let element = schema::parameter_element(dom, group)?;
        let first = group[0];
        if element.is_html_element("select") {
            let multiple = element.has_attribute("multiple");
            let selected = if multiple {
                values(value)?
            } else {
                vec![scalar(value)?]
            };
            let options = dom.select_option_elements(first);
            let choices = schema::option_values(dom, &options);
            if selected.iter().any(|value| !choices.contains(value)) {
                return None;
            }
            // Earlier fields dispatch author events before this field is filled.
            // Retain values, rather than option nodes that those events can replace.
            plan.push(Fill::Select(first, selected));
        } else if element.is_html_input()
            && element.input_type() == InputType::Checkbox
            && group.len() == 1
        {
            plan.push(Fill::Checked(first, boolean(value)?));
        } else if element.is_html_input()
            && matches!(element.input_type(), InputType::Checkbox | InputType::Radio)
        {
            let selected = if element.input_type() == InputType::Checkbox {
                values(value)?
            } else {
                vec![scalar(value)?]
            };
            let choices = schema::checkable_values(dom, group);
            if selected.iter().any(|value| !choices.contains(value)) {
                return None;
            }
            let selected = Arc::new(selected.into_iter().collect::<HashSet<_>>());
            plan.extend(group.iter().copied().map(|handle| Fill::Checkable {
                handle,
                selected: selected.clone(),
                radio: element.input_type() == InputType::Radio,
            }));
        } else {
            let value = scalar(value)?;
            if element.is_html_input() {
                let numeric = matches!(element.input_type(), InputType::Number | InputType::Range);
                if (numeric && value.is_empty())
                    || (!value.is_empty()
                        && sanitize_input_value_for_type_with_multiple(
                            element.input_type(),
                            &value,
                            element.has_attribute("multiple"),
                        )
                        .is_empty())
                {
                    return None;
                }
            }
            let value = if element.is_html_element("textarea") {
                value.replace("\r\n", "\n").replace('\r', "\n")
            } else {
                value
            };
            plan.push(Fill::Value(first, value));
        }
    }
    Some(plan)
}
