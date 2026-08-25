use crate::context_bootstrap::{
    LocationNavigationKind, navigate_location_object_with_source_element,
    selection_value_for_window,
};
use crate::dom::native::{Element, Node, SelectedFile};
use crate::util::{
    call_object_method, get_private_value, node_wrapper_from_handle, object_bool_property,
    object_number_property, object_property_as_object, utf16_len, v8_string, v8str,
};
use crate::{
    RendererInputDispatchOutcome, RendererPendingDownloadActivation,
    RendererPendingFileChooserActivation, RendererPopupDisposition,
    document_runtime::{DomHandle, EventTargetHandle},
    frame_owner_model::DocumentId,
    native_bridge::context_host::ChildBrowsingContextBootstrap,
    runtime::RendererDocumentLifecycleIdentity,
};

use super::super::super::JsContextHost;
use super::super::forms::{FormAssociatedResetCallbackTiming, reset_form_default_action};
use super::super::{
    NodePublicEventDispatchOutcome, cache_input_files_from_selected_files,
    construct_click_event_with_detail_and_modifiers, construct_command_event,
    construct_simple_event, contenteditable_editing_host, dispatch_popover_toggle_events,
    dispatch_public_event, element_attribute, element_has_attribute, form_associated_form_owner,
    is_disabled_form_control, is_focusable, is_valid_submit_button,
    label_activation_control_handle, observable_bounding_client_rect,
    perform_popover_invoker_default_action, perform_summary_click_default_action,
    replace_text_control_selection, resolve_url_like_attribute, scroll_node_into_view_at_start,
    submit_form_with_submit_event, update_focus,
};
use super::targets::{
    SpecialBrowsingContextTarget, named_iframe_target_handle_for_navigation,
    navigate_hyperlink_source_browsing_context, navigate_hyperlink_target_browsing_context,
    navigate_target_browsing_context,
};

const BUTTON_COMMAND_FOR_ELEMENT_SLOT: &str = "__moliButtonCommandForElement";
const BUTTON_POPOVER_TARGET_ELEMENT_SLOT: &str = "__moliButtonPopoverTargetElement";

fn array_like_length(scope: &mut v8::PinScope<'_, '_>, object: v8::Local<'_, v8::Object>) -> u32 {
    object
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u32)
        .unwrap_or(0)
}

fn data_transfer_selected_files(
    scope: &mut v8::PinScope<'_, '_>,
    data_transfer: v8::Local<'_, v8::Object>,
) -> Option<Vec<SelectedFile>> {
    let files = data_transfer.get(scope, v8str(scope, "files").into())?;
    let files = v8::Local::<v8::Object>::try_from(files).ok()?;
    let mut selected = Vec::with_capacity(array_like_length(scope, files) as usize);
    for index in 0..array_like_length(scope, files) {
        let Some(value) = files.get_index(scope, index) else {
            continue;
        };
        let Ok(file) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        let Some(file) = crate::context_bootstrap::selected_file_from_object(scope, file) else {
            continue;
        };
        selected.push(file);
    }
    Some(selected)
}

fn data_transfer_text(
    scope: &mut v8::PinScope<'_, '_>,
    data_transfer: v8::Local<'_, v8::Object>,
) -> Option<String> {
    data_transfer_string(scope, data_transfer, "text")
}

fn data_transfer_html(
    scope: &mut v8::PinScope<'_, '_>,
    data_transfer: v8::Local<'_, v8::Object>,
) -> Option<String> {
    data_transfer_string(scope, data_transfer, "text/html")
}

fn data_transfer_string(
    scope: &mut v8::PinScope<'_, '_>,
    data_transfer: v8::Local<'_, v8::Object>,
    mime_type: &str,
) -> Option<String> {
    let get_data = data_transfer.get(scope, v8str(scope, "getData").into())?;
    let get_data = v8::Local::<v8::Function>::try_from(get_data).ok()?;
    let text_type = v8_string(scope, mime_type)?;
    let value = get_data.call(scope, data_transfer.into(), &[text_type.into()])?;
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn data_transfer_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    data_transfer: v8::Local<'_, v8::Object>,
    property_name: &'static str,
) -> Option<String> {
    data_transfer
        .get(scope, v8str(scope, property_name).into())?
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn drop_effect_allowed(effect_allowed: &str, drop_effect: &str) -> bool {
    match drop_effect {
        "none" => false,
        "copy" => matches!(
            effect_allowed,
            "copy" | "copyLink" | "copyMove" | "all" | "uninitialized"
        ),
        "link" => matches!(
            effect_allowed,
            "link" | "copyLink" | "linkMove" | "all" | "uninitialized"
        ),
        "move" => matches!(
            effect_allowed,
            "move" | "copyMove" | "linkMove" | "all" | "uninitialized"
        ),
        _ => false,
    }
}

fn data_transfer_allows_drop_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    data_transfer: v8::Local<'_, v8::Object>,
) -> bool {
    let drop_effect =
        data_transfer_string_property(scope, data_transfer, "dropEffect").unwrap_or_default();
    let effect_allowed = data_transfer_string_property(scope, data_transfer, "effectAllowed")
        .unwrap_or_else(|| "uninitialized".to_owned());
    drop_effect_allowed(&effect_allowed, &drop_effect)
}

fn event_default_prevented(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) -> bool {
    event
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
        element.input_type().as_str(),
        "" | "text" | "search" | "url" | "tel" | "email" | "password" | "number"
    )
}

fn perform_file_input_drop_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    data_transfer: v8::Local<'_, v8::Object>,
    allow_multiple: bool,
) -> bool {
    let Some(mut files) = data_transfer_selected_files(scope, data_transfer) else {
        return false;
    };
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
    if let Some(event) = construct_simple_event(scope, "input", true, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    if let Some(event) = construct_simple_event(scope, "change", true, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    true
}

fn perform_text_drop_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    data_transfer: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(text) = data_transfer_text(scope, data_transfer) else {
        return false;
    };
    if text.is_empty() {
        return false;
    }
    replace_text_control_selection(scope, runtime_ptr, handle, &text)
}

fn slice_chars(value: &str, start: usize, end: usize) -> String {
    value
        .chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

fn call_node_contains(
    scope: &mut v8::PinScope<'_, '_>,
    target: v8::Local<'_, v8::Object>,
    candidate: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(contains) = target.get(scope, v8str(scope, "contains").into()) else {
        return false;
    };
    let Ok(contains) = v8::Local::<v8::Function>::try_from(contains) else {
        return false;
    };
    contains
        .call(scope, target.into(), &[candidate.into()])
        .is_some_and(|value| value.boolean_value(scope))
}

fn window_selection<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    selection_value_for_window(scope, global)
}

fn collapse_selection_to(
    scope: &mut v8::PinScope<'_, '_>,
    selection: v8::Local<'_, v8::Object>,
    node: v8::Local<'_, v8::Object>,
    offset: u32,
) {
    let Some(collapse) = selection.get(scope, v8str(scope, "collapse").into()) else {
        return;
    };
    let Ok(collapse) = v8::Local::<v8::Function>::try_from(collapse) else {
        return;
    };
    let offset = v8::Integer::new_from_unsigned(scope, offset);
    let _ = collapse.call(scope, selection.into(), &[node.into(), offset.into()]);
}

fn selected_text_node_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, usize, usize)> {
    let anchor_node = selection.get(scope, v8str(scope, "anchorNode").into())?;
    let focus_node = selection.get(scope, v8str(scope, "focusNode").into())?;
    if anchor_node.is_null_or_undefined()
        || focus_node.is_null_or_undefined()
        || !anchor_node.strict_equals(focus_node)
    {
        return None;
    }
    let node = v8::Local::<v8::Object>::try_from(anchor_node).ok()?;
    if object_number_property(scope, node, "nodeType")? as u32 != 3 {
        return None;
    }
    if !call_node_contains(scope, target, node) {
        return None;
    }
    let anchor_offset = object_number_property(scope, selection, "anchorOffset")?;
    let focus_offset = object_number_property(scope, selection, "focusOffset")?;
    if !anchor_offset.is_finite()
        || !focus_offset.is_finite()
        || anchor_offset < 0.0
        || focus_offset < 0.0
    {
        return None;
    }
    let start = anchor_offset.min(focus_offset) as usize;
    let end = anchor_offset.max(focus_offset) as usize;
    Some((node, start, end))
}

