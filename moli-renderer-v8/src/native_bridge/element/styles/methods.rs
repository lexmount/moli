use crate::{
    css_style::{
        CssStyleDeclarationItemArgs, CssStyleDeclarationPropertyArgs,
        CssStyleDeclarationSetPropertyArgs, CssStyleEntry as StyleEntry,
        canonical_style_property_name,
    },
    document_runtime::DomHandle,
    native_bridge::PageScriptHost,
    util::{throw_type_error, v8_string},
    webidl,
};

use super::declaration::{
    InlineStylePropertyWrite, StyleMode, all_shorthand_applies_to,
    cssom_style_property_affected_names_with_pdb, cssom_text_decoration_line_value_is_compat,
    expand_unresolved_box_shorthand_entries_for_mutation,
    parse_style_property_entries_for_cssom_fallback_write,
    parse_style_property_entries_for_cssom_write, set_inline_style_property_with_pdb_storage,
    set_style_entries_if_changed_with_inline_base_url, set_style_entries_with_inline_base_url,
    shorthand_longhands, style_base_url, style_entries_for_style_object,
    style_property_name_at_with_context, style_property_names_with_context,
    style_property_priority, style_property_value, style_runtime_and_handle_from_object,
    supported_declared_property,
};
use super::public_layout::{live_style_read_context, read_live_style_property};
use super::style_object_forces_empty_computed;

pub(crate) fn style_set_property_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, args.this())
    else {
        throw_style_declaration_method_illegal_invocation(scope, "setProperty");
        return;
    };
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationSetPropertyArgs>(scope, &args)
    else {
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
    if parsed.property.starts_with("--")
        && !moli_css_parse::is_cssom_custom_property_name(&parsed.property)
    {
        return;
    }
    let name = canonical_style_property_name(&parsed.property);
    if !parsed.priority.is_empty() && !parsed.priority.eq_ignore_ascii_case("important") {
        return;
    }
    if !supported_declared_property(&name) {
        return;
    }
    let priority = parsed.priority.eq_ignore_ascii_case("important");
    set_live_inline_style_property(
        scope,
        args.this(),
        runtime_ptr,
        handle,
        &name,
        &parsed.value,
        priority,
    );
}

pub(super) fn set_live_inline_style_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    name: &str,
    value: &str,
    priority: bool,
) {
    if !try_write_inline_style_property(scope, style, runtime_ptr, handle, name, value, priority) {
        crate::custom_elements::dispatch_unchanged_attribute_changed_callback(
            scope,
            runtime_ptr,
            handle,
            "style",
        );
    }
}

fn try_write_inline_style_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    name: &str,
    value: &str,
    priority: bool,
) -> bool {
    match set_inline_style_property_with_pdb_storage(
        scope,
        runtime_ptr,
        handle,
        name,
        value,
        priority,
    ) {
        InlineStylePropertyWrite::AttributeWritten => return true,
        InlineStylePropertyWrite::NoAttributeWrite => return false,
        InlineStylePropertyWrite::NeedsCompatibilityEntries => {}
    }
    let (style_object_entries, current_base_url) = {
        let runtime = unsafe { &*runtime_ptr };
        (
            style_entries_for_style_object(scope, style, runtime, handle),
            style_base_url(runtime, handle),
        )
    };
    let mut entries = style_object_entries.entries;
    let update_inline_style_base = name == "background-image" && !value.is_empty();
    if value.is_empty() {
        if !remove_style_entries(&mut entries, name) {
            return false;
        }
    } else if name == "all" {
        let Some(parsed_entries) = parse_style_property_entries_for_cssom_write(
            name,
            value,
            priority,
            Some(&current_base_url),
        ) else {
            return false;
        };
        entries.retain(|entry| entry.name != "all");
        entries.extend(parsed_entries.entries);
    } else {
        let Some(parsed_entries) = parse_style_property_entries_for_cssom_fallback_write(
            &entries,
            name,
            value,
            priority,
            Some(&current_base_url),
        ) else {
            return false;
        };
        expand_unresolved_box_shorthand_entries_for_mutation(
            &mut entries,
            &parsed_entries.affected_names,
        );
        retain_unaffected_style_entries(&mut entries, name, &parsed_entries.affected_names);
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
    )
}

fn remove_style_entries(entries: &mut Vec<StyleEntry>, name: &str) -> bool {
    let before = entries.len();
    if name == "all" {
        entries.retain(|entry| entry.name != "all" && !all_shorthand_applies_to(&entry.name));
    } else if let Some(longhands) = shorthand_longhands(name) {
        entries.retain(|entry| {
            entry.name != name && !longhands.iter().any(|longhand| entry.name == *longhand)
        });
    } else if let Some(affected_names) = cssom_style_property_affected_names_with_pdb(name) {
        retain_unaffected_style_entries(entries, name, &affected_names);
    } else {
        entries.retain(|entry| entry.name != name);
    }
    entries.len() != before
}

