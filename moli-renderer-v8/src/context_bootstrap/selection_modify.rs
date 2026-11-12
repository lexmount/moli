use super::selection::{first_text_descendant, last_text_descendant, text_length};
use super::*;
use crate::document_runtime::DomHandle;
use crate::dom::native::NodeType;
use crate::native_bridge::callback_value_dom_handle;
use crate::native_bridge::element::contenteditable_editing_host;
use crate::util::{context_host_ptr_from_global_bridge, node_wrapper_from_handle};

pub(super) fn selection_modify_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    offset: u32,
    direction: &str,
    granularity: &str,
) -> Option<(v8::Local<'s, v8::Object>, u32)> {
    let direction = normalized_modify_direction(direction)?;
    if let Some(result) =
        selection_modify_target_in_editing_host(scope, node, offset, direction, granularity)
    {
        return result;
    }
    let node_type = object_number_property(scope, node, "nodeType")? as u32;
    if node_type == 3 {
        let text = object_string_property(scope, node, "textContent").unwrap_or_default();
        let len = text.chars().count() as u32;
        let next_offset = match (direction, granularity) {
            ("forward", "character") if offset < len => offset + 1,
            ("backward", "character") if offset > 0 => offset - 1,
            ("forward", "word") => next_word_forward(&text, offset as usize) as u32,
            ("backward", "word") => next_word_backward(&text, offset as usize) as u32,
            _ => {
                return selection_modify_target_from_element_boundary(
                    scope, node, offset, direction,
                );
            }
        };
        return Some((node, next_offset.min(len)));
    }
    selection_modify_target_from_element_boundary(scope, node, offset, direction)
}

fn normalized_modify_direction(direction: &str) -> Option<&'static str> {
    match direction {
        "forward" | "right" => Some("forward"),
        "backward" | "left" => Some("backward"),
        _ => None,
    }
}

fn selection_modify_target_in_editing_host<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    offset: u32,
    direction: &str,
    granularity: &str,
) -> Option<Option<(v8::Local<'s, v8::Object>, u32)>> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let node_handle = callback_value_dom_handle(scope, node.into())?;
    let target = {
        let runtime = unsafe { &*host_ptr };
        let editing_host = contenteditable_editing_host(runtime, node_handle)?;
        native_selection_modify_target_in_editing_host(
            runtime,
            editing_host,
            node_handle,
            offset,
            direction,
            granularity,
        )
    };
    Some(target.and_then(|(handle, offset)| {
        node_wrapper_from_handle(scope, handle).map(|wrapper| (wrapper, offset))
    }))
}

fn native_selection_modify_target_in_editing_host(
    runtime: &PageScriptHost,
    editing_host: DomHandle,
    node: DomHandle,
    offset: u32,
    direction: &str,
    granularity: &str,
) -> Option<(DomHandle, u32)> {
    if runtime
        .dom_host()
        .node(node)
        .is_some_and(|node| node.node_type() == NodeType::Text)
    {
        let text = runtime.dom_host().text_content(node).unwrap_or_default();
        let len = text.chars().count() as u32;
        match (direction, granularity) {
            ("forward", "line" | "paragraph") => {
                let next =
                    next_editable_text_position_after_boundary(runtime, editing_host, node, len)?;
                let next_len = runtime
                    .dom_host()
                    .text_content(next)
                    .unwrap_or_default()
                    .chars()
                    .count() as u32;
                return Some((next, offset.min(next_len)));
            }
            ("backward", "line" | "paragraph") => {
                let previous = previous_editable_text_position_before_boundary(
                    runtime,
                    editing_host,
                    node,
                    0,
                )?;
                let previous_len = runtime
                    .dom_host()
                    .text_content(previous)
                    .unwrap_or_default()
                    .chars()
                    .count() as u32;
                return Some((previous, offset.min(previous_len)));
            }
            ("forward", "character") if offset < len => return Some((node, offset + 1)),
            ("backward", "character") if offset > 0 => return Some((node, offset - 1)),
            ("forward", "word") => {
                let next = next_word_forward(&text, offset as usize) as u32;
                if next != offset.min(len) {
                    return Some((node, next.min(len)));
                }
            }
            ("backward", "word") => {
                let next = next_word_backward(&text, offset as usize) as u32;
                if next != offset.min(len) {
                    return Some((node, next.min(len)));
                }
            }
            _ => {}
        }
    }
    native_selection_modify_target_from_editing_boundary(
        runtime,
        editing_host,
        node,
        offset,
        direction,
    )
}

