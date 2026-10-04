use anyhow::Result;
use moli_file_api::data_transfer::{
    drag_operations_mask_from_effect_allowed, drop_effect_allowed_by_effect_allowed,
    preferred_drop_effect_from_mask,
};
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

mod drag_source;
use drag_source::dispatch_drag_source_event;

use super::input_dispatch_outcome;
use super::input_helpers::{
    MouseFrameCapture, mouse_button_mask, single_changed_mouse_button,
};
use super::inspector::{
    current_selection_state, is_space_key, key_target_info, option_is_disabled, radio_group_members,
};
use super::{
    ActiveDragSession, ActiveScrollbarDrag, ActiveTouchPoint, PendingDragGesture, ScriptVm,
};
use crate::context_bootstrap::DragDataStoreMode;
use crate::document_runtime::DomHandle;
use crate::dom::{
    forms::InputType,
    native::{Node, SelectedFile},
};
use crate::native_bridge::element::{
    NodePublicEventDispatchOutcome, construct_activation_pointer_event, InputEventCoordinates, TextEditInputType, TouchEventPoint,
    activate_handle_after_pointer_release, activate_handle_via_click,
    activate_handle_via_click_with_detail_and_modifiers, cache_input_files_from_selected_files,
    drag_start_frame_input_surface_hit_test, draggable_source_at_point, captured_frame_input_surface_hit_test, construct_drag_event_with_related_target, construct_keyboard_event,
    construct_mouse_event_with_detail_and_modifiers, construct_mouse_event_for_target,
    construct_mouse_event_with_related_target_for_target, construct_pointer_event,
    construct_pointer_event_with_modifiers, construct_pointer_event_with_related_target,
    construct_pointer_event_with_related_target_and_modifiers, construct_simple_event,
    construct_touch_event, construct_touch_event_with_points, construct_wheel_event_for_target,
    contenteditable_editing_host, dispatch_public_event, input_surface_hit_test, is_text_control,
    observable_input_hit_test, perform_auxiliary_link_default_action,
    perform_clipboard_key_default_action, perform_drop_default_action,
    perform_implicit_submission_from_control, perform_mouse_focus_default_action,
    perform_scrollbar_scroll_default_action, perform_wheel_scroll_default_action,
    replace_contenteditable_selection, replace_text_control_selection,
    select_contenteditable_contents, text_control_set_selection_range_internal,
    text_control_set_selection_range_with_direction_internal, text_control_value, update_focus,
};
use crate::native_bridge::{
    CurrentInputEvent, CurrentInputEventScope, MouseReleaseFollowUp, PointerCaptureDispatchEvent,
};
use crate::runtime::{
    RendererDragData, RendererInputDispatchOutcome, RendererInterceptedDrag,
    RendererPointerEventProperties, RendererTouchPoint,
};
use crate::util::{
    node_wrapper_from_handle, utf16_len, utf16_next_scalar_boundary,
    utf16_previous_scalar_boundary, utf16_scalar_boundary_at_or_after, utf16_units,
};

fn related_target_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Value>> {
    node_wrapper_from_handle(scope, handle?).map(Into::into)
}

fn frozen_frame_transform(
    tree: &moli_layout::FrozenLayoutTree<DomHandle>,
    target_root: DomHandle,
) -> Option<moli_layout::LayoutTransform2D> {
    if tree.source_root() == target_root {
        return Some(moli_layout::LayoutTransform2D::IDENTITY);
    }
    tree.embedded_frames().find_map(|frame| {
        let child_to_target = frozen_frame_transform(&frame.tree, target_root)?;
        let frame_box = tree.source_output(frame.source)?.principal_box?;
        let geometry = tree.box_geometry(frame_box)?;
        let viewport_to_local = tree
            .coordinate_space(geometry.coordinate_space)?
            .local_to_viewport
            .inverse()?;
        let parent_to_child = moli_layout::LayoutTransform2D::translation(
            -geometry.content_box.x,
            -geometry.content_box.y,
        )
        .concatenate(viewport_to_local);
        Some(child_to_target.concatenate(parent_to_child))
    })
}

fn root_to_document_frame(
    runtime: &crate::native_bridge::JsContextHost,
    document: DomHandle,
) -> Option<moli_layout::LayoutTransform2D> {
    let dom = runtime.dom_host();
    let root_document = dom.document_handle();
    if document == root_document {
        return Some(moli_layout::LayoutTransform2D::IDENTITY);
    }
    let target_root = dom.dom().document_element_handle_for_document(document)?;
    runtime
        .with_latest_layout_tree_for_document(root_document, |tree| {
            frozen_frame_transform(tree, target_root)
        })
        .flatten()
}

fn drag_related_target_in_document(
    runtime: &crate::native_bridge::JsContextHost,
    document: DomHandle,
    mut related: DomHandle,
) -> Option<DomHandle> {
    loop {
        let related_document = runtime.dom_host().owner_document_handle(related)?;
        if related_document == document {
            return Some(related);
        }
        related = runtime.child_browsing_context_host_for_document_handle(related_document)?;
    }
}

fn create_drag_data_transfer_view_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    target: DomHandle,
    store: &crate::context_bootstrap::NativeDragDataStore,
    mode: DragDataStoreMode,
    drop_effect: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let context =
        crate::native_bridge::node_owner_document_relevant_context(scope, runtime_ptr, target)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    crate::context_bootstrap::create_drag_data_transfer_view(scope, store, mode, drop_effect)
}

#[allow(clippy::too_many_arguments)]
fn dispatch_native_drag_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    event_type: &str,
    target: DomHandle,
    related: Option<DomHandle>,
    coordinates: InputEventCoordinates,
    transfer: v8::Local<'s, v8::Object>,
    modifiers: u8,
) -> Option<crate::native_bridge::element::NodePublicEventDispatchOutcome> {
    let (coordinates, related) = {
        let runtime = unsafe { &*runtime_ptr };
        let dom = runtime.dom_host();
        let document = dom.owner_document_handle(target)?;
        let transform = root_to_document_frame(runtime, document)?;
        let client = transform.map_point(moli_layout::LayoutPoint::new(
            coordinates.screen_x as f32,
            coordinates.screen_y as f32,
        ));
        let related =
            related.and_then(|related| drag_related_target_in_document(runtime, document, related));
        (
            coordinates.with_client(f64::from(client.x), f64::from(client.y)),
            related,
        )
    };
    let context =
        crate::native_bridge::node_owner_document_relevant_context(scope, runtime_ptr, target)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let related = related_target_value(scope, related);
    let event = construct_drag_event_with_related_target(
        scope,
        event_type,
        coordinates,
        transfer.into(),
        related,
        modifiers,
    )?;
    Some(dispatch_public_event(scope, runtime_ptr, target, event))
}

#[derive(Clone, Copy)]
enum DragFinishReason {
    Dropped,
    DropFailed,
    Canceled,
    Rejected,
}

/// Take the session before calling script so every exit path ends it once.
fn finish_drag_session<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    active: &mut Option<ActiveDragSession>,
    reason: DragFinishReason,
    coordinates: InputEventCoordinates,
    modifiers: u8,
) {
    let Some(session) = active.take() else {
        return;
    };
    if matches!(
        reason,
        DragFinishReason::Canceled | DragFinishReason::Rejected
    ) && let Some(target) = session.target
    {
        if !session.intercepted {
            dispatch_drag_source_event(scope, runtime_ptr, &session, coordinates, modifiers);
        }
        if let Some(data_transfer) = create_drag_data_transfer_view_for_target(
            scope,
            runtime_ptr,
            target,
            &session.data_store,
            DragDataStoreMode::Protected,
            "none",
        ) {
            let _ = dispatch_native_drag_event(
                scope,
                runtime_ptr,
                "dragleave",
                target,
                None,
                coordinates,
                data_transfer,
                modifiers,
            );
            crate::context_bootstrap::finish_drag_data_transfer_view(scope, data_transfer);
        }
    }
    if let Some(source) = session.source
        && let Some(source_transfer) = session.source_transfer.as_ref()
    {
        let data_transfer = v8::Local::new(scope, source_transfer);
        // The source keeps its identity and protected original metadata.
        // Only the final operation is published back to dragend listeners;
        // source data never becomes readable during the target's drop.
        let effect = if matches!(reason, DragFinishReason::Dropped) {
            session.drop_effect.as_str()
        } else {
            "none"
        };
        crate::context_bootstrap::native_set_drag_drop_effect(scope, data_transfer, effect);
        let _ = dispatch_native_drag_event(
            scope,
            runtime_ptr,
            "dragend",
            source,
            None,
            coordinates,
            data_transfer,
            modifiers,
        );
        crate::context_bootstrap::finish_drag_data_transfer_view(scope, data_transfer);
    }
    unsafe { &mut *runtime_ptr }.clear_mouse_press();
}

fn author_drag_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transfer: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let allowed = crate::context_bootstrap::native_data_transfer_effect_allowed(scope, transfer);
    let operation = crate::context_bootstrap::native_data_transfer_drop_effect(scope, transfer);
    drop_effect_allowed_by_effect_allowed(Some(&allowed), &operation).then_some(operation)
}

fn accepted_drag_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    target: DomHandle,
    transfer: v8::Local<'s, v8::Object>,
    canceled: bool,
) -> Option<String> {
    if canceled {
        return author_drag_operation(scope, transfer);
    }
    let allowed = crate::context_bootstrap::native_data_transfer_effect_allowed(scope, transfer);
    (crate::native_bridge::element::accepts_drop_default_action(
        scope,
        runtime_ptr,
        target,
        transfer,
    ) && drop_effect_allowed_by_effect_allowed(Some(&allowed), "copy"))
    .then(|| "copy".to_owned())
}

/// One renderer update owns one target view. A transition delivers enter before
/// leave using that view; a later update delivers over using a new view.
fn update_drag_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    session: &mut ActiveDragSession,
    target: Option<DomHandle>,
    coordinates: InputEventCoordinates,
    modifiers: u8,
) {
    session.coordinates = coordinates;
    session.modifiers = modifiers;
    session.drop_allowed = false;
    session.drop_effect = "none".to_owned();
    let target_document = target.and_then(|target| {
        unsafe { &*runtime_ptr }
            .dom_host()
            .owner_document_handle(target)
    });
    if target.is_some() && (session.target != target || !session.only_drag_over_next) {
        dispatch_drag_source_event(scope, runtime_ptr, session, coordinates, modifiers);
    }
    let Some(view_target) = target.or(session.target) else {
        return;
    };
    // Blink shares the transition view between enter and leave, even across
    // Documents. Its wrapper belongs to the newly entered Window; each Event
    // itself belongs to its receiver's Window.
    let Some(transfer) = create_drag_data_transfer_view_for_target(
        scope,
        runtime_ptr,
        view_target,
        &session.data_store,
        DragDataStoreMode::Protected,
        "none",
    ) else {
        return;
    };
    let allowed = crate::context_bootstrap::native_data_transfer_effect_allowed(scope, transfer);
    crate::context_bootstrap::native_set_drag_drop_effect(
        scope,
        transfer,
        preferred_drop_effect_from_mask(drag_operations_mask_from_effect_allowed(&allowed)),
    );
    let mut operation = None;
    if session.target != target {
        if let Some(target) = target
            && let Some(dispatched) = dispatch_native_drag_event(
                scope,
                runtime_ptr,
                "dragenter",
                target,
                session.target,
                coordinates,
                transfer,
                modifiers,
            )
        {
            let canceled = !dispatched.allows_default();
            operation = accepted_drag_operation(scope, runtime_ptr, target, transfer, canceled);
        }
        if let Some(previous) = session.target {
            crate::context_bootstrap::native_set_drag_drop_effect(scope, transfer, "none");
            let _ = dispatch_native_drag_event(
                scope,
                runtime_ptr,
                "dragleave",
                previous,
                target,
                coordinates,
                transfer,
                modifiers,
            );
        }
        session.target = target;
        session.only_drag_over_next = target.is_some();
    } else if let Some(target) = target
        && let Some(dispatched) = dispatch_native_drag_event(
            scope,
            runtime_ptr,
            "dragover",
            target,
            None,
            coordinates,
            transfer,
            modifiers,
        )
    {
        let canceled = !dispatched.allows_default();
        operation = accepted_drag_operation(scope, runtime_ptr, target, transfer, canceled);
        session.only_drag_over_next = false;
    }
    crate::context_bootstrap::finish_drag_data_transfer_view(scope, transfer);
    if !target
        .zip(target_document)
        .is_some_and(|(target, document)| {
            let host = unsafe { &*runtime_ptr }.dom_host();
            host.is_connected(target) && host.owner_document_handle(target) == Some(document)
        })
    {
        session.target = None;
        session.only_drag_over_next = false;
        return;
    }
    if let Some(operation) = operation {
        session.drop_allowed = true;
        session.drop_effect = operation;
    }
}