pub(super) fn retain_unaffected_style_entries(
    entries: &mut Vec<StyleEntry>,
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

pub(crate) fn style_get_property_value_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, args.this())
    else {
        throw_style_declaration_method_illegal_invocation(scope, "getPropertyValue");
        return;
    };
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationPropertyArgs>(scope, &args) else {
        rv.set_empty_string();
        return;
    };
    if style_object_forces_empty_computed(scope, args.this(), mode) {
        rv.set_empty_string();
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let Some(value) =
        read_live_style_property(scope, args.this(), runtime, handle, mode, &parsed.property)
    else {
        return;
    };
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(crate) fn computed_style_property_value_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<String> {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, style) else {
        return None;
    };
    if mode != StyleMode::Computed {
        return None;
    }
    if style_object_forces_empty_computed(scope, style, mode) {
        return Some(String::new());
    }
    let runtime = unsafe { &*runtime_ptr };
    read_live_style_property(scope, style, runtime, handle, mode, property)
}

pub(crate) fn computed_style_property_names_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<Vec<String>> {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, style) else {
        return None;
    };
    if mode != StyleMode::Computed {
        return None;
    }
    if style_object_forces_empty_computed(scope, style, mode) {
        return Some(Vec::new());
    }
    let runtime = unsafe { &*runtime_ptr };
    let context = live_style_read_context(scope, style, runtime, handle, mode, None)?;
    Some(style_property_names_with_context(
        runtime, handle, mode, context,
    ))
}

pub(crate) fn style_remove_property_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, args.this())
    else {
        throw_style_declaration_method_illegal_invocation(scope, "removeProperty");
        return;
    };
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationPropertyArgs>(scope, &args) else {
        rv.set_empty_string();
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
    if parsed.property.starts_with("--")
        && !moli_css_parse::is_cssom_custom_property_name(&parsed.property)
    {
        rv.set_empty_string();
        return;
    }
    let name = canonical_style_property_name(&parsed.property);
    let previous = style_property_value(unsafe { &*runtime_ptr }, handle, mode, &name);
    let previous = if name == "text-decoration"
        && cssom_text_decoration_line_value_is_compat(&style_property_value(
            unsafe { &*runtime_ptr },
            handle,
            mode,
            "text-decoration-line",
        )) {
        String::new()
    } else {
        previous
    };
    let write =
        set_inline_style_property_with_pdb_storage(scope, runtime_ptr, handle, &name, "", false);
    if !matches!(write, InlineStylePropertyWrite::NeedsCompatibilityEntries) {
        if matches!(write, InlineStylePropertyWrite::NoAttributeWrite) {
            crate::custom_elements::dispatch_unchanged_attribute_changed_callback(
                scope,
                runtime_ptr,
                handle,
                "style",
            );
        }
        rv.set(
            v8_string(scope, &previous)
                .unwrap_or_else(|| v8::String::empty(scope))
                .into(),
        );
        return;
    }
    if name != "all" && previous.is_empty() {
        if supported_declared_property(&name) {
            crate::custom_elements::dispatch_unchanged_attribute_changed_callback(
                scope,
                runtime_ptr,
                handle,
                "style",
            );
        }
        rv.set_empty_string();
        return;
    }
    let style_object_entries =
        style_entries_for_style_object(scope, args.this(), unsafe { &*runtime_ptr }, handle);
    let mut entries = style_object_entries.entries;
    let inline_base_url = style_object_entries.base_url;
    let removed = remove_style_entries(&mut entries, &name);
    if name == "all" && previous.is_empty() && !removed {
        crate::custom_elements::dispatch_unchanged_attribute_changed_callback(
            scope,
            runtime_ptr,
            handle,
            "style",
        );
        rv.set_empty_string();
        return;
    }
    set_style_entries_with_inline_base_url(
        scope,
        runtime_ptr,
        handle,
        &entries,
        inline_base_url.as_ref(),
    );
    if let Some(previous) = v8_string(scope, &previous) {
        rv.set(previous.into());
    } else {
        rv.set_empty_string();
    }
}

pub(crate) fn style_get_property_priority_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, args.this())
    else {
        throw_style_declaration_method_illegal_invocation(scope, "getPropertyPriority");
        return;
    };
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationPropertyArgs>(scope, &args) else {
        rv.set(v8::String::empty(scope).into());
        return;
    };
    if style_object_forces_empty_computed(scope, args.this(), mode) {
        rv.set(v8::String::empty(scope).into());
        return;
    }
    let priority = style_property_priority(unsafe { &*runtime_ptr }, handle, &parsed.property);
    if let Some(priority) = v8_string(scope, &priority) {
        rv.set(priority.into());
    } else {
        rv.set(v8::String::empty(scope).into());
    }
}

pub(crate) fn style_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() < 1 {
        throw_type_error(
            scope,
            "Failed to execute 'item' on 'CSSStyleDeclaration': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, args.this())
    else {
        throw_style_declaration_method_illegal_invocation(scope, "item");
        return;
    };
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationItemArgs>(scope, &args) else {
        return;
    };
    if style_object_forces_empty_computed(scope, args.this(), mode) {
        rv.set_empty_string();
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let Some(context) = live_style_read_context(scope, args.this(), runtime, handle, mode, None)
    else {
        return;
    };
    let Some(name) =
        style_property_name_at_with_context(runtime, handle, mode, context, parsed.index as usize)
    else {
        rv.set_empty_string();
        return;
    };
    if let Some(name) = v8_string(scope, &name) {
        rv.set(name.into());
    } else {
        rv.set_empty_string();
    }
}

fn throw_style_declaration_method_illegal_invocation(
    scope: &mut v8::PinScope<'_, '_>,
    method: &str,
) {
    throw_type_error(
        scope,
        &format!("Failed to execute '{method}' on 'CSSStyleDeclaration': Illegal invocation."),
    );
}