fn replace_text_node_range(
    scope: &mut v8::PinScope<'_, '_>,
    node: v8::Local<'_, v8::Object>,
    start: usize,
    end: usize,
    replacement_text: &str,
) -> bool {
    let value = node
        .get(scope, v8str(scope, "textContent").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let next = format!(
        "{}{}{}",
        slice_chars(&value, 0, start),
        replacement_text,
        slice_chars(&value, end, value.chars().count())
    );
    let Some(next_value) = v8_string(scope, &next) else {
        return false;
    };
    node.set(scope, v8str(scope, "textContent").into(), next_value.into())
        .unwrap_or(false)
}

fn selected_dom_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if object_number_property(scope, selection, "rangeCount")? < 1.0 {
        return None;
    }
    let anchor_node = object_property_as_object(scope, selection, "anchorNode")?;
    let focus_node = object_property_as_object(scope, selection, "focusNode")?;
    if !call_node_contains(scope, target, anchor_node)
        || !call_node_contains(scope, target, focus_node)
    {
        return None;
    }
    let zero = v8::Integer::new_from_unsigned(scope, 0);
    call_object_method(scope, selection, "getRangeAt", &[zero.into()])
        .and_then(|range| v8::Local::<v8::Object>::try_from(range).ok())
}

fn create_text_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    text: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let document = object_property_as_object(scope, global, "document")?;
    let text = v8_string(scope, text)?;
    call_object_method(scope, document, "createTextNode", &[text.into()])
        .and_then(|node| v8::Local::<v8::Object>::try_from(node).ok())
}

fn is_text_node(scope: &mut v8::PinScope<'_, '_>, node: v8::Local<'_, v8::Object>) -> bool {
    object_number_property(scope, node, "nodeType") == Some(3.0)
}

fn text_insertion_target_at_range_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, u32)> {
    let container = object_property_as_object(scope, range, "startContainer")?;
    let offset = object_number_property(scope, range, "startOffset")?;
    if !offset.is_finite() || offset < 0.0 || offset > u32::MAX as f64 {
        return None;
    }
    let offset = offset as u32;
    if is_text_node(scope, container) {
        return Some((container, offset));
    }

    let child_nodes = object_property_as_object(scope, container, "childNodes")?;
    let child_count = object_number_property(scope, child_nodes, "length")? as u32;
    if offset < child_count {
        let next = child_nodes.get_index(scope, offset)?;
        let next = v8::Local::<v8::Object>::try_from(next).ok()?;
        if is_text_node(scope, next) {
            return Some((next, 0));
        }
    }
    if offset == 0 || offset > child_count {
        return None;
    }
    let previous = child_nodes.get_index(scope, offset - 1)?;
    let previous = v8::Local::<v8::Object>::try_from(previous).ok()?;
    if !is_text_node(scope, previous) {
        return None;
    }
    let length = object_number_property(scope, previous, "length")?;
    (length.is_finite() && length >= 0.0 && length <= u32::MAX as f64)
        .then_some((previous, length as u32))
}

fn insert_text_into_text_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    offset: u32,
    text: &str,
) -> bool {
    let offset = v8::Integer::new_from_unsigned(scope, offset);
    let Some(text) = v8_string(scope, text) else {
        return false;
    };
    call_object_method(scope, node, "insertData", &[offset.into(), text.into()]).is_some()
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TextRangeInsertionMode {
    Typing,
    Replacement,
}

fn replace_selected_dom_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    selection: v8::Local<'s, v8::Object>,
    replacement_text: &str,
    mode: TextRangeInsertionMode,
) -> bool {
    let Some(range) = selected_dom_range(scope, target, selection) else {
        return false;
    };
    let Some(text_node) = create_text_node(scope, replacement_text) else {
        return false;
    };
    if call_object_method(scope, range, "deleteContents", &[]).is_none() {
        return false;
    }
    if mode == TextRangeInsertionMode::Typing
        && let Some((target, offset)) = text_insertion_target_at_range_start(scope, range)
    {
        if !insert_text_into_text_node(scope, target, offset, replacement_text) {
            return false;
        }
        collapse_selection_to(
            scope,
            selection,
            target,
            offset.saturating_add(utf16_len(replacement_text) as u32),
        );
        return true;
    }
    if call_object_method(scope, range, "insertNode", &[text_node.into()]).is_none() {
        return false;
    }
    collapse_selection_to(
        scope,
        selection,
        text_node,
        utf16_len(replacement_text) as u32,
    );
    true
}

fn replace_selected_dom_range_with_html<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    selection: v8::Local<'s, v8::Object>,
    replacement_html: &str,
) -> bool {
    let Some(range) = selected_dom_range(scope, target, selection) else {
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

fn append_text_to_editing_host(
    scope: &mut v8::PinScope<'_, '_>,
    target: v8::Local<'_, v8::Object>,
    text: &str,
) -> bool {
    let current = target
        .get(scope, v8str(scope, "textContent").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let Some(next) = v8_string(scope, &(current + text)) else {
        return false;
    };
    target
        .set(scope, v8str(scope, "textContent").into(), next.into())
        .unwrap_or(false)
}

pub(crate) fn replace_contenteditable_selection(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    replacement_text: &str,
) -> bool {
    let Some(target) = node_wrapper_from_handle(scope, handle) else {
        return false;
    };
    let Some(before_input) = construct_simple_event(scope, "beforeinput", true, true, true) else {
        return false;
    };
    let _ = dispatch_public_event(scope, runtime_ptr, handle, before_input);
    if event_default_prevented(scope, before_input) {
        return false;
    }
    let inserted = if let Some(selection) = window_selection(scope) {
        if let Some((node, start, end)) = selected_text_node_range(scope, target, selection) {
            if !replace_text_node_range(scope, node, start, end, replacement_text) {
                return false;
            }
            let caret = start.saturating_add(replacement_text.chars().count()) as u32;
            collapse_selection_to(scope, selection, node, caret);
            true
        } else {
            replace_selected_dom_range(
                scope,
                target,
                selection,
                replacement_text,
                TextRangeInsertionMode::Typing,
            ) || append_text_to_editing_host(scope, target, replacement_text)
        }
    } else {
        append_text_to_editing_host(scope, target, replacement_text)
    };
    if !inserted {
        return false;
    }
    if let Some(event) = construct_simple_event(scope, "input", true, false, true) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    true
}

pub(crate) fn select_contenteditable_contents(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> bool {
    let Some(target) = node_wrapper_from_handle(scope, handle) else {
        return false;
    };
    let global = scope.get_current_context().global(scope);
    let Some(document) = object_property_as_object(scope, global, "document") else {
        return false;
    };
    let Some(range) = call_object_method(scope, document, "createRange", &[])
        .and_then(|range| v8::Local::<v8::Object>::try_from(range).ok())
    else {
        return false;
    };
    if call_object_method(scope, range, "selectNodeContents", &[target.into()]).is_none() {
        return false;
    }
    let Some(selection) = window_selection(scope) else {
        return false;
    };
    let _ = call_object_method(scope, selection, "removeAllRanges", &[]);
    call_object_method(scope, selection, "addRange", &[range.into()]).is_some()
}

fn perform_contenteditable_drop_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    data_transfer: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(text) = data_transfer_text(scope, data_transfer) else {
        return false;
    };
    let html = data_transfer_html(scope, data_transfer).filter(|html| !html.is_empty());
    if text.is_empty() && html.is_none() {
        return false;
    }
    let Some(target) = node_wrapper_from_handle(scope, handle) else {
        return false;
    };
    let Some(before_input) = construct_simple_event(scope, "beforeinput", true, true, true) else {
        return false;
    };
    let _ = dispatch_public_event(scope, runtime_ptr, handle, before_input);
    if event_default_prevented(scope, before_input) {
        return false;
    }
    let Some(selection) = window_selection(scope) else {
        return append_text_to_editing_host(scope, target, &text);
    };
    if let Some(html) = html
        && replace_selected_dom_range_with_html(scope, target, selection, &html)
    {
        // HTML fragments are inserted as nodes, then the selection is collapsed after the fragment.
    } else if let Some((node, start, end)) = selected_text_node_range(scope, target, selection) {
        if !replace_text_node_range(scope, node, start, end, &text) {
            return false;
        }
        let caret = start.saturating_add(text.chars().count()) as u32;
        collapse_selection_to(scope, selection, node, caret);
    } else if !replace_selected_dom_range(
        scope,
        target,
        selection,
        &text,
        TextRangeInsertionMode::Replacement,
    ) && !append_text_to_editing_host(scope, target, &text)
    {
        return false;
    }
    if let Some(event) = construct_simple_event(scope, "input", true, false, true) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    true
}

fn is_radio_input(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.is_html_input() && element.input_type() == "radio")
}

fn is_checkbox_input(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.is_html_input() && element.input_type() == "checkbox")
}

fn radio_group_members(runtime: &JsContextHost, handle: DomHandle) -> Vec<DomHandle> {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return Vec::new();
    };
    let Some(root) = runtime.dom_host().root_node_handle(handle) else {
        return Vec::new();
    };
    let name = element.attribute("name").unwrap_or_default();
    if name.is_empty() {
        return vec![handle];
    }
    let form_owner = form_associated_form_owner(runtime, handle);
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
                        && candidate_element.input_type() == "radio"
                        && candidate_element.attribute("name") == Some(name)
                        && form_associated_form_owner(runtime, *candidate) == form_owner
                })
        })
        .collect()
}

