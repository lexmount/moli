use super::ordering::point_order_handles;
use super::*;
use crate::native_bridge::document::XHTML_NS;
use crate::native_bridge::element::{
    StyleMode, observable_sources_with_fragments, style_property_value,
};
use crate::util::string_from_utf16_units_lossy;
use std::{cmp::Ordering, collections::HashSet};

pub(in crate::context_bootstrap) fn range_string_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if let Some(boundaries) = native_range_boundary_handles(scope, range) {
        return range_string_contents_handle(
            scope,
            boundaries.start.container,
            boundaries.start.offset,
            boundaries.end.container,
            boundaries.end.offset,
        );
    }

    let start = range_boundary_container_object(scope, range, RangeBoundarySide::Start)?;
    let end = range_boundary_container_object(scope, range, RangeBoundarySide::End)?;
    let start_offset = range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32;
    let end_offset = range_boundary_offset(scope, range, RangeBoundarySide::End) as u32;
    let start = node_handle_for_tree_op(scope, start)?;
    let end = node_handle_for_tree_op(scope, end)?;
    range_string_contents_handle(scope, start, start_offset, end, end_offset)
}

pub(in crate::context_bootstrap) fn range_selection_string_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let (start, start_offset, end, end_offset) =
        if let Some(boundaries) = native_range_boundary_handles(scope, range) {
            (
                boundaries.start.container,
                boundaries.start.offset,
                boundaries.end.container,
                boundaries.end.offset,
            )
        } else {
            let start = range_boundary_container_object(scope, range, RangeBoundarySide::Start)?;
            let end = range_boundary_container_object(scope, range, RangeBoundarySide::End)?;
            (
                node_handle_for_tree_op(scope, start)?,
                range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32,
                node_handle_for_tree_op(scope, end)?,
                range_boundary_offset(scope, range, RangeBoundarySide::End) as u32,
            )
        };
    if start == end && start_offset == end_offset {
        return Some(String::new());
    }

    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let runtime = unsafe { &*host_ptr };
    let root = common_ancestor_handle(scope, start, end)?;
    let mut text_sources = Vec::new();
    collect_selection_text_sources(runtime, root, &mut text_sources);
    let document = runtime.layout_document_for_source(start)?;
    let rendered_text_sources = match observable_sources_with_fragments(
        runtime,
        document,
        &text_sources,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    ) {
        Ok(rendered) => rendered,
        Err(error) => {
            let message = format!("Layout failed while serializing Selection: {error}");
            if let Some(message) = crate::util::v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            return None;
        }
    };
    let root_state = SelectionTextState {
        active_modal_dialog: selection_text_active_modal_dialog(runtime, document),
        ..Default::default()
    };
    let mut out = String::new();
    append_selection_rendered_text(
        scope,
        runtime,
        root,
        RangeTextBounds {
            start,
            start_offset,
            end,
            end_offset,
        },
        root_state,
        &rendered_text_sources,
        &mut out,
    )?;
    Some(out)
}

fn collect_selection_text_sources(
    runtime: &crate::native_bridge::JsContextHost,
    root: DomHandle,
    out: &mut Vec<DomHandle>,
) {
    if runtime
        .dom_host()
        .node(root)
        .is_some_and(|node| matches!(node.node_type(), NodeType::Text | NodeType::CDataSection))
    {
        out.push(root);
    }
    for child in runtime.dom_host().child_handles(root) {
        collect_selection_text_sources(runtime, child, out);
    }
}

fn range_string_contents_handle(
    scope: &mut v8::PinScope<'_, '_>,
    start: DomHandle,
    start_offset: u32,
    end: DomHandle,
    end_offset: u32,
) -> Option<String> {
    if start == end {
        let node_type = node_type_for_handle(scope, start)?;
        return match node_type {
            NodeType::Text | NodeType::CDataSection => {
                slice_character_data_handle(scope, start, start_offset, end_offset)
            }
            NodeType::Comment | NodeType::ProcessingInstruction => Some(String::new()),
            _ => {
                let mut out = String::new();
                for child in child_handles_between_offsets(scope, start, start_offset, end_offset)?
                {
                    append_text_content_handle(scope, child, &mut out)?;
                }
                Some(out)
            }
        };
    }

    let mut out = String::new();
    for handle in text_nodes_in_tree_order_between(scope, start, end) {
        let data = character_data_string_handle(scope, handle)?;
        let slice = if handle == start {
            range_slice_utf16_string(
                &data,
                start_offset as usize,
                character_data_utf16_units_handle(scope, handle)?.len(),
            )
        } else if handle == end {
            range_slice_utf16_string(&data, 0, end_offset as usize)
        } else {
            data
        };
        out.push_str(&slice);
    }
    Some(out)
}

