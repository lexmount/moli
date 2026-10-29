//! Source selection and destination caret are independent live DOM ranges.
//! DOM mutations repair both through the existing Range implementation.
use super::range::{
    RangeBoundarySide, native_range_boundary_handles, new_range_for_document, set_range_boundary,
};
use super::*;
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use crate::native_bridge::element::{
    TextEditInputType, construct_input_event, contenteditable_editing_host, dispatch_public_event,
    observable_caret_position,
};

pub(crate) struct DragEditingRange {
    pub(crate) document: DomHandle,
    pub(crate) host: DomHandle,
    range: v8::Global<v8::Object>,
}

impl DragEditingRange {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        document: DomHandle,
        host: DomHandle,
        start: DomHandle,
        start_offset: u32,
        end: DomHandle,
        end_offset: u32,
    ) -> Option<Self> {
        let document_object = crate::util::node_wrapper_from_handle(scope, document)?;
        let range = new_range_for_document(scope, document_object)?;
        for (side, node, offset) in [
            (RangeBoundarySide::Start, start, start_offset),
            (RangeBoundarySide::End, end, end_offset),
        ] {
            let node = crate::util::node_wrapper_from_handle(scope, node)?;
            set_range_boundary(scope, range, side, node, offset);
        }
        Some(Self {
            document,
            host,
            range: v8::Global::new(scope, range),
        })
    }

    pub(crate) fn is_valid(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime: &JsContextHost,
    ) -> bool {
        if !self.is_connected(scope, runtime) {
            return false;
        }
        let range = v8::Local::new(scope, &self.range);
        native_range_boundary_handles(scope, range).is_some_and(|bounds| {
            [bounds.start.container, bounds.end.container]
                .into_iter()
                .all(|node| {
                    runtime.dom_host().is_connected(node)
                        && runtime.dom_host().owner_document_handle(node) == Some(self.document)
                        && contenteditable_editing_host(runtime, node) == Some(self.host)
                })
        })
    }

    pub(crate) fn is_connected(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime: &JsContextHost,
    ) -> bool {
        let dom = runtime.dom_host();
        if !dom.is_connected(self.host)
            || dom.owner_document_handle(self.host) != Some(self.document)
        {
            return false;
        }
        let range = v8::Local::new(scope, &self.range);
        native_range_boundary_handles(scope, range).is_some_and(|bounds| {
            [bounds.start.container, bounds.end.container]
                .into_iter()
                .all(|mut node| {
                    loop {
                        if node == self.host {
                            return true;
                        }
                        let Some(parent) = dom.parent_node(node) else {
                            return false;
                        };
                        node = parent;
                    }
                })
        })
    }

    pub(crate) fn contains(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime: &JsContextHost,
        caret: &Self,
    ) -> bool {
        if self.document != caret.document {
            return false;
        }
        let range = v8::Local::new(scope, &self.range);
        let point = v8::Local::new(scope, &caret.range);
        let Some(bounds) = native_range_boundary_handles(scope, range) else {
            return false;
        };
        let Some(point) = native_range_boundary_handles(scope, point).map(|bounds| bounds.start)
        else {
            return false;
        };
        let compare = |node, offset| {
            crate::range_boundary::point_order_in_dom(
                runtime.dom_host(),
                node,
                offset,
                point.container,
                point.offset,
            )
        };
        matches!(
            compare(bounds.start.container, bounds.start.offset),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ) && matches!(
            compare(bounds.end.container, bounds.end.offset),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        )
    }

    pub(crate) fn select(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        let range = v8::Local::new(scope, &self.range);
        let Some(bounds) = native_range_boundary_handles(scope, range) else {
            return false;
        };
        let Some(start) = crate::util::node_wrapper_from_handle(scope, bounds.start.container)
        else {
            return false;
        };
        let Some(end) = crate::util::node_wrapper_from_handle(scope, bounds.end.container) else {
            return false;
        };
        let window = scope.get_current_context().global(scope);
        let Some(selection) = selection_value_for_window(scope, window) else {
            return false;
        };
        selection::selection_store(
            scope,
            selection,
            range,
            start,
            bounds.start.offset,
            end,
            bounds.end.offset,
            "forward",
        );
        true
    }

    pub(crate) fn delete_by_drag(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime_ptr: *mut JsContextHost,
    ) -> bool {
        if !self.is_valid(scope, unsafe { &*runtime_ptr }) || !self.select(scope) {
            return false;
        }
        let Some(event) =
            construct_input_event(scope, "beforeinput", TextEditInputType::DeleteByDrag, None)
        else {
            return false;
        };
        let should_delete =
            dispatch_public_event(scope, runtime_ptr, self.host, event).allows_default();
        if !self.is_connected(scope, unsafe { &*runtime_ptr }) {
            return false;
        }
        if should_delete && self.is_valid(scope, unsafe { &*runtime_ptr }) {
            let range = v8::Local::new(scope, &self.range);
            if range_algorithms::range_delete_contents(scope, range).is_none() {
                return false;
            }
            if let Some(event) =
                construct_input_event(scope, "input", TextEditInputType::DeleteByDrag, None)
            {
                let _ = dispatch_public_event(scope, runtime_ptr, self.host, event);
            }
        }
        true
    }
}