fn native_selection_modify_target_from_editing_boundary(
    runtime: &PageScriptHost,
    editing_host: DomHandle,
    node: DomHandle,
    offset: u32,
    direction: &str,
) -> Option<(DomHandle, u32)> {
    match direction {
        "forward" => {
            next_editable_text_position_after_boundary(runtime, editing_host, node, offset)
                .map(|text| (text, 0))
        }
        "backward" => {
            previous_editable_text_position_before_boundary(runtime, editing_host, node, offset)
                .map(|text| {
                    let len = runtime
                        .dom_host()
                        .text_content(text)
                        .unwrap_or_default()
                        .chars()
                        .count() as u32;
                    (text, len)
                })
        }
        _ => None,
    }
}

fn next_editable_text_position_after_boundary(
    runtime: &PageScriptHost,
    editing_host: DomHandle,
    node: DomHandle,
    offset: u32,
) -> Option<DomHandle> {
    if !runtime
        .dom_host()
        .node(node)
        .is_some_and(|node| node.node_type() == NodeType::Text)
        && let Some(child) = child_handle_at_offset(runtime, node, offset)
        && let Some(text) = first_editable_text_in_following_siblings(runtime, editing_host, child)
    {
        return Some(text);
    }

    let mut current = node;
    while current != editing_host {
        let next = runtime.dom_host().next_sibling(current);
        if let Some(next) = next
            && let Some(text) =
                first_editable_text_in_following_siblings(runtime, editing_host, next)
        {
            return Some(text);
        }
        current = runtime.dom_host().parent_node(current)?;
    }
    None
}

fn previous_editable_text_position_before_boundary(
    runtime: &PageScriptHost,
    editing_host: DomHandle,
    node: DomHandle,
    offset: u32,
) -> Option<DomHandle> {
    if !runtime
        .dom_host()
        .node(node)
        .is_some_and(|node| node.node_type() == NodeType::Text)
        && offset > 0
        && let Some(child) = child_handle_at_offset(runtime, node, offset - 1)
        && let Some(text) = last_editable_text_in_preceding_siblings(runtime, editing_host, child)
    {
        return Some(text);
    }

    let mut current = node;
    while current != editing_host {
        let previous = runtime
            .dom_host()
            .node(current)
            .and_then(|node| node.prev_sibling());
        if let Some(previous) = previous
            && let Some(text) =
                last_editable_text_in_preceding_siblings(runtime, editing_host, previous)
        {
            return Some(text);
        }
        current = runtime.dom_host().parent_node(current)?;
    }
    None
}

fn first_editable_text_in_following_siblings(
    runtime: &PageScriptHost,
    editing_host: DomHandle,
    first: DomHandle,
) -> Option<DomHandle> {
    let mut current = Some(first);
    while let Some(candidate) = current {
        if let Some(text) = first_editable_text_descendant(runtime, editing_host, candidate) {
            return Some(text);
        }
        current = runtime.dom_host().next_sibling(candidate);
    }
    None
}

fn last_editable_text_in_preceding_siblings(
    runtime: &PageScriptHost,
    editing_host: DomHandle,
    first: DomHandle,
) -> Option<DomHandle> {
    let mut current = Some(first);
    while let Some(candidate) = current {
        if let Some(text) = last_editable_text_descendant(runtime, editing_host, candidate) {
            return Some(text);
        }
        current = runtime
            .dom_host()
            .node(candidate)
            .and_then(|node| node.prev_sibling());
    }
    None
}

fn first_editable_text_descendant(
    runtime: &PageScriptHost,
    editing_host: DomHandle,
    node: DomHandle,
) -> Option<DomHandle> {
    if !node_belongs_to_editing_host(runtime, editing_host, node) {
        return None;
    }
    if runtime
        .dom_host()
        .node(node)
        .is_some_and(|node| node.node_type() == NodeType::Text)
    {
        return Some(node);
    }
    for child in runtime.dom_host().child_handles(node) {
        if let Some(text) = first_editable_text_descendant(runtime, editing_host, child) {
            return Some(text);
        }
    }
    None
}

