use super::*;
use crate::util::{call_object_method, object_string_property};

enum NativeRangeGeometryQuery {
    Text,
    Element,
}

fn native_range_client_rects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<Result<Vec<ClientRect>, moli_layout::LayoutError>> {
    let boundaries = native_range_boundary_handles(scope, range)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &*host_ptr };
    if !host.layout_policy().uses_real_layout() {
        // Preserve the explicit Mock policy's pre-layout Range contract: one
        // synthetic box for the common element, or for a text node's parent.
        // Fragment-accurate text ranges belong exclusively to OnDemand.
        let common =
            common_ancestor_handle(scope, boundaries.start.container, boundaries.end.container)?;
        let target = if node_type_for_handle(scope, common) == Some(NodeType::Element) {
            common
        } else {
            parent_handle(scope, common)?
        };
        return Some(Ok(vec![compute_mock_client_rect(host, target)]));
    }
    let document = host
        .dom_host()
        .owner_document_handle(boundaries.start.container)?;
    let common =
        common_ancestor_handle(scope, boundaries.start.container, boundaries.end.container)?;
    let mut handles = Vec::new();
    collect_range_geometry_subtree(host.dom_host(), common, &mut handles);

    let mut queries = Vec::new();
    let mut kinds = Vec::new();
    for handle in handles {
        match node_type_for_handle(scope, handle) {
            Some(NodeType::Text | NodeType::CDataSection) => {
                let length = character_data_utf16_units_handle(scope, handle)
                    .map(|units| units.len())
                    .unwrap_or(0);
                let starts_before_range_end = super::ordering::point_order_handles(
                    scope,
                    handle,
                    0,
                    boundaries.end.container,
                    boundaries.end.offset,
                ) == Some(std::cmp::Ordering::Less)
                    || (handle == boundaries.end.container && boundaries.end.offset == 0);
                let ends_after_range_start = super::ordering::point_order_handles(
                    scope,
                    handle,
                    u32::try_from(length).unwrap_or(u32::MAX),
                    boundaries.start.container,
                    boundaries.start.offset,
                ) == Some(std::cmp::Ordering::Greater)
                    || (handle == boundaries.start.container
                        && boundaries.start.offset as usize == length);
                let collapsed_here = boundaries.start.container == handle
                    && boundaries.end.container == handle
                    && boundaries.start.offset == boundaries.end.offset;
                if (!starts_before_range_end || !ends_after_range_start) && !collapsed_here {
                    continue;
                }
                let start = if handle == boundaries.start.container {
                    boundaries.start.offset as usize
                } else {
                    0
                }
                .min(length);
                let end = if handle == boundaries.end.container {
                    boundaries.end.offset as usize
                } else {
                    length
                }
                .min(length);
                queries.push(moli_layout::LayoutQuery::TextRangeRects {
                    source: handle,
                    utf16_range: start..end,
                });
                kinds.push(NativeRangeGeometryQuery::Text);
            }
            Some(NodeType::Element) if handle != common => {
                let Some(parent) = parent_handle(scope, handle) else {
                    continue;
                };
                let Some(index) = child_index_handle(scope, parent, handle) else {
                    continue;
                };
                let starts_at_or_after_range = !matches!(
                    super::ordering::point_order_handles(
                        scope,
                        parent,
                        index,
                        boundaries.start.container,
                        boundaries.start.offset,
                    ),
                    Some(std::cmp::Ordering::Less)
                );
                let ends_at_or_before_range = !matches!(
                    super::ordering::point_order_handles(
                        scope,
                        parent,
                        index.saturating_add(1),
                        boundaries.end.container,
                        boundaries.end.offset,
                    ),
                    Some(std::cmp::Ordering::Greater)
                );
                if starts_at_or_after_range && ends_at_or_before_range {
                    queries.push(moli_layout::LayoutQuery::ClientRects { source: handle });
                    kinds.push(NativeRangeGeometryQuery::Element);
                }
            }
            _ => {}
        }
    }

    if queries.is_empty() {
        return Some(Ok(Vec::new()));
    }
    let answers = observable_geometry_batch(
        host,
        document,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
        &moli_layout::LayoutQueryBatch::new(queries),
    );
    Some(answers.and_then(|answers| {
        let mut rects = Vec::new();
        for (kind, answer) in kinds.into_iter().zip(answers.answers) {
            let quads = match (kind, answer) {
                (
                    NativeRangeGeometryQuery::Text,
                    moli_layout::LayoutQueryAnswer::TextRangeRects(quads),
                )
                | (
                    NativeRangeGeometryQuery::Element,
                    moli_layout::LayoutQueryAnswer::ClientRects(quads),
                ) => quads,
                _ => {
                    return Err(moli_layout::LayoutError::source_contract(
                        "range geometry",
                        "provider returned a mismatched range answer",
                    ));
                }
            };
            rects.extend(quads.into_iter().map(client_rect_from_layout_quad));
        }
        Ok(rects)
    }))
}

