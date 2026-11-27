use anyhow::Result;
use moli_file_api::data_transfer::{
    drag_operations_mask_from_effect_allowed, drop_effect_allowed_by_effect_allowed,
    preferred_drop_effect_from_mask,
};
use moli_layout::{LayoutPoint, LayoutTransform2D};
use std::cell::Cell;
use std::rc::Rc;

#[cfg(feature = "ace-protocol")]
mod ace_page_tasks;
#[cfg(feature = "molio")]
mod cursor;
mod drag_source;
mod mouse_boundary;
mod text_control_selection;
mod touch_tap;
mod wheel;
use crate::native_bridge::element::InputSurfaceHit;
use drag_source::dispatch_drag_source_event;
use mouse_boundary::dispatch_mouse_boundary_events;
pub(super) use text_control_selection::TextControlMouseSelection;
use touch_tap::dispatch_touch_tap_activation;

use super::input_dispatch_outcome;
use super::inspector::{
    current_selection_state, is_space_key, key_target_info, option_is_disabled, radio_group_members,
};
use super::{
    ActiveDragSession, ActiveScrollbarDrag, ActiveTouchPoint, PendingDragGesture, PendingTouchTap,
    ScriptVm,
};
use crate::context_bootstrap::DragDataStoreMode;
use crate::document_runtime::DomHandle;
use crate::dom::{
    forms::InputType,
    native::{Node, SelectedFile},
};
use crate::native_bridge::element::{
    InputEventCoordinates, InputHitTestMode, InputHitTestPolicy, TextEditInputType,
    TextInputSource, TouchEventPoint, WheelScrollTarget, activate_handle_after_pointer_release,
    activate_handle_after_touch_release, activate_handle_via_keyboard,
    cache_input_files_from_selected_files, construct_drag_event_with_related_target,
    construct_keyboard_event, construct_mouse_event_for_target,
    construct_mouse_event_with_detail_for_target,
    construct_mouse_event_with_related_target_for_target, construct_pointer_event,
    construct_simple_event, construct_touch_event_with_points, construct_wheel_event_for_target,
    contenteditable_editing_host, dispatch_public_event, input_surface_hit_test,
    input_target_in_document, is_disabled_form_control, observable_input_hit_test,
    perform_auxiliary_link_default_action, perform_drop_default_action,
    perform_implicit_submission_from_control, perform_latched_wheel_scroll_default_action,
    perform_mouse_focus_default_action, perform_scrollbar_scroll_default_action,
    perform_text_input, perform_wheel_scroll_default_action, replace_contenteditable_selection,
    replace_text_control_selection, select_contenteditable_contents,
    text_control_set_selection_range_internal, text_control_value, update_focus,
};
use crate::native_bridge::{
    CurrentInputEvent, CurrentInputEventScope, MouseReleaseFollowUp, PageScriptHost,
    PublishedFrameGeometry, WindowDocumentTaskTarget,
};
use crate::runtime::{
    RendererDragData, RendererInputDispatchOutcome, RendererInterceptedDrag,
    RendererPointerEventProperties, RendererTouchPoint, RendererWheelDispatchOutcome,
};
use crate::util::{node_wrapper_from_handle, utf16_len};
#[cfg(feature = "molio")]
use crate::{host::EventTargetHandle, native_bridge::OwnerDispatchScope};

fn related_target_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Value>> {
    node_wrapper_from_handle(scope, handle?).map(Into::into)
}

/// Native input arrives in root viewport coordinates. The headless input
/// adapters use the root point as the screen point when no screen is modeled.
#[derive(Clone, Copy, Default)]
pub(super) struct InputPosition {
    root: (f64, f64),
    screen: (f64, f64),
}

impl InputPosition {
    fn headless(x: f64, y: f64) -> Self {
        Self {
            root: (x, y),
            screen: (x, y),
        }
    }

    fn layout_point(self) -> LayoutPoint {
        LayoutPoint::new(self.root.0 as f32, self.root.1 as f32)
    }

    fn coordinates(self, transform: Option<LayoutTransform2D>) -> InputEventCoordinates {
        let (client_x, client_y) = transform
            .filter(|transform| *transform != LayoutTransform2D::IDENTITY)
            .map_or(self.root, |transform| {
                let point = transform.map_point(self.layout_point());
                (f64::from(point.x), f64::from(point.y))
            });
        InputEventCoordinates {
            screen_x: self.screen.0,
            screen_y: self.screen.1,
            client_x,
            client_y,
        }
    }

    fn for_target(
        self,
        runtime: &PageScriptHost,
        target: DomHandle,
        fallback_sample: Option<&PublishedFrameGeometry>,
    ) -> InputEventCoordinates {
        let transform = runtime
            .dom_host()
            .owner_document_handle(target)
            .and_then(|document| runtime.input_frame_transform(document, fallback_sample));
        self.coordinates(transform)
    }
}

#[derive(Clone, Copy)]
struct InputDocument {
    handle: DomHandle,
    owner: WindowDocumentTaskTarget,
}

impl InputDocument {
    fn for_target(runtime: &PageScriptHost, target: DomHandle) -> Option<Self> {
        let handle = runtime.dom_host().owner_document_handle(target)?;
        Some(Self {
            handle,
            owner: runtime.input_owner_for_document(handle)?,
        })
    }

    fn contains(self, runtime: &PageScriptHost, target: DomHandle) -> bool {
        runtime.input_document_for_owner(self.owner) == Some(self.handle)
            && runtime.dom_host().is_connected(target)
            && runtime.dom_host().owner_document_handle(target) == Some(self.handle)
    }

    fn target_after_capture(
        self,
        runtime: &PageScriptHost,
        pointer_id: i32,
        candidates: impl IntoIterator<Item = DomHandle>,
        position: InputPosition,
    ) -> Option<DomHandle> {
        runtime
            .active_pointer_capture_target(pointer_id)
            .into_iter()
            .chain(candidates)
            .find(|target| self.contains(runtime, *target))
            .or_else(|| {
                let target =
                    input_target_in_document(runtime, self.handle, position.layout_point());
                self.contains(runtime, target).then_some(target)
            })
    }
}

#[derive(Clone, Copy)]
struct DragInput<'a> {
    position: InputPosition,
    sampled: Option<&'a PublishedFrameGeometry>,
    modifiers: u8,
}

