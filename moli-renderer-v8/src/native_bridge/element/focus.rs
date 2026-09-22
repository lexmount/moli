use crate::dom::native::Node;

use super::super::{
    JsContextHost, PendingWindowMessageEndpoint, document, node::node_runtime_and_handle_from_args,
};
use super::forms::{
    dispatch_text_control_event, form_control_is_effectively_disabled, text_control_value,
};
use super::geometry::{observable_element_metrics, scroll_node_into_view_if_needed};
use super::styles::{StyleMode, style_property_value};
use super::{
    construct_click_event, construct_focus_event, construct_interest_event,
    dispatch_popover_show_events, dispatch_public_event,
};
use super::{
    element_attribute, element_has_attribute, label_control_handle,
    label_receives_programmatic_focus,
};
use crate::document_runtime::{DomHandle, EventTargetHandle};
use crate::runtime::RendererDomFocusOutcome;
use crate::util::{get_private_value, node_wrapper_from_handle, v8_string, v8str};

const BUTTON_INTEREST_FOR_ELEMENT_SLOT: &str = "__moliButtonInterestForElement";

struct SequentialFocusEntry {
    tab_index: i32,
    order: usize,
    handles: Vec<DomHandle>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadingFlowMode {
    SourceOrder,
    GridOrder,
    GridRows,
    GridColumns,
    FlexFlow,
    FlexVisual,
}

pub(super) fn is_disabled_form_control(runtime: &JsContextHost, handle: DomHandle) -> bool {
    form_control_is_effectively_disabled(runtime, handle)
}

pub(crate) fn contenteditable_editing_host(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let dom = runtime.dom_host().dom();
    let mut current = Some(handle);
    while let Some(candidate) = current {
        if let Some(element) = dom.node(candidate).and_then(Node::as_element)
            && let Some(value) = element.attribute("contenteditable")
        {
            match contenteditable_state_from_attr(value) {
                Some(true) => return Some(candidate),
                Some(false) => return None,
                None => {}
            }
        }
        current = dom.parent_node(candidate);
    }
    None
}

fn contenteditable_state_from_attr(value: &str) -> Option<bool> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "true" || normalized == "plaintext-only" {
        Some(true)
    } else if normalized == "false" {
        Some(false)
    } else {
        None
    }
}

pub(super) fn is_focusable(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    if !runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_connected)
        || is_disabled_form_control(runtime, handle)
    {
        return false;
    }
    matches!(
        element.local_name(),
        "body" | "input" | "button" | "select" | "textarea" | "a" | "iframe" | "frame"
    ) || element.has_attribute("tabindex")
        || contenteditable_editing_host(runtime, handle) == Some(handle)
        || element_is_scrollable(runtime, handle)
}

fn overflow_makes_scroll_container(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "auto" | "scroll"
    )
}

fn element_is_scrollable(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let scrolls_x = overflow_makes_scroll_container(&style_property_value(
        runtime,
        handle,
        StyleMode::Computed,
        "overflow-x",
    ));
    let scrolls_y = overflow_makes_scroll_container(&style_property_value(
        runtime,
        handle,
        StyleMode::Computed,
        "overflow-y",
    ));
    if !scrolls_x && !scrolls_y {
        return false;
    }
    match observable_element_metrics(
        runtime,
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    ) {
        Ok(Some(metrics)) => {
            metrics.is_scroll_container
                && (scrolls_x && metrics.maximum_scroll_offset.x > metrics.minimum_scroll_offset.x
                    || scrolls_y
                        && metrics.maximum_scroll_offset.y > metrics.minimum_scroll_offset.y)
        }
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(
                ?handle,
                %error,
                "failed to evaluate scroll-container focusability from layout"
            );
            false
        }
    }
}

fn shadow_including_contains(
    runtime: &JsContextHost,
    ancestor: DomHandle,
    node: DomHandle,
) -> bool {
    let mut current = Some(node);
    while let Some(handle) = current {
        if handle == ancestor {
            return true;
        }
        current = runtime.dom_host().parent_node(handle).or_else(|| {
            runtime
                .dom_host()
                .is_shadow_root(handle)
                .then(|| runtime.dom_host().shadow_root_host(handle))
                .flatten()
        });
    }
    false
}

fn first_delegates_focus_descendant(
    runtime: &JsContextHost,
    root: DomHandle,
    autofocus_only: bool,
) -> Option<DomHandle> {
    let mut stack = runtime.dom_host().child_handles(root).collect::<Vec<_>>();
    stack.reverse();
    while let Some(handle) = stack.pop() {
        let autofocus = runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| element.has_attribute("autofocus"));
        if (!autofocus_only || autofocus) && is_focusable(runtime, handle) {
            return Some(handle);
        }

        if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle)
            && runtime.dom_host().shadow_root_delegates_focus(shadow_root) == Some(true)
            && let Some(target) =
                first_delegates_focus_descendant(runtime, shadow_root, autofocus_only)
        {
            return Some(target);
        }

        let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
        children.reverse();
        stack.extend(children);
    }
    None
}

fn first_delegates_focus_target(runtime: &JsContextHost, root: DomHandle) -> Option<DomHandle> {
    first_delegates_focus_descendant(runtime, root, true)
        .or_else(|| first_delegates_focus_descendant(runtime, root, false))
}

