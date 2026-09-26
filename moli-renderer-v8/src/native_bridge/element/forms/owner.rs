use super::*;
use crate::custom_elements::is_form_associated_custom_element_handle;
use crate::native_bridge::document::detached_form_owner_object;
use crate::native_bridge::set_wrapped_handle_or_null_for_receiver;

pub(crate) fn form_associated_form_owner(
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    if is_form_associated_custom_element_handle(runtime, handle) {
        return custom_element_form_owner(runtime, handle, element);
    }
    runtime.dom_host().builtin_form_associated_owner(handle)
}

fn custom_element_form_owner(
    runtime: &PageScriptHost,
    handle: DomHandle,
    element: &crate::dom::native::Element,
) -> Option<DomHandle> {
    if let Some(form_id) = element.attribute("form")
        && runtime.dom_host().is_connected_to_document(handle)
    {
        if form_id.is_empty() {
            return None;
        }
        let tree_root = runtime.dom_host().root_node_handle(handle)?;
        let candidate = runtime
            .dom_host()
            .element_handle_by_id_in_subtree(tree_root, form_id)?;
        let resolved = runtime
            .dom_host()
            .resolve_reference_target_chain(candidate)?;
        return runtime
            .dom_host()
            .is_html_element_named(resolved, "form")
            .then_some(resolved);
    }

    let mut current = runtime.dom_host().parent_node(handle);
    while let Some(parent) = current {
        if runtime.dom_host().is_html_element_named(parent, "form") {
            return Some(parent);
        }
        current = runtime.dom_host().parent_node(parent);
    }
    None
}

use moli_dom::native::is_builtin_reassociateable_form_associated_element as has_reflected_builtin_form_owner;

fn form_associated_reflected_form_owner(
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    if !has_reflected_builtin_form_owner(element) {
        return None;
    }
    if let Some(form_id) = element.attribute("form")
        && runtime.dom_host().is_connected_to_document(handle)
    {
        if form_id.is_empty() {
            return None;
        }
        let tree_root = runtime.dom_host().root_node_handle(handle)?;
        let candidate = runtime
            .dom_host()
            .element_handle_by_id_in_subtree(tree_root, form_id)?;
        let resolved = runtime
            .dom_host()
            .resolve_reference_target_chain(candidate)?;
        return runtime
            .dom_host()
            .is_html_element_named(resolved, "form")
            .then_some(candidate);
    }
    form_associated_form_owner(runtime, handle)
}

pub(crate) fn is_valid_submit_button(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| {
            if element.is_html_input() {
                element.input_type().is_submit_button()
            } else if element.is_html_element("button") {
                runtime.dom_host().button_is_submit_button(handle)
            } else {
                false
            }
        })
}

pub(crate) fn form_control_is_effectively_disabled(
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> bool {
    let dom = runtime.dom_host().dom();
    dom.is_disabled_form_control(handle)
        || (is_form_associated_custom_element_handle(runtime, handle)
            && (dom
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.has_attribute_ns("", "disabled"))
                || dom.disabled_fieldset_ancestor(handle).is_some()))
}

pub(in crate::native_bridge) fn form_associated_form_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(form) = detached_form_owner_object(scope, args.this()) {
        rv.set(form.into());
        return;
    }
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let owner = form_associated_reflected_form_owner(unsafe { &*runtime_ptr }, handle);
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, runtime_ptr, args.this(), owner);
}
