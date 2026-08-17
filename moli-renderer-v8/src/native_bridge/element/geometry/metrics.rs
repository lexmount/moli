use crate::{
    document_runtime::DomHandle, dom::native::Node, native_bridge::JsContextHost, util::v8str,
};
use moli_page_types::DomScrollIntoViewRect;

use super::super::super::{
    document,
    node::{node_runtime_and_handle_from_object, node_runtime_and_handle_from_object_or_detached},
};
use super::super::styles::raw_inline_style_property_value;
use super::super::{queue_revealed_lazy_image_loads, queue_revealed_lazy_media_loads};
use super::{
    ClientRect, compute_mock_client_rect, observable_element_metrics,
    observable_scroll_into_view_geometry,
};

fn node_scroll_position_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    horizontal: bool,
) -> Result<f64, moli_layout::LayoutError> {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) {
        let metrics = observable_element_metrics(
            unsafe { &*runtime_ptr },
            handle,
            moli_layout::LayoutFlushReason::SynchronousGeometry,
        )?;
        return Ok(metrics
            .map(|metrics| {
                if horizontal {
                    f64::from(metrics.scroll_offset.x)
                } else {
                    f64::from(metrics.scroll_offset.y)
                }
            })
            .unwrap_or(0.0));
    }
    Ok(0.0)
}

fn node_scroll_position_setter_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    horizontal: bool,
) -> Result<(), moli_layout::LayoutError> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return Ok(());
    };
    let value = value
        .number_value(scope)
        .filter(|value| value.is_finite())
        .unwrap_or(0.0);
    let receiver_is_detached =
        document::detached_native_handle_for_runtime(scope, runtime_ptr, object).is_some();
    if receiver_is_detached {
        return Ok(());
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let (minimum, maximum) = if runtime.layout_policy().uses_real_layout() {
        observable_element_metrics(
            runtime,
            handle,
            moli_layout::LayoutFlushReason::SynchronousGeometry,
        )?
        .map(|metrics| {
            if horizontal {
                (
                    f64::from(metrics.minimum_scroll_offset.x),
                    f64::from(metrics.maximum_scroll_offset.x),
                )
            } else {
                (
                    f64::from(metrics.minimum_scroll_offset.y),
                    f64::from(metrics.maximum_scroll_offset.y),
                )
            }
        })
        .unwrap_or((0.0, 0.0))
    } else {
        // Mock intentionally preserves the old synthetic geometry behavior:
        // non-negative scroll values are stored even without real overflow.
        (0.0, f64::MAX)
    };
    let value = value.clamp(minimum, maximum);
    let is_scrolling_element = runtime.dom_host().document_element_handle() == Some(handle);
    let document = runtime.dom_host().owner_document_handle(handle);
    let Some(element) = runtime
        .dom_host_mut()
        .node_mut(handle)
        .and_then(|node| node.data_mut().as_element_mut())
    else {
        return Ok(());
    };
    let changed = if horizontal {
        element.set_scroll_left(value)
    } else {
        element.set_scroll_top(value)
    };
    if changed {
        if is_scrolling_element {
            let current = crate::window_host::current_window_scroll_position(scope);
            crate::window_host::scroll_window_to(
                scope,
                runtime_ptr,
                if horizontal { value } else { current.0 },
                if horizontal { current.1 } else { value },
            );
        } else {
            queue_scroll_observable_effects(scope, runtime_ptr, document, false);
        }
    }
    Ok(())
}

fn parse_scroll_coordinates(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    fallback_x: f64,
    fallback_y: f64,
) -> (f64, f64) {
    if args.length() > 0
        && args.get(0).is_object()
        && !args.get(0).is_function()
        && let Some(options) = args.get(0).to_object(scope)
    {
        let x = options
            .get(scope, v8str(scope, "left").into())
            .or_else(|| options.get(scope, v8str(scope, "x").into()))
            .and_then(|value| value.number_value(scope).filter(|value| !value.is_nan()))
            .unwrap_or(fallback_x);
        let y = options
            .get(scope, v8str(scope, "top").into())
            .or_else(|| options.get(scope, v8str(scope, "y").into()))
            .and_then(|value| value.number_value(scope).filter(|value| !value.is_nan()))
            .unwrap_or(fallback_y);
        return (x, y);
    }

    let x = args
        .get(0)
        .number_value(scope)
        .filter(|value| !value.is_nan())
        .unwrap_or(fallback_x);
    let y = args
        .get(1)
        .number_value(scope)
        .filter(|value| !value.is_nan())
        .unwrap_or(fallback_y);
    (x, y)
}

