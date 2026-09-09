use super::*;
use crate::native_bridge::document::{detached_form_owner_object, detached_label_control_object};

fn is_labelable_element(element: &Element) -> bool {
    if element.namespace() != "http://www.w3.org/1999/xhtml" {
        return false;
    }
    match element.local_name() {
        "button" | "meter" | "output" | "progress" | "select" | "textarea" => true,
        "input" => element.input_type() != InputType::Hidden,
        _ => false,
    }
}

fn is_labelable_handle(runtime: &JsContextHost, handle: DomHandle, element: &Element) -> bool {
    if crate::custom_elements::is_form_associated_custom_element_handle(runtime, handle) {
        return true;
    }
    is_labelable_element(element)
}

fn is_label_interactive_content(element: &Element) -> bool {
    if element.namespace() != "http://www.w3.org/1999/xhtml" {
        return false;
    }
    match element.local_name() {
        "button" | "details" | "embed" | "iframe" | "select" | "textarea" => true,
        "a" | "area" => element.has_attribute("href"),
        "audio" | "video" => element.has_attribute("controls"),
        "img" => element.has_attribute("usemap"),
        "input" => element.input_type() != InputType::Hidden,
        "label" => true,
        _ => false,
    }
}

fn label_tree_scope_root(runtime: &JsContextHost, label_handle: DomHandle) -> DomHandle {
    runtime
        .dom_host()
        .root_node_handle(label_handle)
        .unwrap_or_else(|| runtime.dom_host().document_handle())
}

fn element_by_id_in_label_tree_scope(
    runtime: &JsContextHost,
    label_handle: DomHandle,
    id: &str,
) -> Option<DomHandle> {
    let root = label_tree_scope_root(runtime, label_handle);
    runtime.dom_host().element_handle_by_id_in_subtree(root, id)
}

fn collect_shadow_including_labels_from(
    runtime: &JsContextHost,
    root: DomHandle,
    include_root: bool,
    out: &mut Vec<DomHandle>,
) {
    if include_root {
        collect_shadow_including_labels_at(runtime, root, out);
        return;
    }

    let mut child = runtime.dom_host().first_child(root);
    while let Some(handle) = child {
        collect_shadow_including_labels_at(runtime, handle, out);
        child = runtime.dom_host().next_sibling(handle);
    }
}

fn collect_shadow_including_labels_at(
    runtime: &JsContextHost,
    handle: DomHandle,
    out: &mut Vec<DomHandle>,
) {
    let element = runtime.dom_host().node(handle).and_then(Node::as_element);
    if element.is_some_and(Element::is_html_label) {
        out.push(handle);
    }
    if element.is_some()
        && let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle)
    {
        collect_shadow_including_labels_from(runtime, shadow_root, false, out);
    }

    let mut child = runtime.dom_host().first_child(handle);
    while let Some(child_handle) = child {
        collect_shadow_including_labels_at(runtime, child_handle, out);
        child = runtime.dom_host().next_sibling(child_handle);
    }
}

fn shadow_including_label_handles(runtime: &JsContextHost) -> Vec<DomHandle> {
    let mut labels = Vec::new();
    let (root, include_root) = (runtime.dom_host().document_handle(), false);
    collect_shadow_including_labels_from(runtime, root, include_root, &mut labels);
    labels
}

fn shadow_including_contains(
    runtime: &JsContextHost,
    root: DomHandle,
    candidate: DomHandle,
) -> bool {
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        if handle == candidate {
            return true;
        }
        if runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some()
            && let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle)
        {
            stack.push(shadow_root);
        }
        let mut child = runtime.dom_host().first_child(handle);
        while let Some(child_handle) = child {
            stack.push(child_handle);
            child = runtime.dom_host().next_sibling(child_handle);
        }
    }
    false
}