fn delegated_focus_target(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    let root = runtime.dom_host().shadow_root_handle(handle)?;
    if runtime.dom_host().shadow_root_delegates_focus(root) != Some(true) {
        return None;
    }
    if let Some(active) = runtime.active_element_handle()
        && shadow_including_contains(runtime, root, active)
    {
        return Some(active);
    }
    first_delegates_focus_target(runtime, root)
}

fn first_autofocus_candidate(runtime: &JsContextHost) -> Option<DomHandle> {
    let mut stack = runtime
        .dom_host()
        .child_handles(runtime.document_handle())
        .collect::<Vec<_>>();
    stack.reverse();
    while let Some(handle) = stack.pop() {
        let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
            let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
            children.reverse();
            stack.extend(children);
            continue;
        };
        if element.has_attribute("autofocus") && is_focusable(runtime, handle) {
            return Some(handle);
        }
        let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
        children.reverse();
        stack.extend(children);
    }
    None
}

/// Whether the current Document has post-parse autofocus work worth
/// admitting to the rendering-update task source.
///
/// This is only an admission snapshot. The selected rendering task resolves
/// the candidate again because script can move focus or mutate the Document
/// between publication and execution.
pub(crate) fn post_parse_autofocus_is_pending(runtime: &JsContextHost) -> bool {
    runtime.active_element_handle().is_none() && first_autofocus_candidate(runtime).is_some()
}

pub(crate) fn process_post_parse_autofocus(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    if runtime.active_element_handle().is_some() {
        return false;
    }
    let Some(candidate) = first_autofocus_candidate(runtime) else {
        return false;
    };
    if let Some(target) = delegated_focus_target(runtime, candidate) {
        update_focus(scope, runtime_ptr, Some(target));
    } else {
        update_focus(scope, runtime_ptr, Some(candidate));
    }
    true
}

fn focused_chain_requires_async_blur(runtime: &JsContextHost, active: DomHandle) -> bool {
    let mut current = Some(active);
    while let Some(handle) = current {
        if element_has_attribute(runtime, handle, "inert")
            || element_has_attribute(runtime, handle, "hidden")
        {
            return true;
        }
        if style_property_value(runtime, handle, StyleMode::Computed, "display")
            .eq_ignore_ascii_case("none")
        {
            return true;
        }
        current = runtime
            .dom_host()
            .node(handle)
            .and_then(Node::parent_node)
            .or_else(|| {
                runtime
                    .dom_host()
                    .containing_shadow_root(handle)
                    .and_then(|root| runtime.dom_host().shadow_root_host(root))
            });
    }
    false
}

fn blur_if_focused_chain_is_display_none_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(runtime_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(active) = runtime.active_element_handle() else {
        return;
    };
    if focused_chain_requires_async_blur(runtime, active) {
        update_focus(scope, runtime_ptr, None);
    }
}

pub(crate) fn schedule_focus_blur_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    active: DomHandle,
) {
    let runtime = unsafe { &*runtime_ptr };
    if !focused_chain_requires_async_blur(runtime, active) {
        return;
    }
    if let Some(callback) = v8::Function::new(scope, blur_if_focused_chain_is_display_none_callback)
    {
        let global = scope.get_current_context().global(scope);
        let Some(set_timeout) = global
            .get(scope, v8str(scope, "setTimeout").into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        else {
            crate::util::enqueue_host_microtask(scope, callback);
            return;
        };
        let delay = v8::Integer::new(scope, 0);
        let args = [callback.into(), delay.into()];
        let _ = set_timeout.call(scope, global.into(), &args);
    }
}

fn wrap_handle_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Value>> {
    let handle = handle?;
    let runtime = unsafe { &mut *runtime_ptr };
    let wrapped = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, handle)?;
    Some(wrapped.into())
}

fn dispatch_pending_text_control_change_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    let committed_value = unsafe { &mut *runtime_ptr }.take_text_control_change_commit(handle);
    let Some(committed_value) = committed_value else {
        return;
    };
    let current_value = text_control_value(unsafe { &*runtime_ptr }, handle);
    if current_value != committed_value {
        dispatch_text_control_event(scope, runtime_ptr, handle, "change");
    }
}

pub(crate) fn update_focus(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    next: Option<DomHandle>,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    let previous = runtime.active_element_handle();
    update_focus_from_previous(scope, runtime_ptr, previous, next);
}

pub(crate) fn reset_focus_from_previous_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    previous: DomHandle,
) {
    update_focus_from_previous_with_previous_focus_within(
        scope,
        runtime_ptr,
        Some(previous),
        None,
        None,
    );
}

pub(crate) fn reset_focus_from_previous_handle_with_previous_focus_within(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    previous: DomHandle,
    previous_focus_within: Vec<DomHandle>,
) {
    update_focus_from_previous_with_previous_focus_within(
        scope,
        runtime_ptr,
        Some(previous),
        None,
        Some(previous_focus_within),
    );
}

fn update_focus_from_previous(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
) {
    update_focus_from_previous_with_previous_focus_within(scope, runtime_ptr, previous, next, None);
}