fn set_node_scroll_position(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    left: f64,
    top: f64,
    queue_observable_effects: bool,
) {
    let (changed, document) = {
        let runtime = unsafe { &mut *runtime_ptr };
        let document = runtime.dom_host().owner_document_handle(handle);
        let Some(element) = runtime
            .dom_host_mut()
            .node_mut(handle)
            .and_then(|node| node.data_mut().as_element_mut())
        else {
            return;
        };
        (
            element.set_scroll_left(left) | element.set_scroll_top(top),
            document,
        )
    };
    if changed && queue_observable_effects {
        queue_scroll_observable_effects(scope, runtime_ptr, document, false);
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

fn scroll_axis_to_expose(
    target_start: f64,
    target_end: f64,
    current_scroll: f64,
    viewport_extent: f64,
) -> f64 {
    let viewport_start = current_scroll;
    let viewport_end = current_scroll + viewport_extent;
    let target_contains_viewport = target_start <= viewport_start && target_end >= viewport_end;
    let target_is_fully_visible = target_start >= viewport_start && target_end <= viewport_end;
    if target_contains_viewport || target_is_fully_visible {
        return current_scroll;
    }

    let partially_visible = target_end > viewport_start && target_start < viewport_end;
    if partially_visible {
        return if target_start < viewport_start {
            target_start
        } else {
            target_end - viewport_extent
        };
    }

    (target_start + target_end - viewport_extent) / 2.0
}

#[derive(Clone, Copy)]
enum ScrollIntoViewAlignment {
    Start,
    Center,
    End,
    Nearest,
}

fn aligned_scroll_position(
    target_start: f64,
    target_end: f64,
    current_scroll: f64,
    viewport_extent: f64,
    alignment: ScrollIntoViewAlignment,
) -> f64 {
    match alignment {
        ScrollIntoViewAlignment::Start => target_start,
        ScrollIntoViewAlignment::Center => (target_start + target_end - viewport_extent) / 2.0,
        ScrollIntoViewAlignment::End => target_end - viewport_extent,
        ScrollIntoViewAlignment::Nearest => {
            scroll_axis_to_expose(target_start, target_end, current_scroll, viewport_extent)
        }
    }
}

fn apply_observable_window_scroll(
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
    let target = if runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_document)
    {
        let Some(root) = runtime.dom_host().document_element_handle() else {
            return Ok(false);
        };
        root
    } else if runtime.dom_host().is_connected(handle) {
        handle
    } else {
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

pub(crate) fn scroll_node_into_view_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    relative_rect: Option<DomScrollIntoViewRect>,
) -> Result<Option<bool>, moli_layout::LayoutError> {
    scroll_node_into_view_with_geometry(
        scope,
        runtime_ptr,
        handle,
        relative_rect,
        ScrollIntoViewAlignment::Nearest,
        ScrollIntoViewAlignment::Nearest,
        true,
        0,
    )
}

fn scroll_node_into_view(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    horizontal: ScrollIntoViewAlignment,
    vertical: ScrollIntoViewAlignment,
) -> Result<Option<bool>, moli_layout::LayoutError> {
    scroll_node_into_view_with_geometry(
        scope,
        runtime_ptr,
        handle,
        None,
        horizontal,
        vertical,
        false,
        0,
    )
}

pub(crate) fn scroll_node_into_view_at_start(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Result<Option<bool>, moli_layout::LayoutError> {
    scroll_node_into_view(
        scope,
        runtime_ptr,
        handle,
        ScrollIntoViewAlignment::Nearest,
        ScrollIntoViewAlignment::Start,
    )
}

fn scroll_node_into_view_with_geometry(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    relative_rect: Option<DomScrollIntoViewRect>,
    horizontal: ScrollIntoViewAlignment,
    vertical: ScrollIntoViewAlignment,
    center_if_fully_hidden: bool,
    frame_depth: usize,
) -> Result<Option<bool>, moli_layout::LayoutError> {
    let runtime = unsafe { &*runtime_ptr };
    let target = if runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_document)
    {
        let Some(root) = runtime.dom_host().document_element_handle() else {
            return Ok(None);
        };
        root
    } else if runtime.dom_host().is_connected(handle) && runtime.dom_host().node(handle).is_some() {
        // CDP also accepts connected rendered Text nodes. Whether a concrete
        // node owns fragments is a layout-output decision; this DOM boundary
        // only rejects detached or stale handles.
        handle
    } else {
        return Ok(None);
    };
    let Some(mut geometry) = observable_scroll_into_view_geometry(
        runtime,
        target,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )?
    else {
        return Ok(None);
    };
    if geometry.target_rects.is_empty() {
        return Ok(None);
    }
    if let Some(relative) = relative_rect {
        let Some(bounds) = quads_bounding_rect(&geometry.target_rects) else {
            return Ok(None);
        };
        geometry.target_rects =
            vec![
                moli_layout::LayoutTransform2D::IDENTITY.map_rect(moli_layout::LayoutRect::new(
                    bounds.x + relative.x() as f32,
                    bounds.y + relative.y() as f32,
                    relative.width().max(0.0) as f32,
                    relative.height().max(0.0) as f32,
                )),
            ];
    }

    let target_document = runtime.dom_host().owner_document_handle(target);
    let mut changed = false;
    for container in geometry.scroll_containers {
        let metrics = &container.metrics;
        let Some(target_bounds) = target_bounds_in_scroll_content(&geometry.target_rects, metrics)
        else {
            continue;
        };
        let desired_x = if center_if_fully_hidden {
            scroll_axis_to_expose(
                f64::from(target_bounds.x),
                f64::from(target_bounds.right()),
                f64::from(metrics.scroll_offset.x),
                f64::from(metrics.client_size.width),
            )
        } else {
            aligned_scroll_position(
                f64::from(target_bounds.x),
                f64::from(target_bounds.right()),
                f64::from(metrics.scroll_offset.x),
                f64::from(metrics.client_size.width),
                horizontal,
            )
        };
        let desired_y = if center_if_fully_hidden {
            scroll_axis_to_expose(
                f64::from(target_bounds.y),
                f64::from(target_bounds.bottom()),
                f64::from(metrics.scroll_offset.y),
                f64::from(metrics.client_size.height),
            )
        } else {
            aligned_scroll_position(
                f64::from(target_bounds.y),
                f64::from(target_bounds.bottom()),
                f64::from(metrics.scroll_offset.y),
                f64::from(metrics.client_size.height),
                vertical,
            )
        };
        let target_x = desired_x.clamp(
            f64::from(metrics.minimum_scroll_offset.x),
            f64::from(metrics.maximum_scroll_offset.x),
        );
        let target_y = desired_y.clamp(
            f64::from(metrics.minimum_scroll_offset.y),
            f64::from(metrics.maximum_scroll_offset.y),
        );
        let delta_x = target_x - f64::from(metrics.scroll_offset.x);
        let delta_y = target_y - f64::from(metrics.scroll_offset.y);
        if delta_x == 0.0 && delta_y == 0.0 {
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
                f64::from(metrics.scroll_offset.x),
                f64::from(metrics.scroll_offset.y),
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
        translate_quads_for_scroll(
            &mut geometry.target_rects,
            metrics.scrollport,
            metrics.client_size,
            delta_x,
            delta_y,
        );
    }
    if frame_depth < 16
        && let Some(frame) = target_document.and_then(|document| {
            let runtime = unsafe { &*runtime_ptr };
            (document != runtime.document_handle())
                .then(|| runtime.child_browsing_context_host_for_document_handle(document))
                .flatten()
        })
        && let Some(parent_changed) = scroll_node_into_view_with_geometry(
            scope,
            runtime_ptr,
            frame,
            None,
            ScrollIntoViewAlignment::Nearest,
            ScrollIntoViewAlignment::Nearest,
            center_if_fully_hidden,
            frame_depth + 1,
        )?
    {
        changed |= parent_changed;
    }
    Ok(Some(changed))
}

fn quads_bounding_rect(quads: &[moli_layout::LayoutQuad]) -> Option<moli_layout::LayoutRect> {
    quads
        .iter()
        .map(|quad| quad.bounding_rect())
        .reduce(moli_layout::LayoutRect::union)
}

fn target_bounds_in_scroll_content(
    target_quads: &[moli_layout::LayoutQuad],
    metrics: &moli_layout::LayoutElementMetrics<DomHandle>,
) -> Option<moli_layout::LayoutRect> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in target_quads.iter().flat_map(|quad| quad.points) {
        let local =
            map_viewport_point_to_scrollport(point, metrics.scrollport, metrics.client_size)?;
        let x = local.0 + f64::from(metrics.scroll_offset.x);
        let y = local.1 + f64::from(metrics.scroll_offset.y);
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    [min_x, min_y, max_x, max_y]
        .into_iter()
        .all(f64::is_finite)
        .then(|| {
            moli_layout::LayoutRect::new(
                min_x as f32,
                min_y as f32,
                (max_x - min_x).max(0.0) as f32,
                (max_y - min_y).max(0.0) as f32,
            )
        })
}

fn map_viewport_point_to_scrollport(
    point: moli_layout::LayoutPoint,
    scrollport: moli_layout::LayoutQuad,
    size: moli_layout::LayoutSize,
) -> Option<(f64, f64)> {
    let [origin, x_corner, _, y_corner] = scrollport.points;
    let x_basis = (
        f64::from(x_corner.x - origin.x),
        f64::from(x_corner.y - origin.y),
    );
    let y_basis = (
        f64::from(y_corner.x - origin.x),
        f64::from(y_corner.y - origin.y),
    );
    let relative = (f64::from(point.x - origin.x), f64::from(point.y - origin.y));
    let determinant = x_basis.0 * y_basis.1 - x_basis.1 * y_basis.0;
    if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
        return None;
    }
    let u = (relative.0 * y_basis.1 - relative.1 * y_basis.0) / determinant;
    let v = (x_basis.0 * relative.1 - x_basis.1 * relative.0) / determinant;
    Some((u * f64::from(size.width), v * f64::from(size.height)))
}

fn translate_quads_for_scroll(
    quads: &mut [moli_layout::LayoutQuad],
    scrollport: moli_layout::LayoutQuad,
    size: moli_layout::LayoutSize,
    delta_x: f64,
    delta_y: f64,
) {
    if size.width <= 0.0 || size.height <= 0.0 {
        return;
    }
    let [origin, x_corner, _, y_corner] = scrollport.points;
    let shift_x = f64::from(x_corner.x - origin.x) * delta_x / f64::from(size.width)
        + f64::from(y_corner.x - origin.x) * delta_y / f64::from(size.height);
    let shift_y = f64::from(x_corner.y - origin.y) * delta_x / f64::from(size.width)
        + f64::from(y_corner.y - origin.y) * delta_y / f64::from(size.height);
    for point in quads.iter_mut().flat_map(|quad| quad.points.iter_mut()) {
        point.x -= shift_x as f32;
        point.y -= shift_y as f32;
    }
}

fn scroll_alignment_option(
    scope: &mut v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Object>,
    name: &'static str,
    fallback: ScrollIntoViewAlignment,
) -> ScrollIntoViewAlignment {
    let Some(value) = options.get(scope, v8str(scope, name).into()) else {
        return fallback;
    };
    match value.to_rust_string_lossy(scope).as_str() {
        "start" => ScrollIntoViewAlignment::Start,
        "center" => ScrollIntoViewAlignment::Center,
        "end" => ScrollIntoViewAlignment::End,
        "nearest" => ScrollIntoViewAlignment::Nearest,
        _ => fallback,
    }
}

fn element_scroll_into_view_alignments(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> (ScrollIntoViewAlignment, ScrollIntoViewAlignment) {
    if args.length() == 0 {
        return (
            ScrollIntoViewAlignment::Nearest,
            ScrollIntoViewAlignment::Start,
        );
    }
    let value = args.get(0);
    if value.is_boolean() {
        return if value.boolean_value(scope) {
            (
                ScrollIntoViewAlignment::Nearest,
                ScrollIntoViewAlignment::Start,
            )
        } else {
            (
                ScrollIntoViewAlignment::Nearest,
                ScrollIntoViewAlignment::End,
            )
        };
    }
    let Some(options) = value.to_object(scope) else {
        return (
            ScrollIntoViewAlignment::Nearest,
            ScrollIntoViewAlignment::Start,
        );
    };
    (
        scroll_alignment_option(scope, options, "inline", ScrollIntoViewAlignment::Nearest),
        scroll_alignment_option(scope, options, "block", ScrollIntoViewAlignment::Start),
    )
}

fn node_scroll_position(runtime: &JsContextHost, handle: DomHandle) -> (f64, f64) {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| (element.scroll_left(), element.scroll_top()))
        .unwrap_or((0.0, 0.0))
}

fn scroll_node_to<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    relative: bool,
) -> Result<(), moli_layout::LayoutError> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return Ok(());
    };
    let receiver_is_detached =
        document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some();
    if receiver_is_detached {
        return Ok(());
    }
    let (current_left, current_top) = node_scroll_position(unsafe { &*runtime_ptr }, handle);
    let (left, top) = if relative {
        let (delta_left, delta_top) = parse_scroll_coordinates(scope, &args, 0.0, 0.0);
        (current_left + delta_left, current_top + delta_top)
    } else {
        parse_scroll_coordinates(scope, &args, current_left, current_top)
    };
    let metrics = observable_element_metrics(
        unsafe { &*runtime_ptr },
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )?;
    let Some(metrics) = metrics else {
        return Ok(());
    };
    let left = left.clamp(
        f64::from(metrics.minimum_scroll_offset.x),
        f64::from(metrics.maximum_scroll_offset.x),
    );
    let top = top.clamp(
        f64::from(metrics.minimum_scroll_offset.y),
        f64::from(metrics.maximum_scroll_offset.y),
    );
    if unsafe { &*runtime_ptr }
        .dom_host()
        .document_element_handle()
        == Some(handle)
    {
        let _ = apply_observable_window_scroll(
            scope,
            runtime_ptr,
            handle,
            left,
            top,
            current_left,
            current_top,
        );
    } else {
        set_node_scroll_position(scope, runtime_ptr, handle, left, top, true);
    }
    Ok(())
}