fn drag_related_target_in_document(
    runtime: &crate::native_bridge::PageScriptHost,
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
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
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

fn dispatch_native_drag_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    event_type: &str,
    target: DomHandle,
    related: Option<DomHandle>,
    input: DragInput<'_>,
    transfer: v8::Local<'s, v8::Object>,
) -> Option<crate::native_bridge::element::NodePublicEventDispatchOutcome> {
    let (coordinates, related) = {
        let runtime = unsafe { &*runtime_ptr };
        let dom = runtime.dom_host();
        let document = dom.owner_document_handle(target)?;
        let related =
            related.and_then(|related| drag_related_target_in_document(runtime, document, related));
        (
            input.position.for_target(runtime, target, input.sampled),
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
        input.modifiers,
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
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    active: &mut Option<ActiveDragSession>,
    reason: DragFinishReason,
    input: DragInput<'_>,
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
            dispatch_drag_source_event(scope, runtime_ptr, &session, input);
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
                input,
                data_transfer,
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
            input,
            data_transfer,
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
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
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
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    session: &mut ActiveDragSession,
    target: Option<DomHandle>,
    input: DragInput<'_>,
) {
    session.position = input.position;
    session.modifiers = input.modifiers;
    session.drop_allowed = false;
    session.drop_effect = "none".to_owned();
    let target_document = target.and_then(|target| {
        unsafe { &*runtime_ptr }
            .dom_host()
            .owner_document_handle(target)
    });
    if target.is_some() && (session.target != target || !session.only_drag_over_next) {
        dispatch_drag_source_event(scope, runtime_ptr, session, input);
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
                input,
                transfer,
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
                input,
                transfer,
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
            input,
            transfer,
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
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    session: &mut ActiveDragSession,
    input: DragInput<'_>,
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
    let Some(dispatched) =
        dispatch_native_drag_event(scope, runtime_ptr, "drop", target, None, input, transfer)
    else {
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
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    gesture: PendingDragGesture,
) -> Option<(DomHandle, RendererDragData)> {
    let runtime = unsafe { &*runtime_ptr };
    let host = runtime.dom_host();
    if !host.is_connected(gesture.handle) {
        return None;
    }
    let mut candidate = None;
    let mut current = Some(gesture.handle);
    while let Some(handle) = current {
        if let Some(element) = host.node(handle).and_then(Node::as_element) {
            let attribute = element.attribute_ns("", "draggable");
            let is_image = element.is_html_element("img");
            let is_link = element.is_html_element("a") && element.has_attribute_ns("", "href");
            let explicitly_draggable =
                attribute.is_some_and(|value| value.eq_ignore_ascii_case("true"));
            let disabled = attribute.is_some_and(|value| value.eq_ignore_ascii_case("false"));
            if !disabled && (explicitly_draggable || is_image || is_link) {
                candidate = Some((
                    handle,
                    if is_image {
                        Some("src")
                    } else if is_link {
                        Some("href")
                    } else {
                        None
                    },
                ));
                break;
            }
        }
        current = host
            .parent_node(handle)
            .or_else(|| host.shadow_root_host(handle));
    }
    if candidate.is_none_or(|(_, attribute)| attribute != Some("src"))
        && let Some(data) = super::native_drag_data::selection_drag_data_at_point(
            scope,
            runtime_ptr,
            gesture.handle,
            gesture.x,
            gesture.y,
        )
    {
        return Some((gesture.handle, data));
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

fn pointer_event_name_for_mouse_event(event_name: &str) -> Option<&'static str> {
    match event_name {
        "mousedown" => Some("pointerdown"),
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
// Match the default Chromium touch slop in CSS pixels. A contact that leaves
// this radius cannot become a tap, even if it returns to its starting point.
const TOUCH_TAP_SLOP: f64 = 16.0;

pub(crate) enum MouseDispatchCompletion {
    FinishTurn,
    DeferToBatch,
}

pub(crate) struct WheelDispatchOptions {
    pub(crate) button: i32,
    /// An omitted value retains the currently pressed buttons.
    pub(crate) buttons: Option<i32>,
    pub(crate) click_count: i32,
    pub(crate) pointer: RendererPointerEventProperties,
    pub(crate) intercept_drags: bool,
    pub(crate) completion: MouseDispatchCompletion,
}

impl Default for WheelDispatchOptions {
    fn default() -> Self {
        Self {
            button: -1,
            buttons: None,
            click_count: 0,
            pointer: RendererPointerEventProperties::default(),
            intercept_drags: false,
            completion: MouseDispatchCompletion::FinishTurn,
        }
    }
}

#[derive(Clone, Copy)]
struct PreparedMouseInputDispatch {
    button: i32,
    buttons: i32,
    previous_buttons: i32,
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

fn touch_tap_is_live(runtime: &crate::native_bridge::PageScriptHost, tap: PendingTouchTap) -> bool {
    runtime.dom_host().is_connected(tap.target)
        && runtime.dom_host().owner_document_handle(tap.target) == Some(tap.document)
        && runtime
            .document_execution_context_for_node(tap.target)
            .is_some()
}

fn touch_pointer_properties(
    event_name: &str,
    pointer_id: i32,
    is_primary: bool,
) -> RendererPointerEventProperties {
    RendererPointerEventProperties {
        pointer_id,
        pointer_type: "touch".to_owned(),
        is_primary,
        pressure: match event_name {
            "touchend" | "touchcancel" => 0.0,
            _ => 0.5,
        },
        ..RendererPointerEventProperties::default()
    }
}

#[derive(Clone, Copy)]
struct PreparedPointerEvent<'a> {
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    pointer: &'a RendererPointerEventProperties,
    modifiers: u8,
}

impl PreparedPointerEvent<'_> {
    fn construct<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event_name: &str,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.construct_with_related_target(scope, event_name, None)
    }

    fn construct_with_related_target<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event_name: &str,
        related: Option<v8::Local<'s, v8::Value>>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        construct_pointer_event(
            scope,
            event_name,
            self.coordinates,
            self.button,
            self.buttons,
            self.pointer,
            related,
            self.modifiers,
        )
    }

    fn process_capture<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        runtime_ptr: *mut PageScriptHost,
        trigger: Option<v8::Local<'s, v8::Object>>,
    ) {
        let id = self.pointer.pointer_id;
        let (lost, pending) = unsafe { &mut *runtime_ptr }.begin_pointer_capture_transition(id);
        let dispatch = move |scope: &mut v8::PinScope<'s, '_>, name, target| {
            if let Some(event) = self.construct(scope, name) {
                if let Some(trigger) = trigger {
                    moli_webapi::events::state::inherit_event_time_stamp(scope, event, trigger);
                }
                let _ = dispatch_public_event(scope, runtime_ptr, target, event);
            }
        };
        if let Some(lost) = lost {
            dispatch(scope, "lostpointercapture", lost);
        }
        if let Some(pending) = pending
            && unsafe { &*runtime_ptr }.pointer_capture_target_is_current(pending)
        {
            dispatch(scope, "gotpointercapture", pending.element);
            if let Some(document) =
                unsafe { &mut *runtime_ptr }.finish_pointer_capture_transition(id, pending)
            {
                dispatch(scope, "lostpointercapture", document);
            }
        }
    }

    fn release_capture<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        runtime_ptr: *mut PageScriptHost,
        trigger: Option<v8::Local<'s, v8::Object>>,
    ) {
        unsafe { &mut *runtime_ptr }
            .release_pending_pointer_capture_target(self.pointer.pointer_id);
        self.process_capture(scope, runtime_ptr, trigger);
    }
}

#[cfg(feature = "molio")]
fn dispatch_cancel_mouseup(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    owner: WindowDocumentTaskTarget,
    position: InputPosition,
    sampled: Option<&PublishedFrameGeometry>,
    (button, buttons): (i32, i32),
    modifiers: u8,
) {
    let runtime = unsafe { &*runtime_ptr };
    // Pointer listeners may retire an iframe before the compatibility event.
    let Some(document) = runtime.input_document_for_owner(owner) else {
        return;
    };
    let coordinates = position.coordinates(runtime.input_frame_transform(document, sampled));
    let Some(event) = construct_mouse_event_for_target(
        scope,
        runtime_ptr,
        document,
        "mouseup",
        coordinates,
        button,
        buttons,
        modifiers,
    ) else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    match owner.dispatch_scope() {
        OwnerDispatchScope::Top => {
            let _ = runtime.dispatch_public_event_best_effort(
                scope,
                runtime_ptr,
                EventTargetHandle::Window,
                event,
                "pointer cancel mouseup",
            );
        }
        OwnerDispatchScope::Child(frame) => {
            runtime.dispatch_child_window_event(scope, frame, "mouseup", event);
        }
        OwnerDispatchScope::LightweightPopup(popup) => {
            let _ = runtime.dispatch_lightweight_popup_window_event(scope, popup, "mouseup", event);
        }
    }
}

fn dispatch_pointer_exit_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    target: DomHandle,
    data: PreparedPointerEvent<'_>,
) {
    if !unsafe { &*runtime_ptr }.dom_host().is_connected(target) {
        return;
    }
    if let Some(event) = data.construct(scope, "pointerout") {
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
        if let Some(event) = data.construct(scope, "pointerleave") {
            let _ = dispatch_public_event(scope, runtime_ptr, ancestor, event);
        }
    }
}

fn dispatch_mouse_pointer_boundaries(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    previous: Option<DomHandle>,
    current: DomHandle,
    restarting: bool,
    data: PreparedPointerEvent<'_>,
    previous_coordinates: Option<InputEventCoordinates>,
) {
    let dispatch = move |scope: &mut v8::PinScope<'_, '_>, name, target, related, coordinates| {
        let related = related_target_value(scope, related);
        let boundary = PreparedPointerEvent {
            coordinates,
            ..data
        };
        if let Some(event) = boundary.construct_with_related_target(scope, name, related) {
            let _ = dispatch_public_event(scope, runtime_ptr, target, event);
        }
    };
    if let Some(previous) = previous {
        let coordinates = previous_coordinates.unwrap_or(data.coordinates);
        dispatch(scope, "pointerout", previous, Some(current), coordinates);
        dispatch(scope, "pointerleave", previous, Some(current), coordinates);
    }
    dispatch(scope, "pointerover", current, previous, data.coordinates);
    let entering_targets = if restarting {
        let dom = unsafe { &*runtime_ptr }.dom_host();
        let mut ancestors =
            std::iter::successors(Some(current), |node| dom.flat_tree_parent(*node))
                .collect::<Vec<_>>();
        ancestors.reverse();
        ancestors
    } else {
        vec![current]
    };
    for target in entering_targets {
        dispatch(scope, "pointerenter", target, previous, data.coordinates);
    }
}