fn update_focus_from_previous_with_previous_focus_within(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
    previous_focus_within: Option<Vec<DomHandle>>,
) {
    if previous == next {
        return;
    }
    let next_value = wrap_handle_value(scope, runtime_ptr, next);
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.set_active_element_handle(next);
    runtime.mark_focus_changed();
    if let Some(previous_focus_within) = previous_focus_within {
        runtime.note_focus_style_activity_with_previous_focus_within(
            previous,
            next,
            Some(previous_focus_within),
        );
    } else {
        runtime.note_focus_style_activity(previous, next);
    }
    if let Some(previous_handle) = previous {
        dispatch_pending_text_control_change_if_needed(scope, runtime_ptr, previous_handle);
        if let Some(event) = construct_focus_event(scope, "blur", next_value, false) {
            let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
        }
        if let Some(event) = construct_focus_event(scope, "focusout", next_value, true) {
            let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
        }
        dispatch_interest_event_if_needed(scope, runtime_ptr, previous_handle, "loseinterest");
    }
    let window_focus_transition =
        window_focus_transition_for_handles(scope, runtime_ptr, previous, next);
    if let Some((previous_window, _)) = window_focus_transition {
        dispatch_window_focus_event(scope, runtime_ptr, previous_window, "blur");
    }
    let previous_value = wrap_handle_value(scope, runtime_ptr, previous);
    if let Some(next_handle) = next {
        if let Some(event) = construct_focus_event(scope, "focus", previous_value, false) {
            let _ = dispatch_public_event(scope, runtime_ptr, next_handle, event);
        }
        if let Some(event) = construct_focus_event(scope, "focusin", previous_value, true) {
            let _ = dispatch_public_event(scope, runtime_ptr, next_handle, event);
        }
        if let Some(target) =
            dispatch_interest_event_if_needed(scope, runtime_ptr, next_handle, "interest")
        {
            show_interest_popover_if_needed(scope, runtime_ptr, next_handle, target);
        }
        schedule_focus_blur_if_needed(scope, runtime_ptr, next_handle);
    }
    if let Some((_, next_window)) = window_focus_transition {
        dispatch_window_focus_event(scope, runtime_ptr, next_window, "focus");
    }
}

fn window_focus_transition_for_handles(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
) -> Option<(PendingWindowMessageEndpoint, PendingWindowMessageEndpoint)> {
    let runtime = unsafe { &*runtime_ptr };
    let execution_window = current_execution_window_endpoint(scope);
    let previous_window = previous
        .and_then(|handle| focused_window_endpoint(runtime, handle))
        .unwrap_or(execution_window);
    // Clearing an element's focus does not blur its Window. A later focus in
    // another Document supplies the concrete new focused-frame endpoint.
    let next_window = next
        .and_then(|handle| focused_window_endpoint(runtime, handle))
        .unwrap_or(previous_window);
    if previous_window == next_window {
        return None;
    }
    Some((previous_window, next_window))
}

fn current_execution_window_endpoint(
    scope: &mut v8::PinScope<'_, '_>,
) -> PendingWindowMessageEndpoint {
    if let Some(handle) = crate::native_bridge::active_child_window_handle(scope) {
        return PendingWindowMessageEndpoint::ChildWindow(handle);
    }
    if let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope) {
        return PendingWindowMessageEndpoint::LightweightPopup(popup_id);
    }
    PendingWindowMessageEndpoint::TopWindow
}

fn focused_window_endpoint(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<PendingWindowMessageEndpoint> {
    if runtime.dom_host().is_html_element_named(handle, "iframe")
        || runtime.dom_host().is_html_element_named(handle, "frame")
    {
        return Some(PendingWindowMessageEndpoint::ChildWindow(handle));
    }
    let document = runtime.dom_host().owner_document_handle(handle)?;
    if let Some(child_handle) = runtime.child_browsing_context_host_for_document_handle(document) {
        return Some(PendingWindowMessageEndpoint::ChildWindow(child_handle));
    }
    if let Some(popup_id) = runtime.lightweight_popup_id_for_document_handle(document) {
        return Some(PendingWindowMessageEndpoint::LightweightPopup(popup_id));
    }
    (document == runtime.document_handle()).then_some(PendingWindowMessageEndpoint::TopWindow)
}

fn dispatch_window_focus_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    endpoint: PendingWindowMessageEndpoint,
    event_type: &str,
) {
    let event = construct_focus_event(scope, event_type, None, false)
        .expect("Window focus transition must materialize its FocusEvent");
    let runtime = unsafe { &mut *runtime_ptr };
    match endpoint {
        PendingWindowMessageEndpoint::TopWindow => {
            let _ = runtime.dispatch_public_event_best_effort(
                scope,
                runtime_ptr,
                EventTargetHandle::Window,
                event,
                "window focus event",
            );
        }
        PendingWindowMessageEndpoint::ChildWindow(handle) => {
            runtime.dispatch_child_window_event(scope, handle, event_type, event);
        }
        PendingWindowMessageEndpoint::LightweightPopup(popup_id) => {
            let _ =
                runtime.dispatch_lightweight_popup_window_event(scope, popup_id, event_type, event);
        }
    }
}

