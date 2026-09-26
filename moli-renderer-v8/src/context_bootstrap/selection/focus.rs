use super::super::range::{native_range_boundary_handles, new_range_for_document};
use super::editing_caret::initial_editing_caret;
use super::{
    selection_composed_end_node, selection_composed_start_node, selection_direction,
    selection_range, selection_store_with_composed_boundaries,
};
use crate::context_bootstrap::selection_value_for_window;
use crate::document_runtime::DomHandle;
use crate::native_bridge::element::{
    contenteditable_editing_host, queue_text_control_selection_change_event,
};
use crate::native_bridge::{JsContextHost, OwnerDispatchScope, callback_value_dom_handle};
use crate::page_task_queue::RendererPageUserInteractionEventKind;

/// Update the focused area's selection in its owner realm before focus events.
pub(crate) fn focus_element_selection(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target: DomHandle,
) -> Option<()> {
    let runtime = unsafe { &mut *runtime_ptr };
    // Picker controls have a separate focus behavior from text fields.
    let text_field = runtime.text_control_has_selection_editor(target);
    if !text_field && contenteditable_editing_host(runtime, target) != Some(target) {
        return None;
    }
    let document = runtime.dom_host().owner_document_handle(target)?;
    let dispatch_scope = runtime.owner_dispatch_scope_for_node(target)?;
    match dispatch_scope {
        OwnerDispatchScope::Child(child) => {
            runtime
                .ensure_prebootstrapped_child_default_context(scope, child)
                .ok()?;
        }
        OwnerDispatchScope::LightweightPopup(popup) => {
            runtime
                .ensure_lightweight_popup_execution_context(scope, popup)
                .then_some(())?;
        }
        OwnerDispatchScope::Top => {}
    }
    let owner = runtime.current_window_execution_context_owner(dispatch_scope)?;
    let (_, context) = runtime.window_execution_context(scope, owner, dispatch_scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let previous = dispatch_scope.enter(scope);
    let result = if text_field {
        focus_text_control_selection_in_context(scope, runtime_ptr, document, target)
    } else {
        focus_editing_host_selection_in_context(scope, runtime_ptr, document, target)
    };
    dispatch_scope.restore(scope, previous);
    result
}

fn focus_text_control_selection_in_context(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document: DomHandle,
    control: DomHandle,
) -> Option<()> {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let parent = dom.parent_node(control)?;
    let index = u32::try_from(dom.child_index(parent, control)?).ok()?;
    // getRangeAt() exposes the position before the control (or its outermost
    // shadow host). getComposedRanges() instead encloses the internal editor's
    // host, with author shadow roots rescoping through the existing API.
    let (container, offset) = project_caret_out_of_shadow_trees(runtime, parent, index)?;

    let window = scope.get_current_context().global(scope);
    let selection = selection_value_for_window(scope, window)?;
    let selected_control = unsafe { &*runtime_ptr }.document_selected_text_control(document);
    let snapshot = unsafe { &*runtime_ptr }.document_selection_snapshot(document);
    if selected_control == Some(control)
        && let Some(range) = selection_range(scope, selection)
        && let Some(boundaries) = native_range_boundary_handles(scope, range)
        && boundaries.start.container == container
        && boundaries.start.offset == offset
        && boundaries.end.container == container
        && boundaries.end.offset == offset
        && snapshot.is_some_and(|snapshot| {
            snapshot.start.container == parent
                && snapshot.start.offset == index
                && snapshot.end.container == parent
                && snapshot.end.offset == index + 1
        })
    {
        // Refocusing an unchanged control must retain the associated Range.
        return Some(());
    }
    let document = wrap_node(scope, runtime_ptr, document)?;
    let container = wrap_node(scope, runtime_ptr, container)?;
    let parent = wrap_node(scope, runtime_ptr, parent)?;
    let range = new_range_for_document(scope, document)?;
    selection_store_with_composed_boundaries(
        scope,
        selection,
        range,
        container,
        offset,
        container,
        offset,
        "none",
        container,
        offset,
        container,
        offset,
        parent,
        index,
        parent,
        index + 1,
    );
    queue_text_control_selection_change_event(scope, runtime_ptr, control);
    Some(())
}

fn focus_editing_host_selection_in_context(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document: DomHandle,
    editing_host: DomHandle,
) -> Option<()> {
    let window = scope.get_current_context().global(scope);
    let selection = selection_value_for_window(scope, window)?;
    let anchor = if selection_direction(scope, selection).as_deref() == Some("backward") {
        selection_composed_end_node(scope, selection)
    } else {
        selection_composed_start_node(scope, selection)
    }
    .and_then(|anchor| callback_value_dom_handle(scope, anchor.into()));
    let runtime = unsafe { &*runtime_ptr };
    if runtime.document_selected_text_control(document).is_none()
        && anchor.is_some_and(|anchor| {
            contenteditable_editing_host(runtime, anchor) == Some(editing_host)
        })
    {
        // A selection anchored in this editor survives refocusing, including
        // backward selections and selections extending outside the editor.
        return Some(());
    }
    let (container, offset) = initial_editing_caret(runtime, editing_host)?;
    let (container, offset) = project_caret_out_of_shadow_trees(runtime, container, offset)?;
    let document_object = wrap_node(scope, runtime_ptr, document)?;
    let host_object = wrap_node(scope, runtime_ptr, editing_host)?;
    let container = wrap_node(scope, runtime_ptr, container)?;
    let range = new_range_for_document(scope, document_object)?;
    selection_store_with_composed_boundaries(
        scope,
        selection,
        range,
        container,
        offset,
        container,
        offset,
        "none",
        container,
        offset,
        container,
        offset,
        host_object,
        0,
        host_object,
        0,
    );
    let _ = unsafe { &mut *runtime_ptr }.queue_user_interaction_event_task(
        scope,
        RendererPageUserInteractionEventKind::DocumentSelectionChange,
        document,
    );
    Some(())
}

fn project_caret_out_of_shadow_trees(
    runtime: &JsContextHost,
    mut container: DomHandle,
    mut offset: u32,
) -> Option<(DomHandle, u32)> {
    let dom = runtime.dom_host();
    let mut root = dom.root_node_handle(container)?;
    while dom.is_shadow_root(root) {
        let host = dom.shadow_root_host(root)?;
        container = dom.parent_node(host)?;
        offset = u32::try_from(dom.child_index(container, host)?).ok()?;
        root = dom.root_node_handle(container)?;
    }
    Some((container, offset))
}

fn wrap_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, handle)
}
