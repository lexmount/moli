use super::super::range::{native_range_boundary_handles, new_range_for_document};
use super::{selection_range, selection_store_with_composed_boundaries};
use crate::context_bootstrap::selection_value_for_window;
use crate::document_runtime::DomHandle;
use crate::dom::forms::InputType;
use crate::native_bridge::element::queue_text_control_selection_change_event;
use crate::native_bridge::{JsContextHost, OwnerDispatchScope};

/// Project a text field's internal selection into its Document before focus
/// listeners run. Picker controls have separate focus behavior and are not
/// text fields, even when their values contain text.
pub(crate) fn focus_text_control_selection(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    control: DomHandle,
) -> Option<()> {
    let runtime = unsafe { &mut *runtime_ptr };
    let element = runtime.dom_host().node(control)?.as_element()?;
    if !(element.is_html_textarea()
        || (element.is_html_input()
            && matches!(
                element.input_type(),
                InputType::Text
                    | InputType::Search
                    | InputType::Tel
                    | InputType::Url
                    | InputType::Email
                    | InputType::Password
                    | InputType::Number
            )))
    {
        return None;
    }
    let document = runtime.dom_host().owner_document_handle(control)?;
    let dispatch_scope = runtime.owner_dispatch_scope_for_node(control)?;
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
    let result = focus_text_control_selection_in_context(scope, runtime_ptr, document, control);
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
    let (mut container, mut offset) = (parent, index);
    let mut root = dom.root_node_handle(container)?;
    while dom.is_shadow_root(root) {
        let host = dom.shadow_root_host(root)?;
        container = dom.parent_node(host)?;
        offset = u32::try_from(dom.child_index(container, host)?).ok()?;
        root = dom.root_node_handle(container)?;
    }

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

fn wrap_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, handle)
}