fn dispatch_interest_event_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
    event_type: &str,
) -> Option<DomHandle> {
    let target = interest_for_element_target(scope, runtime_ptr, source_handle)?;
    let source = node_wrapper_from_handle(scope, source_handle)?;
    let event = construct_interest_event(scope, event_type, source.into())?;
    let _ = dispatch_public_event(scope, runtime_ptr, target, event);
    Some(target)
}

pub(crate) fn perform_hover_interest_default_action_for_dispatched_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
) {
    let Some(target) =
        dispatch_interest_event_if_needed(scope, runtime_ptr, source_handle, "interest")
    else {
        return;
    };
    show_interest_popover_if_needed(scope, runtime_ptr, source_handle, target);
}

pub(crate) fn perform_access_key_default_action_for_dispatched_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    event: v8::Local<'_, v8::Object>,
) {
    if !event_boolean_property(scope, event, "altKey")
        || !event_boolean_property(scope, event, "shiftKey")
            && !event_boolean_property(scope, event, "ctrlKey")
    {
        return;
    }
    let Some(key) = event_string_property(scope, event, "key") else {
        return;
    };
    if key.chars().count() != 1 {
        return;
    }
    let Some(target) = access_key_target(unsafe { &*runtime_ptr }, &key) else {
        return;
    };
    let Some(click) = construct_click_event(scope, 0.0, 0.0, 0, 0) else {
        return;
    };
    let _ = dispatch_public_event(scope, runtime_ptr, target, click);
}

pub(crate) fn perform_tab_focus_default_action_for_dispatched_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    event: v8::Local<'_, v8::Object>,
) {
    if event_string_property(scope, event, "key").as_deref() != Some("Tab") {
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let active = runtime.active_element_handle();
    let order = sequential_focus_order(runtime, active);
    if order.is_empty() {
        return;
    }
    let reverse = event_boolean_property(scope, event, "shiftKey");
    let next_handle = active
        .and_then(|active| order.iter().position(|candidate| *candidate == active))
        .map(|index| {
            let next_index = if reverse {
                index.checked_sub(1).unwrap_or(order.len() - 1)
            } else {
                (index + 1) % order.len()
            };
            order[next_index]
        })
        .or_else(|| {
            active.and_then(|active| negative_shadow_scope_tab_target(runtime, active, reverse))
        })
        .unwrap_or_else(|| order[if reverse { order.len() - 1 } else { 0 }]);
    update_focus(scope, runtime_ptr, Some(next_handle));
}

fn sequential_focus_order(runtime: &JsContextHost, active: Option<DomHandle>) -> Vec<DomHandle> {
    sequential_focus_order_for_children(
        runtime,
        runtime
            .dom_host()
            .child_handles(runtime.document_handle())
            .collect(),
        active,
    )
}

fn sequential_focus_order_for_children(
    runtime: &JsContextHost,
    children: Vec<DomHandle>,
    active: Option<DomHandle>,
) -> Vec<DomHandle> {
    let mut entries = Vec::new();
    let mut order = 0;
    for child in children {
        collect_sequential_focus_entries(runtime, child, active, &mut entries, &mut order);
    }
    entries.sort_by_key(|entry| {
        if entry.tab_index > 0 {
            (0, entry.tab_index, entry.order)
        } else {
            (1, 0, entry.order)
        }
    });
    entries
        .into_iter()
        .flat_map(|entry| entry.handles)
        .collect()
}

fn collect_sequential_focus_entries(
    runtime: &JsContextHost,
    handle: DomHandle,
    active: Option<DomHandle>,
    entries: &mut Vec<SequentialFocusEntry>,
    order: &mut usize,
) {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        append_child_entries(runtime, handle, active, entries, order);
        return;
    };

    if element.is_html_element("slot") {
        let Some(tab_index) = slot_focus_scope_tab_index(runtime, handle) else {
            return;
        };
        let mut handles = Vec::new();
        if slot_generates_focusable_box(runtime, handle) {
            handles.push(handle);
        }
        let mut slot_children = runtime
            .dom_host()
            .assigned_nodes_for_slot_with_options(handle, false);
        if slot_children.is_empty() {
            slot_children = runtime.dom_host().child_handles(handle).collect();
        }
        let child_handles = if let Some(mode) = reading_flow_mode(runtime, handle) {
            sequential_focus_order_for_reading_flow_items(
                runtime,
                handle,
                mode,
                slot_children,
                active,
            )
        } else {
            sequential_focus_order_for_children(runtime, slot_children, active)
        };
        handles.extend(child_handles);
        push_focus_entry_with_tab_index(tab_index, handles, entries, order);
        return;
    }

    if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle) {
        if explicit_negative_tab_index(runtime, handle) {
            return;
        }
        let mut handles = Vec::new();
        let tab_index = sequential_tab_index(runtime, handle, active);
        let delegates_focus =
            runtime.dom_host().shadow_root_delegates_focus(shadow_root) == Some(true);
        if tab_index.is_some() && !delegates_focus {
            handles.push(handle);
        }
        let shadow_children = runtime
            .dom_host()
            .child_handles(shadow_root)
            .collect::<Vec<_>>();
        if let Some(mode) = reading_flow_mode(runtime, handle) {
            handles.extend(sequential_focus_order_for_reading_flow_items(
                runtime,
                handle,
                mode,
                shadow_children,
                active,
            ));
        } else {
            handles.extend(sequential_focus_order_for_children(
                runtime,
                shadow_children,
                active,
            ));
        }
        push_focus_entry_with_tab_index(tab_index.unwrap_or(0), handles, entries, order);
        return;
    }

    if sequential_tab_index(runtime, handle, active).is_some() {
        push_focus_entry(runtime, handle, vec![handle], entries, order);
    }
    if reading_flow_mode(runtime, handle).is_some() {
        push_focus_entry_with_tab_index(
            0,
            sequential_focus_order_for_reading_flow_container(
                runtime,
                handle,
                runtime.dom_host().child_handles(handle).collect(),
                active,
            ),
            entries,
            order,
        );
        return;
    }
    append_child_entries(runtime, handle, active, entries, order);
}

