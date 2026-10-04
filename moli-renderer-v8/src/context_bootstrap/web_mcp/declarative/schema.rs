//! Discover browser-owned form controls for the shared WebMCP DOM algorithms.

use moli_webmcp::ParameterControls;

use crate::{
    document_runtime::DomHandle,
    dom::native::Node,
    native_bridge::{
        JsContextHost,
        element::{
            control_label_handles, form_control_elements, form_control_is_effectively_disabled,
        },
    },
};

pub(super) fn parameter_controls(
    host: &JsContextHost,
    controls: impl IntoIterator<Item = DomHandle>,
) -> ParameterControls {
    let mut groups = ParameterControls::new();
    for control in controls {
        let Some(element) = host.dom_host().node(control).and_then(Node::as_element) else {
            continue;
        };
        let readonly = element.has_attribute("readonly")
            && (element.is_html_textarea()
                || (element.is_html_input() && element.input_type().supports_readonly()));
        if form_control_is_effectively_disabled(host, control) || readonly {
            continue;
        }
        let name = element.attribute("name").unwrap_or_default().trim();
        if !name.is_empty() {
            groups.entry(name.into()).or_default().push(control);
        }
    }
    groups
}

pub(super) fn input_schema(
    host: &JsContextHost,
    form: DomHandle,
    pattern_is_usable: impl FnMut(&str) -> bool,
) -> String {
    moli_webmcp::input_schema(
        host.dom_host(),
        parameter_controls(host, form_control_elements(host, form)),
        |control| control_label_handles(host, control),
        pattern_is_usable,
    )
}
