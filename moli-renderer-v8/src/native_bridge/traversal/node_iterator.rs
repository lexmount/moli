use super::set_wrapped_traversal_node_or_null;
use super::{
    algorithms::{node_iterator_next_node, node_iterator_previous_node},
    identity::node_iterator_snapshot_from_object,
};
use crate::native_bridge::bridge::throw_dom_exception;

pub(super) fn node_iterator_root_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = node_iterator_snapshot_from_object(scope, holder) else {
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

pub(super) fn node_iterator_what_to_show_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = node_iterator_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    rv.set(v8::Number::new(scope, snapshot.state.what_to_show as f64).into());
}

pub(super) fn node_iterator_filter_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = node_iterator_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    let Some(filter) = snapshot.state.filter.as_ref() else {
        rv.set_null();
        return;
    };
    rv.set(filter.value(scope));
}

pub(super) fn node_iterator_reference_node_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = node_iterator_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        Some(snapshot.state.reference_node),
    );
}

pub(super) fn node_iterator_pointer_before_reference_node_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = node_iterator_snapshot_from_object(scope, holder) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(snapshot.state.pointer_before_reference_node);
}

pub(super) fn node_iterator_next_node_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = node_iterator_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    if !unsafe { &mut *snapshot.runtime_ptr }
        .native_bridge_mut()
        .node_iterator_try_begin(snapshot.id)
    {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "NodeIterator filter recursion",
        );
        return;
    }
    let Ok((result, reference_node, pointer_before_reference_node)) =
        node_iterator_next_node(scope, snapshot.runtime_ptr, &snapshot.state)
    else {
        unsafe { &mut *snapshot.runtime_ptr }
            .native_bridge_mut()
            .node_iterator_end(snapshot.id);
        return;
    };
    let bridge = unsafe { &mut *snapshot.runtime_ptr }.native_bridge_mut();
    bridge.set_node_iterator_position(snapshot.id, reference_node, pointer_before_reference_node);
    bridge.node_iterator_end(snapshot.id);
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        result,
    );
}

pub(super) fn node_iterator_previous_node_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some(snapshot) = node_iterator_snapshot_from_object(scope, holder) else {
        rv.set_null();
        return;
    };
    if !unsafe { &mut *snapshot.runtime_ptr }
        .native_bridge_mut()
        .node_iterator_try_begin(snapshot.id)
    {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "NodeIterator filter recursion",
        );
        return;
    }
    let Ok((result, reference_node, pointer_before_reference_node)) =
        node_iterator_previous_node(scope, snapshot.runtime_ptr, &snapshot.state)
    else {
        unsafe { &mut *snapshot.runtime_ptr }
            .native_bridge_mut()
            .node_iterator_end(snapshot.id);
        return;
    };
    let bridge = unsafe { &mut *snapshot.runtime_ptr }.native_bridge_mut();
    bridge.set_node_iterator_position(snapshot.id, reference_node, pointer_before_reference_node);
    bridge.node_iterator_end(snapshot.id);
    set_wrapped_traversal_node_or_null(
        scope,
        &mut rv,
        snapshot.runtime_ptr,
        snapshot.state.root,
        result,
    );
}

pub(super) fn node_iterator_detach_callback(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}
