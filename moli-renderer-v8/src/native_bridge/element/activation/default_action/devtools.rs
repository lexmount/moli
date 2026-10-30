use super::*;
use crate::context_bootstrap::readonly_data_transfer_for_input;
use crate::dom::native::SelectedFile;
use crate::native_bridge::element::{
    cache_input_files_from_selected_files, construct_drop_input_event, element_attribute,
    focus_element_preserving_selection, replace_text_control_selection,
};

fn data_transfer_selected_files<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data_transfer: v8::Local<'s, v8::Object>,
) -> Vec<SelectedFile> {
    crate::context_bootstrap::native_data_transfer_files(scope, data_transfer)
        .into_iter()
        .filter_map(|file| crate::context_bootstrap::selected_file_from_object(scope, file))
        .collect()
}

fn data_transfer_allows_drop_default_action<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data_transfer: v8::Local<'s, v8::Object>,
) -> bool {
    let drop_effect =
        crate::context_bootstrap::native_data_transfer_drop_effect(scope, data_transfer);
    let allowed =
        crate::context_bootstrap::native_data_transfer_effect_allowed(scope, data_transfer);
    moli_file_api::data_transfer::drop_effect_allowed_by_effect_allowed(
        Some(&allowed),
        &drop_effect,
    )
}

fn event_default_prevented(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) -> bool {
    crate::context_bootstrap::event_backing(scope, event)
        .get(scope, v8str(scope, "defaultPrevented").into())
        .is_some_and(|value| value.boolean_value(scope))
}

fn is_text_drop_target(element: &Element) -> bool {
    if element.is_html_textarea() {
        return element.attribute("readonly").is_none();
    }
    if !element.is_html_input() {
        return false;
    }
    if element.attribute("readonly").is_some() {
        return false;
    }
    matches!(
        element.input_type(),
        InputType::Text
            | InputType::Search
            | InputType::Url
            | InputType::Tel
            | InputType::Email
            | InputType::Password
            | InputType::Number
    )
}

fn perform_file_input_drop_default_action<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    data_transfer: v8::Local<'s, v8::Object>,
    allow_multiple: bool,
) -> bool {
    let mut files = data_transfer_selected_files(scope, data_transfer);
    if files.is_empty() {
        return false;
    }
    if !allow_multiple {
        files.truncate(1);
    }
    let changed = unsafe { &mut *runtime_ptr }.set_input_files(handle, files.clone());
    if !changed {
        return false;
    }
    if let Some(input) = node_wrapper_from_handle(scope, handle) {
        let _ = cache_input_files_from_selected_files(scope, input, &files);
    }
    if let Some(event) = construct_simple_event(scope, "input", true, false, true) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    if let Some(event) = construct_simple_event(scope, "change", true, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    true
}

fn perform_text_drop_default_action<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    data_transfer: v8::Local<'s, v8::Object>,
) -> bool {
    let text =
        crate::context_bootstrap::native_data_transfer_get_data(scope, data_transfer, "text/plain");
    if text.is_empty() {
        return false;
    }
    replace_text_control_selection(
        scope,
        runtime_ptr,
        handle,
        &text,
        TextEditInputType::InsertFromDrop,
    )
}

fn replace_selected_dom_range_with_html<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    selection: v8::Local<'s, v8::Object>,
    replacement_html: &str,
) -> bool {
    let Some(range) = selected_dom_range(scope, target, selection).or_else(|| {
        // A native selection drag can leave the Selection at its source.
        // Preserve a range inside the editing host; otherwise insert the HTML
        // at the host's end, just as the text fallback appends there.
        let document = object_property_as_object(scope, target, "ownerDocument")?;
        let range = call_object_method(scope, document, "createRange", &[])
            .and_then(|range| v8::Local::<v8::Object>::try_from(range).ok())?;
        call_object_method(scope, range, "selectNodeContents", &[target.into()])?;
        let at_start = v8::Boolean::new(scope, false);
        call_object_method(scope, range, "collapse", &[at_start.into()])?;
        Some(range)
    }) else {
        return false;
    };
    let Some(html) = v8_string(scope, replacement_html) else {
        return false;
    };
    let Some(fragment) =
        call_object_method(scope, range, "createContextualFragment", &[html.into()])
            .and_then(|fragment| v8::Local::<v8::Object>::try_from(fragment).ok())
    else {
        return false;
    };
    let Some(last_child) = object_property_as_object(scope, fragment, "lastChild") else {
        return false;
    };
    let _ = call_object_method(scope, range, "deleteContents", &[]);
    if call_object_method(scope, range, "insertNode", &[fragment.into()]).is_none() {
        return false;
    }
    collapse_selection_after_node(scope, selection, last_child);
    true
}

