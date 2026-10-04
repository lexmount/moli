//! Form controls retain DOM order in their JSON schema.

use moli_dom::{
    forms::{InputType, input_step, number_step_mismatch},
    native::{DomHost, Element, NativeNodeId, Node},
};
use serde::Serialize;
use serde_json::value::{RawValue, to_raw_value};

/// Eligible controls grouped by trimmed parameter names in DOM order.
///
/// The browser discovers form ownership and omits disabled and readonly controls.
pub type ParameterControls = indexmap::IndexMap<String, Vec<NativeNodeId>>;

#[derive(Serialize)]
struct OrderedObject<T>(indexmap::IndexMap<String, T>);

impl OrderedObject<Box<RawValue>> {
    fn put(&mut self, key: &str, value: impl Serialize) {
        self.0.insert(
            key.into(),
            to_raw_value(&value).expect("native schema value"),
        );
    }
}

#[derive(Serialize)]
struct FormSchema {
    r#type: &'static str,
    properties: OrderedObject<OrderedObject<Box<RawValue>>>,
    required: Vec<String>,
}

#[derive(Serialize)]
struct Alternative<'a> {
    r#type: &'static str,
    r#const: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

/// Describe eligible form controls in parameter and DOM order.
///
/// Label ownership and HTML pattern validation remain browser responsibilities.
pub fn input_schema(
    dom: &DomHost,
    parameters: ParameterControls,
    label_handles: impl Fn(NativeNodeId) -> Vec<NativeNodeId>,
    mut pattern_is_usable: impl FnMut(&str) -> bool,
) -> String {
    let mut properties = indexmap::IndexMap::new();
    let mut required = Vec::new();
    for (name, controls) in parameters {
        if let Some(property) =
            parameter_schema(dom, &controls, &label_handles, &mut pattern_is_usable)
        {
            if controls.iter().any(|control| {
                dom.node(*control)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.has_attribute("required"))
            }) {
                required.push(name.clone());
            }
            properties.insert(name, property);
        }
    }
    serde_json::to_string(&FormSchema {
        r#type: "object",
        properties: OrderedObject(properties),
        required,
    })
    .expect("native WebMCP form schema")
}

// Check only the control shape. Executing a tool does not need to serialize
// a schema or traverse labels to establish which parameters it accepts.
pub(crate) fn parameter_element<'a>(
    dom: &'a DomHost,
    controls: &[NativeNodeId],
) -> Option<&'a Element> {
    let element = dom.node(*controls.first()?)?.as_element()?;
    if element.is_html_element("select") || element.is_html_textarea() {
        return (controls.len() == 1).then_some(element);
    }
    if !element.is_html_input() {
        return None;
    }
    let kind = element.input_type();
    if controls.iter().any(|handle| {
        dom.node(*handle)
            .and_then(Node::as_element)
            .is_none_or(|input| !input.is_html_input() || input.input_type() != kind)
    }) {
        return None;
    }
    let supported = match kind {
        InputType::Checkbox | InputType::Radio => true,
        InputType::Hidden => {
            controls.len() == 1
                && element
                    .attribute("toolparamdescription")
                    .is_some_and(|value| !value.is_empty())
        }
        InputType::Number
        | InputType::Range
        | InputType::Text
        | InputType::Search
        | InputType::Password
        | InputType::Tel
        | InputType::Url
        | InputType::Email
        | InputType::Color
        | InputType::Date
        | InputType::DatetimeLocal
        | InputType::Month
        | InputType::Week
        | InputType::Time => controls.len() == 1,
        _ => false,
    };
    supported.then_some(element)
}

pub(crate) fn option_values(dom: &DomHost, options: &[NativeNodeId]) -> Vec<String> {
    options
        .iter()
        .map(|option| dom.option_value(*option).expect("select option"))
        .collect()
}

pub(crate) fn checkable_values(dom: &DomHost, controls: &[NativeNodeId]) -> Vec<String> {
    controls
        .iter()
        .map(|control| {
            dom.node(*control)
                .and_then(Node::as_element)
                .expect("input control")
                .attribute("value")
                .unwrap_or("on")
                .to_owned()
        })
        .collect()
}

