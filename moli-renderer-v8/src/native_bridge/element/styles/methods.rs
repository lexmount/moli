use crate::{
    css_style::{
        CssStyleDeclarationItemArgs, CssStyleDeclarationPropertyArgs,
        CssStyleDeclarationSetPropertyArgs, CssStyleEntry as StyleEntry,
        canonical_style_property_name,
    },
    util::{throw_type_error, v8_string},
    webidl,
};

use super::declaration::{
    StyleMode, all_shorthand_applies_to, cssom_style_property_affected_names_with_pdb,
    cssom_text_decoration_line_value_is_compat,
    expand_unresolved_box_shorthand_entries_for_mutation,
    parse_style_property_entries_for_cssom_fallback_write,
    parse_style_property_entries_for_cssom_write, set_inline_style_property_with_pdb_storage,
    set_style_entries_if_changed_with_inline_base_url, set_style_entries_with_inline_base_url,
    shorthand_longhands, style_base_url, style_entries_for_style_object,
    style_property_name_at_with_context, style_property_names_with_context,
    style_property_priority, style_property_value, style_property_value_for_pseudo_with_context,
    style_property_value_with_context, style_runtime_and_handle_from_object,
    supported_declared_property,
};
use super::{
    style_object_computation_context, style_object_forces_empty_computed,
    style_object_pseudo_element,
};

pub(crate) fn style_set_property_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((_, _, mode)) = style_runtime_and_handle_from_object(scope, args.this()) else {
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
    if !parsed.priority.is_empty() && !parsed.priority.eq_ignore_ascii_case("important") {
        return;
    }
    set_style_property_from_object(
        scope,
        args.this(),
        &parsed.property,
        &parsed.value,
        parsed.priority.eq_ignore_ascii_case("important"),
    );
}

/// Mutate the native declaration without consulting author-overridable JS methods.
pub(crate) fn set_style_property_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    property: &str,
    value: &str,
    priority: bool,
) {
    set_typed_style_property_from_object(scope, style, property, value, priority, None);
}

pub(crate) fn set_typed_style_property_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    property: &str,
    value: &str,
    priority: bool,
    unit: Option<moli_css_parse::CssDeclaredUnitValue>,
) {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, style) else {
        return;
    };
    if mode == StyleMode::Computed {
        return;
    }
    if property.starts_with("--") && !moli_css_parse::is_cssom_custom_property_name(property) {
        return;
    }
    let name = canonical_style_property_name(property);
    if !supported_declared_property(&name) {
        return;
    }
    if set_inline_style_property_with_pdb_storage(
        scope,
        runtime_ptr,
        handle,
        &name,
        value,
        priority,
        unit.as_ref(),
    )
    .is_some()
    {
        return;
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
    if let Some(longhands) = shorthand_longhands(&name) {
        if value.is_empty() {
            if !entries.iter().any(|entry| {
                entry.name == name || longhands.iter().any(|longhand| entry.name == *longhand)
            }) {
                return;
            }
            entries.retain(|entry| {
                entry.name != name && !longhands.iter().any(|longhand| entry.name == *longhand)
            });
        } else {
            let Some(parsed_entries) = parse_style_property_entries_for_cssom_fallback_write(
                &entries,
                &name,
                value,
                priority,
                Some(&current_base_url),
            ) else {
                return;
            };
            expand_unresolved_box_shorthand_entries_for_mutation(
                &mut entries,
                &parsed_entries.affected_names,
            );
            retain_unaffected_style_entries(&mut entries, &name, &parsed_entries.affected_names);
            entries.extend(parsed_entries.entries);
        }
    } else if name == "all" {
        if value.is_empty() {
            if !entries
                .iter()
                .any(|entry| entry.name == "all" || all_shorthand_applies_to(&entry.name))
            {
                return;
            }
            entries.retain(|entry| entry.name != "all" && !all_shorthand_applies_to(&entry.name));
        } else {
            let Some(parsed_entries) = parse_style_property_entries_for_cssom_write(
                &name,
                value,
                priority,
                Some(&current_base_url),
            ) else {
                return;
            };
            entries.retain(|entry| entry.name != "all");
            entries.extend(parsed_entries.entries);
        }
    } else if value.is_empty() {
        let before_len = entries.len();
        if let Some(affected_names) = cssom_style_property_affected_names_with_pdb(&name) {
            retain_unaffected_style_entries(&mut entries, &name, &affected_names);
        } else {
            entries.retain(|entry| entry.name != name);
        }
        if entries.len() == before_len {
            return;
        }
    } else {
        let Some(parsed_entries) = parse_style_property_entries_for_cssom_fallback_write(
            &entries,
            &name,
            value,
            priority,
            Some(&current_base_url),
        ) else {
            return;
        };
        expand_unresolved_box_shorthand_entries_for_mutation(
            &mut entries,
            &parsed_entries.affected_names,
        );
        retain_unaffected_style_entries(&mut entries, &name, &parsed_entries.affected_names);
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
        &name,
        unit,
    );
}