fn last_editable_text_descendant(
    runtime: &PageScriptHost,
    editing_host: DomHandle,
    node: DomHandle,
) -> Option<DomHandle> {
    if !node_belongs_to_editing_host(runtime, editing_host, node) {
        return None;
    }
    if runtime
        .dom_host()
        .node(node)
        .is_some_and(|node| node.node_type() == NodeType::Text)
    {
        return Some(node);
    }
    let children = runtime.dom_host().child_handles(node).collect::<Vec<_>>();
    for child in children.into_iter().rev() {
        if let Some(text) = last_editable_text_descendant(runtime, editing_host, child) {
            return Some(text);
        }
    }
    None
}

fn node_belongs_to_editing_host(
    runtime: &PageScriptHost,
    editing_host: DomHandle,
    node: DomHandle,
) -> bool {
    contenteditable_editing_host(runtime, node) == Some(editing_host)
}

fn child_handle_at_offset(
    runtime: &PageScriptHost,
    node: DomHandle,
    offset: u32,
) -> Option<DomHandle> {
    runtime
        .dom_host()
        .child_handles(node)
        .nth(usize::try_from(offset).ok()?)
}

fn selection_modify_target_from_element_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    offset: u32,
    direction: &str,
) -> Option<(v8::Local<'s, v8::Object>, u32)> {
    match direction {
        "forward" => next_text_position_after_boundary(scope, node, offset),
        "backward" => previous_text_position_before_boundary(scope, node, offset),
        _ => None,
    }
}

fn next_word_forward(text: &str, offset: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut index = offset.min(chars.len());
    while index < chars.len() && chars[index].is_whitespace() {
        index += 1;
    }
    while index < chars.len() && !chars[index].is_whitespace() {
        index += 1;
    }
    index
}

fn next_word_backward(text: &str, offset: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut index = offset.min(chars.len());
    while index > 0 && chars[index - 1].is_whitespace() {
        index -= 1;
    }
    while index > 0 && !chars[index - 1].is_whitespace() {
        index -= 1;
    }
    index
}

fn next_text_position_after_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    offset: u32,
) -> Option<(v8::Local<'s, v8::Object>, u32)> {
    let child_nodes = object_property_as_object(scope, node, "childNodes");
    if let Some(child_nodes) = child_nodes {
        let length = object_number_property(scope, child_nodes, "length").unwrap_or(0.0) as u32;
        if offset < length
            && let Some(child) = child_nodes.get_index(scope, offset)
            && let Ok(child) = v8::Local::<v8::Object>::try_from(child)
            && let Some(text) = first_text_descendant(scope, child)
        {
            return Some((text, 1.min(text_length(scope, text))));
        }
    }
    next_text_node_after(scope, node).map(|text| (text, 1.min(text_length(scope, text))))
}

fn previous_text_position_before_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    offset: u32,
) -> Option<(v8::Local<'s, v8::Object>, u32)> {
    let child_nodes = object_property_as_object(scope, node, "childNodes");
    if let Some(child_nodes) = child_nodes
        && offset > 0
        && let Some(child) = child_nodes.get_index(scope, offset - 1)
        && let Ok(child) = v8::Local::<v8::Object>::try_from(child)
        && let Some(text) = last_text_descendant(scope, child)
    {
        let len = text_length(scope, text);
        return Some((text, len.saturating_sub(1)));
    }
    previous_text_node_before(scope, node).map(|text| {
        let len = text_length(scope, text);
        (text, len.saturating_sub(1))
    })
}

fn next_text_node_after<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut current = node;
    loop {
        if let Some(next_sibling) = object_property_as_object(scope, current, "nextSibling") {
            return first_text_descendant(scope, next_sibling)
                .or_else(|| next_text_node_after(scope, next_sibling));
        }
        current = object_property_as_object(scope, current, "parentNode")?;
    }
}

fn previous_text_node_before<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut current = node;
    loop {
        if let Some(prev_sibling) = object_property_as_object(scope, current, "previousSibling") {
            return last_text_descendant(scope, prev_sibling)
                .or_else(|| previous_text_node_before(scope, prev_sibling));
        }
        current = object_property_as_object(scope, current, "parentNode")?;
    }
}