pub(crate) fn editable_drag_selection(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source: DomHandle,
) -> Option<DragEditingRange> {
    let runtime = unsafe { &*runtime_ptr };
    let document = runtime.dom_host().owner_document_handle(source)?;
    let selection = runtime.document_selection_snapshot(document)?;
    let host = contenteditable_editing_host(runtime, selection.start.container)?;
    if contenteditable_editing_host(runtime, selection.end.container) != Some(host) {
        return None;
    }
    let (mut start, mut end) = (selection.start, selection.end);
    if crate::range_boundary::point_order_in_dom(
        runtime.dom_host(),
        start.container,
        start.offset,
        end.container,
        end.offset,
    )? == std::cmp::Ordering::Greater
    {
        std::mem::swap(&mut start, &mut end);
    }
    DragEditingRange::new(
        scope,
        document,
        host,
        start.container,
        start.offset,
        end.container,
        end.offset,
    )
}

pub(crate) fn editing_drag_caret(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target: DomHandle,
    x: f64,
    y: f64,
) -> Option<DragEditingRange> {
    let runtime = unsafe { &*runtime_ptr };
    let host = contenteditable_editing_host(runtime, target)?;
    let document = runtime.dom_host().owner_document_handle(target)?;
    let position = observable_caret_position(
        runtime,
        document,
        moli_layout::LayoutPoint::new(x as f32, y as f32),
    )
    .ok()
    .flatten()?;
    if contenteditable_editing_host(runtime, position.source) != Some(host) {
        return None;
    }
    let mut node = position.source;
    let mut offset = u32::try_from(position.utf16_offset.unwrap_or(0)).ok()?;
    if let Some(length) = runtime
        .character_data_utf16_units(node)
        .map(|units| units.len() as u32)
        && (offset == 0 || offset == length)
    {
        let at_start = offset == 0;
        let mut inline = node;
        // At an inline boundary, inserting a fragment outside the completed
        // inline avoids inheriting its formatting into unrelated dragged text.
        // Block boundaries and the editing host remain intact.
        while let Some(parent) = runtime
            .dom_host()
            .parent_node(inline)
            .filter(|parent| *parent != host)
        {
            let children = runtime.dom_host().child_nodes(parent)?;
            let edge = if at_start {
                children.first()
            } else {
                children.last()
            };
            if edge.copied() != Some(inline)
                || crate::native_bridge::element::style_property_value(
                    runtime,
                    parent,
                    crate::native_bridge::element::StyleMode::Computed,
                    "display",
                ) != "inline"
            {
                break;
            }
            inline = parent;
        }
        if inline != node {
            node = runtime.dom_host().parent_node(inline)?;
            offset = u32::try_from(runtime.dom_host().child_index(node, inline)?).ok()?
                + u32::from(!at_start);
        }
    }
    DragEditingRange::new(scope, document, host, node, offset, node, offset)
}

pub(crate) fn drag_copy_spacing(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    host: DomHandle,
) -> (bool, bool) {
    use super::range::{RangeBoundaryHandle, RangeBoundaryHandles};
    let Some(document) = runtime.dom_host().owner_document_handle(host) else {
        return (false, false);
    };
    let Some(selection) = runtime.document_selection_snapshot(document) else {
        return (false, false);
    };
    let Some(children) = runtime.dom_host().child_nodes(host) else {
        return (false, false);
    };
    let before = RangeBoundaryHandles {
        start: RangeBoundaryHandle {
            container: host,
            offset: 0,
        },
        end: RangeBoundaryHandle {
            container: selection.start.container,
            offset: selection.start.offset,
        },
    };
    let after = RangeBoundaryHandles {
        start: RangeBoundaryHandle {
            container: selection.end.container,
            offset: selection.end.offset,
        },
        end: RangeBoundaryHandle {
            container: host,
            offset: children.len() as u32,
        },
    };
    let before = range_algorithms::range_selection_string_contents_for_boundaries(scope, before)
        .and_then(|text| text.chars().next_back());
    let after = range_algorithms::range_selection_string_contents_for_boundaries(scope, after)
        .and_then(|text| text.chars().next());
    (
        before.is_some_and(|ch| !smart_replace_exempt(ch, true)),
        after.is_some_and(|ch| !smart_replace_exempt(ch, false)),
    )
}

fn smart_replace_exempt(ch: char, previous: bool) -> bool {
    use icu_properties::{
        CodePointMapData,
        props::{GeneralCategory, GeneralCategoryGroup},
    };
    // Blink smart_replace_icu.cc: whitespace, CJK, then side-specific
    // punctuation. Inserting a copied range preserves word separation.
    ch.is_whitespace()
        || matches!(ch as u32,0x1100..=0x1200|0x2e80..=0x2fe0|0x2ff0..=0x31c0|0x3200..=0xa4d0|0xac00..=0xd7af|0xf900..=0xfa60|0xfe30..=0xfe50|0xff00..=0xfff0|0x20000..=0x2a6d7|0x2f800..=0x2fa1e)
        || if previous {
            "([\"'#$/-`{".contains(ch)
        } else {
            ")].,;:?'!\"%*-/}".contains(ch)
                || GeneralCategoryGroup::Punctuation
                    .contains(CodePointMapData::<GeneralCategory>::new().get(ch))
        }
}