fn collect_range_geometry_subtree(
    dom_host: &crate::dom::native::DomHost,
    root: DomHandle,
    out: &mut Vec<DomHandle>,
) {
    out.push(root);
    for child in dom_host.child_handles(root) {
        collect_range_geometry_subtree(dom_host, child, out);
    }
}

fn client_rect_from_layout_quad(quad: moli_layout::LayoutQuad) -> ClientRect {
    let rect = quad.bounding_rect();
    ClientRect {
        left: f64::from(rect.x),
        top: f64::from(rect.y),
        right: f64::from(rect.right()),
        bottom: f64::from(rect.bottom()),
        width: f64::from(rect.width),
        height: f64::from(rect.height),
    }
}

fn bounding_range_rect(rects: &[ClientRect]) -> ClientRect {
    let Some(first) = rects.first().copied() else {
        return ClientRect {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            width: 0.0,
            height: 0.0,
        };
    };
    let mut non_empty = rects
        .iter()
        .copied()
        .filter(|rect| rect.width != 0.0 || rect.height != 0.0);
    let Some(mut bounds) = non_empty.next() else {
        return first;
    };
    for rect in non_empty {
        let left = bounds.left.min(rect.left);
        let top = bounds.top.min(rect.top);
        let right = bounds.right.max(rect.right);
        let bottom = bounds.bottom.max(rect.bottom);
        bounds = ClientRect {
            left,
            top,
            right,
            bottom,
            width: (right - left).max(0.0),
            height: (bottom - top).max(0.0),
        };
    }
    bounds
}

fn throw_range_layout_error(scope: &mut v8::PinScope<'_, '_>, error: moli_layout::LayoutError) {
    let message = format!("Layout failed while resolving Range geometry: {error}");
    if let Some(message) = crate::util::v8_string(scope, &message) {
        let exception = v8::Exception::error(scope, message);
        scope.throw_exception(exception);
    }
}

fn range_geometry_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let common = range_common_ancestor_container(scope, range)?;
    let node_type = object_number_property(scope, common, "nodeType").unwrap_or(0.0) as u32;
    if node_type == 1 {
        Some(common)
    } else {
        object_property_as_object(scope, common, "parentNode")
    }
}

