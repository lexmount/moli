use crate::{
    document_runtime::DomHandle,
    page_task_queue::{RendererPageElementToggleEventKind, RendererPageElementToggleEventState},
};

use super::JsContextHost;

/// Admit one concrete element task into the production DOM-manipulation source.
///
/// Coalescing, cancellation and exact-Document capture live in `JsContextHost`;
/// this element-facing helper deliberately has no timer or local fallback.
pub(super) fn queue_element_toggle_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    kind: RendererPageElementToggleEventKind,
    handle: DomHandle,
    old_state: RendererPageElementToggleEventState,
    new_state: RendererPageElementToggleEventState,
    source: Option<DomHandle>,
) {
    let _ = unsafe { &mut *runtime_ptr }
        .queue_element_toggle_event(scope, kind, handle, old_state, new_state, source);
}

pub(super) fn cancel_element_toggle_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    kind: RendererPageElementToggleEventKind,
    handle: DomHandle,
) {
    let _ = unsafe { &mut *runtime_ptr }.cancel_element_toggle_event(scope, kind, handle);
}