fn node_box_metric_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    metric: &str,
) -> Result<i32, moli_layout::LayoutError> {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) {
        let metrics = observable_element_metrics(
            unsafe { &*runtime_ptr },
            handle,
            moli_layout::LayoutFlushReason::SynchronousGeometry,
        )?;
        return Ok(metrics
            .as_ref()
            .map(|metrics| layout_box_metric(metrics, metric))
            .unwrap_or(0));
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return Ok(0);
    };
    let value = legacy_rect_metric(
        compute_mock_client_rect(unsafe { &*runtime_ptr }, handle),
        metric,
    );
    if value == 0 {
        Ok(
            detached_child_document_box_metric(scope, unsafe { &mut *runtime_ptr }, handle, metric)
                .unwrap_or(value),
        )
    } else {
        Ok(value)
    }
}

fn layout_box_metric(metrics: &moli_layout::LayoutElementMetrics<DomHandle>, metric: &str) -> i32 {
    rounded_layout_value(match metric {
        "clientWidth" => metrics.client_size.width,
        "clientHeight" => metrics.client_size.height,
        "clientTop" => metrics.client_border.y,
        "clientLeft" => metrics.client_border.x,
        "scrollWidth" => metrics.scroll_size.width,
        "scrollHeight" => metrics.scroll_size.height,
        "offsetWidth" => metrics.offset_size.width,
        "offsetHeight" => metrics.offset_size.height,
        "offsetTop" => metrics.offset_position.y,
        "offsetLeft" => metrics.offset_position.x,
        _ => 0.0,
    })
}