fn radio_group_checked_snapshot(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Vec<(DomHandle, bool)> {
    radio_group_members(runtime, handle)
        .iter()
        .copied()
        .map(|radio| {
            let checked = runtime
                .dom_host()
                .node(radio)
                .and_then(Node::as_element)
                .is_some_and(Element::checked);
            (radio, checked)
        })
        .collect()
}

fn dispatch_click_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    x: f64,
    y: f64,
    detail: i32,
    button: i32,
    buttons: i32,
    modifiers: u8,
    trusted: bool,
) -> Option<NodePublicEventDispatchOutcome> {
    let event = construct_click_event_with_detail_and_modifiers(
        scope, x, y, detail, button, buttons, modifiers,
    )?;
    crate::context_bootstrap::set_event_trusted(scope, event, trusted);
    Some(dispatch_public_event(scope, runtime_ptr, handle, event))
}

fn click_handle_internal(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
    click_detail: i32,
    modifiers: u8,
    user_initiated: bool,
) -> RendererInputDispatchOutcome {
    let popup_disposition = current_click_popup_disposition(button, modifiers, user_initiated);
    let previous_popup_disposition = unsafe { &mut *runtime_ptr }
        .replace_current_input_popup_disposition(Some(popup_disposition));
    let outcome = click_handle_internal_with_current_input(
        scope,
        runtime_ptr,
        handle,
        x,
        y,
        button,
        buttons,
        click_detail,
        modifiers,
        user_initiated,
    );
    let replaced_popup_disposition = unsafe { &mut *runtime_ptr }
        .replace_current_input_popup_disposition(previous_popup_disposition);
    assert_eq!(
        replaced_popup_disposition,
        Some(popup_disposition),
        "the current click popup disposition must remain scoped to its activation"
    );
    outcome
}

#[allow(clippy::too_many_arguments)]
fn click_handle_internal_with_current_input(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
    click_detail: i32,
    modifiers: u8,
    user_initiated: bool,
) -> RendererInputDispatchOutcome {
    let runtime = unsafe { &*runtime_ptr };
    if is_disabled_form_control(runtime, handle) {
        return RendererInputDispatchOutcome {
            handled: false,
            triggered_top_level_navigation: false,
            pending_download: None,
            pending_file_chooser: None,
        };
    }
    if user_initiated || synthetic_click_focuses_element(runtime, handle) {
        let focus_handle = if is_focusable(runtime, handle) {
            Some(handle)
        } else {
            contenteditable_editing_host(runtime, handle)
                .filter(|host| is_focusable(runtime, *host))
        };
        if let Some(focus_handle) = focus_handle {
            update_focus(scope, runtime_ptr, Some(focus_handle));
        }
    }
    // A click listener may change an input to or from `type=file`, and may then
    // synchronously call `document.open()`. Chromium decides whether to open a
    // chooser from the input's post-callback type, but reports it against the
    // element and LocalFrame that began click activation. Freeze only that
    // causal identity here; do not freeze the pre-callback file state.
    let file_chooser_source = file_chooser_activation_source(scope, runtime_ptr, handle);
    let had_pending_top_level_navigation_before_click =
        unsafe { &*runtime_ptr }.has_pending_location_navigation();

    if is_checkbox_input(runtime, handle) {
        let (previous, previous_indeterminate) = runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .map(|element| (element.checked(), element.indeterminate()))
            .unwrap_or_default();
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.set_checked_state(scope, runtime_ptr, handle, !previous);
        let _ = runtime.set_indeterminate_state(scope, runtime_ptr, handle, false);
        if let Some(outcome) = dispatch_click_event(
            scope,
            runtime_ptr,
            handle,
            x,
            y,
            click_detail,
            button,
            buttons,
            modifiers,
            user_initiated,
        ) && !outcome.allows_default()
        {
            let runtime = unsafe { &mut *runtime_ptr };
            let _ = runtime.set_checked_state(scope, runtime_ptr, handle, previous);
            let _ =
                runtime.set_indeterminate_state(scope, runtime_ptr, handle, previous_indeterminate);
            return input_dispatch_outcome_after_click(
                runtime_ptr,
                false,
                had_pending_top_level_navigation_before_click,
                None,
                None,
            );
        }
        dispatch_input_and_change_events(scope, runtime_ptr, handle);
        let pending_child_navigations_before_default =
            unsafe { &*runtime_ptr }.pending_live_child_browsing_context_navigation_snapshot();
        let pending_download = perform_click_default_action(
            scope,
            runtime_ptr,
            handle,
            x,
            y,
            button,
            modifiers,
            user_initiated,
            &pending_child_navigations_before_default,
        );
        return input_dispatch_outcome_after_click(
            runtime_ptr,
            true,
            had_pending_top_level_navigation_before_click,
            pending_download,
            None,
        );
    }

    if is_radio_input(runtime, handle) {
        let snapshot = radio_group_checked_snapshot(runtime, handle);
        let already_checked = runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(Element::checked);
        if !already_checked {
            let runtime = unsafe { &mut *runtime_ptr };
            let _ = runtime.set_checked_state(scope, runtime_ptr, handle, true);
        }
        if let Some(outcome) = dispatch_click_event(
            scope,
            runtime_ptr,
            handle,
            x,
            y,
            click_detail,
            button,
            buttons,
            modifiers,
            user_initiated,
        ) && !outcome.allows_default()
        {
            let runtime = unsafe { &mut *runtime_ptr };
            for (radio, checked) in snapshot {
                let _ = runtime.set_checked_state(scope, runtime_ptr, radio, checked);
            }
            return input_dispatch_outcome_after_click(
                runtime_ptr,
                false,
                had_pending_top_level_navigation_before_click,
                None,
                None,
            );
        }
        if !already_checked {
            dispatch_input_and_change_events(scope, runtime_ptr, handle);
        }
        let pending_child_navigations_before_default =
            unsafe { &*runtime_ptr }.pending_live_child_browsing_context_navigation_snapshot();
        let pending_download = perform_click_default_action(
            scope,
            runtime_ptr,
            handle,
            x,
            y,
            button,
            modifiers,
            user_initiated,
            &pending_child_navigations_before_default,
        );
        return input_dispatch_outcome_after_click(
            runtime_ptr,
            true,
            had_pending_top_level_navigation_before_click,
            pending_download,
            None,
        );
    }

    let pending_child_navigations_before_click =
        unsafe { &*runtime_ptr }.pending_live_child_browsing_context_navigation_snapshot();
    if let Some(outcome) = dispatch_click_event(
        scope,
        runtime_ptr,
        handle,
        x,
        y,
        click_detail,
        button,
        buttons,
        modifiers,
        user_initiated,
    ) {
        let _listener_threw = outcome.had_exception();
        if !outcome.allows_default() {
            return input_dispatch_outcome_after_click(
                runtime_ptr,
                false,
                had_pending_top_level_navigation_before_click,
                None,
                None,
            );
        }
        let mut pending_file_chooser = None;
        let pending_download =
            click_activation_default_action_handle(unsafe { &*runtime_ptr }, handle).and_then(
                |default_action| match default_action {
                    ClickActivationDefaultAction::Element(handle) => perform_click_default_action(
                        scope,
                        runtime_ptr,
                        handle,
                        x,
                        y,
                        button,
                        modifiers,
                        user_initiated,
                        &pending_child_navigations_before_click,
                    ),
                    ClickActivationDefaultAction::LabelControl(control) => {
                        let control_outcome = click_handle_internal(
                            scope,
                            runtime_ptr,
                            control,
                            0.0,
                            0.0,
                            0,
                            0,
                            0,
                            modifiers,
                            user_initiated,
                        );
                        pending_file_chooser = control_outcome.pending_file_chooser;
                        control_outcome.pending_download
                    }
                },
            );
        return input_dispatch_outcome_after_click(
            runtime_ptr,
            true,
            had_pending_top_level_navigation_before_click,
            pending_download,
            pending_file_chooser.or_else(|| {
                perform_file_chooser_default_action_with_source(
                    scope,
                    runtime_ptr,
                    handle,
                    file_chooser_source,
                )
            }),
        );
    }
    RendererInputDispatchOutcome {
        handled: true,
        triggered_top_level_navigation: false,
        pending_download: None,
        pending_file_chooser: None,
    }
}

