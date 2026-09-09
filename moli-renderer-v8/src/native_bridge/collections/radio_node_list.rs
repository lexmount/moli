use super::*;
use crate::util::throw_type_error;

pub(in crate::native_bridge::collections) fn radio_node_list_value_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = args.this();
    if collection_kind_from_object(scope, object) != Some(CollectionKind::RadioNodeList) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Ok((runtime_ptr, descriptor)) = live_collection_descriptor_from_object(scope, object)
    else {
        rv.set_empty_string();
        return;
    };
    if descriptor.collection_kind != CollectionKind::RadioNodeList {
        rv.set_empty_string();
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    for handle in descriptor.resolve(runtime).iter().copied() {
        let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
            continue;
        };
        if !element.is_html_input()
            || element.input_type() != InputType::Radio
            || !element.checked()
        {
            continue;
        }
        if let Some(value) = v8_string(scope, &element.input_value()) {
            rv.set(value.into());
            return;
        }
    }
    rv.set_empty_string();
}

pub(in crate::native_bridge::collections) fn radio_node_list_value_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = args.this();
    if collection_kind_from_object(scope, object) != Some(CollectionKind::RadioNodeList) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(target) = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    let Ok((runtime_ptr, descriptor)) = live_collection_descriptor_from_object(scope, object)
    else {
        return;
    };
    if descriptor.collection_kind != CollectionKind::RadioNodeList {
        return;
    }
    let matching_handle = {
        let runtime = unsafe { &*runtime_ptr };
        descriptor.resolve(runtime).iter().copied().find(|handle| {
            runtime
                .dom_host()
                .node(*handle)
                .and_then(Node::as_element)
                .is_some_and(|element| {
                    element.is_html_input()
                        && element.input_type() == InputType::Radio
                        && element.input_value() == target
                })
        })
    };
    if let Some(handle) = matching_handle {
        let _ = unsafe { &mut *runtime_ptr }.set_checked_state(scope, runtime_ptr, handle, true);
    }
}
