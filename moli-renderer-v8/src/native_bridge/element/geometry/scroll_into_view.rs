use moli_page_types::DomScrollIntoViewRect;

use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

use super::provider::{observable_box_model, observable_scroll_into_view_geometry};
use super::scroll::{
    apply_observable_window_scroll, resolve_scroll_target, set_node_scroll_position,
};

const CHILD_FRAME_DEPTH_LIMIT: usize = 16;

#[derive(Clone, Copy)]
pub(super) enum ScrollIntoViewAlignment {
    Start,
    Center,
    End,
    Nearest,
}

#[derive(Clone, Copy)]
struct ScrollIntoViewParams {
    horizontal: ScrollIntoViewAlignment,
    vertical: ScrollIntoViewAlignment,
    center_if_fully_hidden: bool,
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

pub(crate) fn scroll_node_into_view_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    relative_rect: Option<DomScrollIntoViewRect>,
) -> Result<Option<bool>, moli_layout::LayoutError> {
    scroll_node_into_view_with_params(
        scope,
        runtime_ptr,
        handle,
        relative_rect,
        ScrollIntoViewParams {
            horizontal: ScrollIntoViewAlignment::Nearest,
            vertical: ScrollIntoViewAlignment::Nearest,
            center_if_fully_hidden: true,
        },
    )
}

pub(super) fn scroll_node_into_view(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    relative_rect: Option<DomScrollIntoViewRect>,
    horizontal: ScrollIntoViewAlignment,
    vertical: ScrollIntoViewAlignment,
) -> Result<Option<bool>, moli_layout::LayoutError> {
    scroll_node_into_view_with_params(
        scope,
        runtime_ptr,
        handle,
        relative_rect,
        ScrollIntoViewParams {
            horizontal,
            vertical,
            center_if_fully_hidden: false,
        },
    )
}

