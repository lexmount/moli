use super::super::super::styles::ComputedStyleRead;
use super::super::provider::{read_element_metrics, read_scroll_into_view_geometry};
use super::*;

pub(crate) fn perform_scrollbar_scroll_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    source: DomHandle,
    axis: moli_layout::LayoutScrollbarAxis,
    target: f64,
) -> bool {
    let (current_x, current_y) = node_scroll_position(unsafe { &*runtime_ptr }, source);
    let (target_x, target_y) = match axis {
        moli_layout::LayoutScrollbarAxis::Horizontal => (target, current_y),
        moli_layout::LayoutScrollbarAxis::Vertical => (current_x, target),
    };
    let document = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(source);
    let is_document_scroller = document.is_some_and(|document| {
        unsafe { &*runtime_ptr }
            .dom_host()
            .dom()
            .document_element_handle_for_document(document)
            == Some(source)
    });
    if is_document_scroller {
        apply_observable_window_scroll(
            scope,
            runtime_ptr,
            source,
            target_x,
            target_y,
            current_x,
            current_y,
        )
    } else {
        set_node_scroll_position(scope, runtime_ptr, source, target_x, target_y, true)
    }
}

fn consume_wheel_axis(
    current: f64,
    minimum: f64,
    maximum: f64,
    remaining: f64,
    allows_user_scroll: bool,
) -> (f64, f64) {
    if !allows_user_scroll || remaining == 0.0 {
        return (current, remaining);
    }
    let target = (current + remaining).clamp(minimum, maximum);
    (target, remaining - (target - current))
}

/// Run the uncancelled default action for one pixel-mode WheelEvent.
///
/// Each axis starts at the innermost scroll container under the pointer. Any
/// delta left at that container's boundary continues along the layout ancestor
/// chain. Axes the document cannot consume continue through embedding frames,
/// without redispatching the event.
pub(crate) fn perform_wheel_scroll_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    delta_x: f64,
    delta_y: f64,
) -> Result<bool, moli_layout::LayoutError> {
    let mut remaining_x = if delta_x.is_finite() { delta_x } else { 0.0 };
    let mut remaining_y = if delta_y.is_finite() { delta_y } else { 0.0 };
    if remaining_x == 0.0 && remaining_y == 0.0 {
        return Ok(false);
    }

    let mut current = Some(handle);
    let mut changed = false;
    for _ in 0..=super::super::hit_test::CHILD_FRAME_DEPTH_LIMIT {
        let Some(handle) = current else {
            break;
        };
        let runtime = unsafe { &*runtime_ptr };
        let Some(document) = runtime.dom_host().owner_document_handle(handle) else {
            break;
        };
        let target = if handle == document {
            runtime
                .dom_host()
                .dom()
                .document_element_handle_for_document(document)
        } else {
            resolve_scroll_target(runtime, handle)
        };
        if let Some(target) = target {
            let requested_x = remaining_x;
            let requested_y = remaining_y;
            changed |= scroll_wheel_in_document(
                scope,
                runtime_ptr,
                target,
                &mut remaining_x,
                &mut remaining_y,
            )?;
            // Once this document scrolls an axis, keep the current wheel
            // action there even if it reaches the boundary partway through.
            if remaining_x != requested_x {
                remaining_x = 0.0;
            }
            if remaining_y != requested_y {
                remaining_y = 0.0;
            }
        }
        if remaining_x == 0.0 && remaining_y == 0.0 {
            break;
        }
        // The frame's owner starts the next document's scroll chain. Keep
        // the original event in the child and transfer only unused deltas.
        current =
            unsafe { &*runtime_ptr }.child_browsing_context_host_for_document_handle(document);
    }
    Ok(changed)
}

fn scroll_wheel_in_document(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    remaining_x: &mut f64,
    remaining_y: &mut f64,
) -> Result<bool, moli_layout::LayoutError> {
    let runtime = unsafe { &*runtime_ptr };
    let Some(mut geometry) = read_scroll_into_view_geometry(runtime, target)? else {
        return Ok(false);
    };

    // ScrollIntoView geometry begins at the target's parent. Include the
    // target itself so an empty overflow scroller still responds when its own
    // background is the hit-test result.
    if let Some(metrics) = read_element_metrics(runtime, target)?
        && metrics.is_scroll_container
        && geometry
            .scroll_containers
            .iter()
            .all(|container| container.source != target)
    {
        geometry.scroll_containers.insert(
            0,
            moli_layout::LayoutScrollContainerMetrics {
                source: target,
                metrics,
            },
        );
    }

    let mut changed = false;
    for container in geometry.scroll_containers {
        if *remaining_x == 0.0 && *remaining_y == 0.0 {
            break;
        }
        let (current_x, current_y) =
            node_scroll_position(unsafe { &*runtime_ptr }, container.source);
        let (target_x, next_remaining_x) = consume_wheel_axis(
            current_x,
            f64::from(container.metrics.minimum_scroll_offset.x),
            f64::from(container.metrics.maximum_scroll_offset.x),
            *remaining_x,
            container.metrics.allows_user_scroll_x,
        );
        let (target_y, next_remaining_y) = consume_wheel_axis(
            current_y,
            f64::from(container.metrics.minimum_scroll_offset.y),
            f64::from(container.metrics.maximum_scroll_offset.y),
            *remaining_y,
            container.metrics.allows_user_scroll_y,
        );
        *remaining_x = next_remaining_x;
        *remaining_y = next_remaining_y;
        if *remaining_x != 0.0 || *remaining_y != 0.0 {
            let style = ComputedStyleRead::new(unsafe { &*runtime_ptr }, container.source);
            if *remaining_x != 0.0
                && matches!(
                    style.property("overscroll-behavior-x").trim(),
                    "contain" | "none"
                )
            {
                *remaining_x = 0.0;
            }
            if *remaining_y != 0.0
                && matches!(
                    style.property("overscroll-behavior-y").trim(),
                    "contain" | "none"
                )
            {
                *remaining_y = 0.0;
            }
        }
        if target_x == current_x && target_y == current_y {
            continue;
        }

        let container_document = unsafe { &*runtime_ptr }
            .dom_host()
            .owner_document_handle(container.source);
        let is_document_scroller = container_document.is_some_and(|document| {
            unsafe { &*runtime_ptr }
                .dom_host()
                .dom()
                .document_element_handle_for_document(document)
                == Some(container.source)
        });
        if is_document_scroller {
            changed |= apply_observable_window_scroll(
                scope,
                runtime_ptr,
                container.source,
                target_x,
                target_y,
                current_x,
                current_y,
            );
        } else {
            set_node_scroll_position(
                scope,
                runtime_ptr,
                container.source,
                target_x,
                target_y,
                true,
            );
            changed = true;
        }
    }
    Ok(changed)
}
