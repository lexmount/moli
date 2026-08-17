use anyhow::Result;

use super::input_dispatch_outcome;
use super::input_helpers::{
    MouseReleaseFollowUp, PendingMousePress, mouse_button_mask, single_changed_mouse_button,
};
use super::inspector::{
    current_selection_range, current_selection_state, is_space_key, key_target_info,
    option_is_disabled, radio_group_members,
};
use super::{ActiveDragSession, ActiveTouchPoint, ScriptVm};
use crate::document_runtime::DomHandle;
use crate::dom::native::{Node, SelectedFile};
use crate::native_bridge::PointerCaptureDispatchEvent;
use crate::native_bridge::element::{
    TouchEventPoint, activate_handle_via_click,
    activate_handle_via_click_with_detail_and_modifiers, cache_input_files_from_selected_files,
    construct_drag_event, construct_keyboard_event,
    construct_mouse_event_with_detail_and_modifiers, construct_mouse_event_with_modifiers,
    construct_mouse_event_with_related_target_and_modifiers, construct_pointer_event,
    construct_pointer_event_with_modifiers, construct_pointer_event_with_related_target,
    construct_pointer_event_with_related_target_and_modifiers, construct_simple_event,
    construct_touch_event, construct_touch_event_with_points, construct_wheel_event,
    contenteditable_editing_host, dispatch_public_event, observable_input_hit_test,
    perform_drop_default_action, perform_mouse_focus_default_action,
    perform_wheel_scroll_default_action, replace_contenteditable_selection,
    replace_text_control_selection, select_contenteditable_contents,
    text_control_set_selection_range_internal,
    text_control_set_selection_range_with_direction_internal, text_control_value, update_focus,
};
use crate::runtime::{
    RendererDragData, RendererInputDispatchOutcome, RendererPointerEventProperties,
    RendererTouchPoint,
};
use crate::util::node_wrapper_from_handle;

fn related_target_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Value>> {
    node_wrapper_from_handle(scope, handle?).map(Into::into)
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

const MOUSE_POINTER_ID: i32 = 1;
const TOUCH_POINTER_ID: i32 = 2;

fn touch_pointer_id(touch_id: i32) -> i32 {
    TOUCH_POINTER_ID.saturating_add(touch_id.max(0))
}

fn activates_pointer_capture_for_mouse_event(event_name: &str, buttons: i32) -> bool {
    matches!(event_name, "mousedown" | "mouseup") || (event_name == "mousemove" && buttons != 0)
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

fn dispatch_pointer_capture_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    pointer_id: i32,
    events: &[PointerCaptureDispatchEvent],
    x: f64,
    y: f64,
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
            x,
            y,
            button,
            buttons,
            pointer,
            modifiers,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, capture_event.target, event);
        }
        if capture_event.event_name == "gotpointercapture" {
            let lost_target = unsafe { &mut *runtime_ptr }
                .lost_pointer_capture_target_after_got(pointer_id, capture_event.target);
            if let Some(lost_target) = lost_target
                && let Some(event) = construct_pointer_event_with_modifiers(
                    scope,
                    "lostpointercapture",
                    x,
                    y,
                    button,
                    buttons,
                    pointer,
                    modifiers,
                )
            {
                let _ = dispatch_public_event(scope, runtime_ptr, lost_target, event);
            }
        }
    }
}

