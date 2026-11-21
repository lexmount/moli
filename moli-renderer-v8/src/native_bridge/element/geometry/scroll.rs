use crate::{document_runtime::DomHandle, dom::native::Node, native_bridge::PageScriptHost};

#[cfg(feature = "devtools")]
mod input;
#[cfg(feature = "devtools")]
pub(crate) use input::{
    perform_scrollbar_scroll_default_action, perform_wheel_scroll_default_action,
};

use super::super::{queue_revealed_lazy_image_loads, queue_revealed_lazy_media_loads};

pub(super) fn node_scroll_position(runtime: &PageScriptHost, handle: DomHandle) -> (f64, f64) {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| (element.scroll_left(), element.scroll_top()))
        .unwrap_or((0.0, 0.0))
}

pub(super) fn resolve_scroll_target(
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let node = runtime.dom_host().node(handle)?;
    if node.is_document() {
        return runtime.dom_host().document_element_handle();
    }
    runtime.dom_host().is_connected(handle).then_some(handle)
}

pub(super) fn set_node_scroll_position(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    left: f64,
    top: f64,
    queue_observable_effects: bool,
) -> bool {
    let (changed, document) = {
        let runtime = unsafe { &mut *runtime_ptr };
        let document = runtime.dom_host().owner_document_handle(handle);
        let Some(element) = runtime
            .dom_host_mut()
            .node_mut(handle)
            .and_then(|node| node.data_mut().as_element_mut())
        else {
            return false;
        };
        (
            element.set_scroll_left(left) | element.set_scroll_top(top),
            document,
        )
    };
    if changed {
        // Freshness must be visible even while ActionBatch defers events and
        // observer work until its checkpoint.
        unsafe { &*runtime_ptr }.note_realtime_layout_input_change();
    }
    if changed && queue_observable_effects {
        // Admission only: ActionBatch still owns its checkpoints and the
        // Page arbiter decides when the asynchronous frame may execute.
        unsafe { &*runtime_ptr }.queue_realtime_element_scroll_events(scope, handle);
        queue_scroll_observable_effects(scope, runtime_ptr, document, false);
    }
    changed
}

pub(super) fn apply_observable_window_scroll(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    scrolling_element: DomHandle,
    target_x: f64,
    target_y: f64,
    current_x: f64,
    current_y: f64,
) -> bool {
    let changed = target_x != current_x || target_y != current_y;
    if !changed {
        return false;
    }
    set_node_scroll_position(
        scope,
        runtime_ptr,
        scrolling_element,
        target_x,
        target_y,
        false,
    );
    let endpoint = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(scrolling_element)
        .and_then(|document| unsafe { &*runtime_ptr }.window_endpoint_for_document(document));
    if let Some(endpoint) = endpoint {
        unsafe { &mut *runtime_ptr }.scroll_window_endpoint_to(scope, endpoint, target_x, target_y);
    }
    true
}

pub(crate) fn queue_scroll_observable_effects(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: Option<DomHandle>,
    queue_document_events: bool,
) {
    if unsafe { &mut *runtime_ptr }.defer_scroll_observable_effects(document, queue_document_events)
    {
        return;
    }
    apply_scroll_observable_effects(
        scope,
        runtime_ptr,
        [crate::native_bridge::PendingScrollObservableEffects::new(
            document,
            queue_document_events,
        )],
    );
}

pub(crate) fn apply_scroll_observable_effects(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    effects: impl IntoIterator<Item = crate::native_bridge::PendingScrollObservableEffects>,
) {
    let effects = effects.into_iter().collect::<Vec<_>>();
    if effects.is_empty() {
        return;
    }
    // Native lazy loading combines the retained projection with live scroll
    // offsets. Geometry and input retain that projection until fresh visual
    // publication replaces it.
    for effects in &effects {
        if let Some(document) = effects.document() {
            queue_revealed_lazy_image_loads(scope, runtime_ptr, document);
        }
    }
    queue_revealed_lazy_media_loads(scope, runtime_ptr);
    unsafe { &*runtime_ptr }.invalidate_layout_after_interaction_state_change();
    crate::observer_runtime::queue_intersection_checks(scope, runtime_ptr);
    if effects
        .iter()
        .any(|effects| effects.queue_document_events())
    {
        let _ = unsafe { &mut *runtime_ptr }.queue_document_scroll_events(scope);
    }
}