fn collapse_selection_after_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) {
    let Some(parent) = object_property_as_object(scope, node, "parentNode") else {
        return;
    };
    let Some(index) = child_index(scope, parent, node) else {
        return;
    };
    collapse_selection_to(scope, selection, parent, index + 1);
}

fn child_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    let child_nodes = object_property_as_object(scope, parent, "childNodes")?;
    let length = object_number_property(scope, child_nodes, "length")? as u32;
    for index in 0..length {
        let value = child_nodes.get_index(scope, index)?;
        if value.strict_equals(child.into()) {
            return Some(index);
        }
    }
    None
}

fn perform_contenteditable_drop_default_action<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    data_transfer: v8::Local<'s, v8::Object>,
) -> bool {
    let text =
        crate::context_bootstrap::native_data_transfer_get_data(scope, data_transfer, "text/plain");
    let plaintext_only = element_attribute(unsafe { &*runtime_ptr }, handle, "contenteditable")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("plaintext-only"));
    let html =
        crate::context_bootstrap::native_data_transfer_get_data(scope, data_transfer, "text/html");
    let html = (!html.is_empty()).then_some(html);
    if text.is_empty() && html.is_none() {
        return false;
    }
    let Some(document) = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(handle)
    else {
        return false;
    };
    let Some(input_data_transfer) = readonly_data_transfer_for_input(scope, data_transfer) else {
        return false;
    };
    let before_input = if plaintext_only {
        construct_input_event(
            scope,
            "beforeinput",
            TextEditInputType::InsertFromDrop,
            Some(&text),
        )
    } else {
        construct_drop_input_event(scope, "beforeinput", input_data_transfer)
    };
    let Some(before_input) = before_input else {
        return false;
    };
    let _ = dispatch_public_event(scope, runtime_ptr, handle, before_input);
    if event_default_prevented(scope, before_input) {
        return false;
    }
    let Some(handle) =
        contenteditable_editing_host_in_document(unsafe { &*runtime_ptr }, handle, document)
    else {
        return false;
    };
    let Some(target) = node_wrapper_from_handle(scope, handle) else {
        return false;
    };
    // beforeinput can also change the editing host and its inherited mode.
    let plaintext_only = element_attribute(unsafe { &*runtime_ptr }, handle, "contenteditable")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("plaintext-only"));
    let inserted = if let Some(selection) = window_selection(scope) {
        if !plaintext_only
            && let Some(html) = html
            && replace_selected_dom_range_with_html(scope, target, selection, &html)
        {
            true
        } else if let Some((node, start, end)) = selected_text_node_range(scope, target, selection)
        {
            if !replace_text_node_range(scope, node, start, end, &text) {
                return false;
            }
            let caret = u32::try_from(start.saturating_add(utf16_len(&text))).unwrap_or(u32::MAX);
            collapse_selection_to(scope, selection, node, caret);
            true
        } else {
            replace_selected_dom_range(
                scope,
                target,
                selection,
                &text,
                TextRangeInsertionMode::Replacement,
            ) || append_text_to_editing_host(scope, target, &text)
        }
    } else {
        append_text_to_editing_host(scope, target, &text)
    };
    if !inserted {
        return false;
    }
    if let Some(event) = construct_drop_input_event(scope, "input", input_data_transfer) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    true
}

fn hyperlink_activation_default_action_handle(
    runtime: &PageScriptHost,
    target: DomHandle,
) -> Option<DomHandle> {
    let mut current = Some(target);
    while let Some(handle) = current {
        if is_disabled_form_control(runtime, handle) {
            return None;
        }
        if element_is_anchor_with_href(runtime, handle) {
            return Some(handle);
        }
        current = runtime.dom_host().parent_node(handle);
    }
    None
}

