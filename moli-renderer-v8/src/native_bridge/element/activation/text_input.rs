use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

use super::super::events::construct_text_input_event_for_target;
use super::super::forms::{
    is_text_control_user_editable, replace_text_control_selection_after_beforeinput,
};
use super::super::{
    TextEditInputType, contenteditable_editing_host, dispatch_beforeinput, dispatch_public_event,
    is_text_control, update_focus,
};
use super::default_action::replace_contenteditable_selection_after_beforeinput;

#[derive(Clone, Copy)]
pub(crate) enum TextInputSource {
    Keyboard,
    InputMethod,
}

/// Native text entry has two cancelable boundaries before its editing default.
/// Script commands, drops and deletion keep their own existing event sequences.
pub(crate) fn perform_text_input(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    initial_target: DomHandle,
    text: &str,
    input_type: TextEditInputType,
    source: TextInputSource,
) -> bool {
    let Some(context) = crate::native_bridge::node_owner_document_relevant_context(
        scope,
        runtime_ptr,
        initial_target,
    ) else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(document) = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(initial_target)
    else {
        return false;
    };
    let runtime = unsafe { &*runtime_ptr };
    let editing_host = contenteditable_editing_host(runtime, initial_target);
    if !is_text_control(runtime, initial_target) && editing_host.is_none() {
        return false;
    }
    if text.is_empty() {
        let collapsed = if is_text_control(runtime, initial_target) {
            runtime
                .dom_host()
                .node(initial_target)
                .and_then(|node| node.as_element())
                .is_none_or(|element| element.selection_start() == element.selection_end())
        } else {
            let window = scope.get_current_context().global(scope);
            crate::context_bootstrap::window_selection_is_collapsed(scope, window)
        };
        if collapsed {
            return true;
        }
    }
    // IME keeps its original composition caret across beforeinput and legacy
    // listeners, even when beforeinput changes focus within this Document.
    let caret_start = if is_text_control(runtime, initial_target)
        && matches!(source, TextInputSource::InputMethod)
    {
        runtime
            .dom_host()
            .node(initial_target)
            .and_then(|node| node.as_element())
            .map(|element| element.selection_start())
    } else {
        None
    };
    let beforeinput_target = editing_host.unwrap_or(initial_target);
    if !dispatch_beforeinput(
        scope,
        runtime_ptr,
        beforeinput_target,
        input_type,
        input_type.data(text),
    ) {
        return false;
    }
    let runtime = unsafe { &*runtime_ptr };
    if runtime.dom_host().owner_document_handle(initial_target) != Some(document) {
        return false;
    }
    // Physical characters retain their keypress target. IME commits resolve
    // focus after beforeinput, like Blink's composition-confirm insertion.
    let target = match source {
        TextInputSource::Keyboard => Some(initial_target),
        TextInputSource::InputMethod => runtime
            .active_element_handle()
            .or_else(|| runtime.document_focus_fallback_handle()),
    };
    let Some(target) =
        target.filter(|target| runtime.dom_host().owner_document_handle(*target) == Some(document))
    else {
        return false;
    };
    let was_text_control = is_text_control(runtime, target);
    let Some(event) = construct_text_input_event_for_target(scope, runtime_ptr, target, text)
    else {
        return false;
    };
    if !dispatch_public_event(scope, runtime_ptr, target, event).allows_default() {
        return false;
    }
    let runtime = unsafe { &*runtime_ptr };
    if runtime.dom_host().owner_document_handle(target) != Some(document)
        || !runtime.dom_host().is_connected(target)
        || runtime.owner_dispatch_scope_for_node(target).is_none()
    {
        return false;
    }
    // Text controls preserve their event target's selection even if listeners
    // move focus. An editing host instead uses the live focused selection.
    let edit_target = if was_text_control {
        target
    } else {
        runtime
            .active_element_handle()
            .filter(|active| runtime.dom_host().owner_document_handle(*active) == Some(document))
            .unwrap_or(target)
    };
    if is_text_control(runtime, edit_target) {
        if is_text_control_user_editable(runtime, edit_target) {
            update_focus(scope, runtime_ptr, Some(edit_target));
            // Focus callbacks are another author-code boundary: they can move
            // the control into a different Document before the edit commits.
            let runtime = unsafe { &*runtime_ptr };
            if runtime.dom_host().owner_document_handle(edit_target) != Some(document)
                || runtime.owner_dispatch_scope_for_node(edit_target).is_none()
            {
                return false;
            }
        }
        return replace_text_control_selection_after_beforeinput(
            scope,
            runtime_ptr,
            edit_target,
            text,
            input_type,
            caret_start,
        );
    }
    let Some(editing_host) = contenteditable_editing_host(runtime, edit_target) else {
        return false;
    };
    replace_contenteditable_selection_after_beforeinput(
        scope,
        runtime_ptr,
        editing_host,
        text,
        input_type,
    )
}
