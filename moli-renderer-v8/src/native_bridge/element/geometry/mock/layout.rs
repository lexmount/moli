use crate::document_runtime::DomHandle;
use moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE;
use std::time::Instant;

use super::super::super::super::JsContextHost;
use super::super::super::styles::{ComputedStyleRead, iframe_handle_viewport};
use super::super::client_rect::{ClientRect, zero_client_rect};
use crate::native_bridge::element::raw_inline_style_property_value;

const MOCK_FLOW_STEP_PX: f64 = 24.0;
const CHILD_DOCUMENT_VISIBLE_FLOW_ORIGIN_PX: f64 = 1.0;
const MOCK_FLOW_COUNT_LIMIT: usize = 4096;
const HIT_TEST_CHILD_FRAME_DEPTH_LIMIT: usize = 16;

fn viewport_client_rect(runtime: &JsContextHost, handle: DomHandle) -> ClientRect {
    let child_viewport = runtime
        .dom_host()
        .owner_document_handle(handle)
        .filter(|document| *document != runtime.document_handle())
        .and_then(|document| runtime.child_browsing_context_host_for_document_handle(document))
        .and_then(|frame_handle| iframe_handle_viewport(runtime, frame_handle));
    let width = child_viewport
        .and_then(|viewport| viewport.width)
        .unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.inner_width);
    let height = child_viewport
        .and_then(|viewport| viewport.height)
        .unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.inner_height);
    ClientRect {
        left: 0.0,
        top: 0.0,
        right: width,
        bottom: height,
        width,
        height,
    }
}

fn default_element_client_rect() -> ClientRect {
    ClientRect {
        left: 0.0,
        top: 0.0,
        right: 100.0,
        bottom: 20.0,
        width: 100.0,
        height: 20.0,
    }
}

fn default_child_frame_client_rect() -> ClientRect {
    ClientRect {
        left: 0.0,
        top: 0.0,
        right: 300.0,
        bottom: 150.0,
        width: 300.0,
        height: 150.0,
    }
}

// Legacy geometry policy for explicit LayoutPolicy::Mock:
//
// OnDemand consumers must enter the unified GeometryProvider, which answers
// from the latest frozen tree or performs a cold refresh, and must never call
// these helpers. Mock remains deterministic, cheap, and side-effect free for
// the default CLI policy without `--layout`:
//
// - connected document/html/body/main-like roots expose the viewport-sized box;
// - connected full-height SPA shell containers under those roots expose the
//   viewport-sized box too, so virtualized app views do not collapse to the
//   tiny fallback box before their message/list children are mounted. This is a
//   class/style-hint approximation, not a site-specific selector list;
// - connected rendered elements and non-empty rendered text nodes expose a
//   small stable mock box in a deterministic document-flow-like position so
//   hit testing, DOM.getContentQuads, and lazyload probes can distinguish
//   siblings;
// - a rendered element's explicit non-negative inline `height: <px>` contributes
//   bounded flow steps for following elements. This is a cheap authored hint,
//   not cascade or box-layout resolution, and prevents a declared tall spacer
//   from leaving every later node in the synthetic first viewport;
// - non-rendered/disconnected nodes expose an empty box;
// - offsetParent is always null. Pretending that ordinary elements have
//   body/html as an offset parent makes viewport-probing lazyload code believe
//   that every mock-positioned element is renderable and visible. That creates
//   synthetic load-time work instead of browser-compatible data extraction;
// - no CSS box model, animation midpoint, stylesheet cascade, or general
//   authored-size resolution belongs here.
//
// Public consumers reach this contract only through the Mock branch in
// `geometry/provider.rs`; the default OnDemand branch performs a fresh complete
// layout pass and copies owned answers.
fn is_mock_rendered_element(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime.cached_mock_rendered_element(handle, uncached_is_mock_rendered_element)
}

fn uncached_is_mock_rendered_element(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let Some(node) = runtime.dom_host().node(handle) else {
        return false;
    };
    if !node.is_element() || !runtime.dom_host().is_connected(handle) {
        return false;
    }
    if !is_geometry_target_in_flat_tree(runtime, handle) {
        return false;
    }
    !matches!(
        node.local_name(),
        Some("head" | "title" | "meta" | "link" | "style" | "script" | "template")
    )
}