pub(crate) fn activate_handle_after_pointer_release(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    click_detail: i32,
    modifiers: u8,
    pointer: &crate::runtime::RendererPointerEventProperties,
) -> RendererInputDispatchOutcome {
    // Ordinary pointer focus belongs to uncanceled mouse-down default action.
    // Reapplying it here would undo a canceled pointerdown/mousedown. Label
    // activation still applies its separate associated-control focus rule.
    click_handle_internal(
        scope,
        runtime_ptr,
        handle,
        coordinates,
        button,
        buttons,
        click_detail,
        modifiers,
        ClickEventSource::PointerWithProperties(pointer),
        ClickFocusBehavior::Preserve,
    )
}

pub(crate) fn perform_auxiliary_link_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    button: i32,
    modifiers: u8,
    pending_child_navigations_before_default: &[(DomHandle, ChildBrowsingContextBootstrap)],
) -> Option<RendererPendingDownloadActivation> {
    let handle = hyperlink_activation_default_action_handle(unsafe { &*runtime_ptr }, target)?;
    anchor_click_default_action(
        scope,
        runtime_ptr,
        handle,
        button,
        modifiers,
        true,
        pending_child_navigations_before_default,
    )
}

/// Editable controls and file inputs can accept compatible native drag data
/// without an author canceling dragover. Inspect only store metadata here:
/// JavaScript still sees a protected store until the actual drop event.
pub(crate) fn accepts_drop_default_action<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    data_transfer: v8::Local<'s, v8::Object>,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.dom_host().is_connected(handle) || is_disabled_form_control(runtime, handle) {
        return false;
    }
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    if element.is_html_input() && element.input_type() == InputType::File {
        let count = crate::context_bootstrap::native_data_transfer_file_count(scope, data_transfer);
        return count > 0 && (element.has_attribute("multiple") || count == 1);
    }
    let has_text =
        crate::context_bootstrap::native_data_transfer_has_type(scope, data_transfer, "text/plain");
    if element.is_html_input() || element.is_html_textarea() {
        return is_text_drop_target(element) && has_text;
    }
    contenteditable_editing_host(runtime, handle).is_some()
        && (has_text
            || crate::context_bootstrap::native_data_transfer_has_type(
                scope,
                data_transfer,
                "text/html",
            ))
}

pub(crate) fn perform_drop_default_action<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    data_transfer: v8::Local<'s, v8::Object>,
) -> bool {
    let (is_file_input, allow_multiple, is_text_target, editing_host, focus_handle) = {
        let runtime = unsafe { &*runtime_ptr };
        if is_disabled_form_control(runtime, handle) {
            return false;
        }
        let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
            return false;
        };
        let is_file_input = element.is_html_input() && element.input_type() == InputType::File;
        let is_text_target = is_text_drop_target(element);
        let editing_host = (!is_text_target && !is_file_input)
            .then(|| contenteditable_editing_host(runtime, handle))
            .flatten();
        let focus_handle = if is_focusable(runtime, handle) {
            Some(handle)
        } else {
            editing_host.filter(|host| is_focusable(runtime, *host))
        };
        (
            is_file_input,
            element.attribute("multiple").is_some(),
            is_text_target,
            editing_host,
            focus_handle,
        )
    };

    if !data_transfer_allows_drop_default_action(scope, data_transfer) {
        return false;
    }

    if let Some(focus_handle) = focus_handle {
        if editing_host.is_some() {
            // Keep the drop's selection when its editing host gains focus.
            focus_element_preserving_selection(scope, runtime_ptr, focus_handle);
        } else {
            update_focus(scope, runtime_ptr, Some(focus_handle));
        }
    }

    if is_file_input {
        return perform_file_input_drop_default_action(
            scope,
            runtime_ptr,
            handle,
            data_transfer,
            allow_multiple,
        );
    }
    if is_text_target {
        return perform_text_drop_default_action(scope, runtime_ptr, handle, data_transfer);
    }
    if let Some(editing_host) = editing_host {
        return perform_contenteditable_drop_default_action(
            scope,
            runtime_ptr,
            editing_host,
            data_transfer,
        );
    }
    false
}