fn append_text_content_handle(
    scope: &mut v8::PinScope<'_, '_>,
    node: DomHandle,
    out: &mut String,
) -> Option<()> {
    match node_type_for_handle(scope, node)? {
        NodeType::Text | NodeType::CDataSection => {
            out.push_str(&character_data_string_handle(scope, node)?);
        }
        NodeType::Comment | NodeType::ProcessingInstruction => {}
        _ => {
            let length = range_node_length_handle(scope, node)?;
            for child in child_handles_between_offsets(scope, node, 0, length)? {
                append_text_content_handle(scope, child, out)?;
            }
        }
    }
    Some(())
}

#[derive(Clone, Copy, Default)]
struct SelectionTextState {
    active_modal_dialog: Option<DomHandle>,
    suppressed_by_user_select: bool,
    in_visible_script_or_style: bool,
}

#[derive(Clone, Copy)]
struct RangeTextBounds {
    start: DomHandle,
    start_offset: u32,
    end: DomHandle,
    end_offset: u32,
}

fn append_selection_rendered_text(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
    bounds: RangeTextBounds,
    state: SelectionTextState,
    rendered_text_sources: &HashSet<DomHandle>,
    out: &mut String,
) -> Option<()> {
    let node_type = node_type_for_handle(scope, handle)?;
    match node_type {
        NodeType::Text | NodeType::CDataSection => {
            append_selected_character_data(
                scope,
                handle,
                bounds,
                state,
                rendered_text_sources,
                out,
            )?;
        }
        NodeType::Comment | NodeType::ProcessingInstruction | NodeType::DocumentType => {}
        _ => {
            let Some(node) = runtime.dom_host().node(handle) else {
                return Some(());
            };
            let mut next_state = state;
            let mut preformatted_block = false;
            if let Some(element) = node.as_element() {
                if selection_text_element_is_hidden(runtime, handle) {
                    return Some(());
                }
                if selection_text_element_is_inert(runtime, handle)
                    && !state.active_modal_dialog.is_some_and(|dialog| {
                        selection_text_descendant_or_self(runtime, dialog, handle)
                    })
                {
                    return Some(());
                }
                if element.is_html_element("head")
                    || element.is_html_element("noscript")
                    || element.is_html_element("template")
                {
                    return Some(());
                }
                let is_script_or_style =
                    element.is_html_element("script") || element.is_inline_style_element();
                next_state.in_visible_script_or_style = is_script_or_style;
                let user_select = selection_text_user_select_value(runtime, handle);
                if user_select.as_deref() == Some("none")
                    && !state.suppressed_by_user_select
                    && out.ends_with(' ')
                {
                    out.push(' ');
                }
                next_state.suppressed_by_user_select = match user_select.as_deref() {
                    Some("none") => true,
                    Some("text" | "all") => false,
                    _ => state.suppressed_by_user_select,
                };
                preformatted_block = element.is_html_element("pre");
            }
            if preformatted_block && selection_node_may_intersect_range(scope, handle, bounds)? {
                append_newline_allowing_double(out);
            }
            let length = range_node_length_handle(scope, handle)?;
            for child in child_handles_between_offsets(scope, handle, 0, length)? {
                append_selection_rendered_text(
                    scope,
                    runtime,
                    child,
                    bounds,
                    next_state,
                    rendered_text_sources,
                    out,
                )?;
            }
        }
    }
    Some(())
}