fn legacy_rect_metric(rect: ClientRect, metric: &str) -> i32 {
    rounded_layout_value(match metric {
        "clientWidth" | "scrollWidth" | "offsetWidth" => rect.width as f32,
        "clientHeight" | "scrollHeight" | "offsetHeight" => rect.height as f32,
        "offsetTop" => rect.top as f32,
        "offsetLeft" => rect.left as f32,
        _ => 0.0,
    })
}

fn rounded_layout_value(value: f32) -> i32 {
    value.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

fn set_box_metric_return_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    metric: &str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match node_box_metric_from_object(scope, object, metric) {
        Ok(value) => rv.set(v8::Integer::new(scope, value).into()),
        Err(error) => {
            let message = format!("Layout failed while reading {metric}: {error}");
            if let Some(message) = crate::util::v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            rv.set(v8::Integer::new(scope, 0).into());
        }
    }
}

fn detached_child_document_box_metric(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &mut JsContextHost,
    handle: DomHandle,
    metric: &str,
) -> Option<i32> {
    let document = runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::owner_document)?;
    runtime.child_browsing_context_handle_by_document_handle(scope, document)?;
    if detached_metric_element_has_zero_mock_box(runtime, handle) {
        return Some(0);
    }
    Some(match metric {
        "clientWidth" | "scrollWidth" | "offsetWidth" => 100,
        "clientHeight" | "scrollHeight" | "offsetHeight" => 20,
        "offsetTop" => 20,
        "offsetLeft" => 0,
        _ => 0,
    })
}

