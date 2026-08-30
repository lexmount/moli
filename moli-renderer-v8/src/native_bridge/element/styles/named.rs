use crate::util::v8_string;

use super::super::property_string_value;
use super::declaration::{
    StyleMode, cssom_style_property_affected_names_with_pdb,
    expand_unresolved_box_shorthand_entries_for_mutation, is_style_intrinsic_name,
    parse_style_property_entries_for_cssom_fallback_write, resolve_known_style_property_name,
    resolve_style_property_name, set_inline_style_property_with_pdb_storage,
    set_style_entries_if_changed_with_inline_base_url, shorthand_longhands, style_base_url,
    style_entries_for_style_object, style_property_count_with_context,
    style_property_index_exists_with_context, style_property_name_at_with_context,
    style_property_value_for_pseudo_with_context, style_property_value_with_context,
    style_runtime_and_handle_from_object,
};
use super::{
    style_object_computation_context, style_object_forces_empty_computed,
    style_object_pseudo_element,
};

pub(super) fn style_named_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle, mode)) =
        style_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let raw_key = key.to_rust_string_lossy(scope);
    let runtime = unsafe { &*runtime_ptr };
    let forces_empty = style_object_forces_empty_computed(scope, args.holder(), mode);
    if let Some(index) = css_style_index_property(&raw_key) {
        if forces_empty {
            return v8::Intercepted::kNo;
        }
        let context = style_object_computation_context(scope, args.holder());
        let Some(name) = style_property_name_at_with_context(runtime, handle, mode, context, index)
        else {
            return v8::Intercepted::kNo;
        };
        let Some(value) = v8_string(scope, &name) else {
            return v8::Intercepted::kNo;
        };
        rv.set(value.into());
        return v8::Intercepted::kYes;
    }
    let Some(value) = live_style_named_property_value_for_handle(
        scope,
        args.holder(),
        runtime,
        handle,
        mode,
        &raw_key,
        forces_empty,
    ) else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = v8_string(scope, &value) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value.into());
    v8::Intercepted::kYes
}

pub(crate) fn live_style_named_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    raw_key: &str,
) -> Option<String> {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, style) else {
        return None;
    };
    let runtime = unsafe { &*runtime_ptr };
    let forces_empty = style_object_forces_empty_computed(scope, style, mode);
    live_style_named_property_value_for_handle(
        scope,
        style,
        runtime,
        handle,
        mode,
        raw_key,
        forces_empty,
    )
}

fn live_style_named_property_value_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    mode: StyleMode,
    raw_key: &str,
    forces_empty: bool,
) -> Option<String> {
    if is_style_intrinsic_name(raw_key) {
        return None;
    }
    if forces_empty {
        return resolve_known_style_property_name(raw_key).map(|_| String::new());
    }
    let property = resolve_style_property_name(runtime, handle, mode, raw_key)?;
    let context = style_object_computation_context(scope, style);
    Some(
        if let Some(pseudo) = style_object_pseudo_element(scope, style, mode) {
            style_property_value_for_pseudo_with_context(
                runtime, handle, &pseudo, &property, context,
            )
        } else {
            style_property_value_with_context(runtime, handle, mode, &property, context)
        },
    )
}

pub(super) fn style_named_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle, mode)) =
        style_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let raw_key = key.to_rust_string_lossy(scope);
    let runtime = unsafe { &*runtime_ptr };
    if style_object_forces_empty_computed(scope, args.holder(), mode) {
        return v8::Intercepted::kNo;
    }
    let context = style_object_computation_context(scope, args.holder());
    if css_style_index_property(&raw_key).is_some_and(|index| {
        style_property_index_exists_with_context(runtime, handle, mode, context, index)
    }) {
        rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    // CSS property IDL attributes live on CSSStyleDeclaration.prototype; the
    // named getter/setter below provides their dynamic values without making
    // them instance own properties.
    v8::Intercepted::kNo
}