fn release_pointer_capture_after_pointer_end(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    pointer_id: i32,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
    modifiers: u8,
) {
    let release_capture_events = {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime.release_pending_pointer_capture_target(pointer_id);
        let events = runtime.process_pending_pointer_capture(pointer_id);
        runtime.set_pointer_capture_active(pointer_id, false);
        events
    };
    dispatch_pointer_capture_events(
        scope,
        runtime_ptr,
        pointer_id,
        &release_capture_events,
        x,
        y,
        button,
        buttons,
        pointer,
        modifiers,
    );
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
            x,
            y,
            button,
            buttons,
            pointer,
            related_target,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
        }
        let related_target = related_target_value(scope, current);
        if let Some(event) = construct_pointer_event_with_related_target(
            scope,
            "pointerleave",
            x,
            y,
            button,
            buttons,
            pointer,
            related_target,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
        }
    }
    if let Some(current_handle) = current {
        let related_target = related_target_value(scope, previous);
        if let Some(event) = construct_pointer_event_with_related_target(
            scope,
            "pointerover",
            x,
            y,
            button,
            buttons,
            pointer,
            related_target,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, current_handle, event);
        }
        let related_target = related_target_value(scope, previous);
        if let Some(event) = construct_pointer_event_with_related_target(
            scope,
            "pointerenter",
            x,
            y,
            button,
            buttons,
            pointer,
            related_target,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, current_handle, event);
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
    ) -> Result<RendererInputDispatchOutcome> {
        let result = self
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers_without_checkpoint(
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
            );
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
    ) -> Result<RendererInputDispatchOutcome> {
        let previous_pressed_buttons = self.pressed_mouse_buttons;
        let released_press = if event_name == "mouseup" {
            self.pending_mouse_press.take()
        } else {
            None
        };
        let button = if button >= 0 {
            button
        } else {
            match event_name {
                "mousedown" => buttons
                    .and_then(|current_buttons| {
                        single_changed_mouse_button(current_buttons & !previous_pressed_buttons)
                    })
                    .unwrap_or(-1),
                "mouseup" => released_press
                    .as_ref()
                    .map(|press| press.button)
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
                    self.pending_mouse_press = None;
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

        let hit_handle = observable_input_hit_test(
            &self._context_host.borrow(),
            self.document_runtime.document_handle(),
            moli_layout::LayoutPoint::new(x as f32, y as f32),
        )?;
        let pointer_event_name = pointer_event_name_for_mouse_event(event_name);
        let mut pending_pointer_capture_events = Vec::new();
        if pointer_event_name.is_some() {
            let mut context_host = self._context_host.borrow_mut();
            if event_name == "mousemove" && buttons == 0 {
                context_host.release_pending_pointer_capture_target(MOUSE_POINTER_ID);
            }
            context_host.set_pointer_capture_active(
                MOUSE_POINTER_ID,
                activates_pointer_capture_for_mouse_event(event_name, buttons),
            );
            pending_pointer_capture_events =
                context_host.process_pending_pointer_capture(MOUSE_POINTER_ID);
        }
        if !pending_pointer_capture_events.is_empty() {
            self.with_default_context_scope(|scope, runtime_ptr| {
                dispatch_pointer_capture_events(
                    scope,
                    runtime_ptr,
                    MOUSE_POINTER_ID,
                    &pending_pointer_capture_events,
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                    modifiers,
                );
                Ok(())
            })?;
        }
        let capture_handle = pointer_event_name
            .is_some()
            .then(|| {
                self._context_host
                    .borrow()
                    .active_pointer_capture_target(MOUSE_POINTER_ID)
            })
            .flatten();
        if tracks_mouse_hover_for_event(event_name) {
            let mut context_host = self._context_host.borrow_mut();
            context_host.set_hovered_element_for_input(hit_handle);
        }
        let wheel_fallback_handle = (event_name == "wheel")
            .then(|| {
                self._context_host
                    .borrow()
                    .dom_host()
                    .document_element_handle()
            })
            .flatten();
        let Some(handle) = capture_handle.or(hit_handle).or(wheel_fallback_handle) else {
            return Ok(input_dispatch_outcome(false));
        };
        let hover_transition = if tracks_mouse_hover_for_event(event_name) {
            let previous = self.hovered_mouse_handle;
            if previous != Some(handle) {
                self.hovered_mouse_handle = Some(handle);
                Some(previous)
            } else {
                None
            }
        } else {
            None
        };
        let drag_start_handle = if event_name == "mousemove"
            && buttons & 1 != 0
            && self.active_drag_session.is_none()
        {
            self.pending_mouse_press.map(|press| press.handle)
        } else {
            None
        };

        let follow_up = if event_name == "mouseup" {
            match released_press {
                Some(PendingMousePress {
                    handle: pressed_handle,
                    button: pressed_button,
                }) if pressed_handle == handle && pressed_button == button => match button {
                    0 => Some(MouseReleaseFollowUp::ActivateViaClick),
                    1 => Some(MouseReleaseFollowUp::DispatchEvent("auxclick")),
                    2 => Some(MouseReleaseFollowUp::DispatchEvent("contextmenu")),
                    _ => None,
                },
                _ => None,
            }
        } else {
            if event_name == "mousedown" && button >= 0 {
                self.pending_mouse_press = Some(PendingMousePress { handle, button });
            }
            None
        };

        let active_drag_session: *mut Option<ActiveDragSession> = &mut self.active_drag_session;

        let mut suppress_compat_mouse_events = self.suppress_compat_mouse_events;
        if event_name == "mousemove" && buttons == 0 {
            suppress_compat_mouse_events = false;
        }

        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            let mut pointer_dispatch_handle = handle;
            if let Some(Some(previous_handle)) = hover_transition {
                let related_target = related_target_value(scope, Some(handle));
                if let Some(event) = construct_pointer_event_with_related_target_and_modifiers(
                    scope,
                    "pointerout",
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
                }
                let related_target = related_target_value(scope, Some(handle));
                if let Some(event) = construct_pointer_event_with_related_target_and_modifiers(
                    scope,
                    "pointerleave",
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
                }
            }
            if let Some(previous_handle) = hover_transition {
                let related_target = related_target_value(scope, previous_handle);
                if let Some(event) = construct_pointer_event_with_related_target_and_modifiers(
                    scope,
                    "pointerover",
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
                let related_target = related_target_value(scope, previous_handle);
                if let Some(event) = construct_pointer_event_with_related_target_and_modifiers(
                    scope,
                    "pointerenter",
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
            }
            if pointer_event_name == Some("pointermove")
                && let Some(event) = construct_pointer_event_with_modifiers(
                    scope,
                    "pointerrawupdate",
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                    modifiers,
                )
            {
                let _ = dispatch_public_event(scope, runtime_ptr, pointer_dispatch_handle, event);
                let post_raw_update_capture_events = {
                    let runtime = unsafe { &mut *runtime_ptr };
                    runtime.process_pending_pointer_capture(MOUSE_POINTER_ID)
                };
                dispatch_pointer_capture_events(
                    scope,
                    runtime_ptr,
                    MOUSE_POINTER_ID,
                    &post_raw_update_capture_events,
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                    modifiers,
                );
                if !post_raw_update_capture_events.is_empty() {
                    pointer_dispatch_handle = unsafe { &*runtime_ptr }
                        .active_pointer_capture_target(MOUSE_POINTER_ID)
                        .or(hit_handle)
                        .unwrap_or(pointer_dispatch_handle);
                }
            }
            if let Some(Some(previous_handle)) = hover_transition {
                let related_target = related_target_value(scope, Some(handle));
                if let Some(event) = construct_mouse_event_with_related_target_and_modifiers(
                    scope,
                    "mouseout",
                    x,
                    y,
                    button,
                    buttons,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
                }
                let related_target = related_target_value(scope, Some(handle));
                if let Some(event) = construct_mouse_event_with_related_target_and_modifiers(
                    scope,
                    "mouseleave",
                    x,
                    y,
                    button,
                    buttons,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
                }
            }
            if let Some(previous_handle) = hover_transition {
                let related_target = related_target_value(scope, previous_handle);
                if let Some(event) = construct_mouse_event_with_related_target_and_modifiers(
                    scope,
                    "mouseover",
                    x,
                    y,
                    button,
                    buttons,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
                let related_target = related_target_value(scope, previous_handle);
                if let Some(event) = construct_mouse_event_with_related_target_and_modifiers(
                    scope,
                    "mouseenter",
                    x,
                    y,
                    button,
                    buttons,
                    related_target,
                    modifiers,
                ) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
            }
            if let Some(pointer_event_name) = pointer_event_name_for_mouse_event(event_name)
                && let Some(event) = construct_pointer_event_with_modifiers(
                    scope,
                    pointer_event_name,
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                    modifiers,
                )
            {
                let dispatched =
                    dispatch_public_event(scope, runtime_ptr, pointer_dispatch_handle, event);
                if event_name == "mousedown" && !dispatched.allows_default() {
                    suppress_compat_mouse_events = true;
                }
            }
            if event_name == "mouseup" {
                release_pointer_capture_after_pointer_end(
                    scope,
                    runtime_ptr,
                    MOUSE_POINTER_ID,
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                    modifiers,
                );
            }
            let suppress_current_mouse_event =
                suppress_compat_mouse_events && can_suppress_compat_mouse_event(event_name);
            if !suppress_current_mouse_event {
                let event = if event_name == "wheel" {
                    construct_wheel_event(
                        scope, event_name, x, y, delta_x, delta_y, button, buttons, modifiers,
                    )
                } else {
                    construct_mouse_event_with_modifiers(
                        scope, event_name, x, y, button, buttons, modifiers,
                    )
                };
                if let Some(event) = event {
                    let dispatched =
                        dispatch_public_event(scope, runtime_ptr, pointer_dispatch_handle, event);
                    if event_name == "wheel" && dispatched.allows_default() {
                        let _ = perform_wheel_scroll_default_action(
                            scope,
                            runtime_ptr,
                            pointer_dispatch_handle,
                            delta_x,
                            delta_y,
                        )?;
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
            if let Some(drag_start_handle) = drag_start_handle
                && !suppress_current_mouse_event
            {
                // SAFETY: `with_default_context_scope` runs synchronously while `ScriptVm`
                // remains exclusively borrowed, so this field pointer is valid for the
                // duration of the callback.
                let active_drag_session = unsafe { &mut *active_drag_session };
                let empty_drag_data = RendererDragData {
                    items: Vec::new(),
                    files: Vec::new(),
                    directories: Vec::new(),
                    drag_operations_mask: 1,
                };
                if let Some(data_transfer) =
                    crate::context_bootstrap::build_data_transfer_object(scope, &empty_drag_data)
                    && let Some(event) =
                        construct_drag_event(scope, "dragstart", x, y, data_transfer.into(), 0)
                {
                    if dispatch_public_event(scope, runtime_ptr, drag_start_handle, event)
                        .allows_default()
                    {
                        *active_drag_session = Some(ActiveDragSession {
                            data_transfer: v8::Global::new(scope, data_transfer),
                            drop_allowed: false,
                        });
                    } else {
                        active_drag_session.take();
                    }
                }
            }
            if event_name == "mousemove" && buttons & 1 != 0 {
                let active_drag_session = unsafe { &mut *active_drag_session };
                if let Some(session) = active_drag_session.as_mut() {
                    let data_transfer = v8::Local::new(scope, &session.data_transfer);
                    if let Some(event) =
                        construct_drag_event(scope, "dragover", x, y, data_transfer.into(), 0)
                    {
                        session.drop_allowed =
                            !dispatch_public_event(scope, runtime_ptr, handle, event)
                                .allows_default();
                    } else {
                        session.drop_allowed = false;
                    }
                }
            }
            if event_name == "mouseup" && button == 0 {
                let active_drag_session = unsafe { &mut *active_drag_session };
                if let Some(session) = active_drag_session.as_mut() {
                    if session.drop_allowed {
                        let data_transfer = v8::Local::new(scope, &session.data_transfer);
                        let allows_default = if let Some(event) =
                            construct_drag_event(scope, "drop", x, y, data_transfer.into(), 0)
                        {
                            dispatch_public_event(scope, runtime_ptr, handle, event)
                                .allows_default()
                        } else {
                            false
                        };
                        if allows_default {
                            let _ = perform_drop_default_action(
                                scope,
                                runtime_ptr,
                                handle,
                                data_transfer,
                            );
                        }
                    }
                    active_drag_session.take();
                }
            }
            match follow_up {
                Some(MouseReleaseFollowUp::ActivateViaClick) => {
                    if event_name == "mouseup" {
                        suppress_compat_mouse_events = false;
                    }
                    let outcome = activate_handle_via_click_with_detail_and_modifiers(
                        scope,
                        runtime_ptr,
                        handle,
                        x,
                        y,
                        button,
                        buttons,
                        click_count.max(1),
                        modifiers,
                    );
                    if click_count >= 2
                        && let Some(event) = construct_mouse_event_with_detail_and_modifiers(
                            scope,
                            "dblclick",
                            x,
                            y,
                            click_count,
                            button,
                            buttons,
                            modifiers,
                        )
                    {
                        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                    }
                    return Ok(outcome);
                }
                Some(MouseReleaseFollowUp::DispatchEvent(follow_up_event_name)) => {
                    if event_name == "mouseup" {
                        suppress_compat_mouse_events = false;
                    }
                    if let Some(event) = construct_mouse_event_with_modifiers(
                        scope,
                        follow_up_event_name,
                        x,
                        y,
                        button,
                        buttons,
                        modifiers,
                    ) {
                        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                    }
                }
                None => {}
            }
            if event_name == "mouseup" {
                suppress_compat_mouse_events = false;
            }
            Ok(input_dispatch_outcome(true))
        });
        self.suppress_compat_mouse_events = suppress_compat_mouse_events;
        result
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
        )?;
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
            context_host.set_pointer_capture_active(TOUCH_POINTER_ID, true);
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
                    x,
                    y,
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
                    x,
                    y,
                    button,
                    buttons,
                    &pointer,
                )
            {
                let _ = dispatch_public_event(scope, runtime_ptr, pointer_handle, event);
            }
            if should_finish_touch {
                release_pointer_capture_after_pointer_end(
                    scope,
                    runtime_ptr,
                    TOUCH_POINTER_ID,
                    x,
                    y,
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
        if points.is_empty()
            && matches!(event_name, "touchend" | "touchcancel")
            && let Some(point) = self.active_touch_point
        {
            return self.dispatch_touch_event_at_point(point.x, point.y, event_name, activate);
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
            )?;
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
                    context_host.set_pointer_capture_active(pointer_id, true);
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
                        changed.point.x,
                        changed.point.y,
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
                        changed.point.x,
                        changed.point.y,
                        button,
                        buttons,
                        &pointer,
                    )
                {
                    let _ =
                        dispatch_public_event(scope, runtime_ptr, changed.pointer_handle, event);
                }
                if let Some(finish_pointer_handle) = changed.finish_pointer_handle {
                    release_pointer_capture_after_pointer_end(
                        scope,
                        runtime_ptr,
                        changed.pointer_id,
                        changed.point.x,
                        changed.point.y,
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
                let _ = dispatch_public_event(scope, runtime_ptr, event_target, event);
            }
            Ok(input_dispatch_outcome(true))
        });
        if is_end && self.active_touch_points.is_empty() {
            self.active_touch_pointer_handle = None;
            self.active_touch_pointer_handles.clear();
            self.active_touch_event_handle = None;
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
        let Some(handle) = observable_input_hit_test(
            &self._context_host.borrow(),
            self.document_runtime.document_handle(),
            moli_layout::LayoutPoint::new(x as f32, y as f32),
        )?
        else {
            return Ok(input_dispatch_outcome(false));
        };

        let incoming_drag_data_empty = data.items.is_empty() && data.files.is_empty();
        let replace_active_drag_data_transfer = event_name == "dragenter"
            && (!incoming_drag_data_empty || self.active_drag_session.is_none());
        let clear_active_drag_data_transfer = event_name == "drop";
        let active_drag_session: *mut Option<ActiveDragSession> = &mut self.active_drag_session;

        let result = self.with_default_context_scope(move |scope, runtime_ptr| {
            // SAFETY: `with_default_context_scope` runs synchronously while `ScriptVm`
            // remains exclusively borrowed, so the field pointer stays valid for the
            // duration of this callback and is not aliased elsewhere.
            let active_drag_session = unsafe { &mut *active_drag_session };
            let data_transfer =
                if replace_active_drag_data_transfer || active_drag_session.is_none() {
                    let Some(data_transfer) =
                        crate::context_bootstrap::build_data_transfer_object(scope, &data)
                    else {
                        return Ok(input_dispatch_outcome(false));
                    };
                    *active_drag_session = Some(ActiveDragSession {
                        data_transfer: v8::Global::new(scope, data_transfer),
                        drop_allowed: false,
                    });
                    data_transfer
                } else {
                    v8::Local::new(
                        scope,
                        &active_drag_session
                            .as_ref()
                            .expect("active drag session should exist when reusing DataTransfer")
                            .data_transfer,
                    )
                };
            crate::context_bootstrap::apply_drag_modifier_drop_effect(
                scope,
                data_transfer,
                modifiers,
            );
            let data_transfer_value: v8::Local<'_, v8::Value> = data_transfer.into();
            let allows_default = if let Some(event) =
                construct_drag_event(scope, event_name, x, y, data_transfer_value, modifiers)
            {
                dispatch_public_event(scope, runtime_ptr, handle, event).allows_default()
            } else {
                false
            };
            if event_name == "dragover"
                && let Some(session) = active_drag_session.as_mut()
            {
                session.drop_allowed = !allows_default;
            }
            if clear_active_drag_data_transfer && allows_default {
                let _ = perform_drop_default_action(scope, runtime_ptr, handle, data_transfer);
            }
            if clear_active_drag_data_transfer {
                active_drag_session.take();
            }
            Ok(input_dispatch_outcome(true))
        });
        self.finish_input_event_dispatch_turn(result)
    }

    pub(crate) fn clear_active_drag_data_transfer(&mut self) -> Result<()> {
        self.active_drag_session.take();
        Ok(())
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
            if !element.is_html_input() || element.input_type() != "file" {
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
                if let Some(event) = construct_simple_event(scope, "input", true, false, false) {
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
            if replace_text_control_selection(scope, runtime_ptr, handle, text) {
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

        let result = self.with_default_context_scope(|scope, runtime_ptr| {
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
                return Ok(input_dispatch_outcome(false));
            }

            let runtime = unsafe { &mut *runtime_ptr };
            let target = key_target_info(runtime, handle);

            if should_insert_text {
                if text.is_empty() {
                    return Ok(input_dispatch_outcome(true));
                }
                if target.is_text_control {
                    if target.is_textarea && text.contains('\r') {
                        let normalized_text = text.replace("\r\n", "\n").replace('\r', "\n");
                        return Ok(input_dispatch_outcome(replace_text_control_selection(
                            scope,
                            runtime_ptr,
                            handle,
                            &normalized_text,
                        )));
                    }
                    return Ok(input_dispatch_outcome(replace_text_control_selection(
                        scope,
                        runtime_ptr,
                        handle,
                        text,
                    )));
                }
                if let Some(editing_host) = contenteditable_editing_host(runtime, handle) {
                    return Ok(input_dispatch_outcome(replace_contenteditable_selection(
                        scope,
                        runtime_ptr,
                        editing_host,
                        text,
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
                        1,
                        modifiers,
                    ));
                }
                return Ok(input_dispatch_outcome(true));
            }

            if event_name != "keydown" {
                return Ok(input_dispatch_outcome(true));
            }

            if key_lower == "enter" {
                if target.is_textarea {
                    return Ok(input_dispatch_outcome(replace_text_control_selection(
                        scope,
                        runtime_ptr,
                        handle,
                        "\n",
                    )));
                }
                if target.is_button_like || target.is_anchor_like {
                    return Ok(activate_handle_via_click_with_detail_and_modifiers(
                        scope,
                        runtime_ptr,
                        handle,
                        0.0,
                        0.0,
                        0,
                        0,
                        1,
                        modifiers,
                    ));
                }
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
                    if let Some(event) = construct_simple_event(scope, "input", true, false, false)
                    {
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
                if let Some(event) = construct_simple_event(scope, "input", true, false, false) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
                if let Some(event) = construct_simple_event(scope, "change", true, false, false) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
                return Ok(input_dispatch_outcome(true));
            }

            if target.is_text_control && key_lower == "backspace" {
                let (start, end) = current_selection_range(runtime, handle);
                let (from, to) = if start != end {
                    (start, end)
                } else if start == 0 {
                    return Ok(input_dispatch_outcome(true));
                } else {
                    (start - 1, start)
                };
                let _ =
                    text_control_set_selection_range_internal(scope, runtime_ptr, handle, from, to);
                return Ok(input_dispatch_outcome(replace_text_control_selection(
                    scope,
                    runtime_ptr,
                    handle,
                    "",
                )));
            }

            if target.is_text_control && key_lower == "delete" {
                let value_len = text_control_value(runtime, handle).chars().count() as u32;
                let (start, end) = current_selection_range(runtime, handle);
                let (from, to) = if start != end {
                    (start, end)
                } else if start >= value_len {
                    return Ok(input_dispatch_outcome(true));
                } else {
                    (start, start + 1)
                };
                let _ =
                    text_control_set_selection_range_internal(scope, runtime_ptr, handle, from, to);
                return Ok(input_dispatch_outcome(replace_text_control_selection(
                    scope,
                    runtime_ptr,
                    handle,
                    "",
                )));
            }

            if target.is_text_control && (ctrl || meta) && key_lower == "a" {
                let value_len = text_control_value(runtime, handle).chars().count() as u32;
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
                )));
            }

            if target.is_text_control
                && matches!(
                    key_lower.as_str(),
                    "arrowleft" | "left" | "arrowright" | "right" | "home" | "end"
                )
            {
                let value_len = text_control_value(runtime, handle).chars().count() as u32;
                let (start, end, direction) = current_selection_state(runtime, handle);
                if shift {
                    let (anchor, focus) = match direction.as_str() {
                        "backward" => (end, start),
                        "forward" => (start, end),
                        _ => (end, end),
                    };
                    let next_focus = match key_lower.as_str() {
                        "arrowleft" | "left" => focus.saturating_sub(1),
                        "arrowright" | "right" => (focus + 1).min(value_len),
                        "home" => 0,
                        "end" => value_len,
                        _ => focus,
                    };
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
                            start.saturating_sub(1)
                        }
                    }
                    "arrowright" | "right" => {
                        if start != end {
                            end
                        } else {
                            (end + 1).min(value_len)
                        }
                    }
                    "home" => 0,
                    "end" => value_len,
                    _ => end,
                };
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
        self.finish_input_event_dispatch_turn(result)
    }
}
