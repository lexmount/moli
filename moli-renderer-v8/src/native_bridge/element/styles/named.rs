use super::declaration::{
    StyleMode, is_style_intrinsic_name, resolve_known_style_property_name,
    resolve_style_property_name, style_property_count_with_context,
    style_property_index_exists_with_context, style_property_name_at_with_context,
    style_runtime_and_handle_from_object,
};
use super::public_layout::{live_style_read_context, read_live_style_property};
use super::style_object_forces_empty_computed;
use crate::{
    css_style::css_style_declaration_property_value,
    util::{throw_type_error, v8_string},
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
        let Some(context) =
            live_style_read_context(scope, args.holder(), runtime, handle, mode, None)
        else {
            return v8::Intercepted::kYes;
        };
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
    runtime: &crate::native_bridge::PageScriptHost,
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
    read_live_style_property(scope, style, runtime, handle, mode, &property)
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
    // Non-index attributes are defined on CSSStyleDeclaration.prototype.
    let Some(index) = css_style_index_property(&raw_key) else {
        return v8::Intercepted::kNo;
    };
    if style_object_forces_empty_computed(scope, args.holder(), mode) {
        return v8::Intercepted::kNo;
    }
    let runtime = unsafe { &*runtime_ptr };
    let Some(context) = live_style_read_context(scope, args.holder(), runtime, handle, mode, None)
    else {
        return v8::Intercepted::kYes;
    };
    if style_property_index_exists_with_context(runtime, handle, mode, context, index) {
        rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

pub(super) fn style_named_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
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
    let Some(value) = css_style_declaration_property_value(scope, value) else {
        return true;
    };
    // Do not retain a native runtime lookup across author string conversion.
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, style) else {
        throw_type_error(scope, "Illegal invocation");
        return true;
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
    super::methods::set_live_inline_style_property(
        scope,
        style,
        runtime_ptr,
        handle,
        &property,
        &value,
        false,
    );
    true
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
    let runtime = unsafe { &*runtime_ptr };
    let Some(context) = live_style_read_context(scope, args.holder(), runtime, handle, mode, None)
    else {
        return v8::Intercepted::kYes;
    };
    let Some(name) =
        style_property_name_at_with_context(runtime, handle, mode, context, index as usize)
    else {
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
    let runtime = unsafe { &*runtime_ptr };
    let Some(context) = live_style_read_context(scope, args.holder(), runtime, handle, mode, None)
    else {
        return v8::Intercepted::kYes;
    };
    if !style_property_index_exists_with_context(runtime, handle, mode, context, index as usize) {
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
    let runtime = unsafe { &*runtime_ptr };
    let Some(context) = live_style_read_context(scope, args.holder(), runtime, handle, mode, None)
    else {
        return;
    };
    let keys = (0..style_property_count_with_context(runtime, handle, mode, context))
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