fn input_dispatch_outcome_after_click(
    runtime_ptr: *mut JsContextHost,
    handled: bool,
    had_pending_top_level_navigation_before_click: bool,
    pending_download: Option<RendererPendingDownloadActivation>,
    pending_file_chooser: Option<crate::RendererPendingFileChooserActivation>,
) -> RendererInputDispatchOutcome {
    RendererInputDispatchOutcome {
        handled,
        triggered_top_level_navigation: !had_pending_top_level_navigation_before_click
            && unsafe { &*runtime_ptr }.has_pending_location_navigation(),
        pending_download,
        pending_file_chooser,
    }
}

fn click_activation_default_action_handle(
    runtime: &JsContextHost,
    target: DomHandle,
) -> Option<ClickActivationDefaultAction> {
    if let Some(control) = label_activation_control_handle(runtime, target) {
        return Some(ClickActivationDefaultAction::LabelControl(control));
    }

    let mut current = Some(target);
    while let Some(handle) = current {
        if is_disabled_form_control(runtime, handle) {
            return None;
        }
        if element_has_click_activation_behavior(runtime, handle) {
            return Some(ClickActivationDefaultAction::Element(handle));
        }
        current = runtime.dom_host().parent_node(handle);
    }
    None
}

enum ClickActivationDefaultAction {
    Element(DomHandle),
    LabelControl(DomHandle),
}

fn element_has_click_activation_behavior(runtime: &JsContextHost, handle: DomHandle) -> bool {
    element_is_anchor_with_href(runtime, handle)
        || is_html_option_element(runtime, handle)
        || is_valid_submit_button(runtime, handle)
        || is_valid_reset_button(runtime, handle)
        || runtime.dom_host().is_html_element_named(handle, "button")
        || runtime.dom_host().is_html_element_named(handle, "summary")
}

fn element_has_activation_behavior(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    element.is_html_input()
        || element.is_html_button()
        || element.is_html_label()
        || element.is_html_element("summary")
        || element_is_anchor_with_href(runtime, handle)
        || (element.is_html_element("link") && element_has_attribute(runtime, handle, "href"))
}

pub(crate) fn dispatched_click_activation_target(
    runtime: &JsContextHost,
    target: DomHandle,
    bubbles: bool,
    composed: bool,
) -> Option<DomHandle> {
    runtime
        .build_propagation_path(EventTargetHandle::Node(target), composed)
        .into_iter()
        .enumerate()
        .take_while(|(index, _)| *index == 0 || bubbles)
        .find_map(|(_, candidate)| match candidate {
            EventTargetHandle::Node(handle) if element_has_activation_behavior(runtime, handle) => {
                Some(handle)
            }
            EventTargetHandle::Window
            | EventTargetHandle::ChildWindow(_)
            | EventTargetHandle::Node(_) => None,
        })
}

fn synthetic_click_focuses_element(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| {
            matches!(
                element.local_name(),
                "button" | "input" | "select" | "textarea"
            )
        })
}

pub(crate) fn activate_handle_via_click(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
) -> RendererInputDispatchOutcome {
    click_handle_internal(
        scope,
        runtime_ptr,
        handle,
        x,
        y,
        button,
        buttons,
        0,
        0,
        true,
    )
}

pub(crate) fn activate_handle_via_click_with_detail_and_modifiers(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
    click_detail: i32,
    modifiers: u8,
) -> RendererInputDispatchOutcome {
    click_handle_internal(
        scope,
        runtime_ptr,
        handle,
        x,
        y,
        button,
        buttons,
        click_detail,
        modifiers,
        true,
    )
}

pub(crate) fn activate_handle_via_synthetic_click(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
) -> RendererInputDispatchOutcome {
    click_handle_internal(
        scope,
        runtime_ptr,
        handle,
        x,
        y,
        button,
        buttons,
        0,
        0,
        false,
    )
}

pub(crate) fn perform_click_default_action_for_dispatched_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    event: v8::Local<'_, v8::Object>,
) {
    let runtime = unsafe { &*runtime_ptr };
    if is_disabled_form_control(runtime, handle) {
        return;
    }
    let x = object_number_property(scope, event, "clientX").unwrap_or(0.0);
    let y = object_number_property(scope, event, "clientY").unwrap_or(0.0);
    let button = object_number_property(scope, event, "button").unwrap_or(0.0) as i32;
    let modifiers = mouse_event_modifier_bits(scope, event);
    let _ = perform_click_default_action(
        scope,
        runtime_ptr,
        handle,
        x,
        y,
        button,
        modifiers,
        false,
        &[],
    );
}

pub(crate) enum DispatchedClickLegacyActivation {
    Checkbox {
        handle: DomHandle,
        previous: bool,
        previous_indeterminate: bool,
    },
    Radio {
        handle: DomHandle,
        snapshot: Vec<(DomHandle, bool)>,
        already_checked: bool,
    },
}

