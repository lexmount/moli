use crate::{
    dom::native::NodeType,
    native_bridge::{
        document,
        node::{
            node_runtime_and_handle_from_args, receiver_has_detached_state,
            throw_incompatible_getter_receiver, throw_incompatible_setter_receiver,
        },
    },
    webidl,
};
use moli_webapi_declare::WebApiFunctionTemplate;

use super::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CharacterData", enumerable)]
struct CharacterDataPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = character_data_data_getter_callback,
        setter = character_data_data_setter_callback
    )]
    data: (),
    #[webapi(accessor_property, getter = character_data_length_getter_callback)]
    length: (),
    #[webapi(method, length = 2, callback = character_data_substring_data_callback)]
    substring_data: (),
    #[webapi(method, length = 1, callback = character_data_append_data_callback)]
    append_data: (),
    #[webapi(method, length = 2, callback = character_data_insert_data_callback)]
    insert_data: (),
    #[webapi(method, length = 2, callback = character_data_delete_data_callback)]
    delete_data: (),
    #[webapi(method, length = 3, callback = character_data_replace_data_callback)]
    replace_data: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Text", enumerable)]
struct TextPrototypeDeclaration {
    #[webapi(accessor_property, getter = text_whole_text_getter_callback)]
    whole_text: (),
    #[webapi(method, length = 1, callback = text_split_text_callback)]
    split_text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ProcessingInstruction", enumerable)]
struct ProcessingInstructionPrototypeDeclaration {
    #[webapi(accessor_property, getter = processing_instruction_target_getter_callback)]
    target: (),
}

fn receiver_is_live_node<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: &v8::FunctionCallbackArguments<'a>,
) -> bool {
    node_runtime_and_handle_from_args(scope, args).is_ok()
}

fn receiver_is_live_character_data<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: &v8::FunctionCallbackArguments<'a>,
) -> bool {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, args) else {
        return false;
    };
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .is_some_and(|node| {
            matches!(
                node.node_type(),
                NodeType::Text
                    | NodeType::CDataSection
                    | NodeType::Comment
                    | NodeType::ProcessingInstruction
            )
        })
}

fn character_data_append_data_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    rv: v8::ReturnValue<'a, v8::Value>,
) {
    if receiver_is_live_node(scope, &args) {
        node_append_data_callback(scope, args, rv);
    } else {
        document::detached_character_data_append_data_callback(scope, args, rv);
    }
}

fn character_data_delete_data_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    rv: v8::ReturnValue<'a, v8::Value>,
) {
    if receiver_is_live_node(scope, &args) {
        node_delete_data_callback(scope, args, rv);
    } else {
        document::detached_character_data_delete_data_callback(scope, args, rv);
    }
}

fn character_data_insert_data_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    rv: v8::ReturnValue<'a, v8::Value>,
) {
    if receiver_is_live_node(scope, &args) {
        node_insert_data_callback(scope, args, rv);
    } else {
        document::detached_character_data_insert_data_callback(scope, args, rv);
    }
}

fn character_data_replace_data_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    rv: v8::ReturnValue<'a, v8::Value>,
) {
    if receiver_is_live_node(scope, &args) {
        node_replace_data_callback(scope, args, rv);
    } else {
        document::detached_character_data_replace_data_callback(scope, args, rv);
    }
}

fn character_data_substring_data_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    rv: v8::ReturnValue<'a, v8::Value>,
) {
    if receiver_is_live_node(scope, &args) {
        node_substring_data_callback(scope, args, rv);
    } else {
        document::detached_character_data_substring_data_callback(scope, args, rv);
    }
}

fn text_split_text_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    rv: v8::ReturnValue<'a, v8::Value>,
) {
    if receiver_is_live_node(scope, &args) {
        node_split_text_callback(scope, args, rv);
    } else {
        document::detached_text_split_text_callback(scope, args, rv);
    }
}

fn character_data_data_getter_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if receiver_is_live_character_data(scope, &args) {
        let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this())
        else {
            rv.set_undefined();
            return;
        };
        let Some(units) =
            super::helpers::character_data_utf16_units(unsafe { &*runtime_ptr }, handle)
        else {
            rv.set_undefined();
            return;
        };
        super::helpers::set_utf16_return_value(scope, &mut rv, &units);
        return;
    }
    if !receiver_has_detached_state(scope, args.this()) {
        throw_incompatible_getter_receiver(scope, "CharacterData", "data");
        return;
    }
    let value = Some(document::detached_character_data_value(scope, args.this()));
    let Some(value) = value else {
        rv.set_undefined();
        return;
    };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn character_data_data_setter_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if receiver_is_live_character_data(scope, &args) {
        let Some(value) = super::helpers::dom_string_utf16_value_or_throw(
            scope,
            args.get(0),
            webidl::Context::member("CharacterData", "data"),
            true,
        ) else {
            return;
        };
        let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this())
        else {
            return;
        };
        let runtime = unsafe { &mut *runtime_ptr };
        let removed_count = runtime
            .character_data_utf16_units(handle)
            .map(|units| units.len() as u32)
            .unwrap_or(0);
        let inserted_count = value.len() as u32;
        let _ = runtime.set_character_data_utf16_units(scope, runtime_ptr, handle, &value);
        crate::context_bootstrap::live_ranges_character_data_reset(
            scope,
            handle,
            removed_count,
            inserted_count,
        );
    } else if receiver_has_detached_state(scope, args.this()) {
        let removed_count = document::detached_character_data_length(scope, args.this()) as u32;
        let value = if args.get(0).is_null() {
            String::new()
        } else {
            let Some(value) = args.get(0).to_string(scope) else {
                rv.set_undefined();
                return;
            };
            value.to_rust_string_lossy(scope)
        };
        let inserted_count = value.encode_utf16().count() as u32;
        document::set_detached_character_data_value(scope, args.this(), &value);
        crate::context_bootstrap::live_ranges_detached_character_data_reset(
            scope,
            args.this(),
            removed_count,
            inserted_count,
        );
    } else {
        throw_incompatible_setter_receiver(scope, "CharacterData", "data");
        return;
    }
    rv.set_undefined();
}

fn character_data_length_getter_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let length = if receiver_is_live_node(scope, &args) {
        node_character_data_length_from_object(scope, args.this())
    } else {
        document::detached_character_data_length(scope, args.this())
    };
    rv.set(v8::Integer::new(scope, length).into());
}

fn text_whole_text_getter_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = if receiver_is_live_node(scope, &args) {
        node_whole_text_value_from_object(scope, args.this())
    } else {
        document::detached_text_whole_text_value(scope, args.this())
    };
    let Some(value) = value else {
        rv.set_undefined();
        return;
    };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn processing_instruction_target_getter_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value =
        if let Some(value) = document::detached_processing_instruction_target(scope, args.this()) {
            value
        } else {
            let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this())
            else {
                webidl::throw_type_error(
                    scope,
                    "ProcessingInstruction.target getter called on incompatible receiver.",
                );
                rv.set_undefined();
                return;
            };
            let Some(value) = (unsafe { &*runtime_ptr })
                .dom_host()
                .node(handle)
                .and_then(Node::target)
                .map(str::to_owned)
            else {
                webidl::throw_type_error(
                    scope,
                    "ProcessingInstruction.target getter called on incompatible receiver.",
                );
                rv.set_undefined();
                return;
            };
            value
        };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(crate) fn install_character_data_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "CharacterData" => {
            CharacterDataPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "Text" => {
            TextPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "ProcessingInstruction" => {
            ProcessingInstructionPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}
