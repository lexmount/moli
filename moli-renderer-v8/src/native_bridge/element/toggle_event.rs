use crate::{
    document_runtime::DomHandle,
    page_task_queue::{RendererPageElementToggleEventKind, RendererPageElementToggleEventState},
    util::node_wrapper_from_handle,
};

use super::JsContextHost;

pub(super) fn construct_toggle_event_with_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    old_state: &str,
    new_state: &str,
    cancelable: bool,
    source: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Object>> {
    let source = source
        .and_then(|handle| node_wrapper_from_handle(scope, handle).map(Into::into))
        .unwrap_or_else(|| v8::null(scope).into());
    super::events::construct_toggle_event(
        scope, event_type, old_state, new_state, cancelable, source,
    )
}

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