pub(crate) fn prepare_legacy_activation_for_dispatched_click(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<DispatchedClickLegacyActivation> {
    let runtime = unsafe { &*runtime_ptr };
    if is_checkbox_input(runtime, handle) {
        let (previous, previous_indeterminate) = runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .map(|element| (element.checked(), element.indeterminate()))
            .unwrap_or_default();
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.set_checked_state(scope, runtime_ptr, handle, !previous);
        let _ = runtime.set_indeterminate_state(scope, runtime_ptr, handle, false);
        return Some(DispatchedClickLegacyActivation::Checkbox {
            handle,
            previous,
            previous_indeterminate,
        });
    }
    if is_radio_input(runtime, handle) {
        let snapshot = radio_group_checked_snapshot(runtime, handle);
        let already_checked = runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(Element::checked);
        if !already_checked {
            let runtime = unsafe { &mut *runtime_ptr };
            let _ = runtime.set_checked_state(scope, runtime_ptr, handle, true);
        }
        return Some(DispatchedClickLegacyActivation::Radio {
            handle,
            snapshot,
            already_checked,
        });
    }
    None
}

pub(crate) fn finish_legacy_activation_for_dispatched_click(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    activation: DispatchedClickLegacyActivation,
    allows_default: bool,
) {
    match activation {
        DispatchedClickLegacyActivation::Checkbox {
            handle,
            previous,
            previous_indeterminate,
        } => {
            if !allows_default {
                let runtime = unsafe { &mut *runtime_ptr };
                let _ = runtime.set_checked_state(scope, runtime_ptr, handle, previous);
                let _ = runtime.set_indeterminate_state(
                    scope,
                    runtime_ptr,
                    handle,
                    previous_indeterminate,
                );
                return;
            }
            dispatch_input_and_change_events(scope, runtime_ptr, handle);
        }
        DispatchedClickLegacyActivation::Radio {
            handle,
            snapshot,
            already_checked,
        } => {
            if !allows_default {
                let runtime = unsafe { &mut *runtime_ptr };
                for (radio, checked) in snapshot {
                    let _ = runtime.set_checked_state(scope, runtime_ptr, radio, checked);
                }
                return;
            }
            if !already_checked {
                dispatch_input_and_change_events(scope, runtime_ptr, handle);
            }
        }
    }
}

fn dispatch_input_and_change_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    if !unsafe { &*runtime_ptr }.dom_host().is_connected(handle) {
        return;
    }
    if let Some(event) = construct_simple_event(scope, "input", true, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    if let Some(event) = construct_simple_event(scope, "change", true, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
}

fn mouse_event_modifier_bits(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) -> u8 {
    let mut modifiers = 0;
    if object_bool_property(scope, event, "altKey").unwrap_or(false) {
        modifiers |= 1;
    }
    if object_bool_property(scope, event, "ctrlKey").unwrap_or(false) {
        modifiers |= 2;
    }
    if object_bool_property(scope, event, "metaKey").unwrap_or(false) {
        modifiers |= 4;
    }
    if object_bool_property(scope, event, "shiftKey").unwrap_or(false) {
        modifiers |= 8;
    }
    modifiers
}

fn perform_click_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    x: f64,
    y: f64,
    button: i32,
    modifiers: u8,
    user_initiated: bool,
    pending_child_navigations_before_default: &[(DomHandle, ChildBrowsingContextBootstrap)],
) -> Option<RendererPendingDownloadActivation> {
    let runtime = unsafe { &*runtime_ptr };
    if element_is_anchor_with_href(runtime, handle) {
        return anchor_click_default_action(
            scope,
            runtime_ptr,
            handle,
            button,
            modifiers,
            user_initiated,
            pending_child_navigations_before_default,
        );
    }
    if perform_option_click_default_action(scope, runtime_ptr, handle) {
        return None;
    }
    if perform_summary_click_default_action(scope, runtime_ptr, handle) {
        return None;
    }
    if let Some(control) = label_activation_control_handle(runtime, handle) {
        let _ = click_handle_internal(
            scope,
            runtime_ptr,
            control,
            0.0,
            0.0,
            0,
            0,
            0,
            modifiers,
            user_initiated,
        );
        return None;
    }
    dispatch_button_command_event_if_needed(scope, runtime_ptr, handle);
    if is_valid_submit_button(runtime, handle)
        && let Some(form_handle) = form_associated_form_owner(runtime, handle)
    {
        let previous = if is_image_submit_button(runtime, handle) {
            let (local_x, local_y) = match image_submitter_coordinate(runtime, handle, x, y) {
                Ok(coordinates) => coordinates,
                Err(error) => {
                    throw_activation_layout_error(
                        scope,
                        "resolving image submit coordinates",
                        error,
                    );
                    return None;
                }
            };
            let runtime = unsafe { &mut *runtime_ptr };
            runtime.replace_active_image_submitter_coordinate(Some((handle, local_x, local_y)))
        } else {
            None
        };
        let _ = submit_form_with_submit_event(scope, runtime_ptr, form_handle, Some(handle), true);
        if is_image_submit_button(unsafe { &*runtime_ptr }, handle) {
            let runtime = unsafe { &mut *runtime_ptr };
            let _ = runtime.replace_active_image_submitter_coordinate(previous);
        }
        return None;
    }
    if is_valid_reset_button(runtime, handle)
        && let Some(form_handle) = form_associated_form_owner(runtime, handle)
    {
        if let Some(event) = construct_simple_event(scope, "reset", true, true, false)
            && dispatch_public_event(scope, runtime_ptr, form_handle, event).allows_default()
        {
            let _ = reset_form_default_action(
                scope,
                runtime_ptr,
                form_handle,
                FormAssociatedResetCallbackTiming::Microtask,
            );
        }
        return None;
    }
    if dispatch_button_popover_toggle_events_if_needed(scope, runtime_ptr, handle) {
        return None;
    }
    if perform_popover_invoker_default_action(scope, runtime_ptr, handle) {
        return None;
    }
    None
}

fn perform_option_click_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let Some(select_handle) = owner_select_for_option(runtime, handle) else {
        return false;
    };
    if is_disabled_form_control(runtime, handle) || is_disabled_form_control(runtime, select_handle)
    {
        return true;
    }
    let was_selected = runtime
        .dom_host()
        .select_selected_option_elements(select_handle)
        .contains(&handle);
    let is_multiple = element_has_attribute(runtime, select_handle, "multiple");
    let mut changed = false;
    {
        let runtime = unsafe { &mut *runtime_ptr };
        if is_multiple {
            changed = runtime.set_selected_state(scope, runtime_ptr, handle, !was_selected);
        } else if !was_selected {
            for option in runtime.dom_host().select_option_elements(select_handle) {
                changed |= runtime.set_selected_state(scope, runtime_ptr, option, option == handle);
            }
            let _ = runtime.set_select_explicit_none(scope, runtime_ptr, select_handle, false);
        }
    }
    if changed {
        if let Some(event) = construct_simple_event(scope, "input", true, false, false) {
            let _ = dispatch_public_event(scope, runtime_ptr, select_handle, event);
        }
        if let Some(event) = construct_simple_event(scope, "change", true, false, false) {
            let _ = dispatch_public_event(scope, runtime_ptr, select_handle, event);
        }
    }
    true
}

fn owner_select_for_option(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    if !is_html_option_element(runtime, handle) {
        return None;
    }
    let mut current = runtime.dom_host().parent_node(handle);
    while let Some(parent) = current {
        if runtime
            .dom_host()
            .node(parent)
            .and_then(Node::as_element)
            .is_some_and(Element::is_html_select)
        {
            return Some(parent);
        }
        current = runtime.dom_host().parent_node(parent);
    }
    None
}

fn is_html_option_element(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(Element::is_html_option)
}

fn element_is_anchor_with_href(runtime: &JsContextHost, handle: DomHandle) -> bool {
    if !element_has_attribute(runtime, handle, "href") {
        return false;
    }
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    matches!(
        (element.namespace(), element.local_name()),
        ("http://www.w3.org/1999/xhtml", "a" | "area") | ("http://www.w3.org/2000/svg", "a")
    )
}

fn dispatch_button_command_event_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.dom_host().is_html_element_named(handle, "button") {
        return;
    }
    let Some(command) =
        element_attribute(runtime, handle, "command").filter(|value| !value.is_empty())
    else {
        return;
    };
    let Some(target) = command_for_element_target(scope, runtime_ptr, handle) else {
        return;
    };
    let Some(source) = node_wrapper_from_handle(scope, handle) else {
        return;
    };
    let Some(event) = construct_command_event(scope, &command, source.into()) else {
        return;
    };
    if dispatch_public_event(scope, runtime_ptr, target, event).allows_default()
        && command.eq_ignore_ascii_case("toggle-popover")
    {
        dispatch_popover_toggle_events(scope, runtime_ptr, target, handle);
    }
}

fn dispatch_button_popover_toggle_events_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.dom_host().is_html_element_named(handle, "button") {
        return false;
    }
    let Some(target) = popover_target_element_target(scope, runtime_ptr, handle) else {
        return false;
    };
    dispatch_popover_toggle_events(scope, runtime_ptr, target, handle);
    true
}

fn popover_target_element_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    if let Some(target) =
        unsafe { &*runtime_ptr }.button_element_target(handle, BUTTON_POPOVER_TARGET_ELEMENT_SLOT)
    {
        return unsafe { &*runtime_ptr }
            .dom_host()
            .resolve_reference_target_chain(target);
    }
    if let Some(wrapper) = node_wrapper_from_handle(scope, handle)
        && let Some(value) = get_private_value(scope, wrapper, BUTTON_POPOVER_TARGET_ELEMENT_SLOT)
        && !value.is_null_or_undefined()
    {
        if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
            let (index, lossless) = big.u64_value();
            if lossless {
                let target = DomHandle::new(index as usize);
                if unsafe { &*runtime_ptr }.dom_host().node(target).is_some() {
                    return unsafe { &*runtime_ptr }
                        .dom_host()
                        .resolve_reference_target_chain(target);
                }
            }
        } else if let Some(index) = value.uint32_value(scope) {
            let target = DomHandle::new(index as usize);
            if unsafe { &*runtime_ptr }.dom_host().node(target).is_some() {
                return unsafe { &*runtime_ptr }
                    .dom_host()
                    .resolve_reference_target_chain(target);
            }
        }
    }
    let runtime = unsafe { &*runtime_ptr };
    let popover_target = element_attribute(runtime, handle, "popovertarget")?;
    if popover_target.is_empty() {
        return None;
    }
    reference_target_for_id(runtime, handle, &popover_target)
}