fn label_handles_for_control(runtime: &JsContextHost, control_handle: DomHandle) -> Vec<DomHandle> {
    if runtime.dom_host().is_connected(control_handle) {
        return shadow_including_label_handles(runtime);
    }

    let root = runtime
        .dom_host()
        .root_node_handle(control_handle)
        .unwrap_or(control_handle);
    let mut roots = Vec::new();
    for binding in runtime.dom_host().snapshot_shadow_root_bindings() {
        if runtime
            .dom_host()
            .resolve_reference_target_chain(binding.host)
            == Some(control_handle)
            && let Some(host_root) = runtime.dom_host().root_node_handle(binding.host)
            && !roots.contains(&host_root)
        {
            roots.push(host_root);
        }
    }
    if roots.is_empty() {
        roots.push(root);
    }
    let outer_roots = roots
        .iter()
        .copied()
        .filter(|root| {
            !roots.iter().any(|candidate| {
                candidate != root && shadow_including_contains(runtime, *candidate, *root)
            })
        })
        .collect::<Vec<_>>();

    let mut labels = Vec::new();
    for root in outer_roots {
        collect_shadow_including_labels_from(runtime, root, true, &mut labels);
    }
    let mut unique = Vec::new();
    for label in labels {
        if !unique.contains(&label) {
            unique.push(label);
        }
    }
    unique
}

fn first_implicit_label_control_from_node(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let element = runtime.dom_host().node(handle).and_then(Node::as_element);
    if let Some(element) = element {
        if runtime.dom_host().shadow_root_handle(handle).is_some() {
            let resolved = runtime.dom_host().resolve_reference_target_chain(handle)?;
            return runtime
                .dom_host()
                .node(resolved)
                .and_then(Node::as_element)
                .filter(|element| is_labelable_handle(runtime, resolved, element))
                .map(|_| resolved);
        }
        if is_labelable_handle(runtime, handle, element) {
            return Some(handle);
        }
    }

    let mut child = runtime.dom_host().first_child(handle);
    while let Some(child_handle) = child {
        if let Some(control) = first_implicit_label_control_from_node(runtime, child_handle) {
            return Some(control);
        }
        child = runtime.dom_host().next_sibling(child_handle);
    }
    None
}

fn first_implicit_label_control(
    runtime: &JsContextHost,
    label_handle: DomHandle,
) -> Option<DomHandle> {
    let mut child = runtime.dom_host().first_child(label_handle);
    while let Some(child_handle) = child {
        if let Some(control) = first_implicit_label_control_from_node(runtime, child_handle) {
            return Some(control);
        }
        child = runtime.dom_host().next_sibling(child_handle);
    }
    None
}

pub(in crate::native_bridge) fn label_control_handle(
    runtime: &JsContextHost,
    label_handle: DomHandle,
) -> Option<DomHandle> {
    let label = runtime
        .dom_host()
        .node(label_handle)
        .and_then(Node::as_element)?;
    if !label.is_html_label() {
        return None;
    }

    if let Some(for_id) = label.attribute("for").filter(|value| !value.is_empty()) {
        let candidate = element_by_id_in_label_tree_scope(runtime, label_handle, for_id)?;
        let candidate = runtime
            .dom_host()
            .resolve_reference_target_chain(candidate)?;
        return runtime
            .dom_host()
            .node(candidate)
            .and_then(Node::as_element)
            .filter(|element| is_labelable_handle(runtime, candidate, element))
            .map(|_| candidate);
    }

    first_implicit_label_control(runtime, label_handle)
}