fn retain_unaffected_style_entries(
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
    let context = style_object_computation_context(scope, args.this());
    let value = if let Some(pseudo) = style_object_pseudo_element(scope, args.this(), mode) {
        style_property_value_for_pseudo_with_context(
            runtime,
            handle,
            &pseudo,
            &parsed.property,
            context,
        )
    } else {
        style_property_value_with_context(runtime, handle, mode, &parsed.property, context)
    };
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(crate) fn style_property_value_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<String> {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, style) else {
        return None;
    };
    if style_object_forces_empty_computed(scope, style, mode) {
        return Some(String::new());
    }
    let runtime = unsafe { &*runtime_ptr };
    let context = style_object_computation_context(scope, style);
    let value = if let Some(pseudo) = style_object_pseudo_element(scope, style, mode) {
        style_property_value_for_pseudo_with_context(runtime, handle, &pseudo, property, context)
    } else {
        style_property_value_with_context(runtime, handle, mode, property, context)
    };
    Some(value)
}

pub(crate) fn style_property_names_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<Vec<String>> {
    let Ok((runtime_ptr, handle, mode)) = style_runtime_and_handle_from_object(scope, style) else {
        return None;
    };
    if style_object_forces_empty_computed(scope, style, mode) {
        return Some(Vec::new());
    }
    let context = style_object_computation_context(scope, style);
    Some(style_property_names_with_context(
        unsafe { &*runtime_ptr },
        handle,
        mode,
        context,
    ))
}

pub(crate) fn style_typed_unit_value_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<moli_css_parse::CssDeclaredUnitValue> {
    let (runtime_ptr, handle, StyleMode::Inline) =
        style_runtime_and_handle_from_object(scope, style).ok()?
    else {
        return None;
    };
    unsafe { &*runtime_ptr }
        .element_inline_style_declaration_state(handle)?
        .block
        .typed_unit_value(property)
        .cloned()
}