fn command_for_element_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    if let Some(target) =
        unsafe { &*runtime_ptr }.button_element_target(handle, BUTTON_COMMAND_FOR_ELEMENT_SLOT)
    {
        return unsafe { &*runtime_ptr }
            .dom_host()
            .resolve_reference_target_chain(target);
    }
    if let Some(wrapper) = node_wrapper_from_handle(scope, handle)
        && let Some(value) = get_private_value(scope, wrapper, BUTTON_COMMAND_FOR_ELEMENT_SLOT)
        && !value.is_null_or_undefined()
    {
        if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
            let (index, lossless) = big.u64_value();
            if lossless {
                let target = DomHandle::new(index as usize);
                if unsafe { &*runtime_ptr }.dom_host().node(target).is_some() {
                    return Some(target);
                }
            }
        } else if let Some(index) = value.uint32_value(scope) {
            let target = DomHandle::new(index as usize);
            if unsafe { &*runtime_ptr }.dom_host().node(target).is_some() {
                return Some(target);
            }
        }
    }
    let runtime = unsafe { &*runtime_ptr };
    let command_for = element_attribute(runtime, handle, "commandfor")?;
    if command_for.is_empty() {
        return None;
    }
    reference_target_for_id(runtime, handle, &command_for)
}

fn reference_target_for_id(
    runtime: &JsContextHost,
    handle: DomHandle,
    id: &str,
) -> Option<DomHandle> {
    let root = runtime.dom_host().root_node_handle(handle)?;
    let candidate = runtime
        .dom_host()
        .element_handle_by_id_in_subtree(root, id)?;
    runtime.dom_host().resolve_reference_target_chain(candidate)
}

fn is_valid_reset_button(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| {
            (element.is_html_input() && element.input_type() == "reset")
                || (element.is_html_element("button") && element.attribute("type") == Some("reset"))
        })
}

fn is_image_submit_button(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.is_html_input() && element.input_type() == "image")
}

fn image_submitter_coordinate(
    runtime: &JsContextHost,
    handle: DomHandle,
    client_x: f64,
    client_y: f64,
) -> Result<(u32, u32), moli_layout::LayoutError> {
    let rect = observable_bounding_client_rect(
        runtime,
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )?;
    Ok((
        image_submitter_coordinate_component(client_x - rect.left, rect.width),
        image_submitter_coordinate_component(client_y - rect.top, rect.height),
    ))
}

fn throw_activation_layout_error(
    scope: &mut v8::PinScope<'_, '_>,
    operation: &str,
    error: moli_layout::LayoutError,
) {
    let Some(message) = v8_string(scope, &format!("Layout failed while {operation}: {error}"))
    else {
        return;
    };
    let exception = v8::Exception::error(scope, message);
    scope.throw_exception(exception);
}

fn image_submitter_coordinate_component(value: f64, max: f64) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    value.round().clamp(0.0, max.max(0.0)) as u32
}

fn send_anchor_ping_requests(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    destination_url: &str,
) {
    let Some((base_url, ping_urls)) = anchor_ping_urls(unsafe { &*runtime_ptr }, handle) else {
        return;
    };
    for ping_url in ping_urls
        .split_ascii_whitespace()
        .filter_map(|raw_url| base_url.join(raw_url).ok())
    {
        crate::network_host::send_link_audit_ping(scope, runtime_ptr, ping_url, destination_url);
    }
}

fn anchor_ping_urls(runtime: &JsContextHost, handle: DomHandle) -> Option<(url::Url, String)> {
    let ping_urls = element_attribute(runtime, handle, "ping")?;
    if ping_urls.is_empty() {
        return None;
    }
    if (ping_urls.contains('\n') || ping_urls.contains('\r') || ping_urls.contains('\t'))
        && ping_urls.contains('<')
    {
        return None;
    }
    let base_url = runtime
        .dom_host()
        .document_base_url()
        .unwrap_or_else(|| runtime.host_document().url().clone());
    Some((base_url, ping_urls))
}

pub(in crate::native_bridge) fn perform_file_chooser_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<RendererPendingFileChooserActivation> {
    let source = file_chooser_activation_source(scope, runtime_ptr, handle);
    perform_file_chooser_default_action_with_source(scope, runtime_ptr, handle, source)
}

struct FileChooserActivationSource {
    source_document: RendererDocumentLifecycleIdentity,
    source_frame_id: Option<String>,
    source_node_document_id: DocumentId,
}

