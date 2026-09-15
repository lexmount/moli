use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.getElementById")]
struct DocumentGetElementByIdArgs {
    #[webidl(required)]
    element_id: String,
}

pub(in crate::native_bridge) fn bridge_document_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let bridge = args.holder();
    let Ok(runtime_ptr) = runtime_ptr_from_object(scope, bridge) else {
        rv.set_null();
        return;
    };
    let handle = unsafe { &*runtime_ptr }.dom_host().document_handle();
    let runtime = unsafe { &mut *runtime_ptr };
    match runtime
        .native_bridge_mut()
        .wrap_handle_for_receiver(scope, runtime_ptr, bridge, handle)
    {
        Some(document) => rv.set(document.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_get_element_by_id_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentGetElementByIdArgs>(scope, &args) else {
        return;
    };
    let bridge = args.this();
    let Ok(runtime_ptr) = runtime_ptr_from_object(scope, bridge) else {
        rv.set_null();
        return;
    };
    let handle = unsafe { &*runtime_ptr }.get_element_by_id(&parsed.element_id);
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, runtime_ptr, bridge, handle);
}

pub(in crate::native_bridge) fn node_get_element_by_id_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_null();
        return;
    };
    let receiver_is_detached =
        super::super::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some();
    let Some(parsed) = webidl::parse_args::<DocumentGetElementByIdArgs>(scope, &args) else {
        return;
    };
    if parsed.element_id.is_empty() {
        rv.set_null();
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let result = if node_is_document(runtime, handle) {
        runtime
            .dom_host()
            .element_handle_by_id_in_subtree(handle, &parsed.element_id)
    } else {
        find_element_by_id_in_subtree(runtime, handle, &parsed.element_id)
    };
    if receiver_is_detached {
        match result.and_then(|handle| {
            super::super::detached_native_object_for_handle(scope, runtime_ptr, handle)
        }) {
            Some(node) => rv.set(node.into()),
            None => rv.set_null(),
        }
        return;
    }
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, runtime_ptr, args.this(), result);
}

fn find_element_by_id_in_subtree(
    runtime: &JsContextHost,
    root: DomHandle,
    id: &str,
) -> Option<DomHandle> {
    let host = runtime.dom_host();
    let node = host.node(root)?;
    if node
        .as_element()
        .is_some_and(|element| element.id().is_some_and(|candidate| candidate == id))
    {
        return Some(root);
    }
    for child in node.child_ids(host.dom()) {
        if let Some(found) = find_element_by_id_in_subtree(runtime, child, id) {
            return Some(found);
        }
    }
    None
}
