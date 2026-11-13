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

fn coordinates(
    hit: crate::native_bridge::element::InputHit,
    x: f64,
    y: f64,
) -> InputEventCoordinates {
    let point = hit
        .root_to_frame
        .map_point(moli_layout::LayoutPoint::new(x as f32, y as f32));
    InputEventCoordinates::from_root(x, y).with_client(f64::from(point.x), f64::from(point.y))
}

fn transition(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    previous: Option<crate::native_bridge::element::InputHit>,
    current: Option<crate::native_bridge::element::InputHit>,
    x: f64,
    y: f64,
) {
    unsafe { &mut *runtime_ptr }.set_hovered_element_for_input(current.map(|hit| hit.handle));
    dispatch_mouse_boundary_events(
        scope,
        runtime_ptr,
        previous.map(|hit| hit.handle),
        current.map(|hit| hit.handle),
        previous.map_or(InputEventCoordinates::from_root(x, y), |hit| {
            coordinates(hit, x, y)
        }),
        current.map_or(InputEventCoordinates::from_root(x, y), |hit| {
            coordinates(hit, x, y)
        }),
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
    let mut hover = previous_hover
        .filter(|node| unsafe { &*runtime_ptr }.dom_host().is_connected(*node))
        .and_then(|handle| {
            let host = unsafe { &*runtime_ptr };
            let document = host.dom_host().owner_document_handle(handle)?;
            Some(crate::native_bridge::element::InputHit {
                handle,
                root_to_frame: root_to_document_frame(host, document)?,
            })
        });
    if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
        return Ok((input_dispatch_outcome(true), hover.map(|hit| hit.handle)));
    }
    let initial = hit_after_callbacks(unsafe { &*runtime_ptr }, x, y)?;
    transition(scope, runtime_ptr, hover, initial, x, y);
    hover = initial;
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
            coordinates(hit, x, y),
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
        transition(scope, runtime_ptr, hover, next, x, y);
        hover = next;
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
            coordinates(hit, x, y),
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
        transition(scope, runtime_ptr, hover, next, x, y);
        hover = next;
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
            coordinates(hit, x, y),
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
        return Ok((input_dispatch_outcome(true), hover.map(|hit| hit.handle)));
    }
    let Some(target) =
        crate::native_bridge::common_flat_tree_ancestor(host.dom_host(), pressed, hit.handle)
    else {
        return Ok((input_dispatch_outcome(true), hover.map(|hit| hit.handle)));
    };
    let outcome = activate_handle_after_touch_release(
        scope,
        runtime_ptr,
        target,
        coordinates(hit, x, y),
        touch_pointer_id(tap.id),
    );
    Ok((outcome, hover.map(|hit| hit.handle)))
}
