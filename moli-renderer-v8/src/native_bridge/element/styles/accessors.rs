use crate::util::{throw_type_error, v8_string};

use super::declaration::{
    StyleMode, set_inline_style_css_text_with_pdb_storage, style_base_url,
    style_css_text_for_computed, style_entries, style_entries_css_text_with_pdb,
    style_runtime_and_handle_from_object,
};
use super::{
    style_object_computation_context, style_object_forces_empty_computed,
    style_object_property_count_with_context,
};
use crate::{
    css_style::{CssStyleDeclarationCssTextArgs, serialize_css_style_entries},
    webidl,
};

pub(crate) fn style_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_style_length(scope, args.this(), &mut rv);
}

fn set_style_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, style) else {
        throw_style_declaration_illegal_invocation(scope, "get", "length");
        return;
    };
    if style_object_forces_empty_computed(scope, style, mode) {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    }
    let context = style_object_computation_context(scope, style);
    let length = style_object_property_count_with_context(
        scope,
        style,
        unsafe { &*runtime_ptr },
        handle,
        mode,
        context,
    ) as i32;
    rv.set(v8::Integer::new(scope, length).into());
}

pub(crate) fn style_css_text_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_style_css_text(scope, args.this(), &mut rv);
}

fn set_style_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, style) else {
        throw_style_declaration_illegal_invocation(scope, "get", "cssText");
        return;
    };
    if style_object_forces_empty_computed(scope, style, mode) {
        rv.set(v8::String::empty(scope).into());
        return;
    }
    let value = if mode == StyleMode::Computed {
        style_css_text_for_computed(unsafe { &*runtime_ptr }, handle)
    } else {
        let runtime = unsafe { &*runtime_ptr };
        if let Some(state) = runtime.element_inline_style_declaration_state(handle) {
            state.css_text()
        } else {
            let entries = style_entries(runtime, handle);
            style_entries_css_text_with_pdb(&entries)
                .unwrap_or_else(|| serialize_css_style_entries(&entries))
        }
    };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(crate) fn style_css_text_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if style_runtime_and_handle_from_object(scope, args.this()).is_err() {
        throw_style_declaration_illegal_invocation(scope, "set", "cssText");
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationCssTextArgs>(scope, &args) else {
        return;
    };
    // Conversion can run author code. Resolve the native declaration again
    // before accessing its runtime or applying the readonly flag.
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, args.this())
    else {
        throw_style_declaration_illegal_invocation(scope, "set", "cssText");
        return;
    };
    if mode == StyleMode::Computed {
        crate::native_bridge::throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "Cannot modify a read-only CSSStyleDeclaration.",
        );
        return;
    }
    let base_url = style_base_url(unsafe { &*runtime_ptr }, handle);
    unsafe { &mut *runtime_ptr }.set_element_inline_style_base_url(handle, base_url.clone());
    set_inline_style_css_text_with_pdb_storage(scope, runtime_ptr, handle, &parsed.value);
    rv.set_undefined();
}

fn throw_style_declaration_illegal_invocation(
    scope: &mut v8::PinScope<'_, '_>,
    operation: &str,
    member: &str,
) {
    throw_type_error(
        scope,
        &format!("Failed to {operation} '{member}' on 'CSSStyleDeclaration': Illegal invocation."),
    );
}
