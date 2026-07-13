use crate::document_runtime::DomHandle;
use crate::dom::native::{Element, Node};

use super::super::JsContextHost;

fn with_element<T>(
    runtime: &JsContextHost,
    handle: DomHandle,
    map: impl FnOnce(&Element) -> T,
) -> Option<T> {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(map)
}

pub(in crate::native_bridge) fn element_attribute(
    runtime: &JsContextHost,
    handle: DomHandle,
    name: &str,
) -> Option<String> {
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    let normalized_name = element.normalized_attribute_name(name);
    element.attribute(&normalized_name).map(str::to_owned)
}

pub(super) fn element_has_attribute(
    runtime: &JsContextHost,
    handle: DomHandle,
    name: &str,
) -> bool {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    let normalized_name = element.normalized_attribute_name(name);
    element.has_attribute(&normalized_name)
}

pub(super) fn element_attribute_names(runtime: &JsContextHost, handle: DomHandle) -> Vec<String> {
    with_element(runtime, handle, Element::attribute_names).unwrap_or_default()
}

pub(super) fn style_string(runtime: &JsContextHost, handle: DomHandle) -> String {
    element_attribute(runtime, handle, "style").unwrap_or_default()
}
