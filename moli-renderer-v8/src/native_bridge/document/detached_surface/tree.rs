use super::*;
use crate::dom_parser::map_live_value_to_foreign;
use crate::native_bridge::collections::{
    build_live_child_node_list_for_node, build_live_html_children_collection_for_node,
};

fn map_live_object_to_foreign<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    map_live_value_to_foreign(scope, value)
}

fn map_live_array_to_foreign<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    let length = array
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let mapped = v8::Array::new(scope, length as i32);
    for index in 0..length {
        let Some(value) = array.get_index(scope, index) else {
            continue;
        };
        let value = map_live_value_to_foreign(scope, value);
        let _ = mapped.set_index(scope, index, value);
    }
    install_object_array_item_method(scope, mapped.into());
    mapped
}

pub(in crate::native_bridge) fn bridge_detached_parent_node_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_parent_node_object(scope, node) {
        Some(parent) => rv.set(map_live_object_to_foreign(scope, parent.into())),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_parent_element_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_parent_element_object(scope, node) {
        Some(parent) => rv.set(map_live_object_to_foreign(scope, parent.into())),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_owner_document_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_owner_document_object(scope, node) {
        Some(document) => rv.set(map_live_object_to_foreign(scope, document.into())),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_child_nodes_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, node)
    {
        let list = build_live_child_node_list_for_node(scope, runtime_ptr, handle);
        rv.set(list.into());
        return;
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node)
        && let Some(children) = object_property_value(scope, delegate, "childNodes")
    {
        if let Ok(children) = v8::Local::<v8::Object>::try_from(children) {
            rv.set(map_live_array_to_foreign(scope, children).into());
        } else {
            rv.set(v8::Array::new(scope, 0).into());
        }
        return;
    }
    let legacy_children = detached_child_node_objects(scope, node);
    let legacy_children = build_object_array(scope, &legacy_children);
    detached_replace_children_array(scope, node, legacy_children);
    if let Some(state) = detached_state_object(scope, node)
        && let Some(children) = object_property_value(scope, state, "children")
    {
        rv.set(children);
    } else {
        rv.set(legacy_children.into());
    }
}

pub(in crate::native_bridge) fn bridge_detached_first_child_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    if let Some(children) = detached_native_child_node_objects(scope, node) {
        match children.first().copied() {
            Some(child) => rv.set(child.into()),
            None => rv.set_null(),
        }
        return;
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node)
        && let Some(child) = object_property_value(scope, delegate, "firstChild")
    {
        rv.set(map_live_object_to_foreign(scope, child));
        return;
    }
    let children = detached_child_node_objects(scope, node);
    match children.first().copied() {
        Some(child) => rv.set(child.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_last_child_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    if let Some(children) = detached_native_child_node_objects(scope, node) {
        match children.last().copied() {
            Some(child) => rv.set(child.into()),
            None => rv.set_null(),
        }
        return;
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node)
        && let Some(child) = object_property_value(scope, delegate, "lastChild")
    {
        rv.set(map_live_object_to_foreign(scope, child));
        return;
    }
    let children = detached_child_node_objects(scope, node);
    match children.last().copied() {
        Some(child) => rv.set(child.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_previous_sibling_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_sibling_object(scope, node, -1) {
        Some(sibling) => rv.set(map_live_object_to_foreign(scope, sibling.into())),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_next_sibling_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_sibling_object(scope, node, 1) {
        Some(sibling) => rv.set(map_live_object_to_foreign(scope, sibling.into())),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_is_connected_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(detached_is_connected(scope, node));
}

pub(in crate::native_bridge) fn bridge_detached_has_child_nodes_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_bool(false);
        return;
    };
    if let Some(children) = detached_native_child_node_objects(scope, node) {
        rv.set_bool(!children.is_empty());
        return;
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node) {
        rv.set_bool(!object_child_nodes(scope, delegate).is_empty());
        return;
    }
    rv.set_bool(!detached_child_node_objects(scope, node).is_empty());
}

pub(in crate::native_bridge) fn bridge_detached_children_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, node)
    {
        let collection = build_live_html_children_collection_for_node(scope, runtime_ptr, handle);
        rv.set(collection.into());
        return;
    }
    let children = detached_element_children_objects(scope, node)
        .into_iter()
        .filter_map(|child| {
            let mapped = map_live_object_to_foreign(scope, child.into());
            v8::Local::<v8::Object>::try_from(mapped).ok()
        })
        .collect::<Vec<_>>();
    match build_detached_html_collection(scope, &children) {
        Some(collection) => rv.set(collection.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_first_element_child_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_element_children_objects(scope, node)
        .first()
        .copied()
    {
        Some(child) => rv.set(map_live_object_to_foreign(scope, child.into())),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_last_element_child_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_element_children_objects(scope, node)
        .last()
        .copied()
    {
        Some(child) => rv.set(map_live_object_to_foreign(scope, child.into())),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_child_element_count_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    let count = detached_element_children_objects(scope, node).len() as i32;
    rv.set(v8::Integer::new(scope, count).into());
}

pub(in crate::native_bridge) fn bridge_detached_previous_element_sibling_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_element_sibling_object(scope, node, -1) {
        Some(sibling) => rv.set(map_live_object_to_foreign(scope, sibling.into())),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_next_element_sibling_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_element_sibling_object(scope, node, 1) {
        Some(sibling) => rv.set(map_live_object_to_foreign(scope, sibling.into())),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_text_content_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(text_content) = detached_text_content(scope, node) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(text) = v8_string(scope, &text_content) else {
        rv.set(v8::null(scope).into());
        return;
    };
    rv.set(text.into());
}
