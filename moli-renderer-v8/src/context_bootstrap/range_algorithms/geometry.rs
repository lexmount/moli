use super::*;
use crate::native_bridge::document::SVG_NS;
use crate::util::{call_object_method, object_string_property};

enum NativeRangeGeometryQuery {
    Text,
    Element,
}

// Container boundaries use the positions between children. CharacterData
// boundaries share the node's interior position and use the UTF-16 offset.
type GeometryBoundary = (usize, u32);

struct GeometryNode {
    handle: DomHandle,
    depth: usize,
    before: usize,
    inside: usize,
    after: usize,
}

struct RangeGeometryOrder {
    boundaries: RangeBoundaryHandles,
    nodes: Vec<GeometryNode>,
    next_position: usize,
    start: Option<GeometryBoundary>,
    end: Option<GeometryBoundary>,
}

impl RangeGeometryOrder {
    fn collect(
        dom: &crate::dom::native::DomHost,
        common: DomHandle,
        boundaries: RangeBoundaryHandles,
    ) -> Self {
        let mut order = Self {
            boundaries,
            nodes: Vec::new(),
            next_position: 1,
            start: None,
            end: None,
        };
        order.visit(dom, common, 0, 0);
        order
    }

    fn position(&mut self) -> usize {
        let position = self.next_position;
        self.next_position += 1;
        position
    }

    fn record_boundary(&mut self, handle: DomHandle, offset: u32, position: usize) {
        if self.boundaries.start.container == handle && self.boundaries.start.offset == offset {
            self.start = Some((position, 0));
        }
        if self.boundaries.end.container == handle && self.boundaries.end.offset == offset {
            self.end = Some((position, 0));
        }
    }

    fn visit(
        &mut self,
        dom: &crate::dom::native::DomHost,
        handle: DomHandle,
        depth: usize,
        before: usize,
    ) -> usize {
        let inside = self.position();
        let index = self.nodes.len();
        self.nodes.push(GeometryNode {
            handle,
            depth,
            before,
            inside,
            after: 0,
        });
        let mut children = dom.child_handles(handle).enumerate().peekable();
        if children.peek().is_none() {
            if self.boundaries.start.container == handle {
                self.start = Some((inside, self.boundaries.start.offset));
            }
            if self.boundaries.end.container == handle {
                self.end = Some((inside, self.boundaries.end.offset));
            }
        } else {
            self.record_boundary(handle, 0, inside);
            let mut child_before = inside;
            for (child_index, child) in children {
                child_before = self.visit(dom, child, depth + 1, child_before);
                self.record_boundary(handle, (child_index + 1) as u32, child_before);
            }
        }
        let after = self.position();
        self.nodes[index].after = after;
        after
    }
}

fn native_range_client_rects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<Result<Vec<ClientRect>, moli_layout::LayoutError>> {
    let boundaries = native_range_boundary_handles(scope, range)?;
    native_range_client_rects_for_boundaries(scope, boundaries)
}

pub(super) fn native_range_client_rects_for_boundaries(
    scope: &mut v8::PinScope<'_, '_>,
    boundaries: RangeBoundaryHandles,
) -> Option<Result<Vec<ClientRect>, moli_layout::LayoutError>> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &*host_ptr };
    if !host.layout_policy().uses_real_layout() {
        // Preserve the explicit Mock policy's pre-layout Range contract: one
        // synthetic box for the common element, or for a text node's parent.
        // Fragment-accurate text ranges belong exclusively to Lazy.
        let common =
            common_ancestor_handle(scope, boundaries.start.container, boundaries.end.container)?;
        let target = if node_type_for_handle(scope, common) == Some(NodeType::Element) {
            common
        } else {
            parent_handle(scope, common)?
        };
        return Some(Ok(vec![compute_mock_client_rect(host, target)]));
    }
    let dom = host.dom_host();
    let document = dom.owner_document_handle(boundaries.start.container)?;
    let common =
        common_ancestor_handle(scope, boundaries.start.container, boundaries.end.container)?;
    let order = RangeGeometryOrder::collect(dom, common, boundaries);
    let range_start = order.start?;
    let range_end = order.end?;

    let mut queries = Vec::new();
    let mut kinds = Vec::new();
    let mut selected_element_depth = None;
    for GeometryNode {
        handle,
        depth,
        before,
        inside,
        after,
    } in order.nodes
    {
        if selected_element_depth.is_some_and(|selected| depth <= selected) {
            selected_element_depth = None;
        }
        let Some(node) = dom.node(handle) else {
            continue;
        };
        match node.node_type() {
            NodeType::Text | NodeType::CDataSection => {
                if inside < range_start.0 || inside > range_end.0 {
                    continue;
                }
                let length = host
                    .character_data_utf16_units(handle)
                    .map(|units| units.len())
                    .unwrap_or(0);
                if (inside, 0) > range_end
                    || (inside, u32::try_from(length).unwrap_or(u32::MAX)) < range_start
                {
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
                // Inline SVG is published as a replaced scene. For a whole
                // text node that supplies all of an SVG text element's content,
                // its shaped object quad also describes the selected text.
                if start == 0
                    && end == length
                    && length != 0
                    && let Some(parent) = dom.parent_node(handle)
                    && dom
                        .node(parent)
                        .and_then(|node| node.as_element())
                        .is_some_and(|element| element.is_svg_element("text"))
                    && dom.child_handles(parent).eq([handle])
                {
                    queries.push(moli_layout::LayoutQuery::ClientRects { source: parent });
                    kinds.push(NativeRangeGeometryQuery::Element);
                    continue;
                }
                queries.push(moli_layout::LayoutQuery::TextRangeRects {
                    source: handle,
                    utf16_range: start..end,
                });
                kinds.push(NativeRangeGeometryQuery::Text);
            }
            NodeType::Element
                if handle != common && (before, 0) >= range_start && (after, 0) <= range_end =>
            {
                // A replaced SVG scene publishes its object's quads under
                // their own sources, separately from the selected SVG root.
                if selected_element_depth.is_some() && node.namespace() != Some(SVG_NS) {
                    continue;
                }
                queries.push(moli_layout::LayoutQuery::ClientRects { source: handle });
                kinds.push(NativeRangeGeometryQuery::Element);
                // Text descendants still contribute their selected ranges.
                selected_element_depth.get_or_insert(depth);
            }
            _ => {}
        }
    }

    if queries.is_empty() {
        return Some(Ok(Vec::new()));
    }
    let answers = match observable_geometry_batch(
        host,
        document,
        &moli_layout::LayoutQueryBatch::new(queries),
    ) {
        Err(moli_layout::LayoutError::NoLayoutSnapshot) => return Some(Ok(Vec::new())),
        answers => answers,
    };
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
            rects.extend(quads.into_iter().map(ClientRect::from));
        }
        Ok(rects)
    }))
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
            Ok(rects) => {
                new_dom_rect_from_client_rect(scope, ClientRect::bounding_client_rect(rects))
            }
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
