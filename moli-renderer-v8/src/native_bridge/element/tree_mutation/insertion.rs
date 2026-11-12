use crate::{document_runtime::DomHandle, dom::native::Node};

use super::super::super::{
    PageScriptHost,
    node::{
        append_child_in_reaction_scope, insert_before_in_reaction_scope, insertion_document_handle,
    },
};
use super::position::InsertAdjacentPosition;

pub(super) fn insert_adjacent_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    position: InsertAdjacentPosition,
    node: DomHandle,
) -> bool {
    match position {
        InsertAdjacentPosition::BeforeBegin => {
            let Some(parent) = unsafe { &*runtime_ptr }
                .dom_host()
                .node(target)
                .and_then(Node::parent_node)
            else {
                return false;
            };
            insert_before_in_reaction_scope(scope, runtime_ptr, parent, node, Some(target))
        }
        InsertAdjacentPosition::AfterBegin => {
            let reference = unsafe { &*runtime_ptr }
                .dom_host()
                .node(target)
                .and_then(Node::first_child);
            insert_before_in_reaction_scope(scope, runtime_ptr, target, node, reference)
        }
        InsertAdjacentPosition::BeforeEnd => {
            append_child_in_reaction_scope(scope, runtime_ptr, target, node)
        }
        InsertAdjacentPosition::AfterEnd => {
            let Some(parent) = unsafe { &*runtime_ptr }
                .dom_host()
                .node(target)
                .and_then(Node::parent_node)
            else {
                return false;
            };
            let reference = unsafe { &*runtime_ptr }
                .dom_host()
                .node(target)
                .and_then(Node::next_sibling);
            insert_before_in_reaction_scope(scope, runtime_ptr, parent, node, reference)
        }
    }
}

pub(super) fn insert_adjacent_html_fragment_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    position: InsertAdjacentPosition,
    node: DomHandle,
) -> bool {
    match position {
        InsertAdjacentPosition::BeforeBegin => {
            let Some(parent) = unsafe { &*runtime_ptr }
                .dom_host()
                .node(target)
                .and_then(Node::parent_node)
            else {
                return false;
            };
            unsafe { &mut *runtime_ptr }
                .insert_html_fragment_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    node,
                    Some(target),
                )
        }
        InsertAdjacentPosition::AfterBegin => {
            let reference = unsafe { &*runtime_ptr }
                .dom_host()
                .node(target)
                .and_then(Node::first_child);
            unsafe { &mut *runtime_ptr }
                .insert_html_fragment_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    target,
                    node,
                    reference,
                )
        }
        InsertAdjacentPosition::BeforeEnd => unsafe { &mut *runtime_ptr }
            .append_html_fragment_child_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                target,
                node,
            ),
        InsertAdjacentPosition::AfterEnd => {
            let Some(parent) = unsafe { &*runtime_ptr }
                .dom_host()
                .node(target)
                .and_then(Node::parent_node)
            else {
                return false;
            };
            let reference = unsafe { &*runtime_ptr }
                .dom_host()
                .node(target)
                .and_then(Node::next_sibling);
            unsafe { &mut *runtime_ptr }
                .insert_html_fragment_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    node,
                    reference,
                )
        }
    }
}

pub(super) fn insert_adjacent_document_handle(
    runtime: &PageScriptHost,
    target: DomHandle,
    position: InsertAdjacentPosition,
) -> Option<DomHandle> {
    insert_adjacent_context_handle(runtime, target, position)
        .and_then(|context| insertion_document_handle(runtime, context))
}

pub(super) fn insert_adjacent_context_handle(
    runtime: &PageScriptHost,
    target: DomHandle,
    position: InsertAdjacentPosition,
) -> Option<DomHandle> {
    match position {
        InsertAdjacentPosition::BeforeBegin | InsertAdjacentPosition::AfterEnd => {
            runtime.dom_host().node(target).and_then(Node::parent_node)
        }
        InsertAdjacentPosition::AfterBegin | InsertAdjacentPosition::BeforeEnd => Some(target),
    }
}