fn detached_metric_element_has_zero_mock_box(runtime: &JsContextHost, handle: DomHandle) -> bool {
    matches!(
        raw_inline_style_property_value(runtime, handle, "display")
            .unwrap_or_default()
            .as_str(),
        "none" | "contents"
    ) || light_child_suppressed_by_shadow_host(runtime, handle)
}

fn light_child_suppressed_by_shadow_host(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .parent_node(handle)
        .and_then(|parent| runtime.dom_host().shadow_root_handle(parent))
        .is_some_and(|root| {
            runtime
                .dom_host()
                .shadow_root_slot_assignment(root)
                .as_deref()
                != Some("manual")
        })
}

pub(in crate::native_bridge) fn node_client_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "clientWidth", rv);
}

pub(in crate::native_bridge) fn node_client_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "clientHeight", rv);
}

pub(in crate::native_bridge) fn node_client_top_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "clientTop", rv);
}

pub(in crate::native_bridge) fn node_client_left_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "clientLeft", rv);
}

pub(in crate::native_bridge) fn node_scroll_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "scrollWidth", rv);
}

pub(in crate::native_bridge) fn node_scroll_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "scrollHeight", rv);
}

pub(in crate::native_bridge) fn node_scroll_top_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match node_scroll_position_value(scope, args.this(), false) {
        Ok(value) => rv.set(v8::Number::new(scope, value).into()),
        Err(error) => {
            let message = format!("Layout failed while reading scrollTop: {error}");
            if let Some(message) = crate::util::v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            rv.set(v8::Number::new(scope, 0.0).into());
        }
    }
}

