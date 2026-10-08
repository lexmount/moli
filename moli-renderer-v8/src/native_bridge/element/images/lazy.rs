//! Sampled lazy-image request admission.
//!
//! Lazy loading is a resource decision, not observable geometry. It must not
//! force layout or retain a layout world. The renderer instead inspects its
//! one latest owned [`FrozenLayoutTree`] and combines that sampled geometry
//! with live scroll offsets. A later fresh screenshot or screencast frame
//! naturally replaces the sample.

use moli_layout::{FrozenLayoutTree, LayoutPosition, LayoutRect};

use crate::{document_runtime::DomHandle, dom::native::Node, native_bridge::JsContextHost};

use super::src::image_selected_request_key;

// Chromium's fast-network defaults are 1250 CSS px vertically. Its lazy-load
// IntersectionObserver uses half that distance horizontally. Moli has
// no effective-connection-type model, so the smallest Chromium default keeps
// request admission bounded while preserving browser-shaped preloading.
const LAZY_IMAGE_VERTICAL_MARGIN_PX: f32 = 1_250.0;
const LAZY_IMAGE_HORIZONTAL_MARGIN_PX: f32 = LAZY_IMAGE_VERTICAL_MARGIN_PX / 2.0;

pub(super) fn image_load_is_deferred(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let Some(node) = runtime.dom_host().node(handle) else {
        return false;
    };
    let Some(element) = node.as_element() else {
        return false;
    };
    let is_lazy_image = element.is_html_element("img")
        && element
            .attribute("loading")
            .is_some_and(|loading| loading.eq_ignore_ascii_case("lazy"));
    if !is_lazy_image {
        return false;
    }
    if !node.is_connected() {
        return true;
    }

    // A pending exact request proves that this source was already admitted.
    // Keep terminal error events deliverable as well as successful decodes;
    // readiness alone cannot represent a failed admitted request.
    if runtime.pending_image_load_event(handle).is_some() {
        return false;
    }
    if image_selected_request_key(runtime, handle)
        .is_some_and(|request_key| runtime.has_ready_image_request(&request_key))
    {
        return false;
    }
    true
}

pub(super) fn revealed_lazy_image_handles(
    runtime: &JsContextHost,
    document: DomHandle,
    output: &FrozenLayoutTree<DomHandle>,
) -> Vec<DomHandle> {
    (0..runtime.dom_host().dom().len())
        .map(DomHandle::new)
        .filter(|handle| is_unadmitted_lazy_image(runtime, document, *handle))
        .filter(|handle| image_is_near_live_viewport(runtime, document, output, *handle))
        .collect()
}

fn is_unadmitted_lazy_image(
    runtime: &JsContextHost,
    document: DomHandle,
    handle: DomHandle,
) -> bool {
    runtime.dom_host().node(handle).is_some_and(|node| {
        node.is_connected()
            && runtime.dom_host().owner_document_handle(handle) == Some(document)
            && node.as_element().is_some_and(|element| {
                element.is_html_element("img")
                    && element
                        .attribute("loading")
                        .is_some_and(|loading| loading.eq_ignore_ascii_case("lazy"))
                    && !element.image_load_dispatched()
            })
            && runtime.pending_image_load_event(handle).is_none()
    })
}

fn image_is_near_live_viewport(
    runtime: &JsContextHost,
    document: DomHandle,
    output: &FrozenLayoutTree<DomHandle>,
    handle: DomHandle,
) -> bool {
    let Some(geometry) = output.intersection_geometry(handle, None) else {
        return false;
    };
    if !geometry.target_has_layout || !geometry.target_visible {
        return false;
    }

    let target_is_fixed = output
        .source_output(handle)
        .and_then(|source| source.principal_box)
        .and_then(|box_id| output.box_geometry(box_id))
        .is_some_and(|geometry| geometry.position == LayoutPosition::Fixed);
    let (translate_x, translate_y) = if target_is_fixed {
        (0.0, 0.0)
    } else {
        live_scroll_translation(runtime, output, handle)
    };
    let viewport = runtime.layout_viewport_for_document(document);
    let admission_rect = LayoutRect::new(
        -LAZY_IMAGE_HORIZONTAL_MARGIN_PX,
        -LAZY_IMAGE_VERTICAL_MARGIN_PX,
        viewport.css_width as f32 + 2.0 * LAZY_IMAGE_HORIZONTAL_MARGIN_PX,
        viewport.css_height as f32 + 2.0 * LAZY_IMAGE_VERTICAL_MARGIN_PX,
    );
    geometry.target_rects.into_iter().any(|quad| {
        let mut rect = quad.bounding_rect();
        rect.x += translate_x;
        rect.y += translate_y;
        rects_intersect_or_touch(rect, admission_rect)
    })
}

fn live_scroll_translation(
    runtime: &JsContextHost,
    output: &FrozenLayoutTree<DomHandle>,
    handle: DomHandle,
) -> (f32, f32) {
    let Some(geometry) = output.scroll_into_view_geometry_for_source(handle) else {
        return (0.0, 0.0);
    };
    geometry
        .scroll_containers
        .iter()
        .filter_map(|container| {
            let live = runtime
                .dom_host()
                .node(container.source)
                .and_then(Node::as_element)?;
            Some((
                container.metrics.scroll_offset.x - live.scroll_left() as f32,
                container.metrics.scroll_offset.y - live.scroll_top() as f32,
            ))
        })
        .fold((0.0, 0.0), |(x, y), (dx, dy)| (x + dx, y + dy))
}

fn rects_intersect_or_touch(left: LayoutRect, right: LayoutRect) -> bool {
    left.width >= 0.0
        && left.height >= 0.0
        && right.width >= 0.0
        && right.height >= 0.0
        && left.x <= right.right()
        && left.right() >= right.x
        && left.y <= right.bottom()
        && left.bottom() >= right.y
}

#[cfg(test)]
mod tests {
    use super::rects_intersect_or_touch;
    use moli_layout::LayoutRect;

    #[test]
    fn lazy_admission_rect_includes_touching_edges_but_rejects_separation() {
        let viewport = LayoutRect::new(0.0, 0.0, 800.0, 600.0);
        assert!(rects_intersect_or_touch(
            LayoutRect::new(800.0, 100.0, 20.0, 20.0),
            viewport,
        ));
        assert!(!rects_intersect_or_touch(
            LayoutRect::new(800.5, 100.0, 20.0, 20.0),
            viewport,
        ));
    }
}
