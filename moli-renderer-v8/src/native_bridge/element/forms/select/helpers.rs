use super::*;

pub(super) fn element_option_value(runtime: &JsContextHost, handle: DomHandle) -> Option<String> {
    runtime.dom_host().dom().option_value(handle)
}

pub(super) fn select_option_handles(runtime: &JsContextHost, handle: DomHandle) -> Vec<DomHandle> {
    runtime.dom_host().select_option_elements(handle)
}

pub(super) fn select_is_multiple(runtime: &JsContextHost, handle: DomHandle) -> bool {
    element_has_attribute(runtime, handle, "multiple")
}

pub(super) fn effective_option_selected(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    if !element.is_html_option() {
        return false;
    }

    let mut current = runtime.dom_host().parent_node(handle);
    while let Some(parent) = current {
        let Some(parent_element) = runtime.dom_host().node(parent).and_then(Node::as_element)
        else {
            current = runtime.dom_host().parent_node(parent);
            continue;
        };
        if parent_element.is_html_select() {
            return runtime
                .dom_host()
                .select_selected_option_elements(parent)
                .contains(&handle);
        }
        current = runtime.dom_host().parent_node(parent);
    }
    element.selected()
}

pub(super) fn selected_index_for_select(runtime: &JsContextHost, handle: DomHandle) -> i32 {
    for (index, option) in select_option_handles(runtime, handle)
        .into_iter()
        .enumerate()
    {
        if effective_option_selected(runtime, option) {
            return index as i32;
        }
    }
    -1
}