fn dispatch_touch_pointer_boundary_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    previous: Option<DomHandle>,
    current: Option<DomHandle>,
    position: InputPosition,
    data: PreparedPointerEvent<'_>,
    sampled: Option<&PublishedFrameGeometry>,
) {
    if previous == current {
        return;
    }
    for (target, related, names) in [
        (previous, current, ["pointerout", "pointerleave"]),
        (current, previous, ["pointerover", "pointerenter"]),
    ] {
        if let Some(target) = target {
            for name in names {
                let coordinates = position.for_target(unsafe { &*runtime_ptr }, target, sampled);
                let related = related_target_value(scope, related);
                let boundary = PreparedPointerEvent {
                    coordinates,
                    ..data
                };
                if let Some(event) = boundary.construct_with_related_target(scope, name, related) {
                    let _ = dispatch_public_event(scope, runtime_ptr, target, event);
                }
            }
        }
    }
}

fn prepare_touch_point<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    identifier: i32,
    position: InputPosition,
    handle: DomHandle,
    event_target: DomHandle,
    sampled: Option<&PublishedFrameGeometry>,
) -> Option<TouchEventPoint<'s>> {
    let runtime = unsafe { &mut *runtime_ptr };
    let coordinates = position.for_target(runtime, handle, sampled);
    let dom = runtime.dom_host();
    let (scroll_x, scroll_y) = dom
        .owner_document_handle(handle)
        .and_then(|document| dom.dom().document_element_handle_for_document(document))
        .and_then(|root| dom.node(root)?.as_element())
        .map_or((0.0, 0.0), |element| {
            (element.scroll_left(), element.scroll_top())
        });
    let target = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, handle)?;
    Some(TouchEventPoint {
        identifier,
        coordinates,
        page: (
            coordinates.client_x + scroll_x,
            coordinates.client_y + scroll_y,
        ),
        target,
        is_target_touch: handle == event_target,
    })
}

#[cfg(feature = "molio")]
impl ScriptVm {
    pub(crate) fn viewport_selected_text(&mut self) -> Result<Option<String>> {
        self.with_default_context_scope(|scope, host| {
            Ok(crate::context_bootstrap::viewport_edit::selected_viewport_text(scope, host))
        })
    }

    pub(crate) fn cut_viewport_selection(&mut self) -> Result<Option<String>> {
        let result = self.with_default_context_scope(|scope, host| {
            Ok(crate::context_bootstrap::viewport_edit::cut_viewport_selection(scope, host))
        });
        self.finish_input_event_dispatch_turn(result)
    }

    pub(crate) fn viewport_editor_snapshot(
        &mut self,
        editing: &crate::context_bootstrap::viewport_edit::ViewportEditing,
    ) -> Option<crate::runtime::RendererViewportEditor> {
        self.with_current_main_world_scope(|scope, host| {
            unsafe { &*host }.ensure_observable_layout()?;
            Ok(editing.snapshot(scope, host))
        })
        .ok()
        .flatten()
        .flatten()
    }

    pub(crate) fn apply_viewport_editor_input(
        &mut self,
        editing: &mut crate::context_bootstrap::viewport_edit::ViewportEditing,
        edit: crate::runtime::RendererViewportEdit,
    ) -> Result<bool> {
        let result =
            self.with_default_context_scope(|scope, host| Ok(editing.apply(scope, host, edit)));
        self.finish_input_command_checkpoint(result)
    }
}

