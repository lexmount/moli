use crate::{
    document_runtime::DomHandle,
    dom::{forms::InputType, native::Node},
    native_bridge::JsContextHost,
};

#[derive(Clone, Copy)]
pub(in crate::script_vm) struct KeyTargetInfo {
    pub(in crate::script_vm) is_checkbox: bool,
    pub(in crate::script_vm) is_radio: bool,
    pub(in crate::script_vm) is_button_like: bool,
    pub(in crate::script_vm) is_anchor_like: bool,
    pub(in crate::script_vm) is_textarea: bool,
    pub(in crate::script_vm) is_text_control: bool,
    pub(in crate::script_vm) is_select: bool,
}

pub(in crate::script_vm) fn key_target_info(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> KeyTargetInfo {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return KeyTargetInfo {
            is_checkbox: false,
            is_radio: false,
            is_button_like: false,
            is_anchor_like: false,
            is_textarea: false,
            is_text_control: false,
            is_select: false,
        };
    };
    let local_name = element.local_name();
    let input_type = element.input_type();
    let is_checkbox = element.is_html_input() && input_type == InputType::Checkbox;
    let is_radio = element.is_html_input() && input_type == InputType::Radio;
    let is_textarea = element.is_html_textarea();
    let is_select = element.is_html_select();
    let is_button_like = local_name == "button"
        || (element.is_html_input()
            && matches!(
                input_type,
                InputType::Button | InputType::Submit | InputType::Reset
            ));
    let is_anchor_like = local_name == "a"
        && element
            .attribute("href")
            .is_some_and(|href| !href.is_empty());
    let is_text_control = is_textarea
        || (element.is_html_input()
            && !matches!(
                input_type,
                InputType::Checkbox
                    | InputType::Radio
                    | InputType::Button
                    | InputType::Submit
                    | InputType::Reset
            ));
    KeyTargetInfo {
        is_checkbox,
        is_radio,
        is_button_like,
        is_anchor_like,
        is_textarea,
        is_text_control,
        is_select,
    }
}

pub(in crate::script_vm) fn is_space_key(key: &str) -> bool {
    matches!(key, " " | "space" | "spacebar")
}

pub(in crate::script_vm) fn radio_group_members(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Vec<DomHandle> {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return Vec::new();
    };
    let Some(root) = runtime.dom_host().document_element_handle() else {
        return Vec::new();
    };
    let name = element.attribute("name").unwrap_or_default();
    if name.is_empty() {
        return Vec::new();
    }

    runtime
        .dom_host()
        .elements_by_tag_name(root, "input", true)
        .into_iter()
        .filter(|candidate| {
            runtime
                .dom_host()
                .node(*candidate)
                .and_then(Node::as_element)
                .is_some_and(|candidate_element| {
                    candidate_element.is_html_input()
                        && candidate_element.input_type() == InputType::Radio
                        && candidate_element.attribute("name") == Some(name)
                        && !candidate_element.has_attribute("disabled")
                        && runtime
                            .dom_host()
                            .node(*candidate)
                            .is_some_and(Node::is_connected)
                })
        })
        .collect()
}

pub(in crate::script_vm) fn current_selection_range(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> (u32, u32) {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| {
            let start = element.selection_start();
            let end = element.selection_end();
            if start <= end {
                (start, end)
            } else {
                (end, end)
            }
        })
        .unwrap_or((0, 0))
}

pub(in crate::script_vm) fn current_selection_state(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> (u32, u32, String) {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| {
            let start = element.selection_start();
            let end = element.selection_end();
            if start <= end {
                (start, end, element.selection_direction().to_owned())
            } else {
                (end, end, "none".to_owned())
            }
        })
        .unwrap_or((0, 0, "none".to_owned()))
}

pub(in crate::script_vm) fn option_is_disabled(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime.dom_host().option_is_disabled(handle)
}