fn append_child_entries(
    runtime: &JsContextHost,
    handle: DomHandle,
    active: Option<DomHandle>,
    entries: &mut Vec<SequentialFocusEntry>,
    order: &mut usize,
) {
    for child in runtime.dom_host().child_handles(handle) {
        collect_sequential_focus_entries(runtime, child, active, entries, order);
    }
}

fn push_focus_entry(
    runtime: &JsContextHost,
    handle: DomHandle,
    handles: Vec<DomHandle>,
    entries: &mut Vec<SequentialFocusEntry>,
    order: &mut usize,
) {
    push_focus_entry_with_tab_index(
        sequential_tab_index(runtime, handle, None).unwrap_or(0),
        handles,
        entries,
        order,
    );
}

fn push_focus_entry_with_tab_index(
    tab_index: i32,
    handles: Vec<DomHandle>,
    entries: &mut Vec<SequentialFocusEntry>,
    order: &mut usize,
) {
    if handles.is_empty() {
        return;
    }
    entries.push(SequentialFocusEntry {
        tab_index,
        order: *order,
        handles,
    });
    *order += 1;
}

fn sequential_focus_order_for_reading_flow_container(
    runtime: &JsContextHost,
    container: DomHandle,
    children: Vec<DomHandle>,
    active: Option<DomHandle>,
) -> Vec<DomHandle> {
    let Some(mode) = reading_flow_mode(runtime, container) else {
        return sequential_focus_order_for_children(runtime, children, active);
    };
    reading_flow_item_groups(runtime, container, mode, children, active)
        .into_iter()
        .flat_map(|(_, handles)| handles)
        .collect()
}

fn sequential_focus_order_for_reading_flow_items(
    runtime: &JsContextHost,
    container: DomHandle,
    mode: ReadingFlowMode,
    children: Vec<DomHandle>,
    active: Option<DomHandle>,
) -> Vec<DomHandle> {
    reading_flow_item_groups(runtime, container, mode, children, active)
        .into_iter()
        .flat_map(|(_, handles)| handles)
        .collect()
}

fn reading_flow_item_groups(
    runtime: &JsContextHost,
    container: DomHandle,
    mode: ReadingFlowMode,
    children: Vec<DomHandle>,
    active: Option<DomHandle>,
) -> Vec<((i32, i32, i32, usize), Vec<DomHandle>)> {
    let reverse_tree_order = mode == ReadingFlowMode::FlexVisual
        && style_property_value(runtime, container, StyleMode::Computed, "flex-direction")
            .trim()
            .eq_ignore_ascii_case("row-reverse");
    let mut groups = children
        .into_iter()
        .enumerate()
        .flat_map(|(tree_order, child)| {
            reading_flow_item_group(runtime, child, mode, tree_order, reverse_tree_order, active)
        })
        .collect::<Vec<_>>();
    groups.sort_by_key(|(key, _)| *key);
    groups
}

fn reading_flow_item_group(
    runtime: &JsContextHost,
    child: DomHandle,
    mode: ReadingFlowMode,
    tree_order: usize,
    reverse_tree_order: bool,
    active: Option<DomHandle>,
) -> Vec<((i32, i32, i32, usize), Vec<DomHandle>)> {
    if computed_display(runtime, child) != "contents" {
        return vec![(
            reading_flow_child_order_key(runtime, child, mode, tree_order, reverse_tree_order),
            sequential_focus_order_for_children(runtime, vec![child], active),
        )];
    }

    let mut child_groups = runtime
        .dom_host()
        .child_handles(child)
        .enumerate()
        .flat_map(|(index, grandchild)| {
            reading_flow_item_group(runtime, grandchild, mode, index, reverse_tree_order, active)
        })
        .filter(|(_, handles)| !handles.is_empty())
        .collect::<Vec<_>>();
    child_groups.sort_by_key(|(key, _)| *key);
    let Some((key, _)) = child_groups.first().cloned() else {
        return Vec::new();
    };
    let mut handles = Vec::new();
    if sequential_tab_index(runtime, child, active).is_some() {
        handles.push(child);
    }
    handles.extend(child_groups.into_iter().flat_map(|(_, handles)| handles));
    vec![(key, handles)]
}

