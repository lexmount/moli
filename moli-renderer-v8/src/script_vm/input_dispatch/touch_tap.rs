use super::*;

fn update_tap_hover(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    previous: Option<DomHandle>,
    position: InputPosition,
) -> Result<(Option<DomHandle>, Option<Rc<PublishedFrameGeometry>>)> {
    let (current, fallback_sample) = {
        let host = unsafe { &*runtime_ptr };
        let hit = observable_input_hit_test(host, host.document_handle(), position.layout_point())?;
        // Keep the projection available if a boundary listener clears the publication.
        (hit, host.published_frame_geometry())
    };
    unsafe { &mut *runtime_ptr }.set_hovered_element_for_input(current);
    let coordinates =
        |target| position.for_target(unsafe { &*runtime_ptr }, target, fallback_sample.as_deref());
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
    Ok((current, fallback_sample))
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
    let coordinates = |target, sample: Option<&PublishedFrameGeometry>| {
        position.for_target(unsafe { &*runtime_ptr }, target, sample)
    };
    let hover =
        previous_hover.filter(|node| unsafe { &*runtime_ptr }.dom_host().is_connected(*node));
    if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
        return Ok((input_dispatch_outcome(true), hover));
    }
    let (mut hover, mut fallback_sample) = update_tap_hover(scope, runtime_ptr, hover, position)?;
    let Some(mut hit) = hover else {
        return Ok((input_dispatch_outcome(true), None));
    };
    if is_disabled_form_control(unsafe { &*runtime_ptr }, hit) {
        return Ok((input_dispatch_outcome(true), hover));
    }
    let pressed;
    if !tap.suppress_mouse {
        if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
            return Ok((input_dispatch_outcome(true), hover));
        }
        if let Some(event) = construct_mouse_event_with_detail_for_target(
            scope,
            runtime_ptr,
            hit,
            "mousemove",
            coordinates(hit, fallback_sample.as_deref()),
            0,
            0,
            0,
            0,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, hit, event);
        }
        if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
            return Ok((input_dispatch_outcome(true), None));
        }
        (hover, fallback_sample) = update_tap_hover(scope, runtime_ptr, hover, position)?;
        let Some(next) = hover else {
            return Ok((input_dispatch_outcome(true), None));
        };
        hit = next;
        if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap)
            || is_disabled_form_control(unsafe { &*runtime_ptr }, hit)
        {
            return Ok((input_dispatch_outcome(true), hover));
        }
        pressed = hit;
        if let Some(event) = construct_mouse_event_with_detail_for_target(
            scope,
            runtime_ptr,
            hit,
            "mousedown",
            coordinates(hit, fallback_sample.as_deref()),
            1,
            0,
            1,
            0,
        ) && dispatch_public_event(scope, runtime_ptr, hit, event).allows_default()
        {
            if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
                return Ok((input_dispatch_outcome(true), None));
            }
            perform_mouse_focus_default_action(scope, runtime_ptr, hit);
        }
        if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
            return Ok((input_dispatch_outcome(true), None));
        }
        (hover, fallback_sample) = update_tap_hover(scope, runtime_ptr, hover, position)?;
        let Some(next) = hover else {
            return Ok((input_dispatch_outcome(true), None));
        };
        hit = next;
        if !touch_tap_is_live(unsafe { &*runtime_ptr }, tap) {
            return Ok((input_dispatch_outcome(true), hover));
        }
        if let Some(event) = construct_mouse_event_with_detail_for_target(
            scope,
            runtime_ptr,
            hit,
            "mouseup",
            coordinates(hit, fallback_sample.as_deref()),
            1,
            0,
            0,
            0,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, hit, event);
        }
    } else {
        pressed = hit;
    }
    let host = unsafe { &*runtime_ptr };
    if !touch_tap_is_live(host, tap)
        || !host.dom_host().is_connected(pressed)
        || host.dom_host().owner_document_handle(pressed) != Some(tap.document)
        || host.dom_host().owner_document_handle(hit) != Some(tap.document)
    {
        return Ok((input_dispatch_outcome(true), hover));
    }
    let Some(target) =
        crate::native_bridge::common_flat_tree_ancestor(host.dom_host(), pressed, hit)
    else {
        return Ok((input_dispatch_outcome(true), hover));
    };
    let outcome = activate_handle_after_touch_release(
        scope,
        runtime_ptr,
        target,
        coordinates(hit, fallback_sample.as_deref()),
        touch_pointer_id(tap.id),
    );
    Ok((outcome, hover))
}