fn parameter_schema(
    dom: &DomHost,
    controls: &[NativeNodeId],
    label_handles: &impl Fn(NativeNodeId) -> Vec<NativeNodeId>,
    pattern_is_usable: &mut impl FnMut(&str) -> bool,
) -> Option<OrderedObject<Box<RawValue>>> {
    let element = parameter_element(dom, controls)?;
    let mut fields = OrderedObject(indexmap::IndexMap::new());
    if element.is_html_element("select") {
        let options = dom.select_option_elements(controls[0]);
        let values = option_values(dom, &options);
        let alternatives = options
            .iter()
            .zip(&values)
            .map(|(option, value)| alternative(value, Some(text_content(dom, *option))))
            .collect::<Vec<_>>();
        fields = choices_schema(alternatives, &values, element.has_attribute("multiple"));
    } else if element.is_html_textarea() {
        fields.put("type", "string");
    } else if element.is_html_input() {
        let kind = element.input_type();
        match kind {
            InputType::Checkbox if controls.len() == 1 => fields.put("type", "boolean"),
            InputType::Checkbox | InputType::Radio => {
                let values = checkable_values(dom, controls);
                let alternatives = values
                    .iter()
                    .zip(controls)
                    .map(|(value, control)| {
                        let title = label_text(dom, *control, label_handles);
                        alternative(value, (!title.is_empty()).then_some(title))
                    })
                    .collect::<Vec<_>>();
                fields = choices_schema(alternatives, &values, kind == InputType::Checkbox);
            }
            InputType::Number | InputType::Range => {
                fields.put("type", "number");
                let minimum = number_attribute(element, "min")
                    .or_else(|| (kind == InputType::Range).then_some(0.0));
                let maximum = number_attribute(element, "max")
                    .or_else(|| (kind == InputType::Range).then_some(100.0))
                    .map(|maximum| {
                        if kind == InputType::Range {
                            maximum.max(minimum.unwrap_or(0.0))
                        } else {
                            maximum
                        }
                    });
                if let Some(value) = minimum {
                    fields.put("minimum", value);
                }
                if let Some(value) = maximum {
                    fields.put("maximum", value);
                }
                if element
                    .attribute("step")
                    .is_none_or(|step| !step.eq_ignore_ascii_case("any"))
                {
                    let step = number_attribute(element, "step")
                        .filter(|step| *step > 0.0)
                        .unwrap_or(1.0);
                    let attribute = |name| {
                        element
                            .attribute(name)
                            .filter(|_| number_attribute(element, name).is_some())
                    };
                    if number_step_mismatch(
                        "0",
                        attribute("step"),
                        attribute("min"),
                        attribute("value"),
                    ) == Some(false)
                    {
                        fields.put("multipleOf", step);
                    }
                }
            }
            InputType::Text
            | InputType::Search
            | InputType::Password
            | InputType::Tel
            | InputType::Url
            | InputType::Email
            | InputType::Hidden => fields.put("type", "string"),
            InputType::Color => {
                fields.put("type", "string");
                fields.put("format", "^#[0-9a-zA-Z]{6}$");
            }
            InputType::Date
            | InputType::DatetimeLocal
            | InputType::Month
            | InputType::Week
            | InputType::Time => {
                fields.put("type", "string");
                fields.put("format", temporal_format(element));
            }
            _ => return None,
        }
        if matches!(
            kind,
            InputType::Text
                | InputType::Search
                | InputType::Password
                | InputType::Tel
                | InputType::Url
                | InputType::Email
                | InputType::Hidden
                | InputType::Number
        ) && let Some(pattern) = element.attribute("pattern")
            && pattern_is_usable(pattern)
        {
            fields.put("pattern", pattern);
        }
    } else {
        return None;
    }
    let mut description = description(dom, controls, label_handles);
    if element.is_html_input() && element.input_type() == InputType::Date {
        const DATE_CONTEXT: &str = "Dates MUST be provided in 'YYYY-MM-DD' format.";
        description = if description.is_empty() {
            DATE_CONTEXT.into()
        } else {
            format!("{description} ({DATE_CONTEXT})")
        };
    }
    if !description.is_empty() {
        fields.put("description", description);
    }
    Some(fields)
}