fn is_geometry_target_in_flat_tree(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let mut current = handle;
    loop {
        let Some(parent) = runtime
            .dom_host()
            .node(current)
            .and_then(crate::dom::native::Node::parent_node)
        else {
            return runtime
                .dom_host()
                .node(current)
                .is_some_and(crate::dom::native::Node::is_document);
        };
        if runtime.dom_host().is_shadow_root(parent) {
            let Some(shadow_host) = runtime.dom_host().shadow_root_host(parent) else {
                return false;
            };
            current = shadow_host;
            continue;
        }
        if runtime.dom_host().shadow_root_handle(parent).is_some()
            && geometry_node_is_slotable_for_flat_tree(runtime, current)
            && runtime.dom_host().assigned_slot_for_node(current).is_none()
        {
            return false;
        }
        current = parent;
    }
}

fn geometry_node_is_slotable_for_flat_tree(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .is_some_and(|node| node.is_element() || node.is_text())
}

fn is_viewport_mock_root(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime.dom_host().node(handle).is_some_and(|node| {
        runtime.dom_host().is_connected(handle)
            && (node.is_document()
                || matches!(
                    node.local_name(),
                    Some("html" | "body" | "frameset" | "main")
                ))
    })
}

fn element_attribute(runtime: &JsContextHost, handle: DomHandle, name: &str) -> Option<String> {
    runtime.dom_host().get_attribute(handle, name)
}

fn element_has_hidden_attribute(runtime: &JsContextHost, handle: DomHandle) -> bool {
    element_attribute(runtime, handle, "hidden").is_some()
}

fn element_is_hidden_input(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .is_some_and(|element| {
            element.local_name().eq_ignore_ascii_case("input")
                && element
                    .attribute("type")
                    .is_some_and(|input_type| input_type.eq_ignore_ascii_case("hidden"))
        })
}

fn element_suppresses_mock_layout_subtree(runtime: &JsContextHost, handle: DomHandle) -> bool {
    element_has_hidden_attribute(runtime, handle)
        || element_is_hidden_input(runtime, handle)
        || scroll_layout_display(runtime, handle) == "none"
}

fn scroll_layout_display(runtime: &JsContextHost, handle: DomHandle) -> String {
    ComputedStyleRead::new(runtime, handle)
        .property("display")
        .trim()
        .to_ascii_lowercase()
}

fn class_has_token(class: &str, expected: &str) -> bool {
    class
        .split_ascii_whitespace()
        .any(|token| token == expected)
}

fn is_explicit_viewport_shell_class(class: &str) -> bool {
    class_has_token(class, "h-svh")
        || class_has_token(class, "h-screen")
        || class_has_token(class, "min-h-svh")
        || class_has_token(class, "min-h-screen")
}

fn class_has_fill_available_shell_hints(class: &str) -> bool {
    let fills_available_height = class_has_token(class, "flex-1")
        || class_has_token(class, "h-full")
        || class_has_token(class, "min-h-full");
    let is_container_like = class_has_token(class, "flex")
        || class_has_token(class, "flex-col")
        || class_has_token(class, "w-full")
        || class_has_token(class, "min-w-0")
        || class_has_token(class, "min-h-0");
    fills_available_height && is_container_like
}

fn element_class(runtime: &JsContextHost, handle: DomHandle) -> Option<String> {
    element_attribute(runtime, handle, "class")
}

fn has_explicit_viewport_shell_hint(runtime: &JsContextHost, handle: DomHandle) -> bool {
    element_class(runtime, handle)
        .as_deref()
        .is_some_and(is_explicit_viewport_shell_class)
}

fn has_fill_available_shell_hint(runtime: &JsContextHost, handle: DomHandle) -> bool {
    element_class(runtime, handle)
        .as_deref()
        .is_some_and(class_has_fill_available_shell_hints)
}

fn parent_chain_has_viewport_shell_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    max_depth: usize,
) -> bool {
    let mut depth = 0;
    let mut current = mock_flow_parent(runtime, handle);
    while let Some(parent) = current {
        if is_viewport_mock_root(runtime, parent) {
            return true;
        }
        if !is_mock_rendered_element(runtime, parent) {
            return false;
        }
        if has_explicit_viewport_shell_hint(runtime, parent) {
            return has_near_viewport_root_ancestor(runtime, parent, max_depth);
        }
        if !has_fill_available_shell_hint(runtime, parent) {
            return false;
        }
        depth += 1;
        if depth >= max_depth {
            return false;
        }
        current = mock_flow_parent(runtime, parent);
    }
    false
}

fn has_near_viewport_root_ancestor(
    runtime: &JsContextHost,
    handle: DomHandle,
    max_depth: usize,
) -> bool {
    let mut depth = 0;
    let mut current = mock_flow_parent(runtime, handle);
    while let Some(parent) = current {
        if is_viewport_mock_root(runtime, parent) {
            return true;
        }
        depth += 1;
        if depth >= max_depth {
            return false;
        }
        current = mock_flow_parent(runtime, parent);
    }
    false
}