fn reading_flow_child_order_key(
    runtime: &JsContextHost,
    child: DomHandle,
    mode: ReadingFlowMode,
    tree_order: usize,
    reverse_tree_order: bool,
) -> (i32, i32, i32, usize) {
    let position_group = match style_property_value(runtime, child, StyleMode::Computed, "position")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "absolute" | "fixed" => 1,
        _ => 0,
    };
    let reading_order = style_integer_property(runtime, child, "reading-order").unwrap_or(0);
    let layout_order = match mode {
        ReadingFlowMode::SourceOrder => 0,
        _ => style_integer_property(runtime, child, "order").unwrap_or(0),
    };
    let order = if reverse_tree_order {
        usize::MAX - tree_order
    } else {
        tree_order
    };
    (position_group, reading_order, layout_order, order)
}

fn reading_flow_mode(runtime: &JsContextHost, handle: DomHandle) -> Option<ReadingFlowMode> {
    let display = computed_display(runtime, handle);
    let mode = match style_property_value(runtime, handle, StyleMode::Computed, "reading-flow")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "source-order" if display != "inline" => ReadingFlowMode::SourceOrder,
        "grid-order" if display.contains("grid") => ReadingFlowMode::GridOrder,
        "grid-rows" if display.contains("grid") => ReadingFlowMode::GridRows,
        "grid-columns" if display.contains("grid") => ReadingFlowMode::GridColumns,
        "flex-flow" if display.contains("flex") => ReadingFlowMode::FlexFlow,
        "flex-visual" if display.contains("flex") => ReadingFlowMode::FlexVisual,
        _ => return None,
    };
    Some(mode)
}

fn computed_display(runtime: &JsContextHost, handle: DomHandle) -> String {
    style_property_value(runtime, handle, StyleMode::Computed, "display")
        .trim()
        .to_ascii_lowercase()
}

fn style_integer_property(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<i32> {
    style_property_value(runtime, handle, StyleMode::Computed, property)
        .trim()
        .parse::<i32>()
        .ok()
}

fn slot_focus_scope_tab_index(runtime: &JsContextHost, handle: DomHandle) -> Option<i32> {
    let value = element_attribute(runtime, handle, "tabindex")
        .and_then(|value| parse_sequential_tab_index(&value))
        .unwrap_or(0);
    (value >= 0).then_some(value)
}

fn slot_generates_focusable_box(runtime: &JsContextHost, handle: DomHandle) -> bool {
    !matches!(
        computed_display(runtime, handle).as_str(),
        "none" | "contents"
    )
}

fn sequential_tab_index(
    runtime: &JsContextHost,
    handle: DomHandle,
    active: Option<DomHandle>,
) -> Option<i32> {
    if !is_sequentially_focusable(runtime, handle, active) {
        return None;
    }
    sequential_tab_index_without_scroll_descendant_check(runtime, handle)
}

fn sequential_tab_index_without_scroll_descendant_check(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<i32> {
    let value = element_attribute(runtime, handle, "tabindex")
        .and_then(|value| parse_sequential_tab_index(&value))
        .unwrap_or_else(|| default_sequential_tab_index(runtime, handle));
    (value >= 0).then_some(value)
}

fn is_sequentially_focusable(
    runtime: &JsContextHost,
    handle: DomHandle,
    active: Option<DomHandle>,
) -> bool {
    if !is_focusable(runtime, handle) {
        return false;
    }
    if active == Some(handle) {
        return true;
    }
    !element_is_scrollable(runtime, handle) || !has_sequential_focusable_descendant(runtime, handle)
}

fn has_sequential_focusable_descendant(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let mut stack = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
    if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle) {
        stack.extend(runtime.dom_host().child_handles(shadow_root));
    }
    while let Some(candidate) = stack.pop() {
        if is_interactive_sequential_focusable_descendant(runtime, candidate) {
            return true;
        }
        stack.extend(runtime.dom_host().child_handles(candidate));
        if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(candidate) {
            stack.extend(runtime.dom_host().child_handles(shadow_root));
        }
    }
    false
}

fn is_interactive_sequential_focusable_descendant(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    if is_disabled_form_control(runtime, handle) {
        return false;
    }
    let is_interactive_element = matches!(
        element.local_name(),
        "a" | "button" | "input" | "select" | "textarea"
    );
    let has_non_negative_tab_index = element_attribute(runtime, handle, "tabindex")
        .and_then(|value| parse_sequential_tab_index(&value))
        .is_some_and(|value| value >= 0);
    (is_interactive_element || has_non_negative_tab_index)
        && sequential_tab_index_without_scroll_descendant_check(runtime, handle).is_some()
}

fn explicit_negative_tab_index(runtime: &JsContextHost, handle: DomHandle) -> bool {
    element_attribute(runtime, handle, "tabindex")
        .and_then(|value| parse_sequential_tab_index(&value))
        .is_some_and(|value| value < 0)
}

fn default_sequential_tab_index(runtime: &JsContextHost, handle: DomHandle) -> i32 {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return -1;
    };
    if element.namespace() != "http://www.w3.org/1999/xhtml" {
        return -1;
    }
    match element.local_name() {
        "a" | "button" | "input" | "select" | "textarea" => 0,
        _ if element_is_scrollable(runtime, handle) => 0,
        _ => -1,
    }
}

