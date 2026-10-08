use super::*;

pub(in crate::native_bridge) fn bridge_detached_node_type_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    let node_type = detached_node_type(scope, node).unwrap_or(0);
    rv.set(v8::Integer::new(scope, node_type).into());
}

pub(in crate::native_bridge) fn bridge_detached_node_name_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let node_name = detached_node_name_value(scope, node).unwrap_or_default();
    if let Some(value) = crate::util::v8_string_from_dom_string_value(scope, &node_name) {
        rv.set(value.into());
    }
}

pub(in crate::native_bridge) fn bridge_detached_node_value_getter_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_state_kind(scope, node).as_deref() {
        Some("text" | "comment" | "cdataSection" | "processingInstruction") => {
            let data = detached_character_data_value(scope, node);
            set_string_return_value(scope, &mut rv, &data);
        }
        _ => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_node_value_setter_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let Some(kind) = detached_state_kind(scope, node) else {
        return;
    };
    if !matches!(
        kind.as_str(),
        "text" | "comment" | "cdataSection" | "processingInstruction"
    ) {
        return;
    }
    if detached_state_object(scope, node).is_none() {
        return;
    }
    let raw_value = args.get(1);
    let value = if raw_value.is_null_or_undefined() {
        String::new()
    } else {
        match crate::webidl::convert::<crate::webidl::DomString>(
            scope,
            raw_value,
            crate::webidl::Context::member("Node", "nodeValue"),
        ) {
            Ok(value) => value.0,
            Err(error) => {
                crate::webidl::throw_error(scope, &error);
                return;
            }
        }
    };
    let removed_count = crate::util::utf16_len(&detached_character_data_value(scope, node)) as u32;
    let inserted_count = crate::util::utf16_len(&value) as u32;
    set_detached_character_data_value(scope, node, &value);
    crate::context_bootstrap::live_ranges_detached_character_data_reset(
        scope,
        node,
        removed_count,
        inserted_count,
    );
}

pub(in crate::native_bridge) fn bridge_detached_doctype_name_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let name = detached_doctype_name(scope, node).unwrap_or_default();
    if let Some(value) = crate::util::v8_string_from_dom_string_value(scope, &name) {
        rv.set(value.into());
    }
}

pub(in crate::native_bridge) fn bridge_detached_doctype_public_id_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let public_id = detached_doctype_public_id(scope, node).unwrap_or_default();
    if let Some(value) = crate::util::v8_string_from_dom_string_value(scope, &public_id) {
        rv.set(value.into());
    }
}

pub(in crate::native_bridge) fn bridge_detached_doctype_system_id_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let system_id = detached_doctype_system_id(scope, node).unwrap_or_default();
    if let Some(value) = crate::util::v8_string_from_dom_string_value(scope, &system_id) {
        rv.set(value.into());
    }
}

pub(in crate::native_bridge) fn bridge_detached_processing_instruction_target_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let target = detached_processing_instruction_target(scope, node).unwrap_or_default();
    set_string_return_value(scope, &mut rv, &target);
}