fn is_viewport_mock_shell(runtime: &JsContextHost, handle: DomHandle) -> bool {
    if !is_mock_rendered_element(runtime, handle) {
        return false;
    }
    if has_explicit_viewport_shell_hint(runtime, handle) {
        return has_near_viewport_root_ancestor(runtime, handle, 6);
    }
    has_fill_available_shell_hint(runtime, handle)
        && parent_chain_has_viewport_shell_context(runtime, handle, 6)
}

fn is_viewport_mock_box(runtime: &JsContextHost, handle: DomHandle) -> bool {
    is_viewport_mock_root(runtime, handle) || is_viewport_mock_shell(runtime, handle)
}

fn previous_mock_rendered_flow_count(
    runtime: &JsContextHost,
    handle: DomHandle,
    count_limit: usize,
) -> usize {
    if let Some(count) = runtime.cached_preceding_mock_flow_count(handle) {
        return count.min(count_limit);
    }
    let dom = runtime.dom_host().dom();
    let Some(parent) = dom.parent_node(handle) else {
        return 0;
    };
    let Some(parent_node) = dom.node(parent) else {
        return 0;
    };
    let first_child = parent_node.child_ids(dom).next();
    let (mut sibling, mut count) = runtime
        .cached_mock_flow_prefix_cursor(parent)
        .unwrap_or((first_child, 0));
    let mut derived = Vec::new();
    while let Some(current) = sibling {
        derived.push((current, count));
        if current == handle {
            break;
        }
        if count < MOCK_FLOW_COUNT_LIMIT {
            count += mock_rendered_subtree_count(
                runtime,
                current,
                MOCK_FLOW_COUNT_LIMIT.saturating_sub(count),
            );
        }
        sibling = dom.next_sibling(current);
    }
    runtime.cache_preceding_mock_flow_counts(derived);
    runtime.cache_mock_flow_prefix_cursor(parent, sibling, count);
    runtime
        .cached_preceding_mock_flow_count(handle)
        .unwrap_or(count)
        .min(count_limit)
}

fn mock_flow_top(runtime: &JsContextHost, handle: DomHandle) -> f64 {
    runtime.cached_mock_flow_top(handle, uncached_mock_flow_top)
}

fn uncached_mock_flow_top(runtime: &JsContextHost, handle: DomHandle) -> f64 {
    let mut path = Vec::new();
    let mut current = Some(handle);
    while let Some(node) = current {
        path.push(node);
        current = mock_flow_parent(runtime, node)
            .filter(|parent| !is_viewport_mock_box(runtime, *parent))
            .filter(|parent| is_mock_rendered_element(runtime, *parent));
    }

    // Use document-flow order rather than direct sibling order so lazyload
    // fallbacks that read getBoundingClientRect() do not treat a deep tail
    // element as viewport-visible merely because it has few direct siblings.
    // This deliberately remains a bounded, stylesheet-free approximation:
    // once enough preceding mock-rendered boxes have been counted to put the
    // target far below the viewport, additional exactness is not worth making
    // geometry reads proportional to arbitrarily large user-controlled DOMs.
    let mut count: usize = 0;
    for node in path.into_iter().rev() {
        let remaining = MOCK_FLOW_COUNT_LIMIT.saturating_sub(count);
        if remaining == 0 {
            break;
        }
        count += previous_mock_rendered_flow_count(runtime, node, remaining);
    }
    (count as f64) * MOCK_FLOW_STEP_PX
}

fn child_document_visible_flow_top(runtime: &JsContextHost, handle: DomHandle, top: f64) -> f64 {
    if top != 0.0 {
        return top;
    }
    let Some(document) = runtime.dom_host().owner_document_handle(handle) else {
        return top;
    };
    if document == runtime.document_handle()
        || runtime
            .child_browsing_context_host_for_document_handle(document)
            .is_none()
    {
        return top;
    }
    CHILD_DOCUMENT_VISIBLE_FLOW_ORIGIN_PX
}

fn mock_flow_parent(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    let parent = runtime.dom_host().dom().parent_node(handle)?;
    if runtime.dom_host().is_shadow_root(parent) {
        return runtime.dom_host().shadow_root_host(parent);
    }
    Some(parent)
}

