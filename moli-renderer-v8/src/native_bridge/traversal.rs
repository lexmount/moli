mod algorithms;
mod filters;
mod identity;
mod node_iterator;
mod state;
mod tree_walker;
mod wrappers;

// TreeWalker, NodeIterator, and NodeFilter use Blink's NodeWrapInOwnContext
// rule: choose the node's document realm before wrapping a native node.
fn wrapped_traversal_node_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut super::JsContextHost,
    context_node: crate::document_runtime::DomHandle,
    node: crate::document_runtime::DomHandle,
) -> Option<v8::Local<'s, v8::Value>> {
    let document_handle = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(context_node);
    // Look in this world's cache first so an isolated world keeps its own realm.
    let context = document_handle
        .and_then(|document| {
            unsafe { &mut *runtime_ptr }
                .native_bridge_mut()
                .cached_handle_wrapper(scope, document)
        })
        .and_then(|document| document.get_creation_context(scope))
        .or_else(|| {
            super::node::node_owner_document_relevant_context(scope, runtime_ptr, context_node)
        })
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    super::bridge::wrapped_handle_value(scope, runtime_ptr, node)
}

fn set_wrapped_traversal_node_or_null(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    runtime_ptr: *mut super::JsContextHost,
    root: crate::document_runtime::DomHandle,
    node: Option<crate::document_runtime::DomHandle>,
) {
    match node.and_then(|node| wrapped_traversal_node_value(scope, runtime_ptr, root, node)) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(super) use filters::TraversalFilter;
pub(super) use state::{NodeIteratorSnapshot, TraversalStore, TreeWalkerSnapshot};
pub(crate) use wrappers::install_traversal_template_bindings;
pub(super) use wrappers::{
    build_node_iterator_wrapper, build_node_iterator_wrapper_template, build_tree_walker_wrapper,
    build_tree_walker_wrapper_template,
};

pub(in crate::native_bridge::traversal) use node_iterator::{
    node_iterator_detach_callback, node_iterator_filter_getter, node_iterator_next_node_callback,
    node_iterator_pointer_before_reference_node_getter, node_iterator_previous_node_callback,
    node_iterator_reference_node_getter, node_iterator_root_getter,
    node_iterator_what_to_show_getter,
};
pub(in crate::native_bridge::traversal) use tree_walker::{
    tree_walker_current_node_getter, tree_walker_current_node_setter, tree_walker_filter_getter,
    tree_walker_first_child_callback, tree_walker_last_child_callback,
    tree_walker_next_node_callback, tree_walker_next_sibling_callback,
    tree_walker_parent_node_callback, tree_walker_previous_node_callback,
    tree_walker_previous_sibling_callback, tree_walker_root_getter,
    tree_walker_what_to_show_getter,
};