fn append_selected_character_data(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
    bounds: RangeTextBounds,
    state: SelectionTextState,
    rendered_text_sources: &HashSet<DomHandle>,
    out: &mut String,
) -> Option<()> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let runtime = unsafe { &*host_ptr };
    if state.suppressed_by_user_select || selection_text_is_inert(runtime, handle, state) {
        return Some(());
    }
    let data = selected_character_data_slice(scope, handle, bounds)?;
    if data.is_empty() {
        return Some(());
    }
    if !rendered_text_sources.contains(&handle) && !data.chars().all(char::is_whitespace) {
        return Some(());
    }
    if state.in_visible_script_or_style {
        append_collapsed_trimmed_text(out, &data);
    } else {
        append_rendered_text_node(out, &data);
    }
    Some(())
}

fn selected_character_data_slice(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
    bounds: RangeTextBounds,
) -> Option<String> {
    let len = character_data_utf16_units_handle(scope, handle)?.len() as u32;
    if point_order_handles(scope, handle, 0, bounds.end, bounds.end_offset)? != Ordering::Less {
        return Some(String::new());
    }
    if point_order_handles(scope, handle, len, bounds.start, bounds.start_offset)?
        != Ordering::Greater
    {
        return Some(String::new());
    }
    let start = if handle == bounds.start {
        bounds.start_offset.min(len)
    } else {
        0
    };
    let end = if handle == bounds.end {
        bounds.end_offset.min(len)
    } else {
        len
    };
    if start >= end {
        return Some(String::new());
    }
    slice_character_data_handle(scope, handle, start, end)
}

fn selection_node_may_intersect_range(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
    bounds: RangeTextBounds,
) -> Option<bool> {
    let length = range_node_length_handle(scope, handle)?;
    let starts_before_range_end =
        point_order_handles(scope, handle, 0, bounds.end, bounds.end_offset)? == Ordering::Less;
    let ends_after_range_start =
        point_order_handles(scope, handle, length, bounds.start, bounds.start_offset)?
            == Ordering::Greater;
    Some(starts_before_range_end && ends_after_range_start)
}

fn selection_text_element_is_hidden(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
) -> bool {
    style_property_value(runtime, handle, StyleMode::Computed, "display") == "none"
        || selection_text_content_visibility_value(runtime, handle).as_deref() == Some("hidden")
}

fn selection_text_element_is_inert(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .is_some_and(|element| {
            element.namespace() == XHTML_NS && element.attribute("inert").is_some()
        })
}

fn selection_text_has_inert_ancestor(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
    modal_escape_root: Option<DomHandle>,
) -> bool {
    let mut current = Some(handle);
    while let Some(handle) = current {
        if selection_text_element_is_inert(runtime, handle) {
            return true;
        }
        if Some(handle) == modal_escape_root {
            return false;
        }
        current = selection_text_flat_tree_parent(runtime, handle);
    }
    false
}

fn selection_text_active_modal_dialog(
    runtime: &crate::native_bridge::JsContextHost,
    document: DomHandle,
) -> Option<DomHandle> {
    runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .rev()
        .find_map(|node| {
            if !node.is_connected()
                || runtime.dom_host().owner_document_handle(node.id()) != Some(document)
            {
                return None;
            }
            let element = node.as_element()?;
            (element.is_html_element("dialog")
                && element.dialog_modal()
                && element.attribute("open").is_some())
            .then_some(node.id())
        })
}

fn selection_text_is_inert(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
    state: SelectionTextState,
) -> bool {
    let modal_escape_root = match state.active_modal_dialog {
        Some(dialog) if selection_text_descendant_or_self(runtime, handle, dialog) => Some(dialog),
        Some(_) => return true,
        None => None,
    };
    selection_text_has_inert_ancestor(runtime, handle, modal_escape_root)
}

fn selection_text_descendant_or_self(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
    ancestor: DomHandle,
) -> bool {
    let mut current = Some(handle);
    while let Some(handle) = current {
        if handle == ancestor {
            return true;
        }
        current = selection_text_flat_tree_parent(runtime, handle);
    }
    false
}

fn selection_text_flat_tree_parent(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    if let Some(slot) = runtime.dom_host().assigned_slot_for_node(handle) {
        return Some(slot);
    }
    let parent = runtime.dom_host().parent_node(handle)?;
    if runtime.dom_host().is_shadow_root(parent) {
        return runtime.dom_host().shadow_root_host(parent);
    }
    Some(parent)
}