fn number_attribute(element: &Element, name: &str) -> Option<f64> {
    let value = element.attribute(name)?;
    if !matches!(*value.as_bytes().first()?, b'-' | b'.' | b'0'..=b'9') {
        return None;
    }
    value.parse::<f64>().ok().filter(|value| value.is_finite())
}

fn text_content(dom: &DomHost, handle: NativeNodeId) -> String {
    dom.text_content(handle).unwrap_or_default()
}

fn alternative(value: &str, title: Option<String>) -> Alternative<'_> {
    Alternative {
        r#type: "string",
        r#const: value,
        title,
    }
}

fn choices_schema(
    alternatives: Vec<Alternative<'_>>,
    values: &[String],
    multiple: bool,
) -> OrderedObject<Box<RawValue>> {
    let mut choices = OrderedObject(indexmap::IndexMap::new());
    choices.put("type", "string");
    choices.put("anyOf", alternatives);
    choices.put("enum", values);
    if multiple {
        let mut fields = OrderedObject(indexmap::IndexMap::new());
        fields.put("type", "array");
        fields.put("items", choices);
        fields.put("uniqueItems", true);
        fields
    } else {
        choices
    }
}

fn temporal_format(element: &Element) -> String {
    match element.input_type() {
        InputType::Date => "date".into(),
        InputType::Month => "^[0-9]{4}-(0[1-9]|1[0-2])$".into(),
        InputType::Week => "^[0-9]{4}-W(0[1-9]|[1-4][0-9]|5[0-3])$".into(),
        _ => {
            let step = input_step(
                element.input_type(),
                element
                    .attribute("step")
                    .filter(|_| number_attribute(element, "step").is_some()),
            )
            .unwrap_or(60000.0);
            let prefix = if element.input_type() == InputType::DatetimeLocal {
                "[0-9]{4}-(0[1-9]|1[0-2])-[0-9]{2}T"
            } else {
                ""
            };
            let seconds = if step < 1000.0 {
                r"(:[0-5][0-9](\.[0-9]{1,3})?)?"
            } else if step < 60000.0 {
                "(:[0-5][0-9])?"
            } else {
                ""
            };
            format!("^{prefix}([01][0-9]|2[0-3]):[0-5][0-9]{seconds}$")
        }
    }
}

fn label_text(
    dom: &DomHost,
    control: NativeNodeId,
    label_handles: &impl Fn(NativeNodeId) -> Vec<NativeNodeId>,
) -> String {
    label_handles(control)
        .into_iter()
        .map(|label| {
            let mut stack = vec![label];
            let mut text = String::new();
            while let Some(handle) = stack.pop() {
                let node = dom.node(handle).expect("label node");
                if node.as_element().is_some_and(|element| {
                    matches!(
                        element.local_name(),
                        "button"
                            | "input"
                            | "meter"
                            | "output"
                            | "progress"
                            | "select"
                            | "textarea"
                    )
                }) {
                    continue;
                }
                if let Some(value) = node.as_text() {
                    text.push_str(value.data());
                }
                stack.extend(dom.child_handles_reversed(handle));
            }
            text.trim().to_owned()
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn description(
    dom: &DomHost,
    controls: &[NativeNodeId],
    label_handles: &impl Fn(NativeNodeId) -> Vec<NativeNodeId>,
) -> String {
    let element = dom
        .node(controls[0])
        .and_then(Node::as_element)
        .expect("parameter element");
    if controls.len() == 1 {
        if let Some(value) = element
            .attribute("toolparamdescription")
            .filter(|value| !value.is_empty())
        {
            return value.into();
        }
        let text = label_text(dom, controls[0], label_handles);
        return if !text.is_empty() {
            text
        } else {
            element
                .attribute("aria-description")
                .unwrap_or_default()
                .into()
        };
    }
    let mut parent = dom.parent_node(controls[0]);
    while let Some(handle) = parent {
        if let Some(fieldset) = dom
            .node(handle)
            .and_then(Node::as_element)
            .filter(|element| element.is_html_fieldset())
            && controls
                .iter()
                .all(|control| dom.contains(handle, *control))
        {
            return fieldset
                .attribute("toolparamdescription")
                .unwrap_or_default()
                .into();
        }
        parent = dom.parent_node(handle);
    }
    String::new()
}