fn mock_client_rect(runtime: &JsContextHost, handle: DomHandle) -> ClientRect {
    if runtime
        .dom_host()
        .node(handle)
        .is_some_and(crate::dom::native::Node::is_text)
    {
        return mock_text_layout_client_rect(runtime, handle).unwrap_or_else(zero_client_rect);
    }
    if is_viewport_mock_box(runtime, handle) {
        return viewport_client_rect(runtime, handle);
    }
    if !is_mock_rendered_element(runtime, handle) {
        return zero_client_rect();
    }
    if element_has_hidden_attribute(runtime, handle) || element_is_hidden_input(runtime, handle) {
        return zero_client_rect();
    }
    if matches!(
        raw_inline_style_property_value(runtime, handle, "display")
            .unwrap_or_default()
            .as_str(),
        "none" | "contents"
    ) {
        return zero_client_rect();
    }
    let mut rect = if runtime
        .dom_host()
        .node(handle)
        .is_some_and(|node| matches!(node.local_name(), Some("iframe" | "frame")))
    {
        default_child_frame_client_rect()
    } else {
        default_element_client_rect()
    };
    let top = child_document_visible_flow_top(runtime, handle, mock_flow_top(runtime, handle));
    rect.top = top;
    rect.bottom = top + rect.height;
    rect
}

pub(in crate::native_bridge) fn compute_mock_offset_parent(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let trace_started = moli_trace::cdp_nav_timing_enabled().then(Instant::now);
    let parent = mock_offset_parent(runtime, handle);
    if let Some(trace_started) = trace_started {
        runtime.record_offset_parent_trace(trace_started.elapsed());
    }
    parent
}

fn mock_offset_parent(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    let _ = (runtime, handle);
    None
}

pub(crate) fn compute_mock_scroll_adjusted_client_rect(
    runtime: &JsContextHost,
    handle: DomHandle,
    scroll_x: f64,
    scroll_y: f64,
) -> ClientRect {
    let mut rect = runtime.cached_mock_client_rect(handle, mock_client_rect);
    rect.left -= scroll_x;
    rect.right -= scroll_x;
    rect.top -= scroll_y;
    rect.bottom -= scroll_y;
    rect
}

pub(crate) fn compute_mock_client_rect(runtime: &JsContextHost, handle: DomHandle) -> ClientRect {
    let trace_started = moli_trace::cdp_nav_timing_enabled().then(Instant::now);
    let rect = runtime.cached_mock_client_rect(handle, mock_client_rect);
    if let Some(trace_started) = trace_started {
        runtime.record_client_rect_trace(trace_started.elapsed());
    }
    rect
}

fn mock_text_layout_client_rect(runtime: &JsContextHost, handle: DomHandle) -> Option<ClientRect> {
    let node = runtime.dom_host().node(handle)?;
    if !node.is_text()
        || node.data_value().is_none_or(str::is_empty)
        || !runtime.dom_host().is_connected(handle)
        || !is_geometry_target_in_flat_tree(runtime, handle)
    {
        return None;
    }

    let mut ancestor = runtime.dom_host().parent_node(handle);
    while let Some(handle) = ancestor {
        let node = runtime.dom_host().node(handle)?;
        if node.is_element()
            && (element_suppresses_mock_layout_subtree(runtime, handle)
                || matches!(
                    node.local_name(),
                    Some("head" | "title" | "meta" | "link" | "style" | "script" | "template")
                ))
        {
            return None;
        }
        ancestor = runtime.dom_host().parent_node(handle);
    }

    let mut rect = default_element_client_rect();
    let top = child_document_visible_flow_top(runtime, handle, mock_flow_top(runtime, handle));
    rect.top = top;
    rect.bottom = top + rect.height;
    Some(rect)
}