fn parse_sequential_tab_index(value: &str) -> Option<i32> {
    let value = value.trim_start();
    let mut chars = value.chars();
    let (sign, rest) = match chars.next() {
        Some('+') => (1_i64, chars.as_str()),
        Some('-') => (-1_i64, chars.as_str()),
        Some(_) => (1_i64, value),
        None => return None,
    };
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits
        .parse::<i64>()
        .ok()
        .and_then(|value| i32::try_from(sign * value).ok())
}

fn negative_shadow_scope_tab_target(
    runtime: &JsContextHost,
    active: DomHandle,
    reverse: bool,
) -> Option<DomHandle> {
    let root = runtime.dom_host().containing_shadow_root(active)?;
    let host = runtime.dom_host().shadow_root_host(root)?;
    if !explicit_negative_tab_index(runtime, host) {
        return None;
    }
    let local_order = sequential_focus_order_for_children(
        runtime,
        runtime.dom_host().child_handles(root).collect(),
        Some(active),
    );
    let position = local_order
        .iter()
        .position(|candidate| *candidate == active)?;
    if reverse {
        if let Some(previous) = position
            .checked_sub(1)
            .and_then(|index| local_order.get(index))
            .copied()
        {
            return Some(previous);
        }
    } else if let Some(next) = local_order.get(position + 1).copied() {
        return Some(next);
    }
    focusable_adjacent_to_negative_shadow_host(runtime, host, reverse)
}

fn focusable_adjacent_to_negative_shadow_host(
    runtime: &JsContextHost,
    host: DomHandle,
    reverse: bool,
) -> Option<DomHandle> {
    let parent = runtime.dom_host().node(host).and_then(Node::parent_node)?;
    let siblings = runtime.dom_host().child_handles(parent).collect::<Vec<_>>();
    let position = siblings.iter().position(|candidate| *candidate == host)?;
    if reverse {
        for sibling in siblings[..position].iter().rev() {
            let order = sequential_focus_order_for_children(runtime, vec![*sibling], None);
            if let Some(candidate) = order.last().copied() {
                return Some(candidate);
            }
        }
        return None;
    }
    for sibling in siblings.iter().skip(position + 1) {
        let order = sequential_focus_order_for_children(runtime, vec![*sibling], None);
        if let Some(candidate) = order.first().copied() {
            return Some(candidate);
        }
    }
    None
}

fn access_key_target(runtime: &JsContextHost, key: &str) -> Option<DomHandle> {
    let needle = key.to_ascii_lowercase();
    let mut stack = runtime
        .dom_host()
        .child_handles(runtime.document_handle())
        .collect::<Vec<_>>();
    stack.reverse();
    while let Some(handle) = stack.pop() {
        if runtime.dom_host().is_connected(handle)
            && element_attribute(runtime, handle, "accesskey")
                .is_some_and(|value| access_key_attribute_matches(&value, &needle))
        {
            return Some(handle);
        }

        let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
        children.reverse();
        stack.extend(children);
        if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle) {
            let mut shadow_children = runtime
                .dom_host()
                .child_handles(shadow_root)
                .collect::<Vec<_>>();
            shadow_children.reverse();
            stack.extend(shadow_children);
        }
    }
    None
}

fn access_key_attribute_matches(value: &str, needle: &str) -> bool {
    value
        .split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case(needle))
}

fn event_boolean_property(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    name: &str,
) -> bool {
    let Some(key) = v8_string(scope, name) else {
        return false;
    };
    event
        .get(scope, key.into())
        .is_some_and(|value| value.boolean_value(scope))
}

fn event_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    name: &str,
) -> Option<String> {
    let key = v8_string(scope, name)?;
    event
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| value.to_rust_string_lossy(scope))
}

fn show_interest_popover_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
    target: DomHandle,
) {
    let runtime = unsafe { &*runtime_ptr };
    if runtime.dom_host().node(target).is_some() {
        dispatch_popover_show_events(scope, runtime_ptr, target, source_handle);
    }
}

fn interest_for_element_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    if let Some(target) =
        unsafe { &*runtime_ptr }.button_element_target(handle, BUTTON_INTEREST_FOR_ELEMENT_SLOT)
    {
        return unsafe { &*runtime_ptr }
            .dom_host()
            .resolve_reference_target_chain(target);
    }
    if let Some(wrapper) = node_wrapper_from_handle(scope, handle)
        && let Some(value) = get_private_value(scope, wrapper, BUTTON_INTEREST_FOR_ELEMENT_SLOT)
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
    let interest_for = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(|element| element.attribute("interestfor"))
        .filter(|value| !value.is_empty())?;
    let root = runtime.dom_host().root_node_handle(handle)?;
    let candidate = runtime
        .dom_host()
        .element_handle_by_id_in_subtree(root, interest_for)?;
    runtime.dom_host().resolve_reference_target_chain(candidate)
}

pub(crate) fn focus_element(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    if let Err(error) = focus_element_with_options(scope, runtime_ptr, handle, false) {
        tracing::warn!(?handle, %error, "failed to scroll focused element into view");
    }
}

