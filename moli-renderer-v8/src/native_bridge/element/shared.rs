use crate::document_runtime::DomHandle;
use crate::dom::native::{Element, Node};

use super::super::PageScriptHost;

fn with_element<T>(
    runtime: &PageScriptHost,
    handle: DomHandle,
    map: impl FnOnce(&Element) -> T,
) -> Option<T> {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(map)
}

pub(super) fn element_attribute(
    runtime: &PageScriptHost,
    handle: DomHandle,
    name: &str,
) -> Option<String> {
    runtime.dom_host().get_attribute(handle, name)
}

pub(super) fn element_has_attribute(
    runtime: &PageScriptHost,
    handle: DomHandle,
    name: &str,
) -> bool {
    runtime.dom_host().dom().has_attribute(handle, name)
}

pub(super) fn element_attribute_names(runtime: &PageScriptHost, handle: DomHandle) -> Vec<String> {
    with_element(runtime, handle, Element::attribute_names).unwrap_or_default()
}

pub(super) fn style_string(runtime: &PageScriptHost, handle: DomHandle) -> String {
    element_attribute(runtime, handle, "style").unwrap_or_default()
}
