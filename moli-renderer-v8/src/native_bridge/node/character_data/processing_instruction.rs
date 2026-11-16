use moli_dom::native::{DomStringValue, ProcessingInstructionAttributes};

use super::*;
use crate::native_bridge::{document, node::node_runtime_and_handle_from_args_or_detached};
use crate::util::v8_string_from_utf16_units;
use crate::webidl;

fn receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(*mut PageScriptHost, DomHandle)> {
    node_runtime_and_handle_from_args_or_detached(scope, args).ok()
}

fn argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    operation: &'static str,
    index: i32,
) -> Option<DomStringValue> {
    match webidl::convert::<webidl::DomString16>(
        scope,
        args.get(index),
        webidl::Context::argument(operation, (index + 1) as usize),
    ) {
        Ok(value) => Some(DomStringValue::from_utf16(&value.0)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn name_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    operation: &'static str,
    required: i32,
) -> Option<DomStringValue> {
    if !helpers::require_argument_count(scope, args, "ProcessingInstruction", operation, required) {
        return None;
    }
    argument(scope, args, operation, 0)
}

fn validate_name(scope: &mut v8::PinScope<'_, '_>, name: &DomStringValue) -> bool {
    if crate::native_bridge::validate_attribute_name(name.as_str_lossy()) {
        return true;
    }
    throw_dom_exception(
        scope,
        "InvalidCharacterError",
        5,
        "The attribute name contains an invalid character.",
    );
    false
}

fn attributes(
    host: &PageScriptHost,
    handle: DomHandle,
) -> Option<&ProcessingInstructionAttributes> {
    host.dom_host()
        .node(handle)?
        .as_processing_instruction()
        .map(|pi| pi.attributes())
}

fn replace_attributes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: *mut PageScriptHost,
    handle: DomHandle,
    object: v8::Local<'s, v8::Object>,
    attributes: ProcessingInstructionAttributes,
) {
    let runtime = unsafe { &mut *host };
    let old_length = runtime
        .character_data_utf16_units(handle)
        .map_or(0, |units| units.len() as u32);
    if runtime.set_processing_instruction_attributes(scope, host, handle, attributes) {
        document::detached_record_tree_mutation(scope, object);
    }
    let new_length = runtime
        .character_data_utf16_units(handle)
        .map_or(0, |units| units.len() as u32);
    crate::context_bootstrap::live_ranges_character_data_reset(
        scope, handle, old_length, new_length,
    );
}

pub(super) fn processing_instruction_has_attributes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some((host, handle)) = receiver(scope, &args) else {
        return;
    };
    rv.set_bool(
        attributes(unsafe { &*host }, handle).is_some_and(|attributes| !attributes.is_empty()),
    );
}

pub(super) fn processing_instruction_get_attribute_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some((host, handle)) = receiver(scope, &args) else {
        return;
    };
    let Some(attributes) = attributes(unsafe { &*host }, handle) else {
        return;
    };
    let mut values = Vec::new();
    for name in attributes.keys() {
        let Some(name) = v8_string_from_utf16_units(scope, &name.utf16_units()) else {
            return;
        };
        values.push(name.into());
    }
    rv.set(v8::Array::new_with_elements(scope, &values).into());
}

pub(super) fn processing_instruction_get_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(name) = name_argument(scope, &args, "getAttribute", 1) else {
        return;
    };
    let Some((host, handle)) = receiver(scope, &args) else {
        return;
    };
    let value = attributes(unsafe { &*host }, handle).and_then(|attributes| attributes.get(&name));
    if let Some(value) = value {
        helpers::set_utf16_return_value(scope, &mut rv, &value.utf16_units());
    } else {
        rv.set_null();
    }
}

pub(super) fn processing_instruction_has_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(name) = name_argument(scope, &args, "hasAttribute", 1) else {
        return;
    };
    let Some((host, handle)) = receiver(scope, &args) else {
        return;
    };
    rv.set_bool(
        attributes(unsafe { &*host }, handle)
            .is_some_and(|attributes| attributes.get(&name).is_some()),
    );
}

pub(super) fn processing_instruction_set_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(name) = name_argument(scope, &args, "setAttribute", 2) else {
        return;
    };
    let Some(value) = argument(scope, &args, "setAttribute", 1) else {
        return;
    };
    if !validate_name(scope, &name) {
        return;
    }
    let Some((host, handle)) = receiver(scope, &args) else {
        return;
    };
    let Some(mut attributes) = attributes(unsafe { &*host }, handle).cloned() else {
        return;
    };
    attributes.set(name, value);
    replace_attributes(scope, host, handle, args.this(), attributes);
}

pub(super) fn processing_instruction_remove_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(name) = name_argument(scope, &args, "removeAttribute", 1) else {
        return;
    };
    let Some((host, handle)) = receiver(scope, &args) else {
        return;
    };
    let Some(mut attributes) = attributes(unsafe { &*host }, handle).cloned() else {
        return;
    };
    if attributes.remove(&name) {
        replace_attributes(scope, host, handle, args.this(), attributes);
    }
}

pub(super) fn processing_instruction_toggle_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(name) = name_argument(scope, &args, "toggleAttribute", 1) else {
        return;
    };
    let force = (!args.get(1).is_undefined()).then(|| args.get(1).boolean_value(scope));
    if !validate_name(scope, &name) {
        return;
    }
    let Some((host, handle)) = receiver(scope, &args) else {
        return;
    };
    let Some(mut attributes) = attributes(unsafe { &*host }, handle).cloned() else {
        return;
    };
    let present = attributes.get(&name).is_some();
    let next = force.unwrap_or(!present);
    if present != next {
        if next {
            attributes.set(name, DomStringValue::default());
        } else {
            attributes.remove(&name);
        }
        replace_attributes(scope, host, handle, args.this(), attributes);
    }
    rv.set_bool(next);
}
