use moli_webapi::window_values::options::parse_start_view_transition_options;

use super::{
    lifecycle::{
        active_view_transition, new_view_transition, reject_view_transition,
        set_active_view_transition, skip_view_transition,
    },
    *,
};
use crate::{dom::native::Node, native_bridge::node_runtime_and_handle_from_object_or_detached};

pub(super) fn document_start_view_transition_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((host_ptr, document_handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_type_error(
            scope,
            "Failed to execute 'startViewTransition' on 'Document': Illegal invocation.",
        );
        return;
    };
    if !unsafe { &*host_ptr }
        .dom_host()
        .node(document_handle)
        .is_some_and(Node::is_document)
    {
        throw_type_error(
            scope,
            "Failed to execute 'startViewTransition' on 'Document': Illegal invocation.",
        );
        return;
    }

    let Some(options) = parse_start_view_transition_options(scope, &args) else {
        return;
    };
    if let Some(active) = active_view_transition(scope, args.this()) {
        skip_view_transition(scope, active);
    }

    let Some(transition) = new_view_transition(scope, args.this(), &options.types) else {
        return;
    };
    let has_live_document = unsafe { &*host_ptr }
        .owner_dispatch_scope_for_node(document_handle)
        .is_some();
    if has_live_document {
        set_active_view_transition(scope, args.this(), transition.into());
    } else {
        skip_view_transition(scope, transition);
    }

    if !unsafe { &mut *host_ptr }.queue_view_transition_update_callback(
        scope,
        document_handle,
        transition,
        options.callback,
    ) {
        let error = new_dom_exception_value(
            scope,
            "The view transition update callback could not be scheduled.",
            "InvalidStateError",
        );
        reject_view_transition(scope, transition, error);
    }
    rv.set(transition.into());
}

pub(super) fn document_active_view_transition_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_type_error(
            scope,
            "Failed to get 'activeViewTransition' on 'Document': Illegal invocation.",
        );
        return;
    };
    if !unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_document)
    {
        throw_type_error(
            scope,
            "Failed to get 'activeViewTransition' on 'Document': Illegal invocation.",
        );
        return;
    }
    rv.set(
        active_view_transition(scope, args.this())
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}
