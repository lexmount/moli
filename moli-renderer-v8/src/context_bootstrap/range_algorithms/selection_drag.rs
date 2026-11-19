use super::*;

/// Resolve browser-initiated selection drags through the same native Range
/// geometry and visible-text serializer used by the Selection Web API.
fn document_selection_drag_text_at_point(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    handle: DomHandle,
    x: f64,
    y: f64,
) -> Option<String> {
    let runtime = unsafe { &*runtime_ptr };
    let host = runtime.dom_host();
    let document = host.owner_document_handle(handle)?;
    let selection = runtime.document_selection_snapshot(document)?;
    let (mut start, mut end) = (selection.start, selection.end);
    match crate::range_boundary::point_order_in_dom(
        host,
        start.container,
        start.offset,
        end.container,
        end.offset,
    )? {
        std::cmp::Ordering::Equal => return None,
        std::cmp::Ordering::Greater => std::mem::swap(&mut start, &mut end),
        std::cmp::Ordering::Less => {}
    }
    let boundaries = RangeBoundaryHandles {
        start: RangeBoundaryHandle {
            container: start.container,
            offset: start.offset,
        },
        end: RangeBoundaryHandle {
            container: end.container,
            offset: end.offset,
        },
    };
    let rects = geometry::native_range_client_rects_for_boundaries(scope, boundaries)?.ok()?;
    if !rects.iter().any(|rect| {
        rect.width > 0.0
            && rect.height > 0.0
            && x >= rect.left
            && x <= rect.right
            && y >= rect.top
            && y <= rect.bottom
    }) {
        return None;
    }
    text::range_selection_string_contents_for_boundaries(scope, boundaries)
        .filter(|text| !text.is_empty())
}

pub(crate) struct NativeSelectionDragSnapshot {
    pub(crate) document: DomHandle,
    pub(crate) start: DomHandle,
    pub(crate) start_offset: u32,
    pub(crate) end: DomHandle,
    pub(crate) end_offset: u32,
    pub(crate) text: String,
}

pub(crate) fn document_selection_drag_snapshot_at_point(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    handle: DomHandle,
    x: f64,
    y: f64,
) -> Option<NativeSelectionDragSnapshot> {
    let text = document_selection_drag_text_at_point(scope, runtime_ptr, handle, x, y)?;
    let runtime = unsafe { &*runtime_ptr };
    let host = runtime.dom_host();
    let document = host.owner_document_handle(handle)?;
    let selection = runtime.document_selection_snapshot(document)?;
    let (mut start, mut end) = (selection.start, selection.end);
    if crate::range_boundary::point_order_in_dom(
        host,
        start.container,
        start.offset,
        end.container,
        end.offset,
    )? == std::cmp::Ordering::Greater
    {
        std::mem::swap(&mut start, &mut end);
    }
    Some(NativeSelectionDragSnapshot {
        document,
        start: start.container,
        start_offset: start.offset,
        end: end.container,
        end_offset: end.offset,
        text,
    })
}