fn drop_drag_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    session: &mut ActiveDragSession,
    coordinates: InputEventCoordinates,
    modifiers: u8,
) -> DragFinishReason {
    let Some(target) = session.target.filter(|_| session.drop_allowed) else {
        return DragFinishReason::Rejected;
    };
    let host = unsafe { &*runtime_ptr }.dom_host();
    let Some(document) = host
        .owner_document_handle(target)
        .filter(|_| host.is_connected(target))
    else {
        return DragFinishReason::Rejected;
    };
    let Some(transfer) = create_drag_data_transfer_view_for_target(
        scope,
        runtime_ptr,
        target,
        &session.data_store,
        DragDataStoreMode::ReadOnly,
        &session.drop_effect,
    ) else {
        return DragFinishReason::Rejected;
    };
    let Some(dispatched) = dispatch_native_drag_event(
        scope,
        runtime_ptr,
        "drop",
        target,
        None,
        coordinates,
        transfer,
        modifiers,
    ) else {
        crate::context_bootstrap::finish_drag_data_transfer_view(scope, transfer);
        return DragFinishReason::Rejected;
    };
    let default_allowed = dispatched.allows_default();
    let requested_effect =
        crate::context_bootstrap::native_data_transfer_drop_effect(scope, transfer);
    let allowed = crate::context_bootstrap::native_data_transfer_effect_allowed(scope, transfer);
    crate::context_bootstrap::finish_drag_data_transfer_view(scope, transfer);
    if !default_allowed {
        if drop_effect_allowed_by_effect_allowed(Some(&allowed), &requested_effect) {
            session.drop_effect = requested_effect;
            return DragFinishReason::Dropped;
        }
        session.drop_effect = "none".to_owned();
        return DragFinishReason::DropFailed;
    }
    let host = unsafe { &*runtime_ptr }.dom_host();
    if !host.is_connected(target) || host.owner_document_handle(target) != Some(document) {
        session.drop_effect = "none".to_owned();
        return DragFinishReason::DropFailed;
    }
    let mut completed = false;
    if let Some(default_transfer) = create_drag_data_transfer_view_for_target(
        scope,
        runtime_ptr,
        target,
        &session.data_store,
        DragDataStoreMode::ReadOnly,
        &session.drop_effect,
    ) {
        // The DOM drop view is already disabled before beforeinput/input run.
        // The existing editing code consumes a private view of the same store.
        completed = perform_drop_default_action(scope, runtime_ptr, target, default_transfer);
        crate::context_bootstrap::finish_drag_data_transfer_view(scope, default_transfer);
    }
    if completed {
        DragFinishReason::Dropped
    } else {
        session.drop_effect = "none".to_owned();
        DragFinishReason::DropFailed
    }
}

fn native_drag_source(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    gesture: PendingDragGesture,
) -> Option<(DomHandle, RendererDragData)> {
    let runtime = unsafe { &*runtime_ptr };
    let host = runtime.dom_host();
    let point = moli_layout::LayoutPoint::new(gesture.x as f32, gesture.y as f32);
    let root_point = gesture.root_to_frame.inverse()?.map_point(point);
    let hit = drag_start_frame_input_surface_hit_test(
        runtime, gesture.document, root_point, gesture.root_to_frame,
    ).ok()?.input?;
    if host.owner_document_handle(hit.handle) != Some(gesture.document) {
        return None;
    }
    let candidate = draggable_source_at_point(runtime, gesture.document, point).ok()?
        .map(|handle| {
            let source_attribute = host.node(handle).and_then(Node::as_element)
                .and_then(|element| {
                    if element.is_html_element("img") { Some("src") }
                    else if element.is_html_element("a") && element.has_attribute_ns("", "href") { Some("href") }
                    else { None }
                });
            (handle, source_attribute)
        });
    if candidate.is_none_or(|(_, attribute)| attribute != Some("src"))
        && let Some(data) = super::native_drag_data::selection_drag_data_at_point(
            scope,
            runtime_ptr,
            hit.handle,
            gesture.x,
            gesture.y,
        )
    {
        return Some((hit.handle, data));
    }
    let (source, _) = candidate?;
    Some((
        source,
        super::native_drag_data::element_drag_data(runtime, source),
    ))
}

fn tracks_mouse_hover_for_event(event_name: &str) -> bool {
    matches!(event_name, "mousedown" | "mouseup" | "mousemove")
}

fn pointer_event_name_for_mouse_event(
    event_name: &str,
    previous_buttons: i32,
    buttons: i32,
) -> Option<&'static str> {
    match event_name {
        "mousedown" if previous_buttons != 0 => Some("pointermove"),
        "mousedown" => Some("pointerdown"),
        "mouseup" if buttons != 0 => Some("pointermove"),
        "mouseup" => Some("pointerup"),
        "mousemove" => Some("pointermove"),
        _ => None,
    }
}

fn pointer_event_name_for_touch_event(event_name: &str) -> Option<&'static str> {
    match event_name {
        "touchstart" => Some("pointerdown"),
        "touchmove" => Some("pointermove"),
        "touchend" => Some("pointerup"),
        "touchcancel" => Some("pointercancel"),
        _ => None,
    }
}

fn can_suppress_compat_mouse_event(event_name: &str) -> bool {
    matches!(event_name, "mousedown" | "mouseup" | "mousemove")
}

const TOUCH_POINTER_ID: i32 = 2;

struct PreparedMouseInputDispatch {
    button: i32,
    buttons: i32,
    pointer_event_name: Option<&'static str>,
}

fn touch_pointer_id(touch_id: i32) -> i32 {
    TOUCH_POINTER_ID.saturating_add(touch_id.max(0))
}

fn button_for_touch_pointer_event(event_name: &str) -> i32 {
    match event_name {
        "touchstart" | "touchend" => 0,
        _ => -1,
    }
}

fn buttons_for_touch_pointer_event(event_name: &str) -> i32 {
    match event_name {
        "touchend" | "touchcancel" => 0,
        _ => 1,
    }
}

fn touch_pointer_properties(event_name: &str, pointer_id: i32) -> RendererPointerEventProperties {
    RendererPointerEventProperties {
        pointer_id,
        pointer_type: "touch".to_owned(),
        pressure: match event_name {
            "touchend" | "touchcancel" => 0.0,
            _ => 0.5,
        },
        ..RendererPointerEventProperties::default()
    }
}

fn dispatch_native_pointer_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    target: DomHandle,
    event: v8::Local<'s, v8::Object>,
    pointer_id: i32,
) -> NodePublicEventDispatchOutcome {
    // The native event recipient determines the pointer's active document. Authored
    // dispatchEvent() and compatibility mouse events must not change this identity.
    unsafe { &mut *runtime_ptr }.record_pointer_input_target(pointer_id, target);
    dispatch_public_event(scope, runtime_ptr, target, event)
}

fn dispatch_pointer_capture_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    pointer_id: i32,
    events: &[PointerCaptureDispatchEvent],
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
    modifiers: u8,
) {
    for capture_event in events {
        if capture_event.event_name == "gotpointercapture"
            && !unsafe { &*runtime_ptr }.pointer_capture_target_is_connected(capture_event.target)
        {
            unsafe { &mut *runtime_ptr }
                .clear_pointer_capture_target_if_matches(pointer_id, capture_event.target);
            continue;
        }
        if let Some(event) = construct_pointer_event_with_modifiers(
            scope,
            capture_event.event_name,
            coordinates,
            button,
            buttons,
            pointer,
            modifiers,
        ) {
            let _ = dispatch_native_pointer_event(
                scope,
                runtime_ptr,
                capture_event.target,
                event,
                pointer_id,
            );
        }
        if capture_event.event_name == "gotpointercapture" {
            let lost_target = unsafe { &mut *runtime_ptr }
                .lost_pointer_capture_target_after_got(pointer_id, capture_event.target);
            if let Some(lost_target) = lost_target
                && let Some(event) = construct_pointer_event_with_modifiers(
                    scope,
                    "lostpointercapture",
                    coordinates,
                    button,
                    buttons,
                    pointer,
                    modifiers,
                )
            {
                let _ = dispatch_native_pointer_event(
                    scope,
                    runtime_ptr,
                    lost_target,
                    event,
                    pointer_id,
                );
            }
        }
    }
}

fn release_pointer_capture_after_pointer_end(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    pointer_id: i32,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
    modifiers: u8,
) {
    let release_capture_events = {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime.release_pending_pointer_capture_target(pointer_id);
        runtime.process_pending_pointer_capture(pointer_id)
    };
    dispatch_pointer_capture_events(
        scope,
        runtime_ptr,
        pointer_id,
        &release_capture_events,
        coordinates,
        button,
        buttons,
        pointer,
        modifiers,
    );
}

fn dispatch_drag_pointer_suppression_boundaries(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    target: DomHandle,
    coordinates: InputEventCoordinates,
    pointer: &RendererPointerEventProperties,
    modifiers: u8,
) {
    if !unsafe { &*runtime_ptr }.dom_host().is_connected(target) {
        return;
    }
    if let Some(event) = construct_pointer_event_with_related_target_and_modifiers(
        scope,
        "pointerout",
        coordinates,
        -1,
        0,
        pointer,
        None,
        modifiers,
    ) {
        let _ = dispatch_public_event(scope, runtime_ptr, target, event);
    }
    // Like Blink's boundary dispatcher, freeze the flat ancestor chain after
    // pointerout, then send the non-bubbling leaves from child to parent.
    let ancestors = {
        let dom = unsafe { &*runtime_ptr }.dom_host();
        if dom.is_connected(target) {
            std::iter::successors(Some(target), |node| dom.flat_tree_parent(*node))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    };
    for ancestor in ancestors {
        if let Some(event) = construct_pointer_event_with_related_target_and_modifiers(
            scope,
            "pointerleave",
            coordinates,
            -1,
            0,
            pointer,
            None,
            modifiers,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, ancestor, event);
        }
    }
}

fn dispatch_touch_pointer_boundary_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    previous: Option<DomHandle>,
    current: Option<DomHandle>,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
) {
    if previous == current {
        return;
    }
    if let Some(previous_handle) = previous {
        let related_target = related_target_value(scope, current);
        if let Some(event) = construct_pointer_event_with_related_target(
            scope,
            "pointerout",
            InputEventCoordinates::from_root(x, y),
            button,
            buttons,
            pointer,
            related_target,
        ) {
            let _ = dispatch_native_pointer_event(
                scope,
                runtime_ptr,
                previous_handle,
                event,
                pointer.pointer_id,
            );
        }
        let related_target = related_target_value(scope, current);
        if let Some(event) = construct_pointer_event_with_related_target(
            scope,
            "pointerleave",
            InputEventCoordinates::from_root(x, y),
            button,
            buttons,
            pointer,
            related_target,
        ) {
            let _ = dispatch_native_pointer_event(
                scope,
                runtime_ptr,
                previous_handle,
                event,
                pointer.pointer_id,
            );
        }
    }
    if let Some(current_handle) = current {
        let related_target = related_target_value(scope, previous);
        if let Some(event) = construct_pointer_event_with_related_target(
            scope,
            "pointerover",
            InputEventCoordinates::from_root(x, y),
            button,
            buttons,
            pointer,
            related_target,
        ) {
            let _ = dispatch_native_pointer_event(
                scope,
                runtime_ptr,
                current_handle,
                event,
                pointer.pointer_id,
            );
        }
        let related_target = related_target_value(scope, previous);
        if let Some(event) = construct_pointer_event_with_related_target(
            scope,
            "pointerenter",
            InputEventCoordinates::from_root(x, y),
            button,
            buttons,
            pointer,
            related_target,
        ) {
            let _ = dispatch_native_pointer_event(
                scope,
                runtime_ptr,
                current_handle,
                event,
                pointer.pointer_id,
            );
        }
    }
}

impl ScriptVm {
    /// Completes one JS-capable input protocol command.
    ///
    /// Entering a V8 realm only authorizes the command body; it is not a task
    /// boundary. Input commands therefore submit their Page-agent checkpoint
    /// explicitly after all synchronous dispatch/default-action state has been
    /// settled. If the body failed after partially entering author code, the
    /// checkpoint still runs, while the original body error remains primary.
    fn finish_input_command_checkpoint<T>(&mut self, result: Result<T>) -> Result<T> {
        let checkpoint = self.perform_owner_lane_task_microtask_checkpoints();
        match result {
            Ok(value) => {
                checkpoint?;
                Ok(value)
            }
            Err(error) => {
                if let Err(checkpoint_error) = checkpoint {
                    tracing::warn!(
                        %checkpoint_error,
                        "input command body and command-end checkpoint both failed"
                    );
                }
                Err(error)
            }
        }
    }

