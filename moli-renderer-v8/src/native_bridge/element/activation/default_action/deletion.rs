//! Resolve a logical editing boundary before deciding what Delete removes.
//! This reads native DOM and computed style, without refreshing layout.
use super::*;
use crate::native_bridge::element::{StyleMode, style_property_value};
use crate::native_bridge::node::{append_child_in_reaction_scope, remove_child_in_reaction_scope};

#[derive(Clone, Copy)]
pub(super) enum BoundaryDeletion {
    Character {
        node: DomHandle,
        start: u32,
        end: u32,
    },
    JoinParagraphs {
        left: DomHandle,
        right: DomHandle,
        parent: DomHandle,
    },
}

impl BoundaryDeletion {
    pub(super) fn capture_root(self) -> DomHandle {
        match self {
            Self::Character { node, .. } => node,
            Self::JoinParagraphs { parent, .. } => parent,
        }
    }
}

fn display(runtime: &PageScriptHost, node: DomHandle) -> String {
    style_property_value(runtime, node, StyleMode::Computed, "display")
}

fn flow_block(runtime: &PageScriptHost, node: DomHandle) -> bool {
    matches!(display(runtime, node).as_str(), "block" | "flow-root")
}

fn closest_block(
    runtime: &PageScriptHost,
    host: DomHandle,
    mut node: DomHandle,
) -> Option<DomHandle> {
    loop {
        if node == host {
            return Some(node);
        }
        let current = runtime.dom_host().node(node)?;
        if current.as_element().is_some()
            && !matches!(
                display(runtime, node).as_str(),
                "inline" | "contents" | "none" | ""
            )
        {
            return Some(node);
        }
        node = current.parent_node()?;
    }
}

fn hidden_subtree(runtime: &PageScriptHost, node: DomHandle) -> bool {
    display(runtime, node) == "none"
        || style_property_value(runtime, node, StyleMode::Computed, "content-visibility")
            == "hidden"
}

pub(super) fn resolve_boundary_deletion(
    runtime: &PageScriptHost,
    host: DomHandle,
    from: DomHandle,
    backward: bool,
) -> Option<BoundaryDeletion> {
    let original_block = closest_block(runtime, host, from)?;
    let mut current = from;
    loop {
        let sibling = loop {
            let node = runtime.dom_host().node(current)?;
            if let Some(sibling) = if backward {
                node.prev_sibling()
            } else {
                node.next_sibling()
            } {
                break sibling;
            }
            current = node.parent_node()?;
            if current == host {
                return None;
            }
        };
        current = sibling;
        loop {
            let node = runtime.dom_host().node(current)?;
            // A hidden subtree is crossed as a whole by the deletion Range;
            // its character data must never become the character to delete.
            if node.as_element().is_some() && hidden_subtree(runtime, current) {
                break;
            }
            if contenteditable_editing_host(runtime, current) != Some(host)
                || node.as_element().is_some_and(|element| {
                    !crate::native_bridge::element::focus::can_descend_into_editable_content(
                        element,
                    )
                })
            {
                return None;
            }
            if let Some(text) = node.as_text() {
                let parent = node.parent_node()?;
                if matches!(
                    style_property_value(runtime, parent, StyleMode::Computed, "visibility")
                        .as_str(),
                    "hidden" | "collapse"
                ) {
                    break;
                }
                if text.data().is_empty() {
                    break;
                }
                let adjacent_block = closest_block(runtime, host, current)?;
                if adjacent_block != original_block {
                    // Formatting whitespace between block siblings does not
                    // represent an editable character at the paragraph end.
                    if text
                        .data()
                        .chars()
                        .all(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
                    {
                        break;
                    }
                    let (left, right) = if backward {
                        (adjacent_block, original_block)
                    } else {
                        (original_block, adjacent_block)
                    };
                    let parent = runtime.dom_host().node(left)?.parent_node()?;
                    if left == host
                        || right == host
                        || !flow_block(runtime, left)
                        || !flow_block(runtime, right)
                        || runtime.dom_host().node(right)?.parent_node() != Some(parent)
                    {
                        return None;
                    }
                    return Some(BoundaryDeletion::JoinParagraphs {
                        left,
                        right,
                        parent,
                    });
                }
                let units = utf16_units(text.data());
                let (start, end) = if backward {
                    (
                        crate::util::utf16_previous_scalar_boundary(&units, units.len()),
                        units.len(),
                    )
                } else {
                    (0, crate::util::utf16_next_scalar_boundary(&units, 0))
                };
                return Some(BoundaryDeletion::Character {
                    node: current,
                    start: start as u32,
                    end: end as u32,
                });
            }
            let child = if backward {
                node.last_child()
            } else {
                node.first_child()
            };
            let Some(child) = child else { break };
            current = child;
        }
    }
}

pub(super) fn apply_boundary_deletion<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    ptr: *mut PageScriptHost,
    target: v8::Local<'s, v8::Object>,
    selection: v8::Local<'s, v8::Object>,
    caret: (v8::Local<'s, v8::Object>, u32),
    operation: BoundaryDeletion,
    backward: bool,
) -> bool {
    let changed = match operation {
        BoundaryDeletion::Character { node, start, end } => {
            let Some(node) = node_wrapper_from_handle(scope, node) else {
                return false;
            };
            let Some(range) = selected_dom_range(scope, target, selection) else {
                return false;
            };
            let (start_node, start_offset, end_node, end_offset) = if backward {
                (node, start, caret.0, caret.1)
            } else {
                (caret.0, caret.1, node, end)
            };
            let start_offset = v8::Integer::new_from_unsigned(scope, start_offset);
            let end_offset = v8::Integer::new_from_unsigned(scope, end_offset);
            call_object_method(
                scope,
                range,
                "setStart",
                &[start_node.into(), start_offset.into()],
            )
            .is_some()
                && call_object_method(
                    scope,
                    range,
                    "setEnd",
                    &[end_node.into(), end_offset.into()],
                )
                .is_some()
                && call_object_method(scope, range, "deleteContents", &[]).is_some()
        }
        BoundaryDeletion::JoinParagraphs {
            left,
            right,
            parent,
        } => crate::custom_elements::with_custom_element_reaction_scope(scope, ptr, |scope| {
            let mut sibling = unsafe { &*ptr }
                .dom_host()
                .node(left)
                .and_then(Node::next_sibling);
            while let Some(node) = sibling {
                if node == right {
                    break;
                }
                sibling = unsafe { &*ptr }
                    .dom_host()
                    .node(node)
                    .and_then(Node::next_sibling);
                if !remove_child_in_reaction_scope(scope, ptr, parent, node) {
                    return false;
                }
            }
            let children = unsafe { &*ptr }
                .dom_host()
                .child_handles(right)
                .collect::<Vec<_>>();
            for child in children {
                if !append_child_in_reaction_scope(scope, ptr, left, child) {
                    return false;
                }
            }
            remove_child_in_reaction_scope(scope, ptr, parent, right)
        }),
    };
    if changed {
        collapse_selection_to(scope, selection, caret.0, caret.1);
    }
    changed
}
