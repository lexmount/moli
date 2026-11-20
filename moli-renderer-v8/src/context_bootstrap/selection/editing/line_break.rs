use super::{
    DomHandle, NodeType, PageScriptHost, XHTML_NS, collapse_editing_selection,
    fragment_insertion_point, is_text, new_editing_range, node_wrapper_from_handle, range,
    range_insert_node_at_boundary, replace_text_span, selection_value_for_window,
};
use crate::context_bootstrap::range_algorithms::range_delete_contents;
use crate::context_bootstrap::selection::editing_caret::{
    is_collapsible_space, is_replaced_element,
};
use crate::dom::native::{ContentEditableState, Element};
use crate::native_bridge::element::ComputedStyleRead;
use crate::style_engine::{ComputedDisplayKind, ComputedWhiteSpaceCollapseKind};

pub(crate) fn replace_editable_selection_line_break(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    insertion_point_is_editable: impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<bool> {
    let runtime = unsafe { &*runtime_ptr };
    let snapshot = runtime.document_editing_selection(document)?;
    let (start, end) = (snapshot.start, snapshot.end);
    let replacing = start.offset != end.offset;
    if start.container != end.container || replacing && !is_text(runtime, start.container) {
        return None;
    }
    let (start, end) = if start.offset <= end.offset {
        (start, end)
    } else {
        (end, start)
    };
    let window = scope.get_current_context().global(scope);
    let selection = selection_value_for_window(scope, window)?;
    let insertion_range = new_editing_range(scope, document, start, end)?;
    if replacing {
        range_delete_contents(scope, insertion_range)?;
    }
    let start = range::native_range_boundary_handles(scope, insertion_range)?.start;

    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let (container, mut offset) = fragment_insertion_point(runtime, start.container, start.offset);
    if !insertion_point_is_editable(runtime, container) {
        return None;
    }
    let parent = if is_text(runtime, container) {
        dom.parent_node(container)?
    } else {
        container
    };
    let element = dom.node(parent)?.as_element()?;
    if !allows_line_break(element) {
        return Some(replacing);
    }
    if parent == container
        && let Some(previous) = offset
            .checked_sub(1)
            .and_then(|i| dom.child_handles(parent).nth(i as usize))
        && dom
            .node(previous)
            .is_some_and(|node| node.is_html_element_named("br"))
        && following_line_content(runtime, host, parent, offset).is_none()
    {
        // Reuse the existing final break as the caret placeholder.
        offset -= 1;
    }
    let preserves_newlines = matches!(
        white_space(runtime, parent),
        ComputedWhiteSpaceCollapseKind::Preserve
            | ComputedWhiteSpaceCollapseKind::PreserveBreaks
            | ComputedWhiteSpaceCollapseKind::BreakSpaces
    );
    let runtime = unsafe { &mut *runtime_ptr };
    let inserted = if preserves_newlines {
        runtime.create_text_node_for_document(document, "\n")
    } else {
        create_break(runtime, document)
    };
    let container = node_wrapper_from_handle(scope, container)?;
    let line_break = node_wrapper_from_handle(scope, inserted)?;
    range_insert_node_at_boundary(scope, insertion_range, container, offset, line_break)?;

    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let parent = dom.parent_node(inserted)?;
    if !insertion_point_is_editable(runtime, parent) {
        return None;
    }
    let offset = dom.child_index(parent, inserted)? as u32 + 1;
    let (caret, offset) = match following_line_content(runtime, host, parent, offset) {
        Some(next) if is_text(runtime, next) && insertion_point_is_editable(runtime, next) => {
            let data = dom.node(next)?.character_data_value()?;
            let prefix = collapsed_prefix(runtime, dom.parent_node(next)?, &data.utf16_units());
            if prefix != 0 {
                // The leading space would disappear once the suffix starts a
                // new line. Preserve the collapsed space as one NBSP.
                replace_text_span(scope, runtime_ptr, next, 0..prefix as u32, &[0xa0])?;
            }
            (next, 0)
        }
        Some(_) => (parent, offset),
        None => {
            // A terminal break needs a following placeholder to make the
            // blank line reachable without adding a second textual newline.
            let padding = create_break(unsafe { &mut *runtime_ptr }, document);
            let container = node_wrapper_from_handle(scope, parent)?;
            let node = node_wrapper_from_handle(scope, padding)?;
            range_insert_node_at_boundary(scope, insertion_range, container, offset, node)?;
            (parent, offset)
        }
    };
    collapse_editing_selection(scope, selection, caret, offset)?;
    Some(true)
}

fn white_space(runtime: &PageScriptHost, element: DomHandle) -> ComputedWhiteSpaceCollapseKind {
    ComputedStyleRead::new(runtime, element)
        .rendered_style_facts()
        .map_or(ComputedWhiteSpaceCollapseKind::Collapse, |style| {
            style.white_space_collapse
        })
}

fn collapsed_prefix(runtime: &PageScriptHost, parent: DomHandle, units: &[u16]) -> usize {
    let collapses = white_space(runtime, parent);
    units
        .iter()
        .take_while(|&&unit| match collapses {
            ComputedWhiteSpaceCollapseKind::Collapse | ComputedWhiteSpaceCollapseKind::Other => {
                is_collapsible_space(unit)
            }
            ComputedWhiteSpaceCollapseKind::PreserveBreaks => matches!(unit, 0x09 | 0x0c | 0x20),
            ComputedWhiteSpaceCollapseKind::Preserve
            | ComputedWhiteSpaceCollapseKind::BreakSpaces => false,
        })
        .count()
}

fn following_line_content(
    runtime: &PageScriptHost,
    host: DomHandle,
    mut container: DomHandle,
    offset: u32,
) -> Option<DomHandle> {
    let dom = runtime.dom_host();
    let mut next = dom.child_handles(container).nth(offset as usize);
    loop {
        let Some(handle) = next else {
            if container == host
                || !is_inline_container(
                    ComputedStyleRead::new(runtime, container)
                        .rendered_style_facts()?
                        .display,
                )
            {
                return None;
            }
            next = dom.next_sibling(container);
            container = dom.parent_node(container)?;
            continue;
        };
        let node = dom.node(handle)?;
        if let Some(element) = node.as_element() {
            let style = ComputedStyleRead::new(runtime, handle).rendered_style_facts()?;
            if style.display != ComputedDisplayKind::None && !style.hides_contents {
                if element.content_editable_state() == ContentEditableState::False {
                    return None;
                }
                match style.display {
                    ComputedDisplayKind::InlineAtomic | ComputedDisplayKind::InlineTable => {
                        return Some(handle);
                    }
                    ComputedDisplayKind::Inline | ComputedDisplayKind::Contents => {
                        if element.is_html_element("br")
                            || is_replaced_element(element)
                            || element.is_svg_element("svg")
                        {
                            return Some(handle);
                        }
                        container = handle;
                        next = dom.first_child(handle);
                        continue;
                    }
                    _ => return None,
                }
            }
        } else if matches!(node.node_type(), NodeType::Text | NodeType::CDataSection) {
            let data = node.character_data_value()?;
            let units = data.utf16_units();
            if collapsed_prefix(runtime, container, &units) < units.len() {
                return Some(handle);
            }
        }
        next = dom.next_sibling(handle);
    }
}

fn is_inline_container(display: ComputedDisplayKind) -> bool {
    matches!(
        display,
        ComputedDisplayKind::Inline
            | ComputedDisplayKind::InlineAtomic
            | ComputedDisplayKind::Contents
    )
}

fn allows_line_break(element: &Element) -> bool {
    element.namespace() == XHTML_NS
        && !matches!(
            element.local_name(),
            "area"
                | "base"
                | "br"
                | "col"
                | "colgroup"
                | "dl"
                | "embed"
                | "head"
                | "hr"
                | "html"
                | "img"
                | "input"
                | "link"
                | "meta"
                | "ol"
                | "optgroup"
                | "option"
                | "param"
                | "script"
                | "select"
                | "source"
                | "style"
                | "table"
                | "tbody"
                | "template"
                | "textarea"
                | "tfoot"
                | "thead"
                | "title"
                | "tr"
                | "track"
                | "ul"
                | "wbr"
        )
}

fn create_break(runtime: &mut PageScriptHost, document: DomHandle) -> DomHandle {
    runtime
        .dom_host_mut()
        .create_element_without_attributes_for_document(
            document,
            "br".to_owned(),
            XHTML_NS.to_owned(),
            None,
        )
}