pub(in crate::native_bridge) fn node_scroll_top_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Err(error) =
        node_scroll_position_setter_for_object(scope, args.this(), args.get(0), false)
    {
        let message = format!("Layout failed while setting scrollTop: {error}");
        if let Some(message) = crate::util::v8_string(scope, &message) {
            let exception = v8::Exception::error(scope, message);
            scope.throw_exception(exception);
        }
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_scroll_left_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match node_scroll_position_value(scope, args.this(), true) {
        Ok(value) => rv.set(v8::Number::new(scope, value).into()),
        Err(error) => {
            let message = format!("Layout failed while reading scrollLeft: {error}");
            if let Some(message) = crate::util::v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            rv.set(v8::Number::new(scope, 0.0).into());
        }
    }
}

pub(in crate::native_bridge) fn node_scroll_left_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Err(error) =
        node_scroll_position_setter_for_object(scope, args.this(), args.get(0), true)
    {
        let message = format!("Layout failed while setting scrollLeft: {error}");
        if let Some(message) = crate::util::v8_string(scope, &message) {
            let exception = v8::Exception::error(scope, message);
            scope.throw_exception(exception);
        }
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_scroll_to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Err(error) = scroll_node_to(scope, args, false) {
        throw_scroll_layout_error(scope, "scrollTo", error);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_scroll_by_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Err(error) = scroll_node_to(scope, args, true) {
        throw_scroll_layout_error(scope, "scrollBy", error);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_offset_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "offsetWidth", rv);
}

pub(in crate::native_bridge) fn node_offset_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "offsetHeight", rv);
}

