use super::helpers::{
    character_data_string, character_data_utf16_units, require_argument_count,
    utf16_index_value_or_throw,
};
use super::*;
use crate::{native_bridge::document, util::utf16_len, webidl};
use moli_dom::native::NodeType;

pub(in crate::native_bridge) fn node_whole_text_utf16_units_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<Vec<u16>> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return None;
    };
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let node = dom.node(handle)?;
    let is_text = |node: &Node| matches!(node.node_type(), NodeType::Text | NodeType::CDataSection);
    if !is_text(node) {
        return None;
    }
    let mut units = Vec::new();
    let mut start = handle;
    while let Some(prev_id) = dom.node(start).and_then(|n| n.prev_sibling()) {
        if dom.node(prev_id).is_some_and(is_text) {
            start = prev_id;
        } else {
            break;
        }
    }
    let mut current = Some(start);
    while let Some(h) = current {
        let Some(n) = dom.node(h) else { break };
        if !is_text(n) {
            break;
        }
        units.extend(character_data_utf16_units(runtime, h)?);
        current = n.next_sibling();
    }
    Some(units)
}

pub(in crate::native_bridge) fn node_split_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_argument_count(scope, &args, "Text", "splitText", 1) {
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if !runtime
        .dom_host()
        .node(handle)
        .is_some_and(|node| matches!(node.node_type(), NodeType::Text | NodeType::CDataSection))
    {
        throw_named_error(
            scope,
            "TypeError",
            "Text.splitText requires a Text node",
            None,
        );
        return;
    }
    let Some(data) = character_data_string(runtime, handle) else {
        rv.set_undefined();
        return;
    };
    let Some(offset) = utf16_index_value_or_throw(
        scope,
        args.get(0),
        utf16_len(&data),
        webidl::Context::argument("Text", 1),
    ) else {
        return;
    };
    let Some(new_handle) = runtime.split_text(scope, runtime_ptr, handle, offset, &data) else {
        rv.set_undefined();
        return;
    };
    document::detached_record_tree_mutation(scope, args.this());
    set_wrapped_handle_or_null_for_receiver(
        scope,
        &mut rv,
        runtime_ptr,
        args.this(),
        Some(new_handle),
    );
}
