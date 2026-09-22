use crate::{document_runtime::DomHandle, dom::native::Node, native_bridge::JsContextHost};

use super::super::{queue_revealed_lazy_image_loads, queue_revealed_lazy_media_loads};
use super::provider::{observable_element_metrics, observable_scroll_into_view_geometry};

pub(super) fn node_scroll_position(runtime: &JsContextHost, handle: DomHandle) -> (f64, f64) {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| (element.scroll_left(), element.scroll_top()))
        .unwrap_or((0.0, 0.0))
}

pub(super) fn resolve_scroll_target(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let node = runtime.dom_host().node(handle)?;
    if node.is_document() {
        return runtime.dom_host().document_element_handle();
    }
    runtime.dom_host().is_connected(handle).then_some(handle)
}

pub(super) fn set_node_scroll_position(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    left: f64,
    top: f64,
    queue_observable_effects: bool,
) -> bool {
    let (changed, document) = {
        let runtime = unsafe { &mut *runtime_ptr };
        let document = runtime.dom_host().owner_document_handle(handle);
        let Some(element) = runtime
            .dom_host_mut()
            .node_mut(handle)
            .and_then(|node| node.data_mut().as_element_mut())
        else {
            return false;
        };
        (
            element.set_scroll_left(left) | element.set_scroll_top(top),
            document,
        )
    };
    if changed && queue_observable_effects {
        queue_scroll_observable_effects(scope, runtime_ptr, document, false);
    }
    changed
}

pub(super) fn apply_observable_window_scroll(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    scrolling_element: DomHandle,
    target_x: f64,
    target_y: f64,
    current_x: f64,
    current_y: f64,
) -> bool {
    let changed = target_x != current_x || target_y != current_y;
    if !changed {
        return false;
    }
    set_node_scroll_position(
        scope,
        runtime_ptr,
        scrolling_element,
        target_x,
        target_y,
        false,
    );
    let endpoint = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(scrolling_element)
        .and_then(|document| unsafe { &*runtime_ptr }.window_endpoint_for_document(document));
    if let Some(endpoint) = endpoint {
        unsafe { &mut *runtime_ptr }.scroll_window_endpoint_to(scope, endpoint, target_x, target_y);
    }
    true
}

pub(crate) fn perform_scrollbar_scroll_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
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

pub(crate) fn queue_scroll_observable_effects(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document: Option<DomHandle>,
    queue_document_events: bool,
) {
    if unsafe { &mut *runtime_ptr }.defer_scroll_observable_effects(document, queue_document_events)
    {
        return;
    }
    apply_scroll_observable_effects(
        scope,
        runtime_ptr,
        [crate::native_bridge::PendingScrollObservableEffects::new(
            document,
            queue_document_events,
        )],
    );
}

pub(crate) fn apply_scroll_observable_effects(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    effects: impl IntoIterator<Item = crate::native_bridge::PendingScrollObservableEffects>,
) {
    let effects = effects.into_iter().collect::<Vec<_>>();
    if effects.is_empty() {
        return;
    }
    // Native lazy loading deliberately combines the retained pre-scroll
    // projection with live element offsets, so admit those requests before
    // retiring the sampled projection. Observable geometry and author
    // IntersectionObservers, on the other hand, must see a fresh projection.
    for effects in &effects {
        if let Some(document) = effects.document() {
            queue_revealed_lazy_image_loads(scope, runtime_ptr, document);
        }
    }
    queue_revealed_lazy_media_loads(scope, runtime_ptr);
    unsafe { &*runtime_ptr }.invalidate_layout_after_interaction_state_change();
    crate::observer_runtime::queue_intersection_checks(scope, runtime_ptr);
    if effects
        .iter()
        .any(|effects| effects.queue_document_events())
    {
        let _ = unsafe { &mut *runtime_ptr }.queue_document_scroll_events(scope);
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
/// chain, matching the scroll chaining users expect from a trackpad or wheel.
pub(crate) fn perform_wheel_scroll_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    delta_x: f64,
    delta_y: f64,
) -> Result<bool, moli_layout::LayoutError> {
    let mut remaining_x = if delta_x.is_finite() { delta_x } else { 0.0 };
    let mut remaining_y = if delta_y.is_finite() { delta_y } else { 0.0 };
    if remaining_x == 0.0 && remaining_y == 0.0 {
        return Ok(false);
    }

    let runtime = unsafe { &*runtime_ptr };
    let Some(target) = resolve_scroll_target(runtime, handle) else {
        return Ok(false);
    };
    let Some(mut geometry) = observable_scroll_into_view_geometry(
        runtime,
        target,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )?
    else {
        return Ok(false);
    };

    // ScrollIntoView geometry begins at the target's parent. Include the
    // target itself so an empty overflow scroller still responds when its own
    // background is the hit-test result.
    if let Some(metrics) = observable_element_metrics(
        runtime,
        target,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )? && metrics.is_scroll_container
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
        if remaining_x == 0.0 && remaining_y == 0.0 {
            break;
        }
        let (current_x, current_y) =
            node_scroll_position(unsafe { &*runtime_ptr }, container.source);
        let (target_x, next_remaining_x) = consume_wheel_axis(
            current_x,
            f64::from(container.metrics.minimum_scroll_offset.x),
            f64::from(container.metrics.maximum_scroll_offset.x),
            remaining_x,
            container.metrics.allows_user_scroll_x,
        );
        let (target_y, next_remaining_y) = consume_wheel_axis(
            current_y,
            f64::from(container.metrics.minimum_scroll_offset.y),
            f64::from(container.metrics.maximum_scroll_offset.y),
            remaining_y,
            container.metrics.allows_user_scroll_y,
        );
        remaining_x = next_remaining_x;
        remaining_y = next_remaining_y;
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
