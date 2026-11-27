use super::super::provider::{read_element_metrics, read_scroll_into_view_geometry};
use super::*;
use crate::native_bridge::WindowDocumentTaskTarget;
use std::{cell::Cell, rc::Rc};

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
    let requested = current + remaining;
    let target = requested.clamp(minimum, maximum);
    (target, requested - target)
}

/// Run the uncancelled default action for one pixel-mode WheelEvent.
///
/// Each axis starts at the innermost scroll container under the pointer. Any
/// delta left at that container's boundary continues along the layout ancestor
/// chain, matching the scroll chaining users expect from a trackpad or wheel.
pub(crate) fn perform_wheel_scroll_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    delta_x: f64,
    delta_y: f64,
) -> Result<bool, moli_layout::LayoutError> {
    if (!delta_x.is_finite() || delta_x == 0.0) && (!delta_y.is_finite() || delta_y == 0.0) {
        return Ok(false);
    }
    let chain = wheel_scroll_chain(unsafe { &*runtime_ptr }, handle)?;
    perform_wheel_scroll_chain_default_action(scope, runtime_ptr, &chain, delta_x, delta_y)
}

fn wheel_scroll_chain(
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> Result<Vec<moli_layout::LayoutScrollContainerMetrics<DomHandle>>, moli_layout::LayoutError> {
    let Some(target) = resolve_scroll_target(runtime, handle) else {
        return Ok(Vec::new());
    };
    let Some(mut geometry) = read_scroll_into_view_geometry(runtime, target)? else {
        return Ok(Vec::new());
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

    Ok(geometry.scroll_containers)
}

fn perform_wheel_scroll_chain_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    chain: &[moli_layout::LayoutScrollContainerMetrics<DomHandle>],
    delta_x: f64,
    delta_y: f64,
) -> Result<bool, moli_layout::LayoutError> {
    let mut remaining_x = if delta_x.is_finite() { delta_x } else { 0.0 };
    let mut remaining_y = if delta_y.is_finite() { delta_y } else { 0.0 };
    let mut changed = false;
    for container in chain {
        let source = container.source;
        let metrics = &container.metrics;
        if remaining_x == 0.0 && remaining_y == 0.0 {
            break;
        }
        let runtime = unsafe { &*runtime_ptr };
        if !runtime.dom_host().is_connected(source) {
            continue;
        }
        let (current_x, current_y) = node_scroll_position(runtime, source);
        let (target_x, next_remaining_x) = consume_wheel_axis(
            current_x,
            f64::from(metrics.minimum_scroll_offset.x),
            f64::from(metrics.maximum_scroll_offset.x),
            remaining_x,
            metrics.allows_user_scroll_x,
        );
        let (target_y, next_remaining_y) = consume_wheel_axis(
            current_y,
            f64::from(metrics.minimum_scroll_offset.y),
            f64::from(metrics.maximum_scroll_offset.y),
            remaining_y,
            metrics.allows_user_scroll_y,
        );
        remaining_x = next_remaining_x;
        remaining_y = next_remaining_y;
        if target_x == current_x && target_y == current_y {
            continue;
        }

        let container_document = unsafe { &*runtime_ptr }
            .dom_host()
            .owner_document_handle(source);
        let is_document_scroller = container_document.is_some_and(|document| {
            unsafe { &*runtime_ptr }
                .dom_host()
                .dom()
                .document_element_handle_for_document(document)
                == Some(source)
        });
        if is_document_scroller {
            changed |= apply_observable_window_scroll(
                scope,
                runtime_ptr,
                source,
                target_x,
                target_y,
                current_x,
                current_y,
            );
        } else {
            changed |=
                set_node_scroll_position(scope, runtime_ptr, source, target_x, target_y, true);
        }
    }
    Ok(changed)
}

/// The scroller selected for both axes of one wheel transaction.
/// Keep this selection at a boundary and after the scroller is removed.
#[derive(Clone, Debug)]
pub(crate) struct WheelScrollTarget(Option<LatchedScrollBox>);

#[derive(Clone, Debug)]
struct LatchedScrollBox {
    source: DomHandle,
    document: DomHandle,
    owner: WindowDocumentTaskTarget,
    alive: Rc<Cell<bool>>,
}

impl WheelScrollTarget {
    fn new(runtime: &PageScriptHost, source: Option<DomHandle>) -> Self {
        Self(source.and_then(|source| {
            let document = runtime.dom_host().owner_document_handle(source)?;
            let owner = runtime.input_owner_for_document(document)?;
            Some(LatchedScrollBox {
                source,
                document,
                owner,
                alive: runtime.watch_wheel_scroll_target(document, owner, source)?,
            })
        }))
    }
}

pub(crate) fn select_wheel_scroll_target(
    runtime: &PageScriptHost,
    handle: DomHandle,
    delta_x: f64,
    delta_y: f64,
) -> Result<WheelScrollTarget, moli_layout::LayoutError> {
    let can_scroll = |current, minimum: f32, maximum: f32, delta| {
        delta > 0.0 && current < f64::from(maximum) || delta < 0.0 && current > f64::from(minimum)
    };
    for container in wheel_scroll_chain(runtime, handle)? {
        let source = container.source;
        let metrics = container.metrics;
        let (x, y) = node_scroll_position(runtime, source);
        let can_scroll_x = metrics.allows_user_scroll_x
            && can_scroll(
                x,
                metrics.minimum_scroll_offset.x,
                metrics.maximum_scroll_offset.x,
                delta_x,
            );
        let can_scroll_y = metrics.allows_user_scroll_y
            && can_scroll(
                y,
                metrics.minimum_scroll_offset.y,
                metrics.maximum_scroll_offset.y,
                delta_y,
            );
        if can_scroll_x || can_scroll_y {
            return Ok(WheelScrollTarget::new(runtime, Some(source)));
        }
    }

    // The viewport remains a valid target at its scroll boundary.
    let host = runtime.dom_host();
    let viewport = host
        .owner_document_handle(handle)
        .and_then(|document| host.dom().document_element_handle_for_document(document));
    Ok(WheelScrollTarget::new(runtime, viewport))
}

pub(crate) fn perform_latched_wheel_scroll_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: &WheelScrollTarget,
    delta_x: f64,
    delta_y: f64,
) -> Result<bool, moli_layout::LayoutError> {
    let Some(target) = &target.0 else {
        return Ok(false);
    };
    if !target.alive.get() {
        return Ok(false);
    }
    let source = target.source;
    let runtime = unsafe { &*runtime_ptr };
    if runtime.input_document_for_owner(target.owner) != Some(target.document)
        || !runtime.dom_host().is_connected(source)
        || runtime.dom_host().owner_document_handle(source) != Some(target.document)
    {
        target.alive.set(false);
        return Ok(false);
    }
    let Some(metrics) = read_element_metrics(runtime, source)? else {
        target.alive.set(false);
        return Ok(false);
    };
    // Reading the metrics can publish layout and retire this scroll box.
    if !target.alive.get() {
        return Ok(false);
    }
    let is_viewport = runtime
        .dom_host()
        .dom()
        .document_element_handle_for_document(target.document)
        == Some(source);
    if !is_viewport && !metrics.is_scroll_container {
        target.alive.set(false);
        return Ok(false);
    }
    perform_wheel_scroll_chain_default_action(
        scope,
        runtime_ptr,
        &[moli_layout::LayoutScrollContainerMetrics { source, metrics }],
        delta_x,
        delta_y,
    )
}

#[cfg(test)]
#[test]
fn wheel_scroll_chains_only_when_the_inner_scroll_position_is_clamped() {
    let distance = 300.0 / 23.0;
    let (_, remaining) = consume_wheel_axis(40.0, 0.0, 100.0, distance, true);
    assert_eq!(remaining, 0.0);
    assert_eq!(
        consume_wheel_axis(95.0, 0.0, 100.0, 12.0, true),
        (100.0, 7.0)
    );
}