/// Performs the focus default action associated with an uncanceled mouse down.
///
/// Blink dispatches `pointerdown`/`mousedown` before it changes focus. It then
/// walks from the hit element through the flat tree to find a mouse-focusable
/// element; if none exists, the previous element is blurred. Keeping this
/// responsibility next to the focus model avoids teaching the CDP input layer
/// about labels, delegated shadow focus, or focus event ordering.
pub(crate) fn perform_mouse_focus_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    hit: DomHandle,
) {
    let mut candidate = Some(hit);
    while let Some(handle) = candidate {
        let (should_focus, parent) = {
            let runtime = unsafe { &*runtime_ptr };
            let delegates_focus =
                runtime
                    .dom_host()
                    .shadow_root_handle(handle)
                    .is_some_and(|root| {
                        runtime.dom_host().shadow_root_delegates_focus(root) == Some(true)
                            && delegated_focus_target(runtime, handle).is_some()
                    });
            let should_focus = is_focusable(runtime, handle) || delegates_focus;
            let parent = if should_focus {
                None
            } else {
                flat_tree_parent(runtime, handle)
            };
            (should_focus, parent)
        };
        if should_focus {
            focus_element(scope, runtime_ptr, handle);
            return;
        }
        candidate = parent;
    }
    update_focus(scope, runtime_ptr, None);
}

fn flat_tree_parent(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    if let Some(slot) = runtime.dom_host().assigned_slot_for_node(handle) {
        return Some(slot);
    }
    let parent = runtime.dom_host().parent_node(handle)?;
    if runtime.dom_host().is_shadow_root(parent) {
        return runtime.dom_host().shadow_root_host(parent);
    }
    Some(parent)
}

fn focus_element_with_options(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    prevent_scroll: bool,
) -> Result<(), moli_layout::LayoutError> {
    let runtime = unsafe { &*runtime_ptr };
    if runtime.dom_host().is_html_element_named(handle, "label")
        && !label_receives_programmatic_focus(runtime, handle)
    {
        if let Some(control) = label_control_handle(runtime, handle)
            && is_focusable(runtime, control)
        {
            focus_target(scope, runtime_ptr, control, prevent_scroll)?;
        }
        return Ok(());
    }
    if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle)
        && runtime.dom_host().shadow_root_delegates_focus(shadow_root) == Some(true)
    {
        if let Some(target) = delegated_focus_target(runtime, handle) {
            focus_target(scope, runtime_ptr, target, prevent_scroll)?;
        }
        return Ok(());
    }
    if is_focusable(runtime, handle) {
        focus_target(scope, runtime_ptr, handle, prevent_scroll)?;
    }
    Ok(())
}

pub(crate) fn focus_live_element_for_inspector(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> RendererDomFocusOutcome {
    let runtime = unsafe { &*runtime_ptr };
    let Some(node) = runtime.dom_host().node(handle) else {
        return RendererDomFocusOutcome::NodeNotFound;
    };
    if !node.is_element() {
        return RendererDomFocusOutcome::NodeNotElement;
    }
    let delegates_focus = runtime
        .dom_host()
        .shadow_root_handle(handle)
        .is_some_and(|root| {
            runtime.dom_host().shadow_root_delegates_focus(root) == Some(true)
                && delegated_focus_target(runtime, handle).is_some()
        });
    if !is_focusable(runtime, handle) && !delegates_focus {
        return RendererDomFocusOutcome::ElementNotFocusable;
    }

    focus_element(scope, runtime_ptr, handle);
    RendererDomFocusOutcome::Focused
}

fn focus_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    prevent_scroll: bool,
) -> Result<(), moli_layout::LayoutError> {
    update_focus(scope, runtime_ptr, Some(handle));
    if !prevent_scroll {
        let _ = scroll_node_into_view_if_needed(scope, runtime_ptr, handle, None)?;
    }
    Ok(())
}

fn focus_options_prevent_scroll(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<bool> {
    if args.length() == 0 || args.get(0).is_null_or_undefined() {
        return Some(false);
    }
    let options = args.get(0).to_object(scope)?;
    options
        .get(scope, v8str(scope, "preventScroll").into())
        .map(|value| value.boolean_value(scope))
}

fn throw_focus_layout_error(scope: &mut v8::PinScope<'_, '_>, error: moli_layout::LayoutError) {
    let message = format!("Layout failed while focusing element: {error}");
    if let Some(message) = v8_string(scope, &message) {
        let exception = v8::Exception::error(scope, message);
        scope.throw_exception(exception);
    }
}

pub(in crate::native_bridge) fn node_focus_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        document::detached_focus_method_callback(scope, args, rv);
        return;
    };
    let Some(prevent_scroll) = focus_options_prevent_scroll(scope, &args) else {
        return;
    };
    if let Err(error) = focus_element_with_options(scope, runtime_ptr, handle, prevent_scroll) {
        throw_focus_layout_error(scope, error);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_blur_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        document::detached_blur_method_callback(scope, args, rv);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let delegated_active = runtime
        .active_element_handle()
        .zip(runtime.dom_host().shadow_root_handle(handle))
        .is_some_and(|(active, root)| {
            runtime.dom_host().shadow_root_delegates_focus(root) == Some(true)
                && shadow_including_contains(runtime, root, active)
        });
    if runtime.active_element_handle() == Some(handle) || delegated_active {
        update_focus(scope, runtime_ptr, None);
    }
    rv.set_undefined();
}