fn file_chooser_activation_source(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<FileChooserActivationSource> {
    let runtime = unsafe { &*runtime_ptr };
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    if !element.is_html_input() {
        return None;
    }
    Some(FileChooserActivationSource {
        source_document: runtime.root_document_lifecycle_identity()?,
        source_frame_id: crate::context_bootstrap::current_child_frame_id_for_runtime_scope(
            scope, runtime,
        ),
        source_node_document_id: file_chooser_document_id_for_handle(runtime, handle)?,
    })
}

fn perform_file_chooser_default_action_with_source(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    source: Option<FileChooserActivationSource>,
) -> Option<RendererPendingFileChooserActivation> {
    let (allow_multiple, should_auto_cancel) = {
        let runtime = unsafe { &*runtime_ptr };
        let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
        if !element.is_html_input() || element.input_type() != "file" {
            return None;
        }
        (
            element.attribute("multiple").is_some(),
            runtime.webdriver_bidi_should_auto_cancel_file_chooser(),
        )
    };
    if should_auto_cancel {
        if let Some(event) = construct_simple_event(scope, "cancel", true, false, false) {
            let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
        }
        return None;
    }
    let source = source?;
    Some(RendererPendingFileChooserActivation::from_live_node(
        source.source_document,
        source.source_frame_id,
        allow_multiple,
        handle,
        source.source_node_document_id,
    ))
}

fn file_chooser_document_id_for_handle(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DocumentId> {
    let document_handle = runtime.dom_host().owner_document_handle(handle)?;
    if document_handle == runtime.dom_host().document_handle() {
        return runtime
            .current_main_document_task_owner()
            .map(|owner| owner.document_id);
    }

    let child_handle = runtime.child_browsing_context_host_for_document_handle(document_handle)?;
    let snapshot = runtime.frame_owner_current_child_snapshot(child_handle)?;
    (snapshot.document_handle == document_handle).then_some(snapshot.document_id)
}

fn anchor_click_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    button: i32,
    modifiers: u8,
    user_initiated: bool,
    pending_child_navigations_before_default: &[(DomHandle, ChildBrowsingContextBootstrap)],
) -> Option<RendererPendingDownloadActivation> {
    let resolved = resolve_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href");
    if resolved.is_empty() {
        return None;
    }
    send_anchor_ping_requests(scope, runtime_ptr, handle, &resolved);
    let runtime = unsafe { &*runtime_ptr };
    if element_has_attribute(runtime, handle, "download") {
        let suggested_filename =
            element_attribute(runtime, handle, "download").filter(|value| !value.is_empty());
        return anchor_download_activation(
            scope,
            runtime_ptr,
            handle,
            resolved,
            user_initiated,
            suggested_filename,
        );
    }
    let navigation_policy = hyperlink_navigation_policy(button, modifiers, user_initiated);
    if navigation_policy == HyperlinkNavigationPolicy::Download {
        return anchor_download_activation(
            scope,
            runtime_ptr,
            handle,
            resolved,
            user_initiated,
            None,
        );
    }
    let declared_target_name =
        element_attribute(runtime, handle, "target").filter(|value| !value.is_empty());
    let (target_name, popup_disposition) = match navigation_policy {
        HyperlinkNavigationPolicy::Auxiliary(disposition) => {
            (Some("_blank".to_owned()), disposition)
        }
        HyperlinkNavigationPolicy::Current => {
            (declared_target_name, RendererPopupDisposition::Foreground)
        }
        HyperlinkNavigationPolicy::Download => {
            unreachable!("download hyperlink policy returned before target selection")
        }
    };
    let special_target = target_name
        .as_deref()
        .and_then(SpecialBrowsingContextTarget::parse);
    if (target_name.is_none() || special_target == Some(SpecialBrowsingContextTarget::Current))
        && navigate_hyperlink_source_browsing_context(scope, runtime_ptr, handle, &resolved)
    {
        return None;
    }
    if (target_name.is_none() || special_target == Some(SpecialBrowsingContextTarget::Current))
        && runtime.host_document().url().as_str() == resolved
    {
        let global = scope.get_current_context().global(scope);
        if let Some(location) = global
            .get(scope, v8str(scope, "location").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            let source_element = node_wrapper_from_handle(scope, handle);
            navigate_location_object_with_source_element(
                scope,
                location,
                LocationNavigationKind::Assign,
                Some(resolved),
                source_element,
            );
        }
        return None;
    }
    if (target_name.is_none()
        || matches!(
            special_target,
            Some(
                SpecialBrowsingContextTarget::Current
                    | SpecialBrowsingContextTarget::Top
                    | SpecialBrowsingContextTarget::Parent
            )
        ))
        && let Ok(url) = url::Url::parse(&resolved)
        && same_document_fragment_target(runtime.host_document().url(), &url)
    {
        if let Err(error) = scroll_to_document_fragment_target(scope, runtime_ptr, runtime, &url) {
            throw_activation_layout_error(scope, "scrolling to fragment", error);
            return None;
        }
        let global = scope.get_current_context().global(scope);
        if let Some(location) = global
            .get(scope, v8str(scope, "location").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            let source_element = node_wrapper_from_handle(scope, handle);
            navigate_location_object_with_source_element(
                scope,
                location,
                LocationNavigationKind::Assign,
                Some(resolved),
                source_element,
            );
        }
        return None;
    }
    if target_name.is_none() || special_target == Some(SpecialBrowsingContextTarget::Current) {
        let source_element = node_wrapper_from_handle(scope, handle);
        let can_intercept = url::Url::parse(&resolved)
            .is_ok_and(|url| moli_url::same_origin(runtime.document_url(), &url));
        if !crate::context_bootstrap::dispatch_top_level_navigation_event_with_source_element(
            scope,
            &resolved,
            "push",
            source_element,
            can_intercept,
            user_initiated,
            None,
        ) {
            return None;
        }
    }
    let source_element = node_wrapper_from_handle(scope, handle);
    if click_listener_changed_named_child_navigation(
        scope,
        runtime_ptr,
        handle,
        target_name.as_deref(),
        &resolved,
        pending_child_navigations_before_default,
    ) {
        return None;
    }
    let _ = navigate_hyperlink_target_browsing_context(
        scope,
        runtime_ptr,
        handle,
        target_name.as_deref(),
        &resolved,
        source_element,
        popup_disposition,
    );
    None
}

fn anchor_download_activation(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    resolved: String,
    user_initiated: bool,
    suggested_filename: Option<String>,
) -> Option<RendererPendingDownloadActivation> {
    let source_element = node_wrapper_from_handle(scope, handle);
    let download_request = suggested_filename.as_deref().unwrap_or("");
    let proceed =
        navigation_owner_window_for_handle(scope, runtime_ptr, handle).is_none_or(|owner| {
            crate::context_bootstrap::dispatch_cross_document_navigation_navigate_event_for_window(
                scope,
                owner,
                &resolved,
                source_element,
                user_initiated,
                Some(download_request),
            )
        });
    proceed.then_some(RendererPendingDownloadActivation {
        url: resolved,
        suggested_filename,
        response: None,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HyperlinkNavigationPolicy {
    Current,
    Download,
    Auxiliary(RendererPopupDisposition),
}

fn hyperlink_navigation_policy(
    button: i32,
    modifiers: u8,
    user_initiated: bool,
) -> HyperlinkNavigationPolicy {
    const ALT: u8 = 1;
    const SHIFT: u8 = 8;

    #[cfg(target_os = "macos")]
    let platform_new_tab_modifier = 4;
    #[cfg(not(target_os = "macos"))]
    let platform_new_tab_modifier = 2;

    let requests_new_tab = button == 1 || modifiers & platform_new_tab_modifier != 0;
    let shift = modifiers & SHIFT != 0;
    let alt = modifiers & ALT != 0;
    if requests_new_tab {
        let disposition = if shift || !user_initiated {
            RendererPopupDisposition::Foreground
        } else {
            RendererPopupDisposition::Background
        };
        HyperlinkNavigationPolicy::Auxiliary(disposition)
    } else if shift {
        // The renderer target model has no separate window chrome. Preserve
        // Chromium's selected-surface behavior by folding a new window into a
        // foreground auxiliary target.
        HyperlinkNavigationPolicy::Auxiliary(RendererPopupDisposition::Foreground)
    } else if alt && user_initiated {
        HyperlinkNavigationPolicy::Download
    } else {
        HyperlinkNavigationPolicy::Current
    }
}

fn current_click_popup_disposition(
    button: i32,
    modifiers: u8,
    user_initiated: bool,
) -> RendererPopupDisposition {
    match hyperlink_navigation_policy(button, modifiers, user_initiated) {
        HyperlinkNavigationPolicy::Auxiliary(disposition) => disposition,
        HyperlinkNavigationPolicy::Current | HyperlinkNavigationPolicy::Download => {
            RendererPopupDisposition::Foreground
        }
    }
}

fn click_listener_changed_named_child_navigation(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    target_name: Option<&str>,
    resolved: &str,
    pending_before: &[(DomHandle, ChildBrowsingContextBootstrap)],
) -> bool {
    let Some(target_name) = target_name else {
        return false;
    };
    if SpecialBrowsingContextTarget::parse(target_name).is_some() {
        return false;
    }
    let source_document = {
        let runtime = unsafe { &*runtime_ptr };
        runtime.dom_host().owner_document_handle(handle)
    };
    let Some(target_handle) =
        named_iframe_target_handle_for_navigation(scope, runtime_ptr, target_name, source_document)
    else {
        return false;
    };
    if !anchor_default_navigation_yields_to_pending_child_navigation(
        runtime_ptr,
        handle,
        target_handle,
        resolved,
    ) {
        return false;
    }
    let Some(current) =
        unsafe { &*runtime_ptr }.child_browsing_context_pending_live_navigation(target_handle)
    else {
        return false;
    };
    pending_before
        .iter()
        .find(|(handle, _)| *handle == target_handle)
        .is_none_or(|(_, pending)| pending != &current)
}

fn anchor_default_navigation_yields_to_pending_child_navigation(
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    target_handle: DomHandle,
    resolved: &str,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let raw_href = element_attribute(runtime, handle, "href").unwrap_or_default();
    if raw_href.trim_start().starts_with('#') {
        return true;
    }
    let Ok(url) = url::Url::parse(resolved) else {
        return false;
    };
    if url.scheme() == "javascript" {
        let script = resolved
            .strip_prefix("javascript:")
            .or_else(|| resolved.strip_prefix("JAVASCRIPT:"))
            .unwrap_or_default()
            .trim();
        return matches!(script, "void(0)" | "void 0" | "undefined");
    }
    runtime
        .child_browsing_context_document_handle(target_handle)
        .map(|document| runtime.document_url_for_handle(document))
        .is_some_and(|current| same_document_fragment_target(&current, &url))
}

fn navigation_owner_window_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    let runtime = unsafe { &*runtime_ptr };
    let document_handle = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::owner_document)?;
    if document_handle == runtime.document_handle() {
        return Some(scope.get_current_context().global(scope));
    }
    if let Some(popup_id) = runtime.lightweight_popup_id_for_document_handle(document_handle) {
        return runtime.lightweight_popup_window(scope, popup_id);
    }
    let child_handle =
        runtime.child_browsing_context_handle_by_document_handle(scope, document_handle)?;
    runtime.existing_child_browsing_context_window_wrapper(scope, child_handle)
}

fn same_document_fragment_target(current: &url::Url, target: &url::Url) -> bool {
    let mut current_without_fragment = current.clone();
    current_without_fragment.set_fragment(None);
    let mut target_without_fragment = target.clone();
    target_without_fragment.set_fragment(None);
    current_without_fragment == target_without_fragment && target.fragment().is_some()
}

pub(crate) fn scroll_to_url_fragment_or_top(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target_url: &str,
) -> Result<(), moli_layout::LayoutError> {
    let runtime = unsafe { &*runtime_ptr };
    let Ok(target_url) = url::Url::parse(target_url) else {
        return Ok(());
    };
    if target_url.fragment().is_none() {
        crate::window_host::scroll_window_to(scope, runtime_ptr, 0.0, 0.0);
        return Ok(());
    }
    if !scroll_to_document_fragment_target(scope, runtime_ptr, runtime, &target_url)? {
        crate::window_host::scroll_window_to(scope, runtime_ptr, 0.0, 0.0);
    }
    Ok(())
}

fn scroll_to_document_fragment_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    runtime: &JsContextHost,
    target_url: &url::Url,
) -> Result<bool, moli_layout::LayoutError> {
    let Some(fragment) = target_url.fragment() else {
        return Ok(false);
    };
    let fragment = percent_encoding::percent_decode_str(fragment)
        .decode_utf8_lossy()
        .into_owned();
    let Some(target) = document_tree_fragment_target(runtime, &fragment) else {
        return Ok(false);
    };
    let _ = scroll_node_into_view_at_start(scope, runtime_ptr, target)?;
    Ok(true)
}

fn document_tree_fragment_target(runtime: &JsContextHost, fragment: &str) -> Option<DomHandle> {
    let mut stack = Vec::new();
    let mut current = Some(runtime.dom_host().document_handle());
    while let Some(handle) = current {
        if !runtime.dom_host().is_shadow_root(handle)
            && runtime
                .dom_host()
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element_matches_fragment_target(element, fragment))
        {
            return Some(handle);
        }
        if let Some(sibling) = runtime.dom_host().next_sibling(handle) {
            stack.push(sibling);
        }
        current = runtime
            .dom_host()
            .first_child(handle)
            .filter(|child| !runtime.dom_host().is_shadow_root(*child))
            .or_else(|| stack.pop());
    }
    None
}

