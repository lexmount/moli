use super::*;
use crate::page_task_queue::RendererPageUserInteractionEventKind;

pub(crate) fn dispatch_text_control_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    event_type: &str,
) {
    let composed = matches!(event_type, "beforeinput" | "input" | "selectionchange");
    if let Some(event) = construct_simple_event(scope, event_type, true, false, composed) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
}

pub(crate) fn queue_text_control_selection_change_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
) {
    let _ = unsafe { &mut *runtime_ptr }.queue_user_interaction_event_task(
        scope,
        RendererPageUserInteractionEventKind::TextControlSelectionChange,
        handle,
    );
}

pub(crate) fn queue_text_control_document_selection_change_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
) {
    let _ = unsafe { &mut *runtime_ptr }.queue_user_interaction_event_task(
        scope,
        RendererPageUserInteractionEventKind::DocumentSelectionChange,
        handle,
    );
}

pub(crate) fn queue_text_control_select_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
) {
    let _ = unsafe { &mut *runtime_ptr }.queue_user_interaction_event_task(
        scope,
        RendererPageUserInteractionEventKind::TextControlSelect,
        handle,
    );
}