    fn finish_input_event_dispatch_turn<T>(&mut self, result: Result<T>) -> Result<T> {
        let result = self.finish_input_command_checkpoint(result);
        if result.is_ok() {
            self.sync_child_browsing_context_records();
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn dispatch_mouse_event_at_point(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<RendererInputDispatchOutcome> {
        self.dispatch_mouse_event_at_point_with_pointer(
            x,
            y,
            event_name,
            button,
            buttons,
            0,
            delta_x,
            delta_y,
            RendererPointerEventProperties::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn dispatch_mouse_event_at_point_with_pointer(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
    ) -> Result<RendererInputDispatchOutcome> {
        self.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
            x,
            y,
            event_name,
            button,
            buttons,
            click_count,
            delta_x,
            delta_y,
            pointer,
            0,
            false,
        )
    }

    pub(crate) fn dispatch_mouse_event_at_point_with_pointer_and_modifiers(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
        modifiers: u8,
        intercept_drags: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        let surface_hit = self.pointer_mouse_input_surface_hit(
            x,
            y,
            event_name,
            button,
            buttons,
            pointer.pointer_id,
        )?;
        let prepared =
            self.prepare_mouse_input_dispatch(event_name, button, buttons, pointer.pointer_id);
        let _current_input_event = CurrentInputEventScope::enter(
            Rc::clone(&self._context_host),
            CurrentInputEvent::mouse(event_name, prepared.button, modifiers),
        );
        let result = self.dispatch_mouse_event_at_point_with_pointer_and_modifiers_body(
            x,
            y,
            event_name,
            prepared,
            surface_hit,
            click_count,
            delta_x,
            delta_y,
            pointer,
            modifiers,
            intercept_drags,
        );
        if event_name == "mouseup" {
            self._context_host.borrow_mut().clear_mouse_press();
        }
        self.finish_input_event_dispatch_turn(result)
    }

    pub(crate) fn begin_batched_mouse_event_dispatch(&mut self) {
        self._context_host
            .borrow_mut()
            .begin_scroll_observable_effect_batch();
    }

    pub(crate) fn finish_batched_mouse_event_dispatch(
        &mut self,
        dispatch_result: Result<()>,
        commit_effects: bool,
    ) -> Result<()> {
        let effects = self
            ._context_host
            .borrow_mut()
            .finish_scroll_observable_effect_batch();
        let effects_result = match effects.filter(|_| commit_effects) {
            Some(effects) => self.with_default_context_scope(|scope, runtime_ptr| {
                crate::native_bridge::element::apply_scroll_observable_effects(
                    scope,
                    runtime_ptr,
                    effects,
                );
                Ok(())
            }),
            None => Ok(()),
        };
        let result = match (dispatch_result, effects_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(dispatch_error), Ok(())) => Err(dispatch_error),
            (Ok(()), Err(effects_error)) => Err(effects_error),
            (Err(dispatch_error), Err(effects_error)) => Err(anyhow::anyhow!(
                "batched input dispatch failed ({dispatch_error:#}) and its derived scroll effects also failed ({effects_error:#})"
            )),
        };
        self.finish_input_event_dispatch_turn(result)
    }

    pub(crate) fn dispatch_mouse_event_at_point_with_pointer_and_modifiers_without_checkpoint(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
        modifiers: u8,
        intercept_drags: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        let surface_hit = self.pointer_mouse_input_surface_hit(
            x,
            y,
            event_name,
            button,
            buttons,
            pointer.pointer_id,
        )?;
        let prepared =
            self.prepare_mouse_input_dispatch(event_name, button, buttons, pointer.pointer_id);
        let _current_input_event = CurrentInputEventScope::enter(
            Rc::clone(&self._context_host),
            CurrentInputEvent::mouse(event_name, prepared.button, modifiers),
        );
        let result = self.dispatch_mouse_event_at_point_with_pointer_and_modifiers_body(
            x,
            y,
            event_name,
            prepared,
            surface_hit,
            click_count,
            delta_x,
            delta_y,
            pointer,
            modifiers,
            intercept_drags,
        );
        if event_name == "mouseup" {
            self._context_host.borrow_mut().clear_mouse_press();
        }
        result
    }

    pub(crate) fn mouse_input_surface_hit(
        &self,
        x: f64,
        y: f64,
        event_name: &str,
    ) -> Result<crate::native_bridge::element::InputSurfaceHit> {
        Ok(input_surface_hit_test(
            &self._context_host.borrow(),
            self.document_runtime.document_handle(),
            moli_layout::LayoutPoint::new(x as f32, y as f32),
            false,
            matches!(event_name, "mousedown" | "mouseup" | "mousemove"),
        )?)
    }

    fn mouse_input_root_to_frame(
        &self,
        target: DomHandle,
        hit: crate::native_bridge::element::InputSurfaceHit,
    ) -> Result<moli_layout::LayoutTransform2D> {
        let runtime = self._context_host.borrow();
        let dom = runtime.dom_host();
        let document = dom
            .owner_document_handle(target)
            .ok_or(moli_layout::LayoutError::NoLayoutSnapshot)?;
        if let Some(hit) = hit
            .input
            .filter(|hit| dom.owner_document_handle(hit.handle) == Some(document))
        {
            return Ok(hit.root_to_frame);
        }
        // Capture keeps its own frame even when the physical hit moves out of
        // that frame. Read the same frozen content box and inverse transform
        // used by hit testing, without requiring the point to hit the iframe.
        if let Some(transform) = root_to_document_frame(&runtime, document) {
            return Ok(transform);
        }
        if self
            .hovered_mouse_handle
            .and_then(|handle| dom.owner_document_handle(handle))
            == Some(document)
        {
            return Ok(self.hovered_mouse_root_to_frame);
        }
        Err(moli_layout::LayoutError::NoLayoutSnapshot.into())
    }

    fn prepare_mouse_input_dispatch(
        &mut self,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        pointer_id: i32,
    ) -> PreparedMouseInputDispatch {
        let previous_pressed_buttons = self.pressed_mouse_buttons;
        // Moving does not change a button, even when the input protocol carries
        // the held button. Chorded down/up still report their changed button.
        let button = if event_name == "mousemove" {
            -1
        } else if button >= 0 {
            button
        } else {
            match event_name {
                "mousedown" => buttons
                    .and_then(|current_buttons| {
                        single_changed_mouse_button(current_buttons & !previous_pressed_buttons)
                    })
                    .unwrap_or(-1),
                "mouseup" => self
                    ._context_host
                    .borrow()
                    .mouse_press_button()
                    .or_else(|| {
                        buttons.and_then(|current_buttons| {
                            single_changed_mouse_button(previous_pressed_buttons & !current_buttons)
                        })
                    })
                    .or_else(|| single_changed_mouse_button(previous_pressed_buttons))
                    .unwrap_or(-1),
                _ => button,
            }
        };
        let button_mask = mouse_button_mask(button);
        if let Some(explicit_buttons) = buttons {
            self.pressed_mouse_buttons = explicit_buttons;
        } else {
            match event_name {
                "mousedown" => {
                    if button_mask != 0 {
                        self.pressed_mouse_buttons |= button_mask;
                    }
                }
                "mouseup" if button_mask != 0 => {
                    self.pressed_mouse_buttons &= !button_mask;
                }
                _ => {}
            }
        }
        let buttons = buttons.unwrap_or(self.pressed_mouse_buttons);
        if event_name == "mousedown" {
            self._context_host.borrow_mut().clear_mouse_press();
            self.pending_drag_gesture = None;
        } else if event_name == "mouseup" || self.pressed_mouse_buttons & 1 == 0 {
            self.pending_drag_gesture = None;
        }
        // The final release has already been hit tested in the captured frame.
        // Clear its ownership before callbacks can start another input sequence.
        if matches!(event_name, "mouseup" | "mousemove") && buttons == 0 {
            self.mouse_frame_captures.remove(&pointer_id);
        }
        PreparedMouseInputDispatch {
            button,
            buttons,
            pointer_event_name: pointer_event_name_for_mouse_event(
                event_name,
                previous_pressed_buttons,
                buttons,
            ),
        }
    }

    fn captured_mouse_frame_document(&self, capture: MouseFrameCapture) -> Option<DomHandle> {
        let host = self._context_host.borrow();
        if host.current_child_frame_lane_task_owner(capture.frame) != Some(capture.owner) {
            return None;
        }
        let captured_document = host.child_browsing_context_document_handle(capture.frame)?;
        let mut document = captured_document;
        let mut visited = HashSet::new();
        while document != self.document_runtime.document_handle() {
            if !visited.insert(document) {
                return None;
            }
            let frame = host.child_browsing_context_host_for_document_handle(document)?;
            if !host.dom_host().is_connected(frame) {
                return None;
            }
            document = host.dom_host().owner_document_handle(frame)?;
        }
        Some(captured_document)
    }

    fn pointer_mouse_input_surface_hit(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        pointer_id: i32,
    ) -> Result<crate::native_bridge::element::InputSurfaceHit> {
        if !matches!(event_name, "mousedown" | "mouseup" | "mousemove") {
            return self.mouse_input_surface_hit(x, y, event_name);
        }
        let first_press = event_name == "mousedown" && self.pressed_mouse_buttons == 0;
        if first_press || (event_name == "mousemove" && buttons == Some(0)) {
            self.mouse_frame_captures.remove(&pointer_id);
        }
        if let Some(capture) = self.mouse_frame_captures.get(&pointer_id).copied() {
            if let Some(document) = self.captured_mouse_frame_document(capture) {
                return Ok(captured_frame_input_surface_hit_test(
                    &self._context_host.borrow(),
                    document,
                    moli_layout::LayoutPoint::new(x as f32, y as f32),
                    capture.root_to_frame,
                    true,
                )?);
            }
            self.mouse_frame_captures.remove(&pointer_id);
        }
        let hit = self.mouse_input_surface_hit(x, y, event_name)?;
        if first_press && buttons.unwrap_or(mouse_button_mask(button)) != 0 {
            let host = self._context_host.borrow();
            if let Some(input) = hit.input {
                let capture = host
                    .dom_host()
                    .owner_document_handle(input.handle)
                    .and_then(|document| {
                        host.child_browsing_context_host_for_document_handle(document)
                    })
                    .and_then(|frame| {
                        Some(MouseFrameCapture {
                            frame,
                            owner: host.current_child_frame_lane_task_owner(frame)?,
                            root_to_frame: input.root_to_frame,
                        })
                    });
                if let Some(capture) = capture {
                    self.mouse_frame_captures.insert(pointer_id, capture);
                }
            }
        }
        Ok(hit)
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_mouse_event_at_point_with_pointer_and_modifiers_body(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        prepared: PreparedMouseInputDispatch,
        surface_hit: crate::native_bridge::element::InputSurfaceHit,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
        modifiers: u8,
        intercept_drags: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        let PreparedMouseInputDispatch {
            button,
            buttons,
            pointer_event_name,
        } = prepared;

        if self
            .active_drag_session
            .as_ref()
            .is_some_and(|session| session.intercepted && !intercept_drags)
            || event_name == "mousedown" && button == 0
            || event_name == "mousemove" && buttons & 1 == 0
        {
            self.finish_active_drag(DragFinishReason::Canceled, modifiers)?;
        }

        // Native dragging owns the pointer stream until the drag ends. The
        // threshold-crossing move has already sent pointercancel; further
        // moves and the native release drive dragging rather than DOM mouse
        // input. A later explicit CDP release after replay ends is normal input.
        if matches!(event_name, "mousemove" | "mouseup")
            && self
                .active_drag_session
                .as_ref()
                .is_some_and(|session| session.source.is_some())
        {
            let hit = surface_hit.input;
            let target = hit.map(|hit| hit.handle);
            let local = hit.map(|hit| {
                hit.root_to_frame
                    .map_point(moli_layout::LayoutPoint::new(x as f32, y as f32))
            });
            let coordinates = local
                .map(|point| {
                    InputEventCoordinates::from_root(x, y)
                        .with_client(f64::from(point.x), f64::from(point.y))
                })
                .unwrap_or_else(|| InputEventCoordinates::from_root(x, y));
            let active = &mut self.active_drag_session as *mut Option<ActiveDragSession>;
            self.with_default_context_scope(|scope, runtime_ptr| {
                let active = unsafe { &mut *active };
                if let Some(session) = active.as_mut() {
                    session.coordinates = coordinates;
                    if !session.intercepted {
                        update_drag_target(
                            scope,
                            runtime_ptr,
                            session,
                            target,
                            coordinates,
                            modifiers,
                        );
                    }
                    if event_name == "mouseup" && button == 0 {
                        let reason = if session.intercepted {
                            DragFinishReason::Rejected
                        } else {
                            drop_drag_target(scope, runtime_ptr, session, coordinates, modifiers)
                        };
                        finish_drag_session(
                            scope,
                            runtime_ptr,
                            active,
                            reason,
                            coordinates,
                            modifiers,
                        );
                    }
                }
                Ok(())
            })?;
            return Ok(input_dispatch_outcome(true));
        }

        if let Some(outcome) =
            self.dispatch_active_scrollbar_mouse_event(x, y, event_name, button, buttons)?
        {
            return Ok(outcome);
        }

        let root_point = moli_layout::LayoutPoint::new(x as f32, y as f32);
        if let Some(control) = surface_hit.control {
            if event_name == "mouseup" && button == 0 {
                self.finish_active_drag(DragFinishReason::Rejected, modifiers)?;
            }
            return match control {
                moli_layout::LayoutControlSurfaceHit::Scrollbar(scrollbar) => {
                    self.dispatch_native_scrollbar_hit(scrollbar, event_name, button)
                }
                moli_layout::LayoutControlSurfaceHit::ScrollbarCorner(_) => {
                    // The painted corner is user-agent chrome. It owns no
                    // scroll action, but must consume input rather than leak
                    // pointer/mouse events into the DOM underneath it.
                    Ok(input_dispatch_outcome(true))
                }
            };
        }
        let hit = if event_name == "wheel" {
            surface_hit.document_or_element()
        } else {
            surface_hit.input
        };
        let hit_handle = hit.map(|hit| hit.handle);
        let pointer_id = pointer.pointer_id;
        let mut pending_pointer_capture_events = Vec::new();
        if pointer_event_name.is_some() {
            let mut context_host = self._context_host.borrow_mut();
            if event_name == "mousemove" && buttons == 0 {
                context_host.release_pending_pointer_capture_target(pointer_id);
            }
            context_host.update_pointer_input(pointer_id, buttons);
            pending_pointer_capture_events =
                context_host.process_pending_pointer_capture(pointer_id);
        }
        if !pending_pointer_capture_events.is_empty() {
            let capture_events = pending_pointer_capture_events
                .iter()
                .map(|event| {
                    let transform = self.mouse_input_root_to_frame(event.target, surface_hit)?;
                    let client = transform.map_point(root_point);
                    Ok((
                        event,
                        InputEventCoordinates::from_root(x, y)
                            .with_client(f64::from(client.x), f64::from(client.y)),
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            self.with_default_context_scope(|scope, runtime_ptr| {
                for (event, coordinates) in &capture_events {
                    dispatch_pointer_capture_events(
                        scope,
                        runtime_ptr,
                        pointer_id,
                        std::slice::from_ref(*event),
                        *coordinates,
                        button,
                        buttons,
                        &pointer,
                        modifiers,
                    );
                }
                Ok(())
            })?;
        }
        let capture_handle = pointer_event_name
            .is_some()
            .then(|| {
                self._context_host
                    .borrow()
                    .active_pointer_capture_target(pointer_id)
            })
            .flatten();
        if tracks_mouse_hover_for_event(event_name) {
            let mut context_host = self._context_host.borrow_mut();
            context_host.set_hovered_element_for_input(hit_handle);
        }
        let Some(handle) = capture_handle.or(hit_handle) else {
            if event_name == "mouseup" && button == 0 {
                self.finish_active_drag(DragFinishReason::Rejected, modifiers)?;
            } else if event_name == "mousemove" {
                let active = &mut self.active_drag_session as *mut Option<ActiveDragSession>;
                self.with_default_context_scope(|scope, runtime_ptr| {
                    if let Some(session) = unsafe { &mut *active }.as_mut()
                        && !session.intercepted
                    {
                        update_drag_target(
                            scope,
                            runtime_ptr,
                            session,
                            None,
                            InputEventCoordinates::from_root(x, y),
                            modifiers,
                        );
                    }
                    Ok(())
                })?;
            }
            return Ok(input_dispatch_outcome(false));
        };
        let restarting_pointer_stream = pointer_event_name.is_some()
            && self.hovered_mouse_handle.is_some()
            && self._context_host.borrow().mouse_event_target().is_none();
        if pointer_event_name.is_some() {
            self._context_host
                .borrow_mut()
                .set_mouse_event_target(handle);
        }
        let root_to_frame = self.mouse_input_root_to_frame(handle, surface_hit)?;
        let client_point = root_to_frame.map_point(root_point);
        let client_x = f64::from(client_point.x);
        let client_y = f64::from(client_point.y);
        let coordinates = InputEventCoordinates::from_root(x, y).with_client(client_x, client_y);
        let hover_transition = if tracks_mouse_hover_for_event(event_name) {
            let previous = self.hovered_mouse_handle;
            if previous != Some(handle) {
                let previous_root_to_frame = self.hovered_mouse_root_to_frame;
                self.hovered_mouse_handle = Some(handle);
                self.hovered_mouse_root_to_frame = root_to_frame;
                Some((previous, previous_root_to_frame))
            } else {
                self.hovered_mouse_root_to_frame = root_to_frame;
                None
            }
        } else {
            None
        };
        // Dragging suppresses pointer boundaries without moving the legacy
        // mouse hover target. A resumed pointer enters from outside even when
        // the first ordinary mouse event hits that same element again.
        let pointer_hover_transition = if restarting_pointer_stream {
            Some((None, root_to_frame))
        } else {
            hover_transition
        };
        if event_name == "mousemove" && buttons != 1 {
            self.pending_drag_gesture = None;
        }
        let drag_start = if event_name == "mousemove"
            && buttons == 1
            && self.active_drag_session.is_none()
        {
            if self._context_host.borrow().mouse_press_handle().is_none() {
                self.pending_drag_gesture = None;
            }
            let gesture = self.pending_drag_gesture.filter(|gesture| {
                let point = gesture.root_to_frame.map_point(root_point);
                #[cfg(target_os = "macos")]
                let threshold = 3.0;
                #[cfg(not(target_os = "macos"))]
                let threshold = 4.0;
                (f64::from(point.x).floor() - gesture.x.floor()).abs() >= threshold
                    || (f64::from(point.y).floor() - gesture.y.floor()).abs() >= threshold
            });
            if let Some(gesture) = gesture {
                // The gesture is consumed even if dragstart is canceled. A
                // held button cannot start another drag until a fresh press.
                self.pending_drag_gesture = None;
                Some(gesture)
            } else {
                None
            }
        } else {
            None
        };

        if event_name == "mousedown" && button >= 0 {
            self._context_host
                .borrow_mut()
                .begin_mouse_press(handle, button);
            if button == 0 && buttons == 1 && click_count <= 1
                && let Some(document) = self._context_host.borrow().dom_host()
                    .owner_document_handle(hit_handle.unwrap_or(handle))
            {
                self.pending_drag_gesture = Some(PendingDragGesture {
                    document,
                    handle: hit_handle.unwrap_or(handle),
                    x: client_x,
                    y: client_y,
                    root_to_frame,
                });
            }
        }

        let active_drag_session: *mut Option<ActiveDragSession> = &mut self.active_drag_session;
        let intercepted_drag = Cell::new(None);
        let mouse_down_allows_drag = Cell::new(true);

        let mut suppress_compat_mouse_events = self.suppress_compat_mouse_events;
        if event_name == "mousemove" && buttons == 0 {
            suppress_compat_mouse_events = false;
        }

        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            let drag_start = drag_start.and_then(|gesture| {
                let start_root =
                    gesture
                        .root_to_frame
                        .inverse()?
                        .map_point(moli_layout::LayoutPoint::new(
                            gesture.x as f32,
                            gesture.y as f32,
                        ));
                let start_coordinates = InputEventCoordinates::from_root(
                    f64::from(start_root.x),
                    f64::from(start_root.y),
                )
                .with_client(gesture.x, gesture.y);
                native_drag_source(scope, runtime_ptr, gesture)
                    .map(|(source, data)| (source, data, start_coordinates))
            });
            let mut pointer_dispatch_handle = handle;
            if let Some((Some(previous_handle), previous_root_to_frame)) = pointer_hover_transition
            {
                let previous_client_point = previous_root_to_frame.map_point(root_point);
                let previous_coordinates = InputEventCoordinates::from_root(x, y).with_client(
                    f64::from(previous_client_point.x),
                    f64::from(previous_client_point.y),
                );
                let related_target = related_target_value(scope, Some(handle));
                if let Some(event) = construct_pointer_event_with_related_target_and_modifiers(
                    scope,
                    "pointerout",
                    previous_coordinates,
                    button,
                    buttons,
                    &pointer,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_native_pointer_event(
                        scope,
                        runtime_ptr,
                        previous_handle,
                        event,
                        pointer_id,
                    );
                }
                let related_target = related_target_value(scope, Some(handle));
                if let Some(event) = construct_pointer_event_with_related_target_and_modifiers(
                    scope,
                    "pointerleave",
                    previous_coordinates,
                    button,
                    buttons,
                    &pointer,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_native_pointer_event(
                        scope,
                        runtime_ptr,
                        previous_handle,
                        event,
                        pointer_id,
                    );
                }
            }
            if let Some((previous_handle, _)) = pointer_hover_transition {
                let related_target = related_target_value(scope, previous_handle);
                if let Some(event) = construct_pointer_event_with_related_target_and_modifiers(
                    scope,
                    "pointerover",
                    coordinates,
                    button,
                    buttons,
                    &pointer,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_native_pointer_event(
                        scope,
                        runtime_ptr,
                        handle,
                        event,
                        pointer_id,
                    );
                }
                let entering_targets = if restarting_pointer_stream {
                    let dom = unsafe { &*runtime_ptr }.dom_host();
                    let mut ancestors =
                        std::iter::successors(Some(handle), |node| dom.flat_tree_parent(*node))
                            .collect::<Vec<_>>();
                    ancestors.reverse();
                    ancestors
                } else {
                    vec![handle]
                };
                for entering_target in entering_targets {
                    let related_target = related_target_value(scope, previous_handle);
                    if let Some(event) = construct_pointer_event_with_related_target_and_modifiers(
                        scope,
                        "pointerenter",
                        coordinates,
                        button,
                        buttons,
                        &pointer,
                        related_target,
                        modifiers,
                    ) {
                        let _ = dispatch_native_pointer_event(scope, runtime_ptr, entering_target, event, pointer_id);
                    }
                }
            }
            if event_name == "mousemove"
                && let Some(event) = construct_pointer_event_with_modifiers(
                    scope,
                    "pointerrawupdate",
                    coordinates,
                    button,
                    buttons,
                    &pointer,
                    modifiers,
                )
            {
                let _ = dispatch_native_pointer_event(
                    scope,
                    runtime_ptr,
                    pointer_dispatch_handle,
                    event,
                    pointer_id,
                );
                let post_raw_update_capture_events = {
                    let runtime = unsafe { &mut *runtime_ptr };
                    runtime.process_pending_pointer_capture(pointer_id)
                };
                dispatch_pointer_capture_events(
                    scope,
                    runtime_ptr,
                    pointer_id,
                    &post_raw_update_capture_events,
                    coordinates,
                    button,
                    buttons,
                    &pointer,
                    modifiers,
                );
                if !post_raw_update_capture_events.is_empty() {
                    pointer_dispatch_handle = unsafe { &*runtime_ptr }
                        .active_pointer_capture_target(pointer_id)
                        .or(hit_handle)
                        .unwrap_or(pointer_dispatch_handle);
                    unsafe { &mut *runtime_ptr }.set_mouse_event_target(pointer_dispatch_handle);
                }
            }
            if let Some((Some(previous_handle), previous_root_to_frame)) = hover_transition {
                let previous_client_point = previous_root_to_frame.map_point(root_point);
                let previous_coordinates = InputEventCoordinates::from_root(x, y).with_client(
                    f64::from(previous_client_point.x),
                    f64::from(previous_client_point.y),
                );
                let related_target = related_target_value(scope, Some(handle));
                if let Some(event) = construct_mouse_event_with_related_target_for_target(
                    scope,
                    runtime_ptr,
                    previous_handle,
                    "mouseout",
                    previous_coordinates,
                    button,
                    buttons,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
                }
                let remains_inside_previous = {
                    let dom = unsafe { &*runtime_ptr }.dom_host();
                    std::iter::successors(Some(handle), |node| dom.flat_tree_parent(*node))
                        .any(|ancestor| ancestor == previous_handle)
                };
                if !remains_inside_previous {
                    let related_target = related_target_value(scope, Some(handle));
                    if let Some(event) = construct_mouse_event_with_related_target_for_target(
                        scope,
                        runtime_ptr,
                        previous_handle,
                        "mouseleave",
                        previous_coordinates,
                        button,
                        buttons,
                        related_target,
                        modifiers,
                    ) {
                        let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
                    }
                }
            }
            if let Some((previous_handle, _)) = hover_transition {
                let related_target = related_target_value(scope, previous_handle);
                if let Some(event) = construct_mouse_event_with_related_target_for_target(
                    scope,
                    runtime_ptr,
                    handle,
                    "mouseover",
                    coordinates,
                    button,
                    buttons,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
                let related_target = related_target_value(scope, previous_handle);
                if let Some(event) = construct_mouse_event_with_related_target_for_target(
                    scope,
                    runtime_ptr,
                    handle,
                    "mouseenter",
                    coordinates,
                    button,
                    buttons,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
            }
            if let Some(pointer_event_name) = pointer_event_name
                && let Some(event) = construct_pointer_event_with_modifiers(
                    scope,
                    pointer_event_name,
                    coordinates,
                    button,
                    buttons,
                    &pointer,
                    modifiers,
                )
            {
                if (pointer_event_name == "pointerdown" && pointer.pointer_type == "mouse")
                    || (pointer_event_name == "pointerup" && pointer.pointer_type != "mouse")
                {
                    unsafe { &mut *runtime_ptr }
                        .notify_close_watcher_input_activation(pointer_dispatch_handle);
                }
                let dispatched = dispatch_native_pointer_event(
                    scope,
                    runtime_ptr,
                    pointer_dispatch_handle,
                    event,
                    pointer_id,
                );
                if pointer_event_name == "pointerdown" && !dispatched.allows_default() {
                    suppress_compat_mouse_events = true;
                    mouse_down_allows_drag.set(false);
                }
            }
            let mouse_dispatch_handle = if pointer_event_name.is_some() {
                unsafe { &*runtime_ptr }.mouse_event_target()
            } else {
                Some(pointer_dispatch_handle)
            };
            let suppress_current_mouse_event =
                suppress_compat_mouse_events && can_suppress_compat_mouse_event(event_name);
            if !suppress_current_mouse_event {
                let event = match mouse_dispatch_handle {
                    Some(target) if event_name == "wheel" => construct_wheel_event_for_target(
                        scope,
                        runtime_ptr,
                        target,
                        event_name,
                        coordinates,
                        delta_x,
                        delta_y,
                        buttons,
                        modifiers,
                    ),
                    Some(target) => construct_mouse_event_for_target(
                        scope,
                        runtime_ptr,
                        target,
                        event_name,
                        coordinates,
                        button,
                        buttons,
                        modifiers,
                    ),
                    None => None,
                };
                if let Some(event) = event
                    && let Some(mouse_dispatch_handle) = mouse_dispatch_handle
                {
                    if event_name == "mousedown" {
                        unsafe { &mut *runtime_ptr }
                            .notify_close_watcher_input_activation(mouse_dispatch_handle);
                    }
                    let dispatched =
                        dispatch_public_event(scope, runtime_ptr, mouse_dispatch_handle, event);
                    if event_name == "wheel" && dispatched.allows_default() {
                        let _ = perform_wheel_scroll_default_action(
                            scope,
                            runtime_ptr,
                            pointer_dispatch_handle,
                            delta_x,
                            delta_y,
                        )?;
                    }
                    if event_name == "mousedown" && !dispatched.allows_default() {
                        mouse_down_allows_drag.set(false);
                    }
                    if event_name == "mousedown" && dispatched.allows_default() {
                        // Chromium transfers mouse focus only after the
                        // pointer/mouse down events have run and only when
                        // their default action was not canceled. Focus the
                        // actual hit target rather than a pointer-capture
                        // target retained from an earlier event.
                        perform_mouse_focus_default_action(
                            scope,
                            runtime_ptr,
                            hit_handle.unwrap_or(handle),
                        );
                    }
                }
            }
            let captured_click_target = if event_name == "mouseup" {
                let captured =
                    unsafe { &*runtime_ptr }.pointer_capture_target_for_click(pointer_id);
                if pointer_event_name == Some("pointerup") {
                    release_pointer_capture_after_pointer_end(
                    scope,
                    runtime_ptr,
                    pointer_id,
                    coordinates,
                    button,
                    buttons,
                    &pointer,
                    modifiers,
                );
                }
                captured
            } else {
                None
            };
            if let Some((drag_start_handle, initial_data, start_coordinates)) = drag_start
                && !suppress_current_mouse_event
                && let Some(data_store) =
                    crate::context_bootstrap::create_native_drag_data_store(scope)
                && crate::context_bootstrap::populate_native_drag_data_store(
                    scope,
                    &data_store,
                    &initial_data,
                )
                .is_some()
                && let Some(data_transfer) = create_drag_data_transfer_view_for_target(
                    scope,
                    runtime_ptr,
                    drag_start_handle,
                    &data_store,
                    DragDataStoreMode::ReadWrite,
                    "none",
                )
            {
                let source_document = unsafe { &*runtime_ptr }
                    .dom_host()
                    .owner_document_handle(drag_start_handle);
                let allowed = dispatch_native_drag_event(
                    scope,
                    runtime_ptr,
                    "dragstart",
                    drag_start_handle,
                    None,
                    start_coordinates,
                    data_transfer,
                    modifiers,
                )
                .is_some_and(|outcome| outcome.allows_default());
                let (items, drag_operations_mask) =
                    crate::context_bootstrap::read_string_drag_data(scope, &data_store);
                let source_transfer = v8::Global::new(scope, data_transfer);
                if allowed
                    && source_document.is_some()
                    && unsafe { &*runtime_ptr }
                        .dom_host()
                        .is_connected(drag_start_handle)
                    && unsafe { &*runtime_ptr }
                        .dom_host()
                        .owner_document_handle(drag_start_handle)
                        == source_document
                {
                    // Chromium keeps the source's original metadata readable
                    // until dragend, while denying its data during target drop.
                    crate::context_bootstrap::protect_native_drag_source_view(scope, data_transfer);
                    *unsafe { &mut *active_drag_session } = Some(ActiveDragSession {
                        data_store,
                        source_transfer: Some(source_transfer),
                        drop_effect: "none".to_owned(),
                        drop_allowed: false,
                        source: Some(drag_start_handle),
                        intercepted: intercept_drags,
                        target: None,
                        only_drag_over_next: false,
                        coordinates,
                        modifiers,
                    });
                    unsafe { &mut *runtime_ptr }.clear_mouse_press();
                    let cancel_target = unsafe { &*runtime_ptr }
                        .active_pointer_capture_target(pointer_id)
                        .unwrap_or(pointer_dispatch_handle);
                    let cancel_pointer = pointer.clone();
                    if let Some(event) = construct_pointer_event_with_modifiers(
                        scope,
                        "pointercancel",
                        coordinates,
                        -1,
                        0,
                        &cancel_pointer,
                        modifiers,
                    ) {
                        let _ = dispatch_public_event(scope, runtime_ptr, cancel_target, event);
                    }
                    release_pointer_capture_after_pointer_end(
                        scope,
                        runtime_ptr,
                        pointer_id,
                        coordinates,
                        -1,
                        0,
                        &cancel_pointer,
                        modifiers,
                    );
                    dispatch_drag_pointer_suppression_boundaries(
                        scope,
                        runtime_ptr,
                        cancel_target,
                        coordinates,
                        &cancel_pointer,
                        modifiers,
                    );
                    // Preserve ScriptVm's legacy mouse hover cache while
                    // marking this suppressed pointer stream as outside.
                    unsafe { &mut *runtime_ptr }.clear_mouse_input_state();
                    if intercept_drags {
                        intercepted_drag.set(Some(RendererInterceptedDrag {
                            items,
                            drag_operations_mask,
                        }));
                    }
                } else {
                    crate::context_bootstrap::finish_drag_data_transfer_view(scope, data_transfer);
                }
            }
            if event_name == "mousemove"
                && buttons & 1 != 0
                && let Some(session) = unsafe { &mut *active_drag_session }.as_mut()
                && !session.intercepted
            {
                update_drag_target(
                    scope,
                    runtime_ptr,
                    session,
                    hit_handle,
                    coordinates,
                    modifiers,
                );
            }
            if event_name == "mouseup" && button == 0 {
                let active = unsafe { &mut *active_drag_session };
                let mut reason = DragFinishReason::Rejected;
                if let Some(session) = active.as_mut()
                    && !session.intercepted
                {
                    // Revalidate against the release target. Acceptance from
                    // a previously hovered element must never authorize it.
                    update_drag_target(
                        scope,
                        runtime_ptr,
                        session,
                        hit_handle,
                        coordinates,
                        modifiers,
                    );
                    reason = drop_drag_target(scope, runtime_ptr, session, coordinates, modifiers);
                }
                finish_drag_session(scope, runtime_ptr, active, reason, coordinates, modifiers);
            }
            // Keep the host-owned press alive through the release listeners;
            // synchronous DOM removals can invalidate it before click targeting.
            let follow_up = if event_name == "mouseup" {
                mouse_dispatch_handle.and_then(|target| {
                    unsafe { &mut *runtime_ptr }.mouse_release_follow_up(
                        target,
                        button,
                        captured_click_target,
                    )
                })
            } else {
                None
            };
            match follow_up {
                Some(MouseReleaseFollowUp::ActivateViaClick(click_target)) => {
                    if buttons == 0 {
                        suppress_compat_mouse_events = false;
                    }
                    let outcome = activate_handle_after_pointer_release(
                        scope,
                        runtime_ptr,
                        click_target,
                        coordinates,
                        button,
                        buttons,
                        click_count.max(1),
                        modifiers,
                        &pointer,
                    );
                    if click_count >= 2
                        && let Some(event) = construct_mouse_event_with_detail_and_modifiers(
                            scope,
                            "dblclick",
                            coordinates,
                            click_count,
                            button,
                            buttons,
                            modifiers,
                        )
                    {
                        let _ = dispatch_public_event(scope, runtime_ptr, click_target, event);
                    }
                    return Ok(outcome);
                }
                Some(MouseReleaseFollowUp::Auxiliary) => {
                    if buttons == 0 {
                        suppress_compat_mouse_events = false;
                    }
                    let had_pending_top_level_navigation_before_event =
                        unsafe { &*runtime_ptr }.has_pending_location_navigation();
                    let pending_child_navigations_before_event = unsafe { &*runtime_ptr }
                        .pending_live_child_browsing_context_navigation_snapshot();
                    let mut pending_download = None;
                    // Platforms may show the context menu before or after mouseup.
                    // Keep our release-time policy; cancellation does not suppress auxclick.
                    if button == 2
                        && let Some(event) = construct_activation_pointer_event(
                            scope,
                            runtime_ptr,
                            handle,
                            "contextmenu",
                            coordinates,
                            0,
                            button,
                            buttons,
                            modifiers,
                            Some(&pointer),
                        )
                    {
                        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                    }
                    if let Some(target) = unsafe { &mut *runtime_ptr }.take_mouse_click_target(
                        pointer_dispatch_handle,
                        button,
                        captured_click_target,
                    ) && let Some(event) = construct_activation_pointer_event(
                        scope,
                        runtime_ptr,
                        target,
                        "auxclick",
                        coordinates,
                        click_count.max(1),
                        button,
                        buttons,
                        modifiers,
                        Some(&pointer),
                    ) {
                        let dispatched = dispatch_public_event(scope, runtime_ptr, target, event);
                        if dispatched.allows_default() {
                            pending_download = perform_auxiliary_link_default_action(
                                scope,
                                runtime_ptr,
                                target,
                                button,
                                modifiers,
                                &pending_child_navigations_before_event,
                            );
                        }
                    }
                    return Ok(RendererInputDispatchOutcome {
                        handled: true,
                        triggered_top_level_navigation:
                            !had_pending_top_level_navigation_before_event
                                && unsafe { &*runtime_ptr }.has_pending_location_navigation(),
                        pending_download,
                        pending_file_chooser: None,
                        intercepted_drag: None,
                    });
                }
                None => {}
            }
            if event_name == "mouseup" && buttons == 0 {
                suppress_compat_mouse_events = false;
            }
            Ok(input_dispatch_outcome(true))
        });
        self.suppress_compat_mouse_events = suppress_compat_mouse_events;
        if !mouse_down_allows_drag.get() {
            self.pending_drag_gesture = None;
        }
        result.map(|mut outcome| {
            outcome.intercepted_drag = intercepted_drag.take();
            outcome
        })
    }

    fn finish_active_drag(&mut self, reason: DragFinishReason, modifiers: u8) -> Result<()> {
        let coordinates = self
            .active_drag_session
            .as_ref()
            .map(|session| session.coordinates)
            .unwrap_or_default();
        self.pending_drag_gesture = None;
        self._context_host.borrow_mut().clear_mouse_press();
        let active = &mut self.active_drag_session as *mut Option<ActiveDragSession>;
        self.with_default_context_scope(|scope, runtime_ptr| {
            // SAFETY: with_default_context_scope runs synchronously under
            // the exclusive ScriptVm borrow, without aliasing this field.
            finish_drag_session(
                scope,
                runtime_ptr,
                unsafe { &mut *active },
                reason,
                coordinates,
                modifiers,
            );
            Ok(())
        })
    }

    pub(crate) fn cancel_dragging(&mut self) -> Result<RendererInputDispatchOutcome> {
        let modifiers = self
            .active_drag_session
            .as_ref()
            .map(|session| session.modifiers)
            .unwrap_or(0);
        let result = self
            .finish_active_drag(DragFinishReason::Canceled, modifiers)
            .map(|()| input_dispatch_outcome(true));
        self.finish_input_event_dispatch_turn(result)
    }

    fn dispatch_active_scrollbar_mouse_event(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: i32,
    ) -> Result<Option<RendererInputDispatchOutcome>> {
        let Some(drag) = self.active_scrollbar_drag else {
            return Ok(None);
        };
        match event_name {
            "mousemove" if buttons & 1 != 0 => {
                let local = drag
                    .viewport_to_local
                    .map_point(moli_layout::LayoutPoint::new(x as f32, y as f32));
                let coordinate = match drag.scrollbar.axis {
                    moli_layout::LayoutScrollbarAxis::Horizontal => local.x,
                    moli_layout::LayoutScrollbarAxis::Vertical => local.y,
                };
                let target = (drag.scrollbar.current_offset
                    + (coordinate - drag.pointer_origin) * drag.scrollbar.drag_ratio())
                .clamp(drag.scrollbar.minimum_offset, drag.scrollbar.maximum_offset);
                self.with_default_context_scope(|scope, runtime_ptr| {
                    perform_scrollbar_scroll_default_action(
                        scope,
                        runtime_ptr,
                        drag.source,
                        drag.scrollbar.axis,
                        f64::from(target),
                    );
                    Ok(())
                })?;
                Ok(Some(input_dispatch_outcome(true)))
            }
            "mouseup" if button == 0 => {
                self.active_scrollbar_drag = None;
                Ok(Some(input_dispatch_outcome(true)))
            }
            "mousemove" => {
                self.active_scrollbar_drag = None;
                Ok(None)
            }
            _ => Ok(Some(input_dispatch_outcome(true))),
        }
    }

    fn dispatch_native_scrollbar_hit(
        &mut self,
        mut hit: moli_layout::LayoutScrollbarHit<DomHandle>,
        event_name: &str,
        button: i32,
    ) -> Result<RendererInputDispatchOutcome> {
        if event_name == "mousedown" && button == 0 {
            // Geometry and range come from the rendered snapshot. Apply each
            // new scroll action to the live offset, even before the next paint.
            if let Some(element) = self
                ._context_host
                .borrow()
                .dom_host()
                .node(hit.source)
                .and_then(Node::as_element)
            {
                hit.scrollbar.current_offset = match hit.scrollbar.axis {
                    moli_layout::LayoutScrollbarAxis::Horizontal => element.scroll_left() as f32,
                    moli_layout::LayoutScrollbarAxis::Vertical => element.scroll_top() as f32,
                };
            }
            match hit.part {
                moli_layout::LayoutScrollbarPart::Thumb => {
                    self.active_scrollbar_drag = Some(ActiveScrollbarDrag {
                        source: hit.source,
                        scrollbar: hit.scrollbar,
                        pointer_origin: hit.scrollbar.local_axis_coordinate(hit.local_point),
                        viewport_to_local: hit.viewport_to_local,
                    });
                }
                part => {
                    let target = match part {
                        moli_layout::LayoutScrollbarPart::BackButton => {
                            hit.scrollbar.line_target(false)
                        }
                        moli_layout::LayoutScrollbarPart::ForwardButton => {
                            hit.scrollbar.line_target(true)
                        }
                        moli_layout::LayoutScrollbarPart::BackTrack => {
                            hit.scrollbar.page_target(false)
                        }
                        moli_layout::LayoutScrollbarPart::ForwardTrack => {
                            hit.scrollbar.page_target(true)
                        }
                        moli_layout::LayoutScrollbarPart::Thumb => unreachable!(),
                    };
                    self.with_default_context_scope(|scope, runtime_ptr| {
                        perform_scrollbar_scroll_default_action(
                            scope,
                            runtime_ptr,
                            hit.source,
                            hit.scrollbar.axis,
                            f64::from(target),
                        );
                        Ok(())
                    })?;
                }
            }
        }
        Ok(input_dispatch_outcome(true))
    }

    pub(crate) fn dispatch_touch_event_at_point(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        activate: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        let hit_handle = observable_input_hit_test(
            &self._context_host.borrow(),
            self.document_runtime.document_handle(),
            moli_layout::LayoutPoint::new(x as f32, y as f32),
        )?
        .map(|hit| hit.handle);
        if event_name == "touchstart" && hit_handle.is_none() {
            return Ok(input_dispatch_outcome(false));
        }
        if event_name == "touchstart" {
            self.active_touch_event_handle = hit_handle;
        }
        if matches!(event_name, "touchstart" | "touchmove") {
            self.active_touch_point = Some(RendererTouchPoint { id: 0, x, y });
        }
        let touch_handle = match event_name {
            "touchmove" | "touchend" | "touchcancel" => {
                self.active_touch_event_handle.or(hit_handle)
            }
            _ => hit_handle,
        };
        let pointer_event_name = pointer_event_name_for_touch_event(event_name);
        let pointer = touch_pointer_properties(event_name, TOUCH_POINTER_ID);
        let button = button_for_touch_pointer_event(event_name);
        let buttons = buttons_for_touch_pointer_event(event_name);
        let mut pending_pointer_capture_events = Vec::new();
        if pointer_event_name.is_some() {
            let mut context_host = self._context_host.borrow_mut();
            context_host.update_pointer_input(TOUCH_POINTER_ID, buttons);
            if event_name == "touchstart" {
                if let Some(handle) = hit_handle {
                    // Blink sets implicit pending capture for touch before
                    // dispatching pointerdown; it is settled before the next
                    // pointer event in the stream.
                    context_host.set_pending_pointer_capture_target(TOUCH_POINTER_ID, handle);
                }
            } else {
                pending_pointer_capture_events =
                    context_host.process_pending_pointer_capture(TOUCH_POINTER_ID);
            }
        }
        if !pending_pointer_capture_events.is_empty() {
            self.with_default_context_scope(|scope, runtime_ptr| {
                dispatch_pointer_capture_events(
                    scope,
                    runtime_ptr,
                    TOUCH_POINTER_ID,
                    &pending_pointer_capture_events,
                    InputEventCoordinates::from_root(x, y),
                    button,
                    buttons,
                    &pointer,
                    0,
                );
                Ok(())
            })?;
        }
        let pointer_handle = pointer_event_name
            .is_some()
            .then(|| {
                self._context_host
                    .borrow()
                    .active_pointer_capture_target(TOUCH_POINTER_ID)
            })
            .flatten()
            .or(hit_handle);
        let pointer_transition = if pointer_event_name.is_some() {
            if let Some(pointer_handle) = pointer_handle {
                let previous = self.active_touch_pointer_handle;
                if previous != Some(pointer_handle) {
                    self.active_touch_pointer_handle = Some(pointer_handle);
                    Some((previous, Some(pointer_handle)))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let should_finish_touch = matches!(event_name, "touchend" | "touchcancel");
        let finish_pointer_handle = should_finish_touch
            .then_some(self.active_touch_pointer_handle)
            .flatten();

        let Some(handle) = touch_handle.or(pointer_handle) else {
            return Ok(input_dispatch_outcome(false));
        };

        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            if let Some((previous, current)) = pointer_transition {
                dispatch_touch_pointer_boundary_events(
                    scope,
                    runtime_ptr,
                    previous,
                    current,
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                );
            }
            if let Some(pointer_event_name) = pointer_event_name
                && let Some(pointer_handle) = pointer_handle
                && let Some(event) = construct_pointer_event(
                    scope,
                    pointer_event_name,
                    InputEventCoordinates::from_root(x, y),
                    button,
                    buttons,
                    &pointer,
                )
            {
                if pointer_event_name == "pointerup" {
                    unsafe { &mut *runtime_ptr }
                        .notify_close_watcher_input_activation(pointer_handle);
                }
                let _ = dispatch_native_pointer_event(
                    scope,
                    runtime_ptr,
                    pointer_handle,
                    event,
                    TOUCH_POINTER_ID,
                );
            }
            if should_finish_touch {
                release_pointer_capture_after_pointer_end(
                    scope,
                    runtime_ptr,
                    TOUCH_POINTER_ID,
                    InputEventCoordinates::from_root(x, y),
                    button,
                    buttons,
                    &pointer,
                    0,
                );
            }
            let runtime = unsafe { &mut *runtime_ptr };
            if let Some(target) =
                runtime
                    .native_bridge_mut()
                    .wrap_handle(scope, runtime_ptr, handle)
                && let Some(event) = construct_touch_event(scope, event_name, x, y, target)
            {
                if event_name == "touchend" {
                    unsafe { &mut *runtime_ptr }.notify_close_watcher_input_activation(handle);
                }
                let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
            }
            if should_finish_touch {
                dispatch_touch_pointer_boundary_events(
                    scope,
                    runtime_ptr,
                    finish_pointer_handle,
                    None,
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                );
                unsafe { &mut *runtime_ptr }.finish_touch_pointer_input(TOUCH_POINTER_ID);
            }
            if activate {
                return Ok(activate_handle_via_click(
                    scope,
                    runtime_ptr,
                    handle,
                    x,
                    y,
                    0,
                    0,
                    &pointer,
                ));
            }
            Ok(input_dispatch_outcome(true))
        });
        if should_finish_touch {
            self.active_touch_pointer_handle = None;
            self.active_touch_event_handle = None;
            self.active_touch_point = None;
        }
        self.finish_input_event_dispatch_turn(result)
    }

    pub(crate) fn dispatch_touch_event_at_points(
        &mut self,
        points: &[RendererTouchPoint],
        event_name: &str,
        activate: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        if activate
            && event_name == "touchend"
            && self.active_touch_event_handle.is_none()
            && let [point] = points
        {
            let started =
                self.dispatch_touch_event_at_point(point.x, point.y, "touchstart", false)?;
            if !started.handled
                || started.triggered_top_level_navigation
                || started.pending_download.is_some()
                || started.pending_file_chooser.is_some()
            {
                return Ok(started);
            }
        }
        if points.is_empty() && matches!(event_name, "touchend" | "touchcancel") {
            if self.active_touch_points.is_empty()
                && let Some(point) = self.active_touch_point
            {
                return self.dispatch_touch_event_at_point(point.x, point.y, event_name, activate);
            }
            let ending_points = self
                .active_touch_points
                .iter()
                .map(|(id, point)| RendererTouchPoint {
                    id: *id,
                    x: point.x,
                    y: point.y,
                })
                .collect::<Vec<_>>();
            return self.dispatch_multi_touch_event_at_points(&ending_points, event_name);
        }
        if let [point] = points
            && point.id == 0
            && self.active_touch_points.is_empty()
        {
            return self.dispatch_touch_event_at_point(point.x, point.y, event_name, activate);
        }
        self.dispatch_multi_touch_event_at_points(points, event_name)
    }

    fn dispatch_multi_touch_event_at_points(
        &mut self,
        points: &[RendererTouchPoint],
        event_name: &str,
    ) -> Result<RendererInputDispatchOutcome> {
        struct ChangedTouchPoint {
            point: RendererTouchPoint,
            pointer_id: i32,
            pointer_handle: DomHandle,
            pending_capture_events: Vec<PointerCaptureDispatchEvent>,
            pointer_transition: Option<(Option<DomHandle>, DomHandle)>,
            finish_pointer_handle: Option<DomHandle>,
        }

        if points.is_empty() {
            return Ok(input_dispatch_outcome(false));
        }

        let is_end = matches!(event_name, "touchend" | "touchcancel");
        let mut changed_points = Vec::new();
        for point in points {
            let hit_handle = observable_input_hit_test(
                &self._context_host.borrow(),
                self.document_runtime.document_handle(),
                moli_layout::LayoutPoint::new(point.x as f32, point.y as f32),
            )?
            .map(|hit| hit.handle);
            let target = match event_name {
                "touchstart" => hit_handle,
                "touchmove" | "touchend" | "touchcancel" => self
                    .active_touch_points
                    .get(&point.id)
                    .map(|active| active.target)
                    .or(hit_handle),
                _ => hit_handle,
            };
            if let Some(target) = target {
                changed_points.push((*point, target));
            }
        }
        if changed_points.is_empty() {
            return Ok(input_dispatch_outcome(false));
        }

        match event_name {
            "touchstart" | "touchmove" => {
                for (point, target) in &changed_points {
                    self.active_touch_points.insert(
                        point.id,
                        ActiveTouchPoint {
                            x: point.x,
                            y: point.y,
                            target: *target,
                        },
                    );
                }
            }
            "touchend" | "touchcancel" => {
                for (point, _) in &changed_points {
                    self.active_touch_points.remove(&point.id);
                }
            }
            _ => {}
        }

        let pointer_event_name = pointer_event_name_for_touch_event(event_name);
        let button = button_for_touch_pointer_event(event_name);
        let buttons = buttons_for_touch_pointer_event(event_name);
        let mut changed_pointer_points = Vec::new();
        if pointer_event_name.is_some() {
            for (point, target) in &changed_points {
                let pointer_id = touch_pointer_id(point.id);
                let (pointer_handle, pending_capture_events) = {
                    let mut context_host = self._context_host.borrow_mut();
                    context_host.update_pointer_input(pointer_id, buttons);
                    let pending_capture_events = if event_name == "touchstart" {
                        // Touch contacts get implicit pending capture before
                        // pointerdown. It is processed before the next pointer
                        // event for the same contact.
                        context_host.set_pending_pointer_capture_target(pointer_id, *target);
                        Vec::new()
                    } else {
                        context_host.process_pending_pointer_capture(pointer_id)
                    };
                    let pointer_handle = context_host
                        .active_pointer_capture_target(pointer_id)
                        .unwrap_or(*target);
                    (pointer_handle, pending_capture_events)
                };
                let previous_pointer_handle = self
                    .active_touch_pointer_handles
                    .insert(point.id, pointer_handle);
                let pointer_transition = (previous_pointer_handle != Some(pointer_handle))
                    .then_some((previous_pointer_handle, pointer_handle));
                let finish_pointer_handle = is_end.then_some(pointer_handle);
                changed_pointer_points.push(ChangedTouchPoint {
                    point: *point,
                    pointer_id,
                    pointer_handle,
                    pending_capture_events,
                    pointer_transition,
                    finish_pointer_handle,
                });
            }
        }

        let event_target = changed_points[0].1;
        let active_snapshot = self
            .active_touch_points
            .iter()
            .map(|(id, point)| (*id, *point))
            .collect::<Vec<_>>();
        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            for changed in &changed_pointer_points {
                let pointer = touch_pointer_properties(event_name, changed.pointer_id);
                if !changed.pending_capture_events.is_empty() {
                    dispatch_pointer_capture_events(
                        scope,
                        runtime_ptr,
                        changed.pointer_id,
                        &changed.pending_capture_events,
                        InputEventCoordinates::from_root(changed.point.x, changed.point.y),
                        button,
                        buttons,
                        &pointer,
                        0,
                    );
                }
                if let Some((previous, current)) = changed.pointer_transition {
                    dispatch_touch_pointer_boundary_events(
                        scope,
                        runtime_ptr,
                        previous,
                        Some(current),
                        changed.point.x,
                        changed.point.y,
                        button,
                        buttons,
                        &pointer,
                    );
                }
                if let Some(pointer_event_name) = pointer_event_name
                    && let Some(event) = construct_pointer_event(
                        scope,
                        pointer_event_name,
                        InputEventCoordinates::from_root(changed.point.x, changed.point.y),
                        button,
                        buttons,
                        &pointer,
                    )
                {
                    if pointer_event_name == "pointerup" {
                        unsafe { &mut *runtime_ptr }
                            .notify_close_watcher_input_activation(changed.pointer_handle);
                    }
                    let _ = dispatch_native_pointer_event(
                        scope,
                        runtime_ptr,
                        changed.pointer_handle,
                        event,
                        changed.pointer_id,
                    );
                }
                if let Some(finish_pointer_handle) = changed.finish_pointer_handle {
                    release_pointer_capture_after_pointer_end(
                        scope,
                        runtime_ptr,
                        changed.pointer_id,
                        InputEventCoordinates::from_root(changed.point.x, changed.point.y),
                        button,
                        buttons,
                        &pointer,
                        0,
                    );
                    dispatch_touch_pointer_boundary_events(
                        scope,
                        runtime_ptr,
                        Some(finish_pointer_handle),
                        None,
                        changed.point.x,
                        changed.point.y,
                        button,
                        buttons,
                        &pointer,
                    );
                    unsafe { &mut *runtime_ptr }.finish_touch_pointer_input(changed.pointer_id);
                }
            }
            let runtime = unsafe { &mut *runtime_ptr };
            let mut active_event_points = Vec::with_capacity(active_snapshot.len());
            for (id, point) in &active_snapshot {
                let Some(target) =
                    runtime
                        .native_bridge_mut()
                        .wrap_handle(scope, runtime_ptr, point.target)
                else {
                    return Ok(input_dispatch_outcome(false));
                };
                active_event_points.push(TouchEventPoint {
                    identifier: *id,
                    x: point.x,
                    y: point.y,
                    target,
                    is_target_touch: point.target == event_target,
                });
            }

            let mut changed_event_points = Vec::with_capacity(changed_points.len());
            for (point, target_handle) in &changed_points {
                let Some(target) =
                    runtime
                        .native_bridge_mut()
                        .wrap_handle(scope, runtime_ptr, *target_handle)
                else {
                    return Ok(input_dispatch_outcome(false));
                };
                changed_event_points.push(TouchEventPoint {
                    identifier: point.id,
                    x: point.x,
                    y: point.y,
                    target,
                    is_target_touch: *target_handle == event_target,
                });
            }

            if let Some(event) = construct_touch_event_with_points(
                scope,
                event_name,
                &active_event_points,
                &changed_event_points,
            ) {
                if event_name == "touchend" {
                    unsafe { &mut *runtime_ptr }
                        .notify_close_watcher_input_activation(event_target);
                }
                let _ = dispatch_public_event(scope, runtime_ptr, event_target, event);
            }
            Ok(input_dispatch_outcome(true))
        });
        if is_end && self.active_touch_points.is_empty() {
            self.active_touch_pointer_handle = None;
            self.active_touch_pointer_handles.clear();
            self.active_touch_event_handle = None;
            self.active_touch_point = None;
        } else if is_end {
            for (point, _) in &changed_points {
                self.active_touch_pointer_handles.remove(&point.id);
            }
        }
        self.finish_input_event_dispatch_turn(result)
    }

    pub(crate) fn dispatch_drag_event_at_point(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome> {
        if matches!(event_name, "dragleave" | "dragcancel") {
            let result = self
                .finish_active_drag(DragFinishReason::Canceled, modifiers)
                .map(|()| input_dispatch_outcome(true));
            return self.finish_input_event_dispatch_turn(result);
        }
        let hit = observable_input_hit_test(
            &self._context_host.borrow(),
            self.document_runtime.document_handle(),
            moli_layout::LayoutPoint::new(x as f32, y as f32),
        )?;
        let target = hit.map(|hit| hit.handle);
        let local = hit.map(|hit| {
            hit.root_to_frame
                .map_point(moli_layout::LayoutPoint::new(x as f32, y as f32))
        });
        let coordinates = local
            .map(|point| {
                InputEventCoordinates::from_root(x, y)
                    .with_client(f64::from(point.x), f64::from(point.y))
            })
            .unwrap_or_else(|| InputEventCoordinates::from_root(x, y));
        let active = &mut self.active_drag_session as *mut Option<ActiveDragSession>;
        // A finished/canceled session cannot be recreated by a late drop.
        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            // SAFETY: the callback runs synchronously while ScriptVm is
            // exclusively borrowed, without any other access to this field.
            let active = unsafe { &mut *active };
            if active.is_none() {
                if event_name != "dragenter" || target.is_none() {
                    return Ok(input_dispatch_outcome(false));
                }
                let Some(data_store) =
                    crate::context_bootstrap::create_external_drag_data_store(scope, &data)
                else {
                    return Ok(input_dispatch_outcome(false));
                };
                *active = Some(ActiveDragSession {
                    data_store,
                    source_transfer: None,
                    drop_effect: "none".to_owned(),
                    drop_allowed: false,
                    source: None,
                    intercepted: false,
                    target: None,
                    only_drag_over_next: false,
                    coordinates,
                    modifiers,
                });
            } else if event_name == "dragenter"
                && crate::context_bootstrap::replace_drag_data_store_contents(
                    scope,
                    &active.as_ref().expect("active drag").data_store,
                    &data,
                )
                .is_none()
            {
                return Ok(input_dispatch_outcome(false));
            }
            let session = active.as_mut().expect("drag session initialized");
            if matches!(event_name, "dragover" | "drop") {
                // Chromium's over command replaces only the operation mask;
                // its items are ignored. Drop's pre-over also uses the cached
                // enter payload, with the new command's source operation mask.
                crate::context_bootstrap::update_drag_data_store_operations_mask(
                    scope,
                    &session.data_store,
                    data.drag_operations_mask,
                );
            }
            if matches!(event_name, "dragenter" | "dragover" | "drop") {
                update_drag_target(scope, runtime_ptr, session, target, coordinates, modifiers);
            }
            if event_name == "drop" {
                // Empty data is an explicit replacement too. Import it only
                // after the pre-over, before exposing the new drop view.
                let reason = if crate::context_bootstrap::replace_drag_data_store_contents(
                    scope,
                    &session.data_store,
                    &data,
                )
                .is_some()
                {
                    drop_drag_target(scope, runtime_ptr, session, coordinates, modifiers)
                } else {
                    DragFinishReason::Rejected
                };
                finish_drag_session(scope, runtime_ptr, active, reason, coordinates, modifiers);
            }
            Ok(input_dispatch_outcome(target.is_some()))
        });
        self.finish_input_event_dispatch_turn(result)
    }

    pub(crate) fn clear_active_drag_data_transfer(&mut self) -> Result<()> {
        let modifiers = self
            .active_drag_session
            .as_ref()
            .map(|session| session.modifiers)
            .unwrap_or(0);
        let result = self.finish_active_drag(DragFinishReason::Canceled, modifiers);
        self.finish_input_event_dispatch_turn(result)
    }

    pub(crate) fn set_file_input_files(
        &mut self,
        handle: DomHandle,
        files: Vec<SelectedFile>,
        append: bool,
    ) -> Result<bool> {
        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            let runtime = unsafe { &mut *runtime_ptr };
            let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
                return Ok(false);
            };
            if !element.is_html_input() || element.input_type() != InputType::File {
                return Ok(false);
            }
            let multiple = element.attribute("multiple").is_some();
            if (append || files.len() > 1) && !multiple {
                return Ok(false);
            }

            let mut selected_files = if append {
                element.selected_files().to_vec()
            } else {
                Vec::new()
            };
            selected_files.extend(files);
            let changed = runtime.set_input_files(handle, selected_files.clone());
            if let Some(input) = node_wrapper_from_handle(scope, handle) {
                let _ = cache_input_files_from_selected_files(scope, input, &selected_files);
            }
            if changed {
                if let Some(event) = construct_simple_event(scope, "input", true, false, true) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
                if let Some(event) = construct_simple_event(scope, "change", true, false, false) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
            } else if let Some(event) = construct_simple_event(scope, "cancel", true, false, false)
            {
                let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
            }
            Ok(true)
        });
        self.finish_input_command_checkpoint(result)
    }

    pub(crate) fn insert_text_into_active_control(&mut self, text: &str) -> Result<bool> {
        let handle = self
            .document_runtime
            .active_element_handle()
            .or_else(|| self.document_runtime.document_focus_fallback_handle());
        let Some(handle) = handle else {
            return Ok(false);
        };

        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            if is_text_control(unsafe { &*runtime_ptr }, handle) {
                let _ = replace_text_control_selection(
                    scope,
                    runtime_ptr,
                    handle,
                    text,
                    TextEditInputType::InsertText,
                );
                return Ok(true);
            }
            let runtime = unsafe { &*runtime_ptr };
            let Some(editing_host) = contenteditable_editing_host(runtime, handle) else {
                return Ok(false);
            };
            Ok(replace_contenteditable_selection(
                scope,
                runtime_ptr,
                editing_host,
                text,
                TextEditInputType::InsertText,
            ))
        });
        self.finish_input_command_checkpoint(result)
    }

    pub(crate) fn dispatch_key_event(
        &mut self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        if event_name == "keypress" && self.suppress_next_keypress_after_canceled_raw_keydown {
            self.suppress_next_keypress_after_canceled_raw_keydown = false;
            return Ok(input_dispatch_outcome(false));
        }
        if matches!(event_name, "keydown" | "keyup") {
            self.suppress_next_keypress_after_canceled_raw_keydown = false;
        }

        let _current_input_event = CurrentInputEventScope::enter(
            Rc::clone(&self._context_host),
            CurrentInputEvent::keyboard(key, modifiers),
        );
        let handle = self
            .document_runtime
            .active_element_handle()
            .or_else(|| self.document_runtime.document_focus_fallback_handle());
        let Some(handle) = handle else {
            return Ok(input_dispatch_outcome(false));
        };

        let key_lower = key.to_ascii_lowercase();
        let alt = modifiers & 1 == 1;
        let ctrl = modifiers & 2 == 2;
        let meta = modifiers & 4 == 4;
        let shift = modifiers & 8 == 8;
        let canceled_raw_keydown = Cell::new(false);

        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            let runtime = unsafe { &mut *runtime_ptr };
            let close_request_context =
                runtime
                    .owner_dispatch_scope_for_node(handle)
                    .and_then(|target| {
                        let owner = runtime.current_window_execution_context_owner(target)?;
                        runtime
                            .window_execution_context(scope, owner, target)
                            .map(|(_, context)| context)
                    });
            if event_name == "keydown" && key_lower != "escape" {
                runtime.notify_close_watcher_input_activation(handle);
            }
            let Some(event) = construct_keyboard_event(
                scope,
                event_name,
                key,
                code,
                alt,
                ctrl,
                meta,
                shift,
                auto_repeat,
            ) else {
                return Ok(input_dispatch_outcome(false));
            };
            let dispatched = dispatch_public_event(scope, runtime_ptr, handle, event);
            if !dispatched.allows_default() {
                if event_name == "keydown" && !should_insert_text {
                    canceled_raw_keydown.set(true);
                }
                return Ok(input_dispatch_outcome(false));
            }

            if event_name == "keydown"
                && key_lower == "escape"
                && let Some(context) = close_request_context
            {
                crate::context_bootstrap::process_close_watchers(scope, context);
            }

            // Combined keydown/text input includes a cancelable keypress before
            // editing. Keep both events in this input turn; an explicit char
            // command already dispatched keypress and must not emit it twice.
            if event_name == "keydown"
                && let Some(handled) =
                    perform_clipboard_key_default_action(scope, runtime_ptr, &key_lower, modifiers)
            {
                return Ok(input_dispatch_outcome(handled));
            }
            let handle = if event_name == "keydown" && should_insert_text && !text.is_empty() {
                // Like Blink's KeyboardEventManager, resolve focus again after
                // keydown listeners before targeting the character event.
                let runtime = unsafe { &*runtime_ptr };
                let Some(handle) = runtime
                    .active_element_handle()
                    .or_else(|| runtime.document_focus_fallback_handle())
                else {
                    return Ok(input_dispatch_outcome(false));
                };
                let Some(keypress) = construct_keyboard_event(
                    scope,
                    "keypress",
                    key,
                    code,
                    alt,
                    ctrl,
                    meta,
                    shift,
                    auto_repeat,
                ) else {
                    return Ok(input_dispatch_outcome(false));
                };
                if !dispatch_public_event(scope, runtime_ptr, handle, keypress).allows_default() {
                    return Ok(input_dispatch_outcome(false));
                }
                handle
            } else {
                handle
            };

            let runtime = unsafe { &mut *runtime_ptr };
            let target = key_target_info(runtime, handle);

            let is_enter_key = key_lower == "enter" || (should_insert_text && text == "\r");
            if is_enter_key {
                let is_character_phase =
                    should_insert_text && matches!(event_name, "keydown" | "keypress");
                if target.is_textarea && is_character_phase {
                    return Ok(input_dispatch_outcome(replace_text_control_selection(
                        scope,
                        runtime_ptr,
                        handle,
                        "\n",
                        TextEditInputType::InsertLineBreak,
                    )));
                }
                if is_character_phase
                    && let Some(outcome) = perform_implicit_submission_from_control(
                        scope,
                        runtime_ptr,
                        handle,
                        modifiers,
                    )
                {
                    return Ok(outcome);
                }
                if event_name == "keydown" && (target.is_button_like || target.is_anchor_like) {
                    return Ok(activate_handle_via_click_with_detail_and_modifiers(
                        scope,
                        runtime_ptr,
                        handle,
                        0.0,
                        0.0,
                        0,
                        0,
                        0,
                        modifiers,
                    ));
                }
            }

            if should_insert_text {
                if text.is_empty() {
                    return Ok(input_dispatch_outcome(true));
                }
                if target.is_text_control {
                    return Ok(input_dispatch_outcome(replace_text_control_selection(
                        scope,
                        runtime_ptr,
                        handle,
                        text,
                        TextEditInputType::InsertText,
                    )));
                }
                if let Some(editing_host) = contenteditable_editing_host(runtime, handle) {
                    return Ok(input_dispatch_outcome(replace_contenteditable_selection(
                        scope,
                        runtime_ptr,
                        editing_host,
                        text,
                        TextEditInputType::InsertText,
                    )));
                }
                return Ok(input_dispatch_outcome(true));
            }

            if event_name == "keyup" && is_space_key(&key_lower) {
                if target.is_checkbox || target.is_button_like || target.is_radio {
                    return Ok(activate_handle_via_click_with_detail_and_modifiers(
                        scope,
                        runtime_ptr,
                        handle,
                        0.0,
                        0.0,
                        0,
                        0,
                        0,
                        modifiers,
                    ));
                }
                return Ok(input_dispatch_outcome(true));
            }

            if event_name != "keydown" {
                return Ok(input_dispatch_outcome(true));
            }

            if target.is_radio
                && matches!(
                    key_lower.as_str(),
                    "arrowdown"
                        | "down"
                        | "arrowright"
                        | "right"
                        | "arrowup"
                        | "up"
                        | "arrowleft"
                        | "left"
                )
            {
                let radios = radio_group_members(runtime, handle);
                if radios.is_empty() {
                    return Ok(input_dispatch_outcome(true));
                }
                let current_index = radios.iter().position(|candidate| *candidate == handle);
                let delta = if matches!(
                    key_lower.as_str(),
                    "arrowdown" | "down" | "arrowright" | "right"
                ) {
                    1usize
                } else {
                    radios.len() - 1
                };
                let next_index = current_index
                    .map(|index| (index + delta) % radios.len())
                    .unwrap_or(0);
                let next_handle = radios[next_index];
                let already_checked = runtime
                    .dom_host()
                    .node(next_handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.checked());
                if !already_checked {
                    let _ = runtime.set_checked_state(scope, runtime_ptr, next_handle, true);
                    if let Some(event) = construct_simple_event(scope, "input", true, false, true) {
                        let _ = dispatch_public_event(scope, runtime_ptr, next_handle, event);
                    }
                    if let Some(event) = construct_simple_event(scope, "change", true, false, false)
                    {
                        let _ = dispatch_public_event(scope, runtime_ptr, next_handle, event);
                    }
                }
                update_focus(scope, runtime_ptr, Some(next_handle));
                return Ok(input_dispatch_outcome(true));
            }

            if target.is_select
                && matches!(key_lower.as_str(), "arrowdown" | "down" | "arrowup" | "up")
            {
                let options = runtime.dom_host().select_option_elements(handle);
                if options.is_empty() {
                    return Ok(input_dispatch_outcome(true));
                }
                let current_index = options
                    .iter()
                    .position(|candidate| {
                        runtime
                            .dom_host()
                            .node(*candidate)
                            .and_then(Node::as_element)
                            .is_some_and(|element| element.selected())
                    })
                    .unwrap_or(0);
                let step: i32 = if matches!(key_lower.as_str(), "arrowdown" | "down") {
                    1
                } else {
                    -1
                };
                let mut next_index = current_index as i32;
                loop {
                    let candidate = next_index + step;
                    if candidate < 0 || candidate >= options.len() as i32 {
                        next_index = current_index as i32;
                        break;
                    }
                    next_index = candidate;
                    if !option_is_disabled(runtime, options[next_index as usize]) {
                        break;
                    }
                }
                if next_index as usize == current_index {
                    return Ok(input_dispatch_outcome(true));
                }
                for (index, option) in options.iter().copied().enumerate() {
                    let _ = runtime.set_selected_state(
                        scope,
                        runtime_ptr,
                        option,
                        index == next_index as usize,
                    );
                }
                let _ = runtime.set_select_explicit_none(scope, runtime_ptr, handle, false);
                if let Some(event) = construct_simple_event(scope, "input", true, false, true) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
                if let Some(event) = construct_simple_event(scope, "change", true, false, false) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
                return Ok(input_dispatch_outcome(true));
            }

            if target.is_text_control && matches!(key_lower.as_str(), "backspace" | "delete") {
                return Ok(input_dispatch_outcome(replace_text_control_selection(
                    scope,
                    runtime_ptr,
                    handle,
                    "",
                    if key_lower == "backspace" {
                        TextEditInputType::DeleteContentBackward
                    } else {
                        TextEditInputType::DeleteContentForward
                    },
                )));
            }

            if target.is_text_control && (ctrl || meta) && key_lower == "a" {
                let value_len = utf16_len(&text_control_value(runtime, handle)) as u32;
                let _ = text_control_set_selection_range_internal(
                    scope,
                    runtime_ptr,
                    handle,
                    0,
                    value_len,
                );
                return Ok(input_dispatch_outcome(true));
            }

            if (ctrl || meta)
                && key_lower == "a"
                && let Some(editing_host) = contenteditable_editing_host(runtime, handle)
            {
                return Ok(input_dispatch_outcome(select_contenteditable_contents(
                    scope,
                    editing_host,
                )));
            }

            if matches!(key_lower.as_str(), "backspace" | "delete")
                && let Some(editing_host) = contenteditable_editing_host(runtime, handle)
            {
                return Ok(input_dispatch_outcome(replace_contenteditable_selection(
                    scope,
                    runtime_ptr,
                    editing_host,
                    "",
                    if key_lower == "backspace" {
                        TextEditInputType::DeleteContentBackward
                    } else {
                        TextEditInputType::DeleteContentForward
                    },
                )));
            }

            if target.is_text_control
                && matches!(
                    key_lower.as_str(),
                    "arrowleft" | "left" | "arrowright" | "right" | "home" | "end"
                )
            {
                let value_units = utf16_units(&text_control_value(runtime, handle));
                let value_len = value_units.len() as u32;
                let (start, end, direction) = current_selection_state(runtime, handle);
                let start = utf16_scalar_boundary_at_or_after(&value_units, start as usize) as u32;
                let end = utf16_scalar_boundary_at_or_after(&value_units, end as usize) as u32;
                if shift {
                    let (anchor, focus) = match direction.as_str() {
                        "backward" => (end, start),
                        "forward" => (start, end),
                        _ => (end, end),
                    };
                    let next_focus = match key_lower.as_str() {
                        "arrowleft" | "left" => {
                            utf16_previous_scalar_boundary(&value_units, focus as usize) as u32
                        }
                        "arrowright" | "right" => {
                            utf16_next_scalar_boundary(&value_units, focus as usize) as u32
                        }
                        "home" => 0,
                        "end" => value_len,
                        _ => focus,
                    };
                    if next_focus == focus
                        && matches!(
                            key_lower.as_str(),
                            "arrowleft" | "left" | "arrowright" | "right"
                        )
                    {
                        return Ok(input_dispatch_outcome(true));
                    }
                    let (next_start, next_end, next_direction) = if next_focus < anchor {
                        (next_focus, anchor, "backward")
                    } else if next_focus > anchor {
                        (anchor, next_focus, "forward")
                    } else {
                        (anchor, anchor, "none")
                    };
                    let _ = text_control_set_selection_range_with_direction_internal(
                        scope,
                        runtime_ptr,
                        handle,
                        next_start,
                        next_end,
                        next_direction,
                    );
                    return Ok(input_dispatch_outcome(true));
                }
                let caret = match key_lower.as_str() {
                    "arrowleft" | "left" => {
                        if start != end {
                            start
                        } else {
                            utf16_previous_scalar_boundary(&value_units, start as usize) as u32
                        }
                    }
                    "arrowright" | "right" => {
                        if start != end {
                            end
                        } else {
                            utf16_next_scalar_boundary(&value_units, end as usize) as u32
                        }
                    }
                    "home" => 0,
                    "end" => value_len,
                    _ => end,
                };
                if start == end
                    && caret == start
                    && matches!(
                        key_lower.as_str(),
                        "arrowleft" | "left" | "arrowright" | "right"
                    )
                {
                    return Ok(input_dispatch_outcome(true));
                }
                let _ = text_control_set_selection_range_internal(
                    scope,
                    runtime_ptr,
                    handle,
                    caret,
                    caret,
                );
                return Ok(input_dispatch_outcome(true));
            }

            Ok(input_dispatch_outcome(true))
        });
        if canceled_raw_keydown.get() {
            self.suppress_next_keypress_after_canceled_raw_keydown = true;
        }
        self.finish_input_event_dispatch_turn(result)
    }
}