pub(in crate::native_bridge) fn node_offset_parent_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let metrics = match observable_element_metrics(
        runtime,
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    ) {
        Ok(metrics) => metrics,
        Err(error) => {
            let message = format!("Layout failed while reading offsetParent: {error}");
            if let Some(message) = crate::util::v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            rv.set_null();
            return;
        }
    };
    let Some(parent) = metrics.and_then(|metrics| metrics.offset_parent) else {
        rv.set_null();
        return;
    };
    match runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, parent)
    {
        Some(parent) => rv.set(parent.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn node_offset_top_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "offsetTop", rv);
}

pub(in crate::native_bridge) fn node_offset_left_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "offsetLeft", rv);
}

pub(in crate::native_bridge) fn node_scroll_into_view_if_needed_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this())
        && let Err(error) = scroll_node_into_view_if_needed(scope, runtime_ptr, handle, None)
    {
        throw_scroll_layout_error(scope, "scrollIntoViewIfNeeded", error);
    }
    reveal_lazy_images_for_scroll(scope, args.this());
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_scroll_into_view_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this()) {
        let (horizontal, vertical) = element_scroll_into_view_alignments(scope, &args);
        if let Err(error) = scroll_node_into_view(scope, runtime_ptr, handle, horizontal, vertical)
        {
            throw_scroll_layout_error(scope, "scrollIntoView", error);
        }
    }
    reveal_lazy_images_for_scroll(scope, args.this());
    rv.set_undefined();
}

fn throw_scroll_layout_error(
    scope: &mut v8::PinScope<'_, '_>,
    operation: &str,
    error: moli_layout::LayoutError,
) {
    let message = format!("Layout failed while running {operation}: {error}");
    if let Some(message) = crate::util::v8_string(scope, &message) {
        let exception = v8::Exception::error(scope, message);
        scope.throw_exception(exception);
    }
}

fn reveal_lazy_images_for_scroll(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) else {
        return;
    };
    let document = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(handle);
    if let Some(document) = document {
        queue_revealed_lazy_image_loads(scope, runtime_ptr, document);
    }
    queue_revealed_lazy_media_loads(scope, runtime_ptr);
}

#[cfg(test)]
mod scroll_alignment_tests {
    use super::scroll_axis_to_expose;

    #[test]
    fn center_if_needed_returns_the_unclamped_chromium_alignment_position() {
        let viewport = 100.0;
        assert_eq!(scroll_axis_to_expose(20.0, 40.0, 0.0, viewport), 0.0);
        assert_eq!(
            scroll_axis_to_expose(-20.0, 120.0, 0.0, viewport),
            0.0,
            "a target containing the viewport stays put"
        );
        assert_eq!(scroll_axis_to_expose(-10.0, 20.0, 0.0, viewport), -10.0);
        assert_eq!(scroll_axis_to_expose(90.0, 120.0, 0.0, viewport), 20.0);
        assert_eq!(scroll_axis_to_expose(200.0, 220.0, 0.0, viewport), 160.0);
        assert_eq!(scroll_axis_to_expose(0.0, 20.0, 200.0, viewport), -40.0);
    }
}