fn selection_text_user_select_value(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let value = inline_css_property_value(runtime, handle, "user-select")
        .or_else(|| inline_css_property_value(runtime, handle, "-webkit-user-select"))
        .unwrap_or_else(|| {
            style_property_value(runtime, handle, StyleMode::Computed, "user-select")
        });
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Some("none".to_owned()),
        "text" => Some("text".to_owned()),
        "all" => Some("all".to_owned()),
        _ => None,
    }
}

fn selection_text_content_visibility_value(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let value =
        inline_css_property_value(runtime, handle, "content-visibility").unwrap_or_else(|| {
            style_property_value(runtime, handle, StyleMode::Computed, "content-visibility")
        });
    match value.trim().to_ascii_lowercase().as_str() {
        "hidden" => Some("hidden".to_owned()),
        _ => None,
    }
}

fn inline_css_property_value(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let style = runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .and_then(|element| element.attribute("style"))?;
    crate::css_style::css_declaration_list_property_value(style, property)
}

fn append_collapsed_trimmed_text(out: &mut String, text: &str) {
    let normalized = collapse_whitespace(text).trim().to_owned();
    if !normalized.is_empty() {
        out.push_str(&normalized);
    }
}

fn append_rendered_text_node(out: &mut String, text: &str) {
    if text.chars().all(char::is_whitespace) {
        if text.contains('\n') {
            append_newline_collapsed(out);
        } else if !out.ends_with([' ', '\n']) {
            out.push(' ');
        }
        return;
    }

    let first_non_ws = text.find(|ch: char| !ch.is_whitespace()).unwrap_or(0);
    let last_non_ws = text
        .rfind(|ch: char| !ch.is_whitespace())
        .map(|index| index + text[index..].chars().next().unwrap().len_utf8())
        .unwrap_or(text.len());
    let leading = &text[..first_non_ws];
    let trailing = &text[last_non_ws..];
    let body = collapse_whitespace(&text[first_non_ws..last_non_ws]);

    if leading.chars().any(char::is_whitespace) && !out.is_empty() && !out.ends_with([' ', '\n']) {
        out.push(' ');
    }
    out.push_str(&body);
    if trailing.chars().any(char::is_whitespace) {
        out.push(' ');
    }
}

fn collapse_whitespace(text: &str) -> String {
    let mut normalized = String::new();
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && !normalized.is_empty() {
            normalized.push(' ');
        }
        pending_space = false;
        normalized.push(ch);
    }
    normalized
}

fn append_newline_collapsed(out: &mut String) {
    if !out.ends_with('\n') {
        out.push('\n');
    }
}

fn append_newline_allowing_double(out: &mut String) {
    if out.is_empty() {
        return;
    }
    if out.ends_with("\n\n") {
        return;
    }
    out.push('\n');
}

fn slice_character_data_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
    start: u32,
    end: u32,
) -> Option<String> {
    let data = character_data_string_handle(scope, handle)?;
    Some(range_slice_utf16_string(
        &data,
        start as usize,
        end as usize,
    ))
}

fn character_data_string_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> Option<String> {
    let units = character_data_utf16_units_handle(scope, handle)?;
    Some(string_from_utf16_units_lossy(&units))
}

fn text_nodes_in_tree_order_between(
    scope: &mut v8::PinScope<'_, '_>,
    start: DomHandle,
    end: DomHandle,
) -> Vec<DomHandle> {
    let mut out = Vec::new();
    let mut current = Some(start);
    while let Some(handle) = current {
        if matches!(
            node_type_for_handle(scope, handle),
            Some(NodeType::Text | NodeType::CDataSection)
        ) {
            out.push(handle);
        }
        if handle == end {
            break;
        }
        current = next_node_in_tree_order(scope, handle);
    }
    out
}

fn next_node_in_tree_order(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> Option<DomHandle> {
    if let Some(first_child) = child_handle_at_offset_optional(scope, handle, 0)? {
        return Some(first_child);
    }

    let mut current = handle;
    loop {
        if let Some(next_sibling) = next_sibling_handle(scope, current) {
            return Some(next_sibling);
        }
        current = parent_handle(scope, current)?;
    }
}
