use super::*;

fn hit_after_callbacks(
    host: &crate::native_bridge::PageScriptHost,
    x: f64,
    y: f64,
) -> Result<Option<crate::native_bridge::element::InputHit>> {
    // Coordinate input shares the published tree. Only a cold start may
    // build layout; callbacks do not change the snapshot's refresh lifecycle.
    Ok(observable_input_hit_test(
        host,
        host.document_handle(),
        moli_layout::LayoutPoint::new(x as f32, y as f32),
    )?)
}

fn transition(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    previous: Option<DomHandle>,
    current: Option<DomHandle>,
    position: InputPosition,
) {
    unsafe { &mut *runtime_ptr }.set_hovered_element_for_input(current);
    let coordinates = |target| position.for_target(unsafe { &*runtime_ptr }, target, None);
    dispatch_mouse_boundary_events(
        scope,
        runtime_ptr,
        previous,
        current,
        previous.map(coordinates).unwrap_or_default(),
        current.map(coordinates).unwrap_or_default(),
        0,
        0,
        0,
    );
}

pub(super) fn dispatch_touch_tap_activation(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    tap: PendingTouchTap,
    x: f64,
    y: f64,
    previous_hover: Option<DomHandle>,
) -> Result<(RendererInputDispatchOutcome, Option<DomHandle>)> {
    let position = InputPosition::headless(x, y);
    let coordinates = |hit: crate::native_bridge::element::InputHit| {
        let host = unsafe { &*runtime_ptr };
        if host.layout_policy().uses_real_layout() {
            position.for_target(host, hit.handle, None)
        } else {
            position.coordinates(Some(hit.root_to_frame))
        }
    };
    let mut hover =
        previous_hover.filter(|node| unsafe { &*runtime_ptr }.dom_host().is_connected(*node));
    if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
        return Ok((input_dispatch_outcome(true), hover));
    }
    let initial = hit_after_callbacks(unsafe { &*runtime_ptr }, x, y)?;
    let initial_hover = initial.map(|hit| hit.handle);
    transition(scope, runtime_ptr, hover, initial_hover, position);
    hover = initial_hover;
    let Some(mut hit) = initial else {
        return Ok((input_dispatch_outcome(true), None));
    };
    if is_disabled_form_control(unsafe { &*runtime_ptr }, hit.handle) {
        return Ok((input_dispatch_outcome(true), Some(hit.handle)));
    }
    let pressed;
    if !tap.suppress_mouse {
        if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
            return Ok((input_dispatch_outcome(true), Some(hit.handle)));
        }
        if let Some(event) = construct_mouse_event_with_detail_for_target(
            scope,
            runtime_ptr,
            hit.handle,
            "mousemove",
            coordinates(hit),
            0,
            0,
            0,
            0,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, hit.handle, event);
        }
        if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
            return Ok((input_dispatch_outcome(true), None));
        }
        let next = hit_after_callbacks(unsafe { &*runtime_ptr }, x, y)?;
        transition(
            scope,
            runtime_ptr,
            hover,
            next.map(|hit| hit.handle),
            position,
        );
        hover = next.map(|hit| hit.handle);
        let Some(next) = next else {
            return Ok((input_dispatch_outcome(true), None));
        };
        hit = next;
        if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap)
            || is_disabled_form_control(unsafe { &*runtime_ptr }, hit.handle)
        {
            return Ok((input_dispatch_outcome(true), Some(hit.handle)));
        }
        pressed = hit.handle;
        if let Some(event) = construct_mouse_event_with_detail_for_target(
            scope,
            runtime_ptr,
            hit.handle,
            "mousedown",
            coordinates(hit),
            1,
            0,
            1,
            0,
        ) && dispatch_public_event(scope, runtime_ptr, hit.handle, event).allows_default()
        {
            if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
                return Ok((input_dispatch_outcome(true), None));
            }
            perform_mouse_focus_default_action(scope, runtime_ptr, hit.handle);
        }
        if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
            return Ok((input_dispatch_outcome(true), None));
        }
        let next = hit_after_callbacks(unsafe { &*runtime_ptr }, x, y)?;
        transition(
            scope,
            runtime_ptr,
            hover,
            next.map(|hit| hit.handle),
            position,
        );
        hover = next.map(|hit| hit.handle);
        let Some(next) = next else {
            return Ok((input_dispatch_outcome(true), None));
        };
        hit = next;
        if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
            return Ok((input_dispatch_outcome(true), Some(hit.handle)));
        }
        if let Some(event) = construct_mouse_event_with_detail_for_target(
            scope,
            runtime_ptr,
            hit.handle,
            "mouseup",
            coordinates(hit),
            1,
            0,
            0,
            0,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, hit.handle, event);
        }
    } else {
        pressed = hit.handle;
    }
    let host = unsafe { &*runtime_ptr };
    if !touch_tap_is_live(host, tap)
        || !host.dom_host().is_connected(pressed)
        || host.dom_host().owner_document_handle(pressed) != Some(tap.document)
        || host.dom_host().owner_document_handle(hit.handle) != Some(tap.document)
    {
        return Ok((input_dispatch_outcome(true), hover));
    }
    let Some(target) =
        crate::native_bridge::common_flat_tree_ancestor(host.dom_host(), pressed, hit.handle)
    else {
        return Ok((input_dispatch_outcome(true), hover));
    };
    let outcome = activate_handle_after_touch_release(
        scope,
        runtime_ptr,
        target,
        coordinates(hit),
        touch_pointer_id(tap.id),
    );
    Ok((outcome, hover))
}