pub(super) fn style_named_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let raw_key = key.to_rust_string_lossy(scope);
    if set_live_style_named_property_value(scope, args.holder(), &raw_key, value) {
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

pub(crate) fn set_live_style_named_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    raw_key: &str,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, style) else {
        return false;
    };
    if mode == StyleMode::Computed {
        crate::native_bridge::throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "Cannot modify a read-only CSSStyleDeclaration.",
        );
        return true;
    }
    if is_style_intrinsic_name(raw_key) {
        return false;
    }
    let property = {
        let runtime = unsafe { &*runtime_ptr };
        let Some(property) = resolve_style_property_name(runtime, handle, mode, raw_key) else {
            return false;
        };
        property
    };
    let value = if value.is_null_or_undefined() {
        String::new()
    } else {
        property_string_value(scope, value).unwrap_or_default()
    };
    if set_inline_style_property_with_pdb_storage(
        scope,
        runtime_ptr,
        handle,
        &property,
        &value,
        false,
    )
    .is_some()
    {
        return true;
    }
    let (style_object_entries, current_base_url) = {
        let runtime = unsafe { &*runtime_ptr };
        (
            style_entries_for_style_object(scope, style, runtime, handle),
            style_base_url(runtime, handle),
        )
    };
    let mut entries = style_object_entries.entries;
    let update_inline_style_base = property == "background-image" && !value.is_empty();
    if let Some(longhands) = shorthand_longhands(&property) {
        if value.is_empty() {
            if !entries.iter().any(|entry| {
                entry.name == property || longhands.iter().any(|longhand| entry.name == *longhand)
            }) {
                return true;
            }
            entries.retain(|entry| {
                entry.name != property && !longhands.iter().any(|longhand| entry.name == *longhand)
            });
        } else {
            let Some(parsed_entries) = parse_style_property_entries_for_cssom_fallback_write(
                &entries,
                &property,
                &value,
                false,
                Some(&current_base_url),
            ) else {
                return true;
            };
            expand_unresolved_box_shorthand_entries_for_mutation(
                &mut entries,
                &parsed_entries.affected_names,
            );
            retain_unaffected_style_entries(
                &mut entries,
                &property,
                &parsed_entries.affected_names,
            );
            entries.extend(parsed_entries.entries);
        }
    } else if value.is_empty() {
        let before_len = entries.len();
        if let Some(affected_names) = cssom_style_property_affected_names_with_pdb(&property) {
            retain_unaffected_style_entries(&mut entries, &property, &affected_names);
        } else {
            entries.retain(|entry| entry.name != property);
        }
        if entries.len() == before_len {
            return true;
        }
    } else {
        let Some(parsed_entries) = parse_style_property_entries_for_cssom_fallback_write(
            &entries,
            &property,
            &value,
            false,
            Some(&current_base_url),
        ) else {
            return true;
        };
        expand_unresolved_box_shorthand_entries_for_mutation(
            &mut entries,
            &parsed_entries.affected_names,
        );
        retain_unaffected_style_entries(&mut entries, &property, &parsed_entries.affected_names);
        entries.extend(parsed_entries.entries);
    }
    if update_inline_style_base {
        unsafe { &mut *runtime_ptr }
            .set_element_inline_style_base_url(handle, current_base_url.clone());
    }
    let inline_base_url = if update_inline_style_base {
        Some(current_base_url)
    } else {
        style_object_entries.base_url
    };
    set_style_entries_if_changed_with_inline_base_url(
        scope,
        runtime_ptr,
        handle,
        &entries,
        inline_base_url.as_ref(),
    );
    true
}

fn retain_unaffected_style_entries(
    entries: &mut Vec<crate::css_style::CssStyleEntry>,
    property: &str,
    affected_names: &[String],
) {
    entries.retain(|entry| {
        entry.name != property
            && !affected_names.iter().any(|name| name == &entry.name)
            && shorthand_longhands(&entry.name).is_none_or(|longhands| {
                !affected_names
                    .iter()
                    .any(|name| longhands.iter().any(|longhand| longhand == name))
            })
    });
}

fn css_style_index_property(raw_key: &str) -> Option<usize> {
    if raw_key.is_empty() || (raw_key.len() > 1 && raw_key.starts_with('0')) {
        return None;
    }
    raw_key.parse::<usize>().ok()
}

pub(super) fn style_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle, mode)) =
        style_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if style_object_forces_empty_computed(scope, args.holder(), mode) {
        return v8::Intercepted::kNo;
    }
    let context = style_object_computation_context(scope, args.holder());
    let Some(name) = style_property_name_at_with_context(
        unsafe { &*runtime_ptr },
        handle,
        mode,
        context,
        index as usize,
    ) else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = v8_string(scope, &name) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value.into());
    v8::Intercepted::kYes
}

pub(super) fn style_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle, mode)) =
        style_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if style_object_forces_empty_computed(scope, args.holder(), mode) {
        return v8::Intercepted::kNo;
    }
    let context = style_object_computation_context(scope, args.holder());
    if !style_property_index_exists_with_context(
        unsafe { &*runtime_ptr },
        handle,
        mode,
        context,
        index as usize,
    ) {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
    v8::Intercepted::kYes
}

pub(super) fn style_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Ok((runtime_ptr, handle, mode)) =
        style_runtime_and_handle_from_object(scope, args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    if style_object_forces_empty_computed(scope, args.holder(), mode) {
        rv.set(v8::Array::new(scope, 0));
        return;
    }
    let context = style_object_computation_context(scope, args.holder());
    let keys =
        (0..style_property_count_with_context(unsafe { &*runtime_ptr }, handle, mode, context))
            .map(|index| v8::Integer::new_from_unsigned(scope, index as u32).into())
            .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

pub(super) fn style_named_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    rv.set(v8::Array::new(scope, 0));
}