fn collapsed_text_range_client_rect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<ClientRect> {
    if native_range_boundary_handles(scope, range).is_some() {
        // Native ranges are handled by `native_range_client_rects`. If that
        // path cannot establish a Document/common ancestor, do not fabricate
        // geometry from a parent box.
        return None;
    }

    if !range_is_collapsed(scope, range) {
        return None;
    }
    let start = range_boundary_container_object(scope, range, RangeBoundarySide::Start)?;
    if object_number_property(scope, start, "nodeType")? as u32 != 3 {
        return None;
    }
    let offset = range_boundary_offset(scope, range, RangeBoundarySide::Start) as usize;
    let parent = object_property_as_object(scope, start, "parentNode")?;
    let parent_rect = call_object_method(scope, parent, "getBoundingClientRect", &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let parent_left = object_number_property(scope, parent_rect, "x")?;
    let parent_top = object_number_property(scope, parent_rect, "y")?;
    let parent_width = object_number_property(scope, parent_rect, "width")?;
    let parent_height = object_number_property(scope, parent_rect, "height")?;
    let parent_rect = ClientRect {
        left: parent_left,
        top: parent_top,
        right: parent_left + parent_width,
        bottom: parent_top + parent_height,
        width: parent_width,
        height: parent_height,
    };
    let value_len = object_string_property(scope, start, "nodeValue")
        .map(|value| value.encode_utf16().count())
        .unwrap_or(0);
    if value_len == 0 || parent_rect.width <= 0.0 {
        return Some(ClientRect {
            right: parent_rect.left,
            width: 0.0,
            ..parent_rect
        });
    }
    let offset = offset.min(value_len);
    let left = parent_rect.left + parent_rect.width * offset as f64 / value_len as f64;
    Some(ClientRect {
        left,
        right: left,
        width: 0.0,
        ..parent_rect
    })
}

fn client_rect_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<ClientRect> {
    let rect = call_object_method(scope, target, "getBoundingClientRect", &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let left = object_number_property(scope, rect, "x")?;
    let top = object_number_property(scope, rect, "y")?;
    let width = object_number_property(scope, rect, "width")?;
    let height = object_number_property(scope, rect, "height")?;
    Some(ClientRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
        width,
        height,
    })
}

pub(in crate::context_bootstrap) fn range_geometry_dom_rect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(rects) = native_range_client_rects(scope, range) {
        return match rects {
            Ok(rects) => new_dom_rect_from_client_rect(scope, bounding_range_rect(&rects)),
            Err(error) => {
                throw_range_layout_error(scope, error);
                None
            }
        };
    }
    if let Some(rect) = collapsed_text_range_client_rect(scope, range) {
        return new_dom_rect_from_client_rect(scope, rect);
    }
    let target = range_geometry_target(scope, range)?;
    let rect = client_rect_for_object(scope, target)?;
    new_dom_rect_from_client_rect(scope, rect)
}

pub(in crate::context_bootstrap) fn range_geometry_client_rects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(rects) = native_range_client_rects(scope, range) {
        return match rects {
            Ok(rects) => {
                let rects = rects
                    .into_iter()
                    .filter_map(|rect| new_dom_rect_from_client_rect(scope, rect))
                    .collect::<Vec<_>>();
                Some(dom_rect_list::build_dom_rect_list_object(scope, &rects))
            }
            Err(error) => {
                throw_range_layout_error(scope, error);
                None
            }
        };
    }
    if let Some(rect) = collapsed_text_range_client_rect(scope, range) {
        let rect = new_dom_rect_from_client_rect(scope, rect)?;
        return Some(dom_rect_list::build_dom_rect_list_object(scope, &[rect]));
    }
    let target = range_geometry_target(scope, range)?;
    let rect = client_rect_for_object(scope, target)?;
    let rect = new_dom_rect_from_client_rect(scope, rect)?;
    Some(dom_rect_list::build_dom_rect_list_object(scope, &[rect]))
}

pub(in crate::context_bootstrap) fn new_dom_rect_zero<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    new_dom_rect_with_source(scope, None)
}

fn new_dom_rect_from_client_rect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: ClientRect,
) -> Option<v8::Local<'s, v8::Object>> {
    Some(build_dom_rect_object(
        scope,
        rect.left,
        rect.top,
        rect.width,
        rect.height,
    ))
}

fn new_dom_rect_with_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let x = source
        .and_then(|source| object_number_property(scope, source, "x"))
        .unwrap_or(0.0);
    let y = source
        .and_then(|source| object_number_property(scope, source, "y"))
        .unwrap_or(0.0);
    let width = source
        .and_then(|source| object_number_property(scope, source, "width"))
        .unwrap_or(0.0);
    let height = source
        .and_then(|source| object_number_property(scope, source, "height"))
        .unwrap_or(0.0);
    Some(build_dom_rect_object(scope, x, y, width, height))
}