fn element_matches_fragment_target(element: &Element, fragment: &str) -> bool {
    element.id().is_some_and(|id| id == fragment)
        || (element.is_html_element("a")
            && element
                .attribute("name")
                .is_some_and(|name| name == fragment))
}

pub(crate) fn perform_drop_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    data_transfer: v8::Local<'_, v8::Object>,
) -> bool {
    let (is_file_input, allow_multiple, is_text_target, editing_host, focus_handle) = {
        let runtime = unsafe { &*runtime_ptr };
        if is_disabled_form_control(runtime, handle) {
            return false;
        }
        let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
            return false;
        };
        let is_file_input = element.is_html_input() && element.input_type() == "file";
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
        update_focus(scope, runtime_ptr, Some(focus_handle));
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

pub(in crate::native_bridge) fn navigate_form_target_browsing_context(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    target_name: Option<&str>,
    resolved_url: &str,
) -> bool {
    let special_target = target_name.and_then(SpecialBrowsingContextTarget::parse);
    let exposes_opener = {
        let runtime = unsafe { &*runtime_ptr };
        let rel = runtime
            .dom_host()
            .node(form_handle)
            .and_then(Node::as_element)
            .and_then(|element| element.attribute("rel"))
            .unwrap_or_default();
        let mut has_opener = false;
        let mut has_noopener = false;
        let mut has_noreferrer = false;
        for token in rel.split_ascii_whitespace() {
            if token.eq_ignore_ascii_case("opener") {
                has_opener = true;
            } else if token.eq_ignore_ascii_case("noopener") {
                has_noopener = true;
            } else if token.eq_ignore_ascii_case("noreferrer") {
                has_noreferrer = true;
            }
        }
        !has_noreferrer
            && !has_noopener
            && (has_opener || special_target != Some(SpecialBrowsingContextTarget::Blank))
    };
    if target_name.is_none() || special_target == Some(SpecialBrowsingContextTarget::Current) {
        let runtime = unsafe { &*runtime_ptr };
        let document_handle = runtime
            .dom_host()
            .node(form_handle)
            .and_then(Node::owner_document);
        if let Some(document_handle) = document_handle
            && document_handle != runtime.document_handle()
            && let Some(child_handle) =
                runtime.child_browsing_context_handle_by_document_handle(scope, document_handle)
        {
            let runtime = unsafe { &mut *runtime_ptr };
            return runtime.navigate_child_browsing_context_to_url(
                scope,
                child_handle,
                resolved_url,
            );
        }
        let source_element = node_wrapper_from_handle(scope, form_handle);
        if !crate::context_bootstrap::dispatch_top_level_navigation_event_with_source_element(
            scope,
            resolved_url,
            "replace",
            source_element,
            true,
            false,
            None,
        ) {
            return true;
        }
        let Ok(url) = url::Url::parse(resolved_url) else {
            return false;
        };
        unsafe { &mut *runtime_ptr }.record_pending_location_navigation(url, None);
        return true;
    }
    navigate_target_browsing_context(
        scope,
        runtime_ptr,
        target_name,
        resolved_url,
        None,
        exposes_opener,
    )
}

#[cfg(test)]
mod hyperlink_popup_disposition_tests {
    use super::*;

    const CTRL: u8 = 2;
    const META: u8 = 4;
    const SHIFT: u8 = 8;
    const ALT: u8 = 1;

    #[cfg(target_os = "macos")]
    const PLATFORM_NEW_TAB_MODIFIER: u8 = META;
    #[cfg(not(target_os = "macos"))]
    const PLATFORM_NEW_TAB_MODIFIER: u8 = CTRL;

    #[cfg(target_os = "macos")]
    const NON_PLATFORM_NEW_TAB_MODIFIER: u8 = CTRL;
    #[cfg(not(target_os = "macos"))]
    const NON_PLATFORM_NEW_TAB_MODIFIER: u8 = META;

    #[test]
    fn ordinary_click_uses_the_declared_target() {
        assert_eq!(
            hyperlink_navigation_policy(0, 0, true),
            HyperlinkNavigationPolicy::Current
        );
        assert_eq!(
            hyperlink_navigation_policy(0, NON_PLATFORM_NEW_TAB_MODIFIER, true),
            HyperlinkNavigationPolicy::Current
        );
    }

    #[test]
    fn trusted_new_context_input_matches_chromium_surface_selection() {
        assert_eq!(
            hyperlink_navigation_policy(1, 0, true),
            HyperlinkNavigationPolicy::Auxiliary(RendererPopupDisposition::Background)
        );
        assert_eq!(
            hyperlink_navigation_policy(0, PLATFORM_NEW_TAB_MODIFIER, true),
            HyperlinkNavigationPolicy::Auxiliary(RendererPopupDisposition::Background)
        );
        assert_eq!(
            hyperlink_navigation_policy(0, SHIFT, true),
            HyperlinkNavigationPolicy::Auxiliary(RendererPopupDisposition::Foreground)
        );
        assert_eq!(
            hyperlink_navigation_policy(1, SHIFT, true),
            HyperlinkNavigationPolicy::Auxiliary(RendererPopupDisposition::Foreground)
        );
    }

    #[test]
    fn synthetic_new_tab_input_cannot_create_a_background_tab_under() {
        assert_eq!(
            hyperlink_navigation_policy(1, 0, false),
            HyperlinkNavigationPolicy::Auxiliary(RendererPopupDisposition::Foreground)
        );
        assert_eq!(
            hyperlink_navigation_policy(0, PLATFORM_NEW_TAB_MODIFIER, false),
            HyperlinkNavigationPolicy::Auxiliary(RendererPopupDisposition::Foreground)
        );
    }

    #[test]
    fn only_trusted_alt_click_requests_a_download() {
        assert_eq!(
            hyperlink_navigation_policy(0, ALT, true),
            HyperlinkNavigationPolicy::Download
        );
        assert_eq!(
            hyperlink_navigation_policy(0, ALT, false),
            HyperlinkNavigationPolicy::Current
        );
    }
}