pub(super) fn mock_layout_client_rect_for_node(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<ClientRect> {
    let node = runtime.dom_host().node(handle)?;
    if node.is_element() {
        if matches!(
            scroll_layout_display(runtime, handle).as_str(),
            "none" | "contents"
        ) {
            return None;
        }
        let rect = compute_mock_client_rect(runtime, handle);
        return (rect.width > 0.0 && rect.height > 0.0).then_some(rect);
    }
    node.is_text()
        .then(|| mock_text_layout_client_rect(runtime, handle))
        .flatten()
}

pub(crate) fn compute_mock_intersection_client_rect(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> ClientRect {
    runtime.cached_mock_client_rect(handle, mock_client_rect)
}

pub(crate) fn compute_mock_intersection_scrollport_client_rect(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> ClientRect {
    runtime.cached_mock_client_rect(handle, mock_client_rect)
}

fn mock_rendered_subtree_count(
    runtime: &JsContextHost,
    root: DomHandle,
    count_limit: usize,
) -> usize {
    let dom = runtime.dom_host().dom();
    let mut count: usize = 0;
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        #[cfg(test)]
        runtime.note_mock_flow_subtree_node_visit_for_test();
        if is_mock_rendered_element(runtime, handle) {
            count = count.saturating_add(mock_flow_step_count(runtime, handle));
            if count >= count_limit {
                count = count_limit;
                break;
            }
        }
        if let Some(node) = dom.node(handle) {
            stack.extend(node.child_ids(dom));
        }
    }
    count
}

fn mock_flow_step_count(runtime: &JsContextHost, handle: DomHandle) -> usize {
    let Some(value) = raw_inline_style_property_value(runtime, handle, "height") else {
        return 1;
    };
    let value = value.trim();
    let Some(px) = value.strip_suffix("px") else {
        return 1;
    };
    let Ok(px) = px.trim().parse::<f64>() else {
        return 1;
    };
    if !px.is_finite() || px <= 0.0 {
        return 1;
    }
    (px / MOCK_FLOW_STEP_PX)
        .ceil()
        .clamp(1.0, MOCK_FLOW_COUNT_LIMIT as f64) as usize
}

fn document_elements_in_hit_order(
    runtime: &JsContextHost,
    document_handle: DomHandle,
) -> Vec<DomHandle> {
    fn visit(runtime: &JsContextHost, node: DomHandle, out: &mut Vec<DomHandle>) {
        let Some(entry) = runtime.dom_host().node(node) else {
            return;
        };
        if entry.is_element() {
            out.push(node);
        }
        for child in entry.child_ids(runtime.dom_host().dom()) {
            visit(runtime, child, out);
        }
    }

    let mut elements = Vec::new();
    visit(runtime, document_handle, &mut elements);
    elements
}

pub(crate) fn mock_hit_test_handle(
    runtime: &JsContextHost,
    document_handle: DomHandle,
    x: f64,
    y: f64,
) -> Option<DomHandle> {
    mock_hit_test_handle_with_child_frames(runtime, document_handle, x, y, 0)
}

fn hit_test_shadow_root_handle(
    runtime: &JsContextHost,
    shadow_root: DomHandle,
    x: f64,
    y: f64,
    depth: usize,
) -> Option<DomHandle> {
    if depth >= HIT_TEST_CHILD_FRAME_DEPTH_LIMIT {
        return None;
    }
    let mut children = runtime
        .dom_host()
        .child_handles(shadow_root)
        .collect::<Vec<_>>();
    children.reverse();
    for child in children {
        let Some(node) = runtime.dom_host().node(child) else {
            continue;
        };
        if !node.is_element() {
            continue;
        }
        let rect = mock_client_rect(runtime, child);
        if rect.width <= 0.0
            || rect.height <= 0.0
            || x < rect.left
            || x > rect.right
            || y < rect.top
            || y > rect.bottom
        {
            continue;
        }
        if let Some(nested_shadow_root) = runtime.dom_host().shadow_root_handle(child)
            && let Some(hit) =
                hit_test_shadow_root_handle(runtime, nested_shadow_root, x, y, depth + 1)
        {
            return Some(hit);
        }
        return Some(child);
    }
    None
}

fn mock_hit_test_handle_with_child_frames(
    runtime: &JsContextHost,
    document_handle: DomHandle,
    x: f64,
    y: f64,
    depth: usize,
) -> Option<DomHandle> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let hit = document_elements_in_hit_order(runtime, document_handle)
        .into_iter()
        .rev()
        .find(|element| {
            let rect = mock_client_rect(runtime, *element);
            rect.width > 0.0
                && rect.height > 0.0
                && x >= rect.left
                && x <= rect.right
                && y >= rect.top
                && y <= rect.bottom
        })?;
    if depth < HIT_TEST_CHILD_FRAME_DEPTH_LIMIT
        && let Some(child_document) = runtime.child_browsing_context_document_handle(hit)
    {
        let rect = mock_client_rect(runtime, hit);
        if let Some(child_hit) = mock_hit_test_handle_with_child_frames(
            runtime,
            child_document,
            x - rect.left,
            y - rect.top,
            depth + 1,
        ) {
            return Some(child_hit);
        }
    }
    if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(hit)
        && let Some(shadow_hit) = hit_test_shadow_root_handle(runtime, shadow_root, x, y, depth + 1)
    {
        return Some(shadow_hit);
    }
    Some(hit)
}