impl ScriptVm {
    /// Native keyboard delivery has no outer JavaScript invocation. Checkpoint
    /// when each outermost callback returns, including between listeners and
    /// before defaults. Nested dispatchEvent() stays inside its calling script.
    fn with_native_keyboard_event_scope<T>(
        &mut self,
        operation: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut PageScriptHost) -> Result<T>,
    ) -> Result<T> {
        self.with_default_context_scope(|scope, host_ptr| {
            moli_v8_util::with_microtasks_policy(scope, v8::MicrotasksPolicy::Scoped, |scope| {
                operation(scope, host_ptr)
            })
        })
    }

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
        self.dispatch_mouse_input(
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
            MouseDispatchCompletion::FinishTurn,
        )
    }

    pub(crate) fn dispatch_mouse_input(
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
        completion: MouseDispatchCompletion,
    ) -> Result<RendererInputDispatchOutcome> {
        if event_name == "wheel" {
            return self.dispatch_direct_wheel(
                (x, y),
                (delta_x, delta_y),
                modifiers,
                WheelDispatchOptions {
                    button,
                    buttons,
                    click_count,
                    pointer,
                    intercept_drags,
                    completion,
                },
            );
        }
        self.dispatch_mouse_input_to_target(
            x,
            y,
            event_name,
            button,
            buttons,
            click_count,
            delta_x,
            delta_y,
            pointer,
            modifiers,
            intercept_drags,
            completion,
            None,
        )
    }

    fn dispatch_mouse_input_to_target(
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
        completion: MouseDispatchCompletion,
        wheel: Option<(InputSurfaceHit, crate::wheel_input::PreparedWheel)>,
    ) -> Result<RendererInputDispatchOutcome> {
        let (surface_hit, prepared_wheel) = if let Some((hit, prepared)) = wheel {
            (hit, Some(prepared))
        } else {
            (self.mouse_input_surface_hit(x, y, event_name)?, None)
        };
        let wheel_cancelable = prepared_wheel.as_ref().is_none_or(|wheel| wheel.cancelable);
        let wheel_scroll_target = prepared_wheel
            .as_ref()
            .and_then(|wheel| wheel.scroll_target.as_ref());
        let prepared = self.prepare_mouse_input_dispatch(event_name, button, buttons);
        let is_release = event_name == "mouseup";
        let full_release = is_release && prepared.buttons == 0;
        let idle_move = event_name == "mousemove" && prepared.buttons == 0;
        if full_release || idle_move {
            // A release can land outside the viewport or on a native control.
            let mut runtime = self._context_host.borrow_mut();
            if runtime.pointer_is_known(pointer.pointer_id) {
                runtime.set_pointer_buttons_active(pointer.pointer_id, false);
            }
            #[cfg(feature = "molio")]
            let _ = runtime.take_compat_mouse_press_owners();
        }
        #[cfg(feature = "molio")]
        if is_release && !full_release {
            self._context_host
                .borrow_mut()
                .set_compat_mouse_press_owner(prepared.button, None);
        }
        if idle_move {
            self.suppress_compat_mouse_events = false;
        }
        let _current_input_event = CurrentInputEventScope::enter(
            Rc::clone(&self._context_host),
            CurrentInputEvent::mouse(event_name, prepared.button, modifiers),
        );
        let result = self.dispatch_mouse_input_body(
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
            wheel_cancelable,
            wheel_scroll_target,
        );
        if is_release {
            self.suppress_compat_mouse_events = false;
            if prepared.button == 0 || full_release {
                self.text_control_mouse_selection = None;
            }
        }
        if full_release {
            self.mouse_routed_frame = None;
            self._context_host.borrow_mut().clear_mouse_press();
        }
        // Capture the actual ACK before a checkpoint error can replace the
        // command result. Keep the input scope through the entire checkpoint.
        let wheel_outcome = result.as_ref().ok().and_then(|outcome| outcome.wheel);
        let result = match completion {
            MouseDispatchCompletion::FinishTurn => self.finish_input_event_dispatch_turn(result),
            MouseDispatchCompletion::DeferToBatch => result,
        };
        if let Some(prepared) = prepared_wheel {
            let document = self.wheel_document_owner();
            self.wheel_input
                .borrow_mut()
                .acknowledge(prepared, wheel_outcome, document);
        }
        result
    }

    #[cfg(feature = "molio")]
    pub(crate) fn cancel_viewport_mouse(
        &mut self,
        (x, y): (f64, f64),
        modifiers: u8,
    ) -> Result<()> {
        self.pressed_mouse_buttons = 0;
        self.mouse_routed_frame = None;
        self.pending_drag_gesture = None;
        self.text_control_mouse_selection = None;
        self.active_scrollbar_drag = None;

        let _current_input_event = CurrentInputEventScope::enter(
            Rc::clone(&self._context_host),
            CurrentInputEvent::mouse("pointercancel", -1, modifiers),
        );
        let active_drag: *mut Option<ActiveDragSession> = &mut self.active_drag_session;
        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            let runtime = move || unsafe { &*runtime_ptr };
            let sampled = runtime().published_frame_geometry();
            let presses = unsafe { &mut *runtime_ptr }.take_compat_mouse_press_owners();
            // Native dragging has already canceled the DOM pointer stream.
            let active_drag = unsafe { &mut *active_drag };
            if let Some(session) = active_drag.as_ref() {
                let input = DragInput {
                    position: session.position,
                    sampled: sampled.as_deref(),
                    modifiers: session.modifiers,
                };
                finish_drag_session(
                    scope,
                    runtime_ptr,
                    active_drag,
                    DragFinishReason::Canceled,
                    input,
                );
            }

            let pointer = RendererPointerEventProperties::default();
            let id = pointer.pointer_id;
            if runtime().pointer_buttons_active(id) {
                let position = InputPosition::headless(x, y);
                let coordinates_for = |target: Option<DomHandle>| {
                    target.map_or_else(
                        || position.coordinates(None),
                        |target| position.for_target(runtime(), target, sampled.as_deref()),
                    )
                };
                let last_target = || {
                    runtime()
                        .mouse_event_target()
                        .or_else(|| runtime().mouse_press_handle())
                        .filter(|target| runtime().dom_host().is_connected(*target))
                };
                let mut cancel = PreparedPointerEvent {
                    coordinates: coordinates_for(
                        runtime()
                            .pending_pointer_capture_target(id)
                            .or_else(last_target),
                    ),
                    button: -1,
                    buttons: 0,
                    pointer: &pointer,
                    modifiers,
                };
                unsafe { &mut *runtime_ptr }.set_pointer_buttons_active(id, false);
                cancel.process_capture(scope, runtime_ptr, None);
                let target = runtime()
                    .active_pointer_capture_target(id)
                    .or_else(last_target);
                cancel.coordinates = coordinates_for(target);
                let event = cancel.construct(scope, "pointercancel");
                if let (Some(target), Some(event)) = (target, event) {
                    let _ = dispatch_public_event(scope, runtime_ptr, target, event);
                }
                // End every observed compatibility press at its owning Window.
                let mut buttons = presses
                    .iter()
                    .enumerate()
                    .filter(|(_, owner)| owner.is_some())
                    .fold(0, |buttons, (button, _)| {
                        buttons | mouse_button_mask(button as i32)
                    });
                for (button, owner) in presses.into_iter().enumerate() {
                    if let Some(owner) = owner {
                        let button = button as i32;
                        buttons &= !mouse_button_mask(button);
                        dispatch_cancel_mouseup(
                            scope,
                            runtime_ptr,
                            owner,
                            position,
                            sampled.as_deref(),
                            (button, buttons),
                            modifiers,
                        );
                    }
                }
                cancel.release_capture(scope, runtime_ptr, event);
                if let Some(target) = target {
                    dispatch_pointer_exit_events(scope, runtime_ptr, target, cancel);
                }
                unsafe { &mut *runtime_ptr }.clear_mouse_input_state();
            } else {
                unsafe { &mut *runtime_ptr }.clear_mouse_press();
            }
            Ok(())
        });
        self.suppress_compat_mouse_events = false;
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

    pub(crate) fn mouse_input_surface_hit(
        &self,
        x: f64,
        y: f64,
        event_name: &str,
    ) -> Result<crate::native_bridge::element::InputSurfaceHit> {
        let mode = match event_name {
            "wheel" => InputHitTestMode::Wheel,
            "mousedown" | "mouseup" | "mousemove" => InputHitTestMode::Mouse,
            _ => InputHitTestMode::Dom,
        };
        Ok(input_surface_hit_test(
            &self._context_host.borrow(),
            self.document_runtime.document_handle(),
            moli_layout::LayoutPoint::new(x as f32, y as f32),
            InputHitTestPolicy::INTERACTIVE,
            mode,
        )?)
    }

    fn mouse_routed_target(
        &mut self,
        hit: Option<DomHandle>,
        position: InputPosition,
        event_name: &str,
        prepared: &PreparedMouseInputDispatch,
    ) -> Option<DomHandle> {
        if !tracks_mouse_hover_for_event(event_name) {
            return hit;
        }
        let runtime = self._context_host.borrow();
        let hit_document = hit.and_then(|target| runtime.dom_host().owner_document_handle(target));
        if event_name == "mousedown" && prepared.previous_buttons == 0 {
            self.mouse_routed_frame = hit_document
                .filter(|document| *document != runtime.document_handle())
                .and_then(|document| runtime.input_owner_for_document(document));
        } else if event_name == "mousemove" && prepared.buttons == 0 {
            self.mouse_routed_frame = None;
        }
        let route = self
            .mouse_routed_frame
            .and_then(|owner| runtime.input_document_for_owner(owner));
        let Some(document) = route else {
            self.mouse_routed_frame = None;
            return hit;
        };
        if hit_document == Some(document) {
            return hit;
        }
        Some(input_target_in_document(
            &runtime,
            document,
            position.layout_point(),
        ))
    }

    fn prepare_mouse_input_dispatch(
        &mut self,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
    ) -> PreparedMouseInputDispatch {
        let previous_pressed_buttons = self.pressed_mouse_buttons;
        let button = if button >= 0 {
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
            let mut runtime = self._context_host.borrow_mut();
            runtime.clear_mouse_press();
            #[cfg(feature = "molio")]
            runtime.set_compat_mouse_press_owner(button, None);
            self.pending_drag_gesture = None;
            self.text_control_mouse_selection = None;
        } else if event_name == "mouseup" || self.pressed_mouse_buttons & 1 == 0 {
            self.pending_drag_gesture = None;
        }
        if event_name == "mousemove" && buttons & 1 == 0 {
            self.text_control_mouse_selection = None;
        }
        PreparedMouseInputDispatch {
            button,
            buttons,
            previous_buttons: previous_pressed_buttons,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_mouse_input_body(
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
        wheel_cancelable: bool,
        wheel_scroll_target: Option<&WheelScrollTarget>,
    ) -> Result<RendererInputDispatchOutcome> {
        let PreparedMouseInputDispatch {
            button, buttons, ..
        } = prepared;
        let position = InputPosition::headless(x, y);
        let sampled = self._context_host.borrow().published_frame_geometry();
        let drag_input = DragInput {
            position,
            sampled: sampled.as_deref(),
            modifiers,
        };
        let pointer_id = pointer.pointer_id;
        let releases_capture = event_name == "mouseup" && buttons == 0;

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
            let target = surface_hit.target;
            let active = &mut self.active_drag_session as *mut Option<ActiveDragSession>;
            self.with_default_context_scope(|scope, runtime_ptr| {
                let active = unsafe { &mut *active };
                if let Some(session) = active.as_mut() {
                    session.position = position;
                    if !session.intercepted {
                        update_drag_target(scope, runtime_ptr, session, target, drag_input);
                    }
                    if event_name == "mouseup" && button == 0 {
                        let reason = if session.intercepted {
                            DragFinishReason::Rejected
                        } else {
                            drop_drag_target(scope, runtime_ptr, session, drag_input)
                        };
                        finish_drag_session(scope, runtime_ptr, active, reason, drag_input);
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

        let hit_handle = surface_hit.target;
        let pointer_event_name = pointer_event_name_for_mouse_event(event_name);
        let tracks_hover = tracks_mouse_hover_for_event(event_name);
        let routed_target = self.mouse_routed_target(hit_handle, position, event_name, &prepared);
        if self.mouse_routed_frame.is_none()
            && let Some(control) = surface_hit.control
        {
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
        let initial_target = {
            let mut runtime = self._context_host.borrow_mut();
            if tracks_hover {
                #[cfg(feature = "molio")]
                let hover_source = surface_hit.hover_source();
                #[cfg(not(feature = "molio"))]
                let hover_source = hit_handle;
                runtime.set_hovered_element_for_input(hover_source);
            }
            if pointer_event_name.is_some() {
                if event_name == "mousemove" && buttons == 0 {
                    runtime.release_pending_pointer_capture_target(pointer_id);
                }
                runtime
                    .pending_pointer_capture_target(pointer_id)
                    .or(routed_target)
            } else {
                hit_handle.or_else(|| {
                    (event_name == "wheel")
                        .then(|| runtime.dom_host().document_element_handle())
                        .flatten()
                })
            }
        };
        let Some(initial_target) = initial_target else {
            if event_name == "mouseup" && button == 0 {
                self.finish_active_drag(DragFinishReason::Rejected, modifiers)?;
            } else if event_name == "mousemove" {
                let active = &mut self.active_drag_session as *mut Option<ActiveDragSession>;
                self.with_default_context_scope(|scope, runtime_ptr| {
                    if let Some(session) = unsafe { &mut *active }.as_mut()
                        && !session.intercepted
                    {
                        update_drag_target(scope, runtime_ptr, session, None, drag_input);
                    }
                    Ok(())
                })?;
            }
            if tracks_hover {
                let had_pointer_target = {
                    let mut runtime = self._context_host.borrow_mut();
                    let had_target = runtime.mouse_event_target().is_some();
                    runtime.set_mouse_event_target(None);
                    had_target
                };
                let previous = self.hovered_mouse_handle.take();
                if previous.is_some() || releases_capture {
                    self.with_default_context_scope(|scope, runtime_ptr| {
                        let runtime = unsafe { &*runtime_ptr };
                        let coordinates = previous
                            .or_else(|| runtime.active_pointer_capture_target(pointer_id))
                            .map_or_else(
                                || position.coordinates(None),
                                |target| position.for_target(runtime, target, sampled.as_deref()),
                            );
                        let data = PreparedPointerEvent {
                            coordinates,
                            button,
                            buttons,
                            pointer: &pointer,
                            modifiers,
                        };
                        if releases_capture {
                            data.release_capture(scope, runtime_ptr, None);
                        }
                        if let Some(previous) = previous {
                            if had_pointer_target {
                                dispatch_pointer_exit_events(scope, runtime_ptr, previous, data);
                            }
                            dispatch_mouse_boundary_events(
                                scope,
                                runtime_ptr,
                                Some(previous),
                                None,
                                coordinates,
                                coordinates,
                                button,
                                buttons,
                                modifiers,
                            );
                        }
                        Ok(())
                    })?;
                }
            }
            return Ok(input_dispatch_outcome(false));
        };
        let previous_hover = self.hovered_mouse_handle;
        let restarting_pointer_stream = pointer_event_name.is_some()
            && previous_hover.is_some()
            && self._context_host.borrow().mouse_event_target().is_none();
        let mut next_hover = previous_hover;
        let drag_start = if event_name == "mousemove"
            && buttons & 1 != 0
            && self.active_drag_session.is_none()
        {
            if self._context_host.borrow().mouse_press_handle().is_none() {
                self.pending_drag_gesture = None;
            }
            let gesture = self.pending_drag_gesture.filter(|gesture| {
                let point = gesture.root_to_frame.map_point(position.layout_point());
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

        if event_name == "mousedown" && button == 0 {
            let runtime = self._context_host.borrow();
            let document = runtime.dom_host().owner_document_handle(initial_target);
            let root_to_frame = document
                .and_then(|document| runtime.input_frame_transform(document, sampled.as_deref()))
                .unwrap_or(LayoutTransform2D::IDENTITY);
            let coordinates = position.coordinates(Some(root_to_frame));
            self.pending_drag_gesture = Some(PendingDragGesture {
                handle: initial_target,
                position,
                x: coordinates.client_x,
                y: coordinates.client_y,
                root_to_frame,
            });
        }

        let active_drag_session: *mut Option<ActiveDragSession> = &mut self.active_drag_session;
        let intercepted_drag = Cell::new(None);
        let mouse_down_allows_drag = Cell::new(true);
        let mut text_control_selection = self.text_control_mouse_selection;

        let mut suppress_compat_mouse_events = self.suppress_compat_mouse_events;

        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            let drag_start = drag_start.and_then(|gesture| {
                native_drag_source(scope, runtime_ptr, gesture)
                    .map(|(source, data)| (source, data, gesture.position))
            });
            let runtime = move || unsafe { &*runtime_ptr };
            let coordinates_for =
                |target| position.for_target(runtime(), target, sampled.as_deref());
            let mut pointer_data = PreparedPointerEvent {
                coordinates: coordinates_for(initial_target),
                button,
                buttons,
                pointer: &pointer,
                modifiers,
            };
            let initial_document = InputDocument::for_target(runtime(), initial_target);
            let mut pointer_dispatch_handle = initial_target;
            let is_move = event_name == "mousemove";
            let mut pointer_event = None;
            if let Some(name) = pointer_event_name {
                let trigger_name = if is_move { "pointerrawupdate" } else { name };
                let Some(trigger) = pointer_data.construct(scope, trigger_name) else {
                    return Ok(input_dispatch_outcome(false));
                };
                // Capture callbacks still see the active Document from the
                // previous pointerdown, pointermove or pointerup.
                unsafe { &mut *runtime_ptr }.set_pointer_buttons_active(pointer_id, buttons != 0);
                pointer_data.process_capture(scope, runtime_ptr, Some(trigger));
                let Some((mut input_document, target)) = initial_document.and_then(|document| {
                    let target = document.target_after_capture(
                        runtime(),
                        pointer_id,
                        routed_target,
                        position,
                    )?;
                    Some((document, target))
                }) else {
                    return Ok(input_dispatch_outcome(false));
                };
                pointer_dispatch_handle = target;
                let host = unsafe { &mut *runtime_ptr };
                if !is_move {
                    host.set_pointer_active_document(pointer_id, input_document.owner);
                }
                host.set_mouse_event_target(Some(pointer_dispatch_handle));
                if event_name == "mousedown" && button >= 0 {
                    unsafe { &mut *runtime_ptr }.begin_mouse_press(pointer_dispatch_handle, button);
                }
                if previous_hover != Some(pointer_dispatch_handle) || restarting_pointer_stream {
                    dispatch_mouse_pointer_boundaries(
                        scope,
                        runtime_ptr,
                        if restarting_pointer_stream {
                            None
                        } else {
                            previous_hover
                        },
                        pointer_dispatch_handle,
                        restarting_pointer_stream,
                        pointer_data,
                        previous_hover.map(coordinates_for),
                    );
                }
                let event = if is_move {
                    if input_document.contains(runtime(), pointer_dispatch_handle) {
                        let _ = dispatch_public_event(
                            scope,
                            runtime_ptr,
                            pointer_dispatch_handle,
                            trigger,
                        );
                    }
                    // Raw updates and the following move are separate events.
                    // Capture changes and any layout published in the raw
                    // listener are visible when the move is constructed.
                    if runtime().layout_policy().is_realtime() {
                        runtime().ensure_observable_layout()?;
                    }
                    let next_target = runtime()
                        .pending_pointer_capture_target(pointer_id)
                        .or(routed_target
                            .filter(|target| input_document.contains(runtime(), *target)))
                        .or_else(|| {
                            runtime()
                                .mouse_event_target()
                                .filter(|target| input_document.contains(runtime(), *target))
                        })
                        .unwrap_or(pointer_dispatch_handle);
                    pointer_data.coordinates = coordinates_for(next_target);
                    let Some(next_document) = InputDocument::for_target(runtime(), next_target)
                        .filter(|document| document.contains(runtime(), next_target))
                    else {
                        return Ok(input_dispatch_outcome(false));
                    };
                    input_document = next_document;
                    let Some(event) = pointer_data.construct(scope, name) else {
                        return Ok(input_dispatch_outcome(false));
                    };
                    pointer_data.process_capture(scope, runtime_ptr, Some(event));
                    let Some(target) = input_document.target_after_capture(
                        runtime(),
                        pointer_id,
                        routed_target
                            .into_iter()
                            .chain(runtime().mouse_event_target()),
                        position,
                    ) else {
                        return Ok(input_dispatch_outcome(false));
                    };
                    let previous = pointer_dispatch_handle;
                    pointer_dispatch_handle = target;
                    let host = unsafe { &mut *runtime_ptr };
                    host.set_pointer_active_document(pointer_id, input_document.owner);
                    host.set_mouse_event_target(Some(pointer_dispatch_handle));
                    if previous != target {
                        dispatch_mouse_pointer_boundaries(
                            scope,
                            runtime_ptr,
                            Some(previous),
                            target,
                            false,
                            pointer_data,
                            Some(coordinates_for(previous)),
                        );
                    }
                    event
                } else {
                    trigger
                };
                pointer_event = Some((event, input_document));
                next_hover = Some(pointer_dispatch_handle);
                if previous_hover != next_hover {
                    dispatch_mouse_boundary_events(
                        scope,
                        runtime_ptr,
                        previous_hover,
                        next_hover,
                        previous_hover.map(coordinates_for).unwrap_or_default(),
                        coordinates_for(pointer_dispatch_handle),
                        button,
                        buttons,
                        modifiers,
                    );
                }
            } else if initial_document.is_none() {
                return Ok(input_dispatch_outcome(false));
            }
            if let Some((event, input_document)) = pointer_event
                && input_document.contains(runtime(), pointer_dispatch_handle)
            {
                let dispatched =
                    dispatch_public_event(scope, runtime_ptr, pointer_dispatch_handle, event);
                if event_name == "mousedown" && !dispatched.allows_default() {
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
            if !suppress_current_mouse_event
                && let Some(mouse_dispatch_handle) = mouse_dispatch_handle
            {
                if runtime().layout_policy().is_realtime() {
                    runtime().ensure_observable_layout()?;
                }
                let mouse_coordinates = coordinates_for(mouse_dispatch_handle);
                let event = if event_name == "wheel" {
                    construct_wheel_event_for_target(
                        scope,
                        runtime_ptr,
                        mouse_dispatch_handle,
                        event_name,
                        mouse_coordinates,
                        delta_x,
                        delta_y,
                        buttons,
                        modifiers,
                        wheel_cancelable,
                    )
                } else if matches!(event_name, "mousedown" | "mouseup") {
                    construct_mouse_event_with_detail_for_target(
                        scope,
                        runtime_ptr,
                        mouse_dispatch_handle,
                        event_name,
                        mouse_coordinates,
                        click_count.max(0),
                        button,
                        buttons,
                        modifiers,
                    )
                } else {
                    construct_mouse_event_for_target(
                        scope,
                        runtime_ptr,
                        mouse_dispatch_handle,
                        event_name,
                        mouse_coordinates,
                        button,
                        buttons,
                        modifiers,
                    )
                };
                if let Some(event) = event {
                    #[cfg(feature = "molio")]
                    if event_name == "mousedown" {
                        // The element can disappear or later moves can reach another
                        // frame while the Window still owns the mouse press.
                        let owner = InputDocument::for_target(runtime(), mouse_dispatch_handle)
                            .map(|document| document.owner);
                        unsafe { &mut *runtime_ptr }.set_compat_mouse_press_owner(button, owner);
                    }
                    let dispatched =
                        dispatch_public_event(scope, runtime_ptr, mouse_dispatch_handle, event);
                    if event_name == "wheel" {
                        let wheel = if dispatched.allows_default() {
                            let scrolled = if delta_x == 0.0 && delta_y == 0.0 {
                                false
                            } else if let Some(scroll_target) = wheel_scroll_target {
                                perform_latched_wheel_scroll_default_action(
                                    scope,
                                    runtime_ptr,
                                    scroll_target,
                                    delta_x,
                                    delta_y,
                                )?
                            } else {
                                perform_wheel_scroll_default_action(
                                    scope,
                                    runtime_ptr,
                                    pointer_dispatch_handle,
                                    delta_x,
                                    delta_y,
                                )?
                            };
                            if scrolled {
                                RendererWheelDispatchOutcome::Scrolled
                            } else {
                                RendererWheelDispatchOutcome::Unconsumed
                            }
                        } else {
                            RendererWheelDispatchOutcome::Canceled
                        };
                        return Ok(RendererInputDispatchOutcome {
                            wheel: Some(wheel),
                            ..input_dispatch_outcome(true)
                        });
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
                        let target = routed_target.unwrap_or(initial_target);
                        perform_mouse_focus_default_action(scope, runtime_ptr, target);
                        if button == 0 {
                            text_control_selection = TextControlMouseSelection::begin(
                                scope,
                                runtime_ptr,
                                target,
                                position,
                                click_count,
                                moli_input::InputModifiers::from_bits(modifiers),
                            )?;
                        }
                    }
                    if dispatched.allows_default() {
                        if event_name == "mousemove"
                            && buttons & 1 != 0
                            && let Some(selection) = &mut text_control_selection
                            && !selection.extend(scope, runtime_ptr, position)?
                        {
                            text_control_selection = None;
                        }
                        if event_name == "mouseup"
                            && button == 0
                            && let Some(selection) = text_control_selection.take()
                        {
                            selection.release(scope, runtime_ptr, position);
                        }
                    }
                }
            }
            let captured_click_target = if releases_capture {
                let captured = runtime().pointer_capture_target_for_click(pointer_id);
                if let Some((event, _)) = pointer_event {
                    pointer_data.release_capture(scope, runtime_ptr, Some(event));
                }
                captured
            } else {
                None
            };
            if let Some((drag_start_handle, initial_data, start_position)) = drag_start
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
                    DragInput {
                        position: start_position,
                        ..drag_input
                    },
                    data_transfer,
                )
                .is_some_and(|outcome| outcome.allows_default());
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
                    // Snapshot only an accepted intercepted drag, before any
                    // subsequent pointer callbacks can change its source.
                    let intercepted = intercept_drags.then(|| {
                        let (items, drag_operations_mask) =
                            crate::context_bootstrap::read_string_drag_data(scope, &data_store);
                        RendererInterceptedDrag {
                            items,
                            drag_operations_mask,
                        }
                    });
                    let source_transfer = v8::Global::new(scope, data_transfer);
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
                        position,
                        modifiers,
                    });
                    unsafe { &mut *runtime_ptr }.clear_mouse_press();
                    let cancel_target = runtime()
                        .active_pointer_capture_target(pointer_id)
                        .unwrap_or(pointer_dispatch_handle);
                    let cancel_pointer = RendererPointerEventProperties {
                        pressure: 0.0,
                        ..pointer.clone()
                    };
                    let cancel = PreparedPointerEvent {
                        coordinates: coordinates_for(cancel_target),
                        button: -1,
                        buttons: 0,
                        pointer: &cancel_pointer,
                        modifiers,
                    };
                    unsafe { &mut *runtime_ptr }.set_pointer_buttons_active(pointer_id, false);
                    if let Some(event) = cancel.construct(scope, "pointercancel") {
                        let _ = dispatch_public_event(scope, runtime_ptr, cancel_target, event);
                        cancel.release_capture(scope, runtime_ptr, Some(event));
                    }
                    dispatch_pointer_exit_events(scope, runtime_ptr, cancel_target, cancel);
                    // Preserve ScriptVm's legacy mouse hover cache while
                    // marking this suppressed pointer stream as outside.
                    unsafe { &mut *runtime_ptr }.clear_mouse_input_state();
                    if let Some(intercepted) = intercepted {
                        intercepted_drag.set(Some(intercepted));
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
                update_drag_target(scope, runtime_ptr, session, hit_handle, drag_input);
            }
            if event_name == "mouseup" && button == 0 {
                let active = unsafe { &mut *active_drag_session };
                let mut reason = DragFinishReason::Rejected;
                if let Some(session) = active.as_mut()
                    && !session.intercepted
                {
                    // Revalidate against the release target. Acceptance from
                    // a previously hovered element must never authorize it.
                    update_drag_target(scope, runtime_ptr, session, hit_handle, drag_input);
                    reason = drop_drag_target(scope, runtime_ptr, session, drag_input);
                }
                finish_drag_session(scope, runtime_ptr, active, reason, drag_input);
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
                    let coordinates = coordinates_for(click_target);
                    let outcome = activate_handle_after_pointer_release(
                        scope,
                        runtime_ptr,
                        click_target,
                        coordinates,
                        button,
                        buttons,
                        click_count.max(1),
                        modifiers,
                    );
                    if click_count == 2
                        && let Some(event) = construct_mouse_event_with_detail_for_target(
                            scope,
                            runtime_ptr,
                            click_target,
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
                Some(MouseReleaseFollowUp::DispatchEvent {
                    name: follow_up_event_name,
                    target,
                }) => {
                    let had_pending_top_level_navigation_before_event =
                        unsafe { &*runtime_ptr }.has_pending_location_navigation();
                    let pending_child_navigations_before_event = unsafe { &*runtime_ptr }
                        .pending_live_child_browsing_context_navigation_snapshot();
                    let mut pending_download = None;
                    if let Some(event) = construct_mouse_event_for_target(
                        scope,
                        runtime_ptr,
                        target,
                        follow_up_event_name,
                        coordinates_for(target),
                        button,
                        buttons,
                        modifiers,
                    ) {
                        let dispatched = dispatch_public_event(scope, runtime_ptr, target, event);
                        if follow_up_event_name == "auxclick" && dispatched.allows_default() {
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
                    if follow_up_event_name == "auxclick" {
                        return Ok(RendererInputDispatchOutcome {
                            handled: true,
                            wheel: None,
                            triggered_top_level_navigation:
                                !had_pending_top_level_navigation_before_event
                                    && unsafe { &*runtime_ptr }.has_pending_location_navigation(),
                            pending_download,
                            pending_file_chooser: None,
                            intercepted_drag: None,
                        });
                    }
                }
                None => {}
            }
            Ok(input_dispatch_outcome(true))
        });
        if self
            .active_drag_session
            .as_ref()
            .is_some_and(|session| session.source.is_some())
        {
            self.mouse_routed_frame = None;
        }
        self.hovered_mouse_handle = next_hover
            .filter(|target| self._context_host.borrow().dom_host().is_connected(*target));
        self.suppress_compat_mouse_events = suppress_compat_mouse_events;
        if text_control_selection.is_some() || !mouse_down_allows_drag.get() {
            self.pending_drag_gesture = None;
        }
        self.text_control_mouse_selection = text_control_selection;
        result.map(|mut outcome| {
            outcome.intercepted_drag = intercepted_drag.take();
            outcome
        })
    }

    fn finish_active_drag(&mut self, reason: DragFinishReason, modifiers: u8) -> Result<()> {
        let (position, sampled, layout) = if let Some(session) = &self.active_drag_session {
            let host = self._context_host.borrow();
            let layout = host.ensure_observable_layout();
            (session.position, host.published_frame_geometry(), layout)
        } else {
            (InputPosition::default(), None, Ok(()))
        };
        let drag_input = DragInput {
            position,
            sampled: sampled.as_deref(),
            modifiers,
        };
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
                drag_input,
            );
            Ok(())
        })?;
        // Always finish the session before reporting a layout failure. A page
        // without a root has no frame coordinates to prepare.
        match layout {
            Ok(()) | Err(moli_layout::LayoutError::NoLayoutRoot) => Ok(()),
            Err(error) => Err(error.into()),
        }
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

    #[cfg(test)]
    pub(crate) fn dispatch_touch_event_at_point(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        activate: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        self.dispatch_touch_event_at_points(
            &[RendererTouchPoint { id: 0, x, y }],
            event_name,
            activate,
        )
    }

    pub(crate) fn dispatch_touch_event_at_points(
        &mut self,
        points: &[RendererTouchPoint],
        event_name: &str,
        activate: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        // synthesizeTapGesture supplies a complete gesture rather than a raw
        // end packet. Both APIs finish through the same tap recognition path.
        if activate
            && event_name == "touchend"
            && self.active_touch_points.is_empty()
            && let [point] = points
        {
            let started = self.dispatch_multi_touch_event_at_points(&[*point], "touchstart")?;
            if !started.handled
                || started.triggered_top_level_navigation
                || started.pending_download.is_some()
                || started.pending_file_chooser.is_some()
            {
                return Ok(started);
            }
        }
        if points.is_empty() && matches!(event_name, "touchend" | "touchcancel") {
            let ended = self
                .active_touch_points
                .iter()
                .map(|(id, point)| RendererTouchPoint {
                    id: *id,
                    x: point.x,
                    y: point.y,
                })
                .collect::<Vec<_>>();
            return self.dispatch_multi_touch_event_at_points(&ended, event_name);
        }
        self.dispatch_multi_touch_event_at_points(points, event_name)
    }

    fn dispatch_multi_touch_event_at_points(
        &mut self,
        points: &[RendererTouchPoint],
        event_name: &str,
    ) -> Result<RendererInputDispatchOutcome> {
        if points.is_empty() {
            return Ok(input_dispatch_outcome(false));
        }

        let is_end = matches!(event_name, "touchend" | "touchcancel");
        let was_empty = self.active_touch_points.is_empty();
        let mut changed_points = Vec::new();
        for point in points {
            let previous = self.active_touch_points.get(&point.id).copied();
            let target = match event_name {
                "touchstart" if previous.is_none() => observable_input_hit_test(
                    &self._context_host.borrow(),
                    self.document_runtime.document_handle(),
                    moli_layout::LayoutPoint::new(point.x as f32, point.y as f32),
                )?,
                "touchmove" if previous.is_some_and(|p| p.x != point.x || p.y != point.y) => {
                    previous.map(|p| p.target)
                }
                "touchend" | "touchcancel" => previous.map(|p| p.target),
                _ => None,
            };
            if let Some(target) = target {
                changed_points.push((*point, target, previous));
            }
        }
        if changed_points.is_empty() {
            return Ok(input_dispatch_outcome(false));
        }
        let mut tap = self.pending_touch_tap.take();
        if event_name == "touchstart"
            && was_empty
            && let [(point, target, _)] = changed_points.as_slice()
            && let Some(document) = self
                ._context_host
                .borrow()
                .dom_host()
                .owner_document_handle(*target)
        {
            tap = Some(PendingTouchTap {
                id: point.id,
                target: *target,
                document,
                start_x: point.x,
                start_y: point.y,
                suppress_mouse: false,
            });
        }
        if event_name == "touchcancel" {
            tap = None;
        }
        if let Some(candidate) = tap
            && changed_points.iter().any(|(p, _, _)| {
                p.id == candidate.id
                    && (p.x - candidate.start_x).powi(2) + (p.y - candidate.start_y).powi(2)
                        >= TOUCH_TAP_SLOP * TOUCH_TAP_SLOP
            })
        {
            tap = None;
        }
        match event_name {
            "touchstart" | "touchmove" => {
                for (point, target, previous) in &changed_points {
                    let is_primary =
                        previous.map_or(self.active_touch_points.is_empty(), |p| p.is_primary);
                    self.active_touch_points.insert(
                        point.id,
                        ActiveTouchPoint {
                            x: point.x,
                            y: point.y,
                            target: *target,
                            is_primary,
                        },
                    );
                }
                if self.active_touch_points.len() > 1 {
                    tap = None;
                }
            }
            "touchend" | "touchcancel" => {
                for (point, _, _) in &changed_points {
                    self.active_touch_points.remove(&point.id);
                }
            }
            _ => {}
        }

        let pointer_event_name = pointer_event_name_for_touch_event(event_name);
        let button = button_for_touch_pointer_event(event_name);
        let buttons = buttons_for_touch_pointer_event(event_name);
        let event_target = changed_points[0].1;
        let active_snapshot = self
            .active_touch_points
            .iter()
            .map(|(id, point)| (*id, *point))
            .collect::<Vec<_>>();
        let previous_mouse_hover = self.hovered_mouse_handle;
        let mut compatibility_hover = None;
        let document = self.document_runtime.document_handle();
        let mut pointer_handles = std::mem::take(&mut self.active_touch_pointer_handles);
        let sampled = self._context_host.borrow().published_frame_geometry();
        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            for (index, (point, target, previous)) in changed_points.iter().enumerate() {
                let Some(event_name_for_pointer) = pointer_event_name else {
                    break;
                };
                let position = InputPosition::headless(point.x, point.y);
                let pointer_id = touch_pointer_id(point.id);
                let is_primary = previous.map_or(was_empty && index == 0, |p| p.is_primary);
                let pointer = touch_pointer_properties(event_name, pointer_id, is_primary);
                let runtime = move || unsafe { &*runtime_ptr };
                if runtime().layout_policy().is_realtime() {
                    runtime().ensure_observable_layout()?;
                }
                let capture = runtime().pending_pointer_capture_target(pointer_id);
                let route = if event_name == "touchstart" || event_name == "touchcancel" {
                    *target
                } else if let Some(capture) = capture {
                    capture
                } else {
                    observable_input_hit_test(runtime(), document, position.layout_point())?
                        .unwrap_or(*target)
                };
                let chosen_target = capture.unwrap_or(route);
                let input_document = InputDocument::for_target(runtime(), chosen_target);
                let data = PreparedPointerEvent {
                    coordinates: position.for_target(runtime(), chosen_target, sampled.as_deref()),
                    button,
                    buttons,
                    pointer: &pointer,
                    modifiers: 0,
                };
                'pointer: {
                    let Some(event) = data.construct(scope, event_name_for_pointer) else {
                        break 'pointer;
                    };
                    // Pending capture still belongs to the previous active Document.
                    unsafe { &mut *runtime_ptr }
                        .set_pointer_buttons_active(pointer_id, buttons != 0);
                    data.process_capture(scope, runtime_ptr, Some(event));
                    let Some(input_document) = input_document else {
                        break 'pointer;
                    };
                    let Some(pointer_handle) = input_document.target_after_capture(
                        runtime(),
                        pointer_id,
                        Some(route),
                        position,
                    ) else {
                        break 'pointer;
                    };
                    let host = unsafe { &mut *runtime_ptr };
                    if event_name_for_pointer != "pointercancel" {
                        host.set_pointer_active_document(pointer_id, input_document.owner);
                    }
                    if event_name == "touchstart" {
                        host.set_pending_pointer_capture_target(pointer_id, pointer_handle);
                    }
                    let previous_pointer = pointer_handles.insert(point.id, pointer_handle);
                    dispatch_touch_pointer_boundary_events(
                        scope,
                        runtime_ptr,
                        previous_pointer,
                        Some(pointer_handle),
                        position,
                        data,
                        sampled.as_deref(),
                    );
                    if input_document.contains(runtime(), pointer_handle) {
                        let dispatched =
                            dispatch_public_event(scope, runtime_ptr, pointer_handle, event);
                        if event_name == "touchstart"
                            && !dispatched.allows_default()
                            && let Some(candidate) = &mut tap
                            && candidate.id == point.id
                        {
                            candidate.suppress_mouse = true;
                        }
                    }
                    if is_end {
                        data.release_capture(scope, runtime_ptr, Some(event));
                        dispatch_touch_pointer_boundary_events(
                            scope,
                            runtime_ptr,
                            Some(pointer_handle),
                            None,
                            position,
                            data,
                            sampled.as_deref(),
                        );
                    }
                }
                if is_end {
                    unsafe { &mut *runtime_ptr }.remove_touch_pointer_input(pointer_id);
                }
            }
            if unsafe { &*runtime_ptr }.layout_policy().is_realtime() {
                unsafe { &*runtime_ptr }.ensure_observable_layout()?;
            }
            let mut active_event_points = Vec::with_capacity(active_snapshot.len());
            for (id, point) in &active_snapshot {
                let Some(point) = prepare_touch_point(
                    scope,
                    runtime_ptr,
                    *id,
                    InputPosition::headless(point.x, point.y),
                    point.target,
                    event_target,
                    sampled.as_deref(),
                ) else {
                    return Ok(input_dispatch_outcome(false));
                };
                active_event_points.push(point);
            }
            let mut changed_event_points = Vec::with_capacity(changed_points.len());
            for (point, target, _) in &changed_points {
                let Some(point) = prepare_touch_point(
                    scope,
                    runtime_ptr,
                    point.id,
                    InputPosition::headless(point.x, point.y),
                    *target,
                    event_target,
                    sampled.as_deref(),
                ) else {
                    return Ok(input_dispatch_outcome(false));
                };
                changed_event_points.push(point);
            }

            if let Some(event) = construct_touch_event_with_points(
                scope,
                event_name,
                &active_event_points,
                &changed_event_points,
            ) && !dispatch_public_event(scope, runtime_ptr, event_target, event).allows_default()
            {
                tap = None;
            }
            if event_name == "touchend"
                && let Some(candidate) = tap.take()
                && let Some((point, _, _)) =
                    changed_points.iter().find(|(p, _, _)| p.id == candidate.id)
            {
                let (outcome, hover) = dispatch_touch_tap_activation(
                    scope,
                    runtime_ptr,
                    candidate,
                    point.x,
                    point.y,
                    previous_mouse_hover,
                )?;
                compatibility_hover = Some(hover);
                return Ok(outcome);
            }
            Ok(input_dispatch_outcome(true))
        });
        if is_end && result.is_err() {
            let mut host = self._context_host.borrow_mut();
            for (point, _, _) in &changed_points {
                host.remove_touch_pointer_input(touch_pointer_id(point.id));
            }
        }
        pointer_handles.retain(|id, _| self.active_touch_points.contains_key(id));
        self.active_touch_pointer_handles = pointer_handles;
        self.pending_touch_tap = tap.filter(|candidate| {
            self.active_touch_points.contains_key(&candidate.id)
                && touch_tap_is_live(&self._context_host.borrow(), *candidate)
        });
        if let Some(hover) = compatibility_hover {
            self.hovered_mouse_handle =
                hover.filter(|node| self._context_host.borrow().dom_host().is_connected(*node));
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
        let target = observable_input_hit_test(
            &self._context_host.borrow(),
            self.document_runtime.document_handle(),
            moli_layout::LayoutPoint::new(x as f32, y as f32),
        )?;
        let position = InputPosition::headless(x, y);
        let sampled = self._context_host.borrow().published_frame_geometry();
        let drag_input = DragInput {
            position,
            sampled: sampled.as_deref(),
            modifiers,
        };
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
                    position,
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
                update_drag_target(scope, runtime_ptr, session, target, drag_input);
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
                    drop_drag_target(scope, runtime_ptr, session, drag_input)
                } else {
                    DragFinishReason::Rejected
                };
                finish_drag_session(scope, runtime_ptr, active, reason, drag_input);
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
            let multiple =
                element.has_attribute("multiple") || element.has_attribute("webkitdirectory");
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
            Ok(perform_text_input(
                scope,
                runtime_ptr,
                handle,
                text,
                TextEditInputType::InsertText,
                TextInputSource::InputMethod,
            ))
        });
        self.finish_input_command_checkpoint(result)
    }

    #[cfg(test)]
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
        self.dispatch_key_event_with_location(
            event_name,
            key,
            code,
            text,
            modifiers,
            auto_repeat,
            should_insert_text,
            0,
        )
    }

    pub(crate) fn dispatch_key_event_with_location(
        &mut self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
        location: u32,
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

        let result = self.with_native_keyboard_event_scope(|scope, runtime_ptr| {
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
                location,
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

            // Combined keydown/text input includes a cancelable keypress before
            // editing. Callback cleanup checkpoints between native listeners;
            // an explicit char command must not emit keypress twice.
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
                    location,
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
                    return Ok(input_dispatch_outcome(perform_text_input(
                        scope,
                        runtime_ptr,
                        handle,
                        "\n",
                        TextEditInputType::InsertLineBreak,
                        TextInputSource::Keyboard,
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
                if is_character_phase && contenteditable_editing_host(runtime, handle).is_some() {
                    return Ok(input_dispatch_outcome(perform_text_input(
                        scope,
                        runtime_ptr,
                        handle,
                        "\n",
                        if shift {
                            TextEditInputType::InsertLineBreak
                        } else {
                            TextEditInputType::InsertParagraph
                        },
                        TextInputSource::Keyboard,
                    )));
                }
                if event_name == "keydown" && (target.is_button_like || target.is_anchor_like) {
                    return Ok(activate_handle_via_keyboard(
                        scope,
                        runtime_ptr,
                        handle,
                        modifiers,
                    ));
                }
            }

            if should_insert_text {
                if text.is_empty() {
                    return Ok(input_dispatch_outcome(true));
                }
                if target.is_text_control || contenteditable_editing_host(runtime, handle).is_some()
                {
                    return Ok(input_dispatch_outcome(perform_text_input(
                        scope,
                        runtime_ptr,
                        handle,
                        text,
                        TextEditInputType::InsertText,
                        TextInputSource::Keyboard,
                    )));
                }
                return Ok(input_dispatch_outcome(true));
            }

            if event_name == "keyup" && is_space_key(&key_lower) {
                if target.is_checkbox || target.is_button_like || target.is_radio {
                    return Ok(activate_handle_via_keyboard(
                        scope,
                        runtime_ptr,
                        handle,
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
                    runtime_ptr,
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

            if target.is_text_control {
                text_control_selection::move_text_control_selection(
                    scope,
                    runtime_ptr,
                    handle,
                    &key_lower,
                    moli_input::InputModifiers::from_bits(modifiers),
                );
            }

            Ok(input_dispatch_outcome(true))
        });
        if canceled_raw_keydown.get() {
            self.suppress_next_keypress_after_canceled_raw_keydown = true;
        }
        self.finish_input_event_dispatch_turn(result)
    }
}

fn mouse_button_mask(button: i32) -> i32 {
    match button {
        0 => 1,
        1 => 4,
        2 => 2,
        3 => 8,
        4 => 16,
        _ => 0,
    }
}

fn mouse_button_from_mask(mask: i32) -> Option<i32> {
    match mask {
        1 => Some(0),
        4 => Some(1),
        2 => Some(2),
        8 => Some(3),
        16 => Some(4),
        _ => None,
    }
}

fn single_changed_mouse_button(mask: i32) -> Option<i32> {
    if mask.count_ones() == 1 {
        mouse_button_from_mask(mask)
    } else {
        None
    }
}
