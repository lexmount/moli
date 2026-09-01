use super::*;
use crate::webidl;

const SHOW_ALL: u32 = u32::MAX;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createNodeIterator")]
struct CreateNodeIteratorArgs<'s> {
    #[webidl(required)]
    root: v8::Local<'s, v8::Value>,
    #[webidl(default = SHOW_ALL)]
    what_to_show: u32,
    #[webidl(index = 2, converter = "callback_interface", nullable)]
    filter: Option<webidl::WebIdlCallbackInterface>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createTreeWalker")]
struct CreateTreeWalkerArgs<'s> {
    #[webidl(required)]
    root: v8::Local<'s, v8::Value>,
    #[webidl(default = SHOW_ALL)]
    what_to_show: u32,
    #[webidl(index = 2, converter = "callback_interface", nullable)]
    filter: Option<webidl::WebIdlCallbackInterface>,
}

pub(in crate::native_bridge) fn node_create_node_iterator_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_null();
        return;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    let Some(parsed) = webidl::parse_args::<CreateNodeIteratorArgs<'s>>(scope, &args) else {
        rv.set_null();
        return;
    };
    let Some(root) =
        node_or_foreign_arg_handle_preserve_detached(scope, runtime_ptr, Some(handle), parsed.root)
    else {
        throw_type_error(
            scope,
            "Failed to execute 'createNodeIterator' on 'Document': parameter 1 is not of type 'Node'.",
        );
        rv.set_null();
        return;
    };
    let relevant_context = crate::native_bridge::node_relevant_context(scope, args.this())
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    let runtime = unsafe { &mut *runtime_ptr };
    let iterator = traversal::build_node_iterator_wrapper(
        target_scope,
        runtime_ptr,
        runtime.native_bridge_mut(),
        root,
        parsed.what_to_show,
        parsed.filter,
    );
    rv.set(iterator.into());
}

pub(in crate::native_bridge) fn node_create_tree_walker_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_null();
        return;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    let Some(parsed) = webidl::parse_args::<CreateTreeWalkerArgs<'s>>(scope, &args) else {
        rv.set_null();
        return;
    };
    let Some(root) =
        node_or_foreign_arg_handle_preserve_detached(scope, runtime_ptr, Some(handle), parsed.root)
    else {
        throw_type_error(
            scope,
            "Failed to execute 'createTreeWalker' on 'Document': parameter 1 is not of type 'Node'.",
        );
        rv.set_null();
        return;
    };
    let relevant_context = crate::native_bridge::node_relevant_context(scope, args.this())
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    let runtime = unsafe { &mut *runtime_ptr };
    let walker = traversal::build_tree_walker_wrapper(
        target_scope,
        runtime_ptr,
        runtime.native_bridge_mut(),
        root,
        parsed.what_to_show,
        parsed.filter,
    );
    rv.set(walker.into());
}