/// Typed OM observes computed values, before CSSOM resolves percentages and
/// auto sizes against layout. Reuse the declaration's document/viewport and
/// retained style observation rather than reading serialized used values.
pub(crate) fn computed_typed_style_value_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<moli_css_parse::ParsedTypedStyleValue> {
    let (runtime_ptr, handle, StyleMode::Computed) =
        style_runtime_and_handle_from_object(scope, style).ok()?
    else {
        return None;
    };
    if style_object_forces_empty_computed(scope, style, StyleMode::Computed) {
        return None;
    }
    let runtime = unsafe { &*runtime_ptr };
    let context = style_object_computation_context(scope, style);
    let read = super::ComputedStyleRead::new_with_context(runtime, handle, context);
    let computed = read.computed_values()?;
    let id = style::properties::PropertyId::parse_enabled_for_all_content(property).ok()?;
    if let style::properties::PropertyId::Custom(name) = &id
        && computed.custom_properties().inherited.get(name).is_none()
        && computed
            .custom_properties()
            .non_inherited
            .get(name)
            .is_none()
    {
        return None;
    }
    let mut css_text = String::new();
    computed
        .computed_or_resolved_property_value(id.clone(), None, &mut css_text)
        .ok()?;
    if css_text.is_empty() && !property.starts_with("--") {
        return None;
    }
    let values = if property.starts_with("--") {
        Some(style::typed_om::TypedValueList {
            values: [style::typed_om::TypedValue::Unparsed(
                moli_css_parse::reify_unparsed_style_value(&css_text, None)?,
            )]
            .into_iter()
            .collect(),
        })
    } else {
        id.longhand_id()
            .and_then(|id| computed.property_value_to_typed_value_list(id))
    };
    Some(moli_css_parse::ParsedTypedStyleValue { css_text, values })
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
    if name == "all" {
        if set_inline_style_property_with_pdb_storage(
            scope,
            runtime_ptr,
            handle,
            &name,
            "",
            false,
            None,
        )
        .is_some()
        {
            if let Some(previous) = v8_string(scope, &previous) {
                rv.set(previous.into());
            } else {
                rv.set_empty_string();
            }
            return;
        }
    } else {
        if set_inline_style_property_with_pdb_storage(
            scope,
            runtime_ptr,
            handle,
            &name,
            "",
            false,
            None,
        )
        .is_some()
        {
            if let Some(previous) = v8_string(scope, &previous) {
                rv.set(previous.into());
            } else {
                rv.set_empty_string();
            }
            return;
        }
        if previous.is_empty() {
            rv.set_empty_string();
            return;
        }
    }
    let style_object_entries =
        style_entries_for_style_object(scope, args.this(), unsafe { &*runtime_ptr }, handle);
    let mut entries = style_object_entries.entries;
    let inline_base_url = style_object_entries.base_url;
    if name == "all" {
        if previous.is_empty()
            && !entries
                .iter()
                .any(|entry| entry.name == "all" || all_shorthand_applies_to(&entry.name))
        {
            rv.set_empty_string();
            return;
        }
        entries.retain(|entry| entry.name != "all" && !all_shorthand_applies_to(&entry.name));
        set_style_entries_with_inline_base_url(
            scope,
            runtime_ptr,
            handle,
            &entries,
            inline_base_url.as_ref(),
            &name,
        );
        if let Some(previous) = v8_string(scope, &previous) {
            rv.set(previous.into());
        } else {
            rv.set_empty_string();
        }
        return;
    }
    if let Some(longhands) = shorthand_longhands(&name) {
        entries.retain(|entry| {
            entry.name != name && !longhands.iter().any(|longhand| entry.name == *longhand)
        });
    } else if let Some(affected_names) = cssom_style_property_affected_names_with_pdb(&name) {
        retain_unaffected_style_entries(&mut entries, &name, &affected_names);
    } else {
        entries.retain(|entry| entry.name != name);
    }
    set_style_entries_with_inline_base_url(
        scope,
        runtime_ptr,
        handle,
        &entries,
        inline_base_url.as_ref(),
        &name,
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
    let context = style_object_computation_context(scope, args.this());
    let Some(name) = style_property_name_at_with_context(
        unsafe { &*runtime_ptr },
        handle,
        mode,
        context,
        parsed.index as usize,
    ) else {
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

/// Whether this native CSS declaration represents computed rather than specified style.
pub(crate) fn style_declaration_is_computed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> bool {
    style_runtime_and_handle_from_object(scope, style)
        .is_ok_and(|(_, _, mode)| mode == StyleMode::Computed)
}

pub(crate) fn clear_inline_style_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) {
    let Ok((runtime_ptr, handle, StyleMode::Inline)) =
        style_runtime_and_handle_from_object(scope, style)
    else {
        return;
    };
    super::declaration::set_inline_style_css_text_with_pdb_storage(scope, runtime_ptr, handle, "");
}