fn scroll_node_into_view_with_params(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    relative_rect: Option<DomScrollIntoViewRect>,
    params: ScrollIntoViewParams,
) -> Result<Option<bool>, moli_layout::LayoutError> {
    let runtime = unsafe { &*runtime_ptr };
    let Some(target) = resolve_scroll_target(runtime, handle) else {
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
    if geometry.target_rects.is_empty() {
        return Ok(None);
    }
    perform_bubbling_scroll_into_view(scope, runtime_ptr, target, geometry, params, 0).map(Some)
}

pub(crate) fn scroll_node_into_view_at_start(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Result<Option<bool>, moli_layout::LayoutError> {
    scroll_node_into_view_with_params(
        scope,
        runtime_ptr,
        handle,
        None,
        ScrollIntoViewParams {
            horizontal: ScrollIntoViewAlignment::Nearest,
            vertical: ScrollIntoViewAlignment::Start,
            center_if_fully_hidden: false,
        },
    )
}

/// Blink's `PerformBubblingScrollIntoView` follows the same shape: scroll the
/// local containers, convert the resulting target geometry into the parent
/// frame, then continue with the frame owner.
fn perform_bubbling_scroll_into_view(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target: DomHandle,
    mut geometry: moli_layout::LayoutScrollIntoViewGeometry<DomHandle>,
    params: ScrollIntoViewParams,
    frame_depth: usize,
) -> Result<bool, moli_layout::LayoutError> {
    let target_document = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(target);
    let mut changed = false;
    for container in geometry.scroll_containers {
        let metrics = &container.metrics;
        let Some(target_bounds) = target_bounds_in_scroll_content(&geometry.target_rects, metrics)
        else {
            continue;
        };
        let desired_x = if params.center_if_fully_hidden {
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
                params.horizontal,
            )
        };
        let desired_y = if params.center_if_fully_hidden {
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
                params.vertical,
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
    if frame_depth >= CHILD_FRAME_DEPTH_LIMIT {
        return Ok(changed);
    }
    let Some(document) = target_document else {
        return Ok(changed);
    };
    let runtime = unsafe { &*runtime_ptr };
    if document == runtime.document_handle() {
        return Ok(changed);
    }
    let Some(frame) = runtime.child_browsing_context_host_for_document_handle(document) else {
        return Ok(changed);
    };
    let Some(frame_content) = observable_box_model(
        runtime,
        frame,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )?
    .map(|model| model.content) else {
        return Ok(changed);
    };
    let Some(parent_target_rects) = convert_quads_to_parent_frame(
        &geometry.target_rects,
        runtime.layout_viewport_for_document(document),
        frame_content,
    ) else {
        return Ok(changed);
    };
    let Some(mut parent_geometry) = observable_scroll_into_view_geometry(
        runtime,
        frame,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )?
    else {
        return Ok(changed);
    };
    parent_geometry.target_rects = parent_target_rects;
    let parent_params = ScrollIntoViewParams {
        horizontal: ScrollIntoViewAlignment::Nearest,
        vertical: ScrollIntoViewAlignment::Nearest,
        ..params
    };
    changed |= perform_bubbling_scroll_into_view(
        scope,
        runtime_ptr,
        frame,
        parent_geometry,
        parent_params,
        frame_depth + 1,
    )?;
    Ok(changed)
}

/// Convert a rectangle carried by a child frame into the parent document's
/// viewport coordinates. This mirrors Blink's cross-frame scroll bubbling:
/// the target rectangle is converted through the frame owner's content box,
/// rather than replacing it with the bounds of the whole frame.
fn convert_quads_to_parent_frame(
    child_quads: &[moli_layout::LayoutQuad],
    child_viewport: moli_layout::LayoutViewport,
    parent_content: moli_layout::LayoutQuad,
) -> Option<Vec<moli_layout::LayoutQuad>> {
    let width = child_viewport.css_width as f32;
    let height = child_viewport.css_height as f32;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let [origin, x_corner, _, y_corner] = parent_content.points;
    let x_basis = moli_layout::LayoutPoint::new(x_corner.x - origin.x, x_corner.y - origin.y);
    let y_basis = moli_layout::LayoutPoint::new(y_corner.x - origin.x, y_corner.y - origin.y);
    Some(
        child_quads
            .iter()
            .map(|quad| moli_layout::LayoutQuad {
                points: quad.points.map(|point| {
                    let u = point.x / width;
                    let v = point.y / height;
                    moli_layout::LayoutPoint::new(
                        origin.x + x_basis.x * u + y_basis.x * v,
                        origin.y + x_basis.y * u + y_basis.y * v,
                    )
                }),
            })
            .collect(),
    )
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

#[cfg(test)]
mod tests {
    use super::{convert_quads_to_parent_frame, scroll_axis_to_expose};

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

    #[test]
    fn child_viewport_rect_maps_through_the_parent_content_quad() {
        let child_quad = moli_layout::LayoutTransform2D::IDENTITY
            .map_rect(moli_layout::LayoutRect::new(50.0, 25.0, 100.0, 50.0));
        let parent_content = moli_layout::LayoutQuad {
            points: [
                moli_layout::LayoutPoint::new(10.0, 20.0),
                moli_layout::LayoutPoint::new(110.0, 40.0),
                moli_layout::LayoutPoint::new(90.0, 100.0),
                moli_layout::LayoutPoint::new(-10.0, 80.0),
            ],
        };

        let mapped = convert_quads_to_parent_frame(
            &[child_quad],
            moli_layout::LayoutViewport::new(200, 100, 1.0),
            parent_content,
        )
        .expect("non-empty child viewport should map");

        assert_eq!(
            mapped[0].points,
            [
                moli_layout::LayoutPoint::new(30.0, 40.0),
                moli_layout::LayoutPoint::new(80.0, 50.0),
                moli_layout::LayoutPoint::new(70.0, 80.0),
                moli_layout::LayoutPoint::new(20.0, 70.0),
            ]
        );
    }

    #[test]
    fn empty_child_viewport_cannot_produce_parent_geometry() {
        let child_quad = moli_layout::LayoutTransform2D::IDENTITY
            .map_rect(moli_layout::LayoutRect::new(0.0, 0.0, 10.0, 10.0));
        let parent_content = moli_layout::LayoutTransform2D::IDENTITY
            .map_rect(moli_layout::LayoutRect::new(0.0, 0.0, 10.0, 10.0));

        assert!(
            convert_quads_to_parent_frame(
                &[child_quad],
                moli_layout::LayoutViewport::new(0, 100, 1.0),
                parent_content,
            )
            .is_none()
        );
    }
}
