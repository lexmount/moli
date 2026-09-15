use super::set_wrapped_traversal_node_or_null;
use super::{
    TreeWalkerSnapshot,
    algorithms::{
        first_accepted_descendant, last_accepted_descendant, tree_walker_next_node,
        tree_walker_next_sibling, tree_walker_parent_node, tree_walker_previous_node,
        tree_walker_previous_sibling,
    },
    identity::{TraversalSnapshot, traversal_identity, tree_walker_snapshot_from_object},
};
use crate::native_bridge::{
    bridge::throw_dom_exception, callback_value_dom_handle,
    node::node_or_foreign_arg_handle_preserve_detached,
};
use crate::util::throw_type_error;

fn tree_walker_begin_or_throw(
    scope: &mut v8::PinScope<'_, '_>,
    snapshot: &TraversalSnapshot<TreeWalkerSnapshot>,
) -> bool {
    if unsafe { &mut *snapshot.runtime_ptr }
        .native_bridge_mut()
        .tree_walker_try_begin(snapshot.id)
    {
        return true;
    }
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "TreeWalker filter recursion",
    );
    false
}

fn tree_walker_end(snapshot: &TraversalSnapshot<TreeWalkerSnapshot>) {
    unsafe { &mut *snapshot.runtime_ptr }
        .native_bridge_mut()
        .tree_walker_end(snapshot.id);
}

pub(super) fn tree_walker_root_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        Some(snapshot.state.root),
    );
}

pub(super) fn tree_walker_what_to_show_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    rv.set(v8::Number::new(scope, snapshot.state.what_to_show as f64).into());
}

pub(super) fn tree_walker_filter_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    let Some(filter) = snapshot.state.filter.as_ref() else {
        rv.set_null();
        return;
    };
    rv.set(filter.value(scope));
}

pub(super) fn tree_walker_current_node_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        Some(snapshot.state.current_node),
    );
}

pub(super) fn tree_walker_current_node_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some((runtime_ptr, traversal_id)) = traversal_identity(scope, holder) else {
        return;
    };
    let value = args.get(0);
    let handle = callback_value_dom_handle(scope, value)
        .or_else(|| node_or_foreign_arg_handle_preserve_detached(scope, runtime_ptr, None, value));
    let Some(handle) = handle else {
        throw_type_error(
            scope,
            "Failed to set 'currentNode' on 'TreeWalker': The provided value is not of type 'Node'.",
        );
        return;
    };
    unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .set_tree_walker_current_node(traversal_id, handle);
}

pub(super) fn tree_walker_parent_node_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    if !tree_walker_begin_or_throw(scope, &snapshot) {
        return;
    }
    let Ok(result) = tree_walker_parent_node(scope, snapshot.runtime_ptr, &snapshot.state) else {
        tree_walker_end(&snapshot);
        return;
    };
    if let Some(handle) = result {
        unsafe { &mut *snapshot.runtime_ptr }
            .native_bridge_mut()
            .set_tree_walker_current_node(snapshot.id, handle);
    }
    tree_walker_end(&snapshot);
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        result,
    );
}

pub(super) fn tree_walker_first_child_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    if !tree_walker_begin_or_throw(scope, &snapshot) {
        return;
    }
    let Ok(result) = first_accepted_descendant(
        scope,
        snapshot.runtime_ptr,
        snapshot.state.current_node,
        snapshot.state.what_to_show,
        snapshot.state.filter.as_deref(),
    ) else {
        tree_walker_end(&snapshot);
        return;
    };
    if let Some(handle) = result {
        unsafe { &mut *snapshot.runtime_ptr }
            .native_bridge_mut()
            .set_tree_walker_current_node(snapshot.id, handle);
    }
    tree_walker_end(&snapshot);
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        result,
    );
}

pub(super) fn tree_walker_last_child_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    if !tree_walker_begin_or_throw(scope, &snapshot) {
        return;
    }
    let Ok(result) = last_accepted_descendant(
        scope,
        snapshot.runtime_ptr,
        snapshot.state.current_node,
        snapshot.state.what_to_show,
        snapshot.state.filter.as_deref(),
    ) else {
        tree_walker_end(&snapshot);
        return;
    };
    if let Some(handle) = result {
        unsafe { &mut *snapshot.runtime_ptr }
            .native_bridge_mut()
            .set_tree_walker_current_node(snapshot.id, handle);
    }
    tree_walker_end(&snapshot);
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        result,
    );
}

pub(super) fn tree_walker_next_sibling_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    if !tree_walker_begin_or_throw(scope, &snapshot) {
        return;
    }
    let Ok(result) = tree_walker_next_sibling(scope, snapshot.runtime_ptr, &snapshot.state) else {
        tree_walker_end(&snapshot);
        return;
    };
    if let Some(handle) = result {
        unsafe { &mut *snapshot.runtime_ptr }
            .native_bridge_mut()
            .set_tree_walker_current_node(snapshot.id, handle);
    }
    tree_walker_end(&snapshot);
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        result,
    );
}

pub(super) fn tree_walker_previous_sibling_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    if !tree_walker_begin_or_throw(scope, &snapshot) {
        return;
    }
    let Ok(result) = tree_walker_previous_sibling(scope, snapshot.runtime_ptr, &snapshot.state)
    else {
        tree_walker_end(&snapshot);
        return;
    };
    if let Some(handle) = result {
        unsafe { &mut *snapshot.runtime_ptr }
            .native_bridge_mut()
            .set_tree_walker_current_node(snapshot.id, handle);
    }
    tree_walker_end(&snapshot);
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        result,
    );
}

pub(super) fn tree_walker_next_node_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    if !tree_walker_begin_or_throw(scope, &snapshot) {
        return;
    }
    let Ok(result) = tree_walker_next_node(scope, snapshot.runtime_ptr, &snapshot.state) else {
        tree_walker_end(&snapshot);
        return;
    };
    if let Some(handle) = result {
        unsafe { &mut *snapshot.runtime_ptr }
            .native_bridge_mut()
            .set_tree_walker_current_node(snapshot.id, handle);
    }
    tree_walker_end(&snapshot);
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        result,
    );
}

pub(super) fn tree_walker_previous_node_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = tree_walker_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    if !tree_walker_begin_or_throw(scope, &snapshot) {
        return;
    }
    let Ok(result) = tree_walker_previous_node(scope, snapshot.runtime_ptr, &snapshot.state) else {
        tree_walker_end(&snapshot);
        return;
    };
    if let Some(handle) = result {
        unsafe { &mut *snapshot.runtime_ptr }
            .native_bridge_mut()
            .set_tree_walker_current_node(snapshot.id, handle);
    }
    tree_walker_end(&snapshot);
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        result,
    );
}