fn label_reflected_control_handle(
    runtime: &JsContextHost,
    label_handle: DomHandle,
) -> Option<DomHandle> {
    let label = runtime
        .dom_host()
        .node(label_handle)
        .and_then(Node::as_element)?;
    if !label.is_html_label() {
        return None;
    }

    if let Some(for_id) = label.attribute("for").filter(|value| !value.is_empty()) {
        let candidate = element_by_id_in_label_tree_scope(runtime, label_handle, for_id)?;
        let resolved = runtime
            .dom_host()
            .resolve_reference_target_chain(candidate)?;
        return runtime
            .dom_host()
            .node(resolved)
            .and_then(Node::as_element)
            .filter(|element| is_labelable_handle(runtime, resolved, element))
            // `label.control` is a reflected IDL surface. Reference-target
            // reflection exposes the host while validation uses the resolved
            // inner control.
            .map(|_| candidate);
    }

    label_control_handle(runtime, label_handle)
}

pub(in crate::native_bridge) fn control_label_handles(
    runtime: &JsContextHost,
    control_handle: DomHandle,
) -> Vec<DomHandle> {
    let Some(element) = runtime
        .dom_host()
        .node(control_handle)
        .and_then(Node::as_element)
    else {
        return Vec::new();
    };
    if !is_labelable_handle(runtime, control_handle, element) {
        return Vec::new();
    }

    label_handles_for_control(runtime, control_handle)
        .into_iter()
        .filter(|label_handle| label_control_handle(runtime, *label_handle) == Some(control_handle))
        .collect()
}

pub(in crate::native_bridge) fn label_activation_control_handle(
    runtime: &JsContextHost,
    target_handle: DomHandle,
) -> Option<DomHandle> {
    let mut current = Some(target_handle);
    let mut blocked_by_interactive_content = false;
    while let Some(handle) = current {
        let element = runtime.dom_host().node(handle).and_then(Node::as_element);
        if element.is_some_and(Element::is_html_label) {
            let control = label_control_handle(runtime, handle)?;
            return (control != target_handle && !blocked_by_interactive_content)
                .then_some(control);
        }
        if element.is_some_and(is_label_interactive_content) {
            blocked_by_interactive_content = true;
        }
        current = runtime.dom_host().parent_node(handle);
    }
    None
}

pub(in crate::native_bridge) fn label_receives_programmatic_focus(
    runtime: &JsContextHost,
    label_handle: DomHandle,
) -> bool {
    let Some(label) = runtime
        .dom_host()
        .node(label_handle)
        .and_then(Node::as_element)
    else {
        return false;
    };
    if !label.is_html_label() {
        return false;
    }
    if label.attribute("style").is_some_and(|style| {
        style
            .split(';')
            .map(str::trim)
            .any(|declaration| declaration.eq_ignore_ascii_case("display:none"))
    }) {
        return false;
    }
    label
        .attribute("tabindex")
        .is_some_and(|value| !value.trim().is_empty())
}

pub(in crate::native_bridge) fn label_html_for_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "for", rv);
}

pub(in crate::native_bridge) fn label_html_for_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_form_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "for",
        args.get(0),
        "HTMLLabelElement",
        "htmlFor",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn label_control_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(control) = detached_label_control_object(scope, args.this()) {
        rv.set(control.into());
        return;
    }
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let owner = label_reflected_control_handle(unsafe { &*runtime_ptr }, handle);
    set_wrapped_node_or_null(scope, &mut rv, runtime_ptr, owner);
}

pub(in crate::native_bridge) fn label_form_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(control) = detached_label_control_object(scope, args.this()) {
        match detached_form_owner_object(scope, control) {
            Some(form) => rv.set(form.into()),
            None => rv.set_null(),
        }
        return;
    }
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let owner = label_control_handle(runtime, handle)
        .and_then(|control| super::owner::form_associated_form_owner(runtime, control));
    set_wrapped_node_or_null(scope, &mut rv, runtime_ptr, owner);
}

pub(in crate::native_bridge) fn control_labels_getter_function<'s>(
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
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        rv.set_null();
        return;
    };
    if !is_labelable_handle(runtime, handle, element) {
        rv.set_null();
        return;
    }

    let labels = collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        handle,
        CollectionKind::NodeList,
        LiveCollectionQueryKind::Labels,
        None,
        false,
    );
    rv.set(labels.into());
}
