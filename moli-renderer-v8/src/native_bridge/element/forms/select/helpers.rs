use super::*;

pub(super) fn element_option_value(runtime: &JsContextHost, handle: DomHandle) -> Option<String> {
    let dom = runtime.dom_host().dom();
    let element = dom.node(handle).and_then(Node::as_element)?;
    if !element.is_html_option() {
        return None;
    }
    Some(element.option_value(dom, handle))
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

    if let Some(select) = runtime.dom_host().option_nearest_ancestor_select(handle) {
        return runtime
            .dom_host()
            .select_selected_option_elements(select)
            .contains(&handle);
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
