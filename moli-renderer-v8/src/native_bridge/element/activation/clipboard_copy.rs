use super::clipboard::{dispatch_clipboard_action_event, native_clipboard_text_bytes};
use crate::context_bootstrap::selection_text_for_clipboard;
use crate::dom::{forms::InputType, native::Node};
use crate::native_bridge::element::*;
use crate::util::utf16_units;

pub(crate) fn document_copy_command_supported(
    runtime: &JsContextHost,
    document: DomHandle,
) -> bool {
    document == runtime.document_handle()
        || runtime
            .child_browsing_context_host_for_document_handle(document)
            .is_some_and(|host| runtime.dom_host().is_connected(host))
        || runtime
            .lightweight_popup_id_for_document_handle(document)
            .is_some()
}

fn selected_control(runtime: &JsContextHost, document: DomHandle) -> Option<DomHandle> {
    runtime
        .document_selected_text_control(document)
        .filter(|control| {
            runtime.dom_host().is_connected(*control)
                && runtime.dom_host().owner_document_handle(*control) == Some(document)
        })
}

fn selected_password(runtime: &JsContextHost, document: DomHandle) -> bool {
    selected_control(runtime, document)
        .and_then(|control| runtime.dom_host().node(control))
        .and_then(Node::as_element)
        .is_some_and(|element| {
            element.is_html_input() && element.input_type() == InputType::Password
        })
}

fn copy_event_target(runtime: &JsContextHost, document: DomHandle) -> Option<DomHandle> {
    if runtime.document_selected_text_control(document).is_some() {
        return selected_control(runtime, document).or_else(|| {
            runtime
                .dom_host()
                .document_body_handle_for_document(document)
        });
    }
    let container = runtime
        .document_selection_snapshot(document)
        .map(|selection| selection.start.container);
    container
        .and_then(|container| {
            let node = runtime.dom_host().node(container)?;
            if !node.is_connected() {
                return None;
            }
            if node.is_element() {
                Some(container)
            } else {
                runtime.dom_host().parent_node(container)
            }
        })
        .or_else(|| {
            runtime
                .dom_host()
                .document_body_handle_for_document(document)
        })
}

fn copy_selection_text(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    document: DomHandle,
) -> Option<String> {
    if runtime.document_selected_text_control(document).is_some() {
        let control = selected_control(runtime, document)?;
        let element = runtime.dom_host().node(control)?.as_element()?;
        if selected_password(runtime, document) {
            return None;
        }
        let units = utf16_units(&text_control_value(runtime, control));
        let start = (element.selection_start() as usize).min(units.len());
        let end = (element.selection_end() as usize).min(units.len());
        return (start < end).then(|| String::from_utf16_lossy(&units[start..end]));
    }
    let selection = runtime.document_selection_snapshot(document)?;
    if selection.start == selection.end {
        return None;
    }
    selection_text_for_clipboard(scope).filter(|text| !text.is_empty())
}

pub(crate) fn run_document_copy_command(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document: DomHandle,
    query_enabled: bool,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let activated = runtime.protocol_user_gesture_activation()
        || runtime
            .owner_dispatch_scope_for_node(document)
            .is_some_and(|owner| runtime.window_has_transient_user_activation(owner));
    if !activated || !document_copy_command_supported(runtime, document) {
        return false;
    }
    let context = runtime
        .owner_dispatch_scope_for_node(document)
        .and_then(|target| {
            runtime
                .current_window_execution_context_owner(target)
                .and_then(|owner| runtime.window_execution_context(scope, owner, target))
        })
        .map(|(_, context)| context);
    let Some(context) = context else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);

    // EnabledCopy dispatches beforecopy; execution performs that check too,
    // but a user-initiated copy is still handled when the selection is empty.
    let mut canceled = false;
    if !selected_password(unsafe { &*runtime_ptr }, document)
        && let Some(target) = copy_event_target(unsafe { &*runtime_ptr }, document)
    {
        canceled =
            dispatch_clipboard_action_event(scope, runtime_ptr, target, "beforecopy", &[], false)
                == Some(false);
    }
    if query_enabled {
        return canceled
            || copy_selection_text(scope, unsafe { &*runtime_ptr }, document).is_some();
    }
    if !document_copy_command_supported(unsafe { &*runtime_ptr }, document) {
        return true;
    }
    if selected_password(unsafe { &*runtime_ptr }, document) {
        return true;
    }
    if let Some(target) = copy_event_target(unsafe { &*runtime_ptr }, document) {
        if dispatch_clipboard_action_event(scope, runtime_ptr, target, "copy", &[], false)
            != Some(true)
        {
            return true;
        }
        if let Some(text) = copy_selection_text(scope, unsafe { &*runtime_ptr }, document) {
            unsafe { &*runtime_ptr }
                .browser_context_runtime()
                .set_clipboard_data(vec![(
                    "text/plain".to_owned(),
                    native_clipboard_text_bytes(&text),
                )]);
        }
    }
    true
}
