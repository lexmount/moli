use crate::{context_bootstrap, native_bridge::document, webidl};

use super::helpers::{
    character_data_utf16_units, dom_string_utf16_value_or_throw, set_utf16_return_value,
};
use super::*;

#[derive(crate::webidl::WebIdlArgs)]
#[webidl(prefix = "CharacterData.deleteData")]
struct CharacterDataDeleteDataArgs {
    #[webidl(
        required,
        missing_message = "Failed to execute 'deleteData' on 'CharacterData': 2 arguments required, but only 0 present."
    )]
    offset: u32,
    #[webidl(
        required,
        missing_message = "Failed to execute 'deleteData' on 'CharacterData': 2 arguments required, but only 1 present."
    )]
    count: u32,
}

#[derive(crate::webidl::WebIdlArgs)]
#[webidl(prefix = "CharacterData.substringData")]
struct CharacterDataSubstringDataArgs {
    #[webidl(
        required,
        missing_message = "Failed to execute 'substringData' on 'CharacterData': 2 arguments required, but only 0 present."
    )]
    offset: u32,
    #[webidl(
        required,
        missing_message = "Failed to execute 'substringData' on 'CharacterData': 2 arguments required, but only 1 present."
    )]
    count: u32,
}

pub(in crate::native_bridge) fn node_append_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_argument_count(scope, &args, "CharacterData", "appendData", 1) {
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        return;
    };
    let Some(mut next) = character_data_utf16_units(unsafe { &*runtime_ptr }, handle) else {
        return;
    };
    let Some(data) = dom_string_utf16_value_or_throw(
        scope,
        args.get(0),
        webidl::Context::argument("CharacterData", 1),
        false,
    ) else {
        return;
    };
    next.extend_from_slice(&data);
    let runtime = unsafe { &mut *runtime_ptr };
    if runtime.set_character_data_utf16_units_for_edit(scope, runtime_ptr, handle, &next) {
        document::detached_record_tree_mutation(scope, args.this());
    }
}

pub(in crate::native_bridge) fn node_delete_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = crate::webidl::parse_args::<CharacterDataDeleteDataArgs>(scope, &args)
    else {
        return;
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        return;
    };
    let Some(units) = character_data_utf16_units(unsafe { &*runtime_ptr }, handle) else {
        return;
    };
    let Some(start) = checked_utf16_offset(scope, parsed.offset, units.len()) else {
        return;
    };
    let count = parsed.count as usize;
    let end = start.saturating_add(count).min(units.len());
    let mut next = Vec::with_capacity(units.len().saturating_sub(end - start));
    next.extend_from_slice(&units[..start]);
    next.extend_from_slice(&units[end..]);
    let runtime = unsafe { &mut *runtime_ptr };
    if runtime.set_character_data_utf16_units_for_edit(scope, runtime_ptr, handle, &next) {
        document::detached_record_tree_mutation(scope, args.this());
    }
    context_bootstrap::live_ranges_character_data_edit(
        scope,
        handle,
        start as u32,
        (end - start) as u32,
        0,
    );
}

pub(in crate::native_bridge) fn node_insert_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_argument_count(scope, &args, "CharacterData", "insertData", 2) {
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        return;
    };
    let Some(units) = character_data_utf16_units(unsafe { &*runtime_ptr }, handle) else {
        return;
    };
    let Some(start) = utf16_index_value_or_throw(
        scope,
        args.get(0),
        units.len(),
        webidl::Context::argument("CharacterData", 1),
    ) else {
        return;
    };
    let Some(insert) = dom_string_utf16_value_or_throw(
        scope,
        args.get(1),
        webidl::Context::argument("CharacterData", 2),
        false,
    ) else {
        return;
    };
    let mut next = Vec::with_capacity(units.len() + insert.len());
    next.extend_from_slice(&units[..start]);
    next.extend_from_slice(&insert);
    next.extend_from_slice(&units[start..]);
    let runtime = unsafe { &mut *runtime_ptr };
    if runtime.set_character_data_utf16_units_for_edit(scope, runtime_ptr, handle, &next) {
        document::detached_record_tree_mutation(scope, args.this());
    }
    context_bootstrap::live_ranges_character_data_edit(
        scope,
        handle,
        start as u32,
        0,
        insert.len() as u32,
    );
}

pub(in crate::native_bridge) fn node_replace_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_argument_count(scope, &args, "CharacterData", "replaceData", 3) {
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        return;
    };
    let Some(units) = character_data_utf16_units(unsafe { &*runtime_ptr }, handle) else {
        return;
    };
    let Some(start) = utf16_index_value_or_throw(
        scope,
        args.get(0),
        units.len(),
        webidl::Context::argument("CharacterData", 1),
    ) else {
        return;
    };
    let Some(count) = utf16_count_value(
        scope,
        args.get(1),
        webidl::Context::argument("CharacterData", 2),
    ) else {
        return;
    };
    let end = start.saturating_add(count).min(units.len());
    let Some(replacement) = dom_string_utf16_value_or_throw(
        scope,
        args.get(2),
        webidl::Context::argument("CharacterData", 3),
        false,
    ) else {
        return;
    };
    let mut next = Vec::with_capacity(units.len() + replacement.len());
    next.extend_from_slice(&units[..start]);
    next.extend_from_slice(&replacement);
    next.extend_from_slice(&units[end..]);
    let runtime = unsafe { &mut *runtime_ptr };
    if runtime.set_character_data_utf16_units_for_edit(scope, runtime_ptr, handle, &next) {
        document::detached_record_tree_mutation(scope, args.this());
    }
    context_bootstrap::live_ranges_character_data_edit(
        scope,
        handle,
        start as u32,
        (end - start) as u32,
        replacement.len() as u32,
    );
}

pub(in crate::native_bridge) fn node_substring_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = crate::webidl::parse_args::<CharacterDataSubstringDataArgs>(scope, &args)
    else {
        return;
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        return;
    };
    let Some(units) = character_data_utf16_units(unsafe { &*runtime_ptr }, handle) else {
        return;
    };
    let Some(start) = checked_utf16_offset(scope, parsed.offset, units.len()) else {
        return;
    };
    let count = parsed.count as usize;
    let end = start.saturating_add(count).min(units.len());
    set_utf16_return_value(scope, &mut rv, &units[start..end]);
}

fn checked_utf16_offset(
    scope: &mut v8::PinScope<'_, '_>,
    offset: u32,
    len: usize,
) -> Option<usize> {
    let offset = offset as usize;
    if offset > len {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "Index or size is negative or greater than the allowed amount",
        );
        return None;
    }
    Some(offset)
}
