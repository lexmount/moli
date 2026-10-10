use super::declaration_parser::parse_style_property_entries_with_base;
use super::inline_state::{
    inline_css_text_pdb_storage_state, inline_state_block_entries_for_property_mutation,
    inline_state_has_replaceable_side_entries_for_property,
    inline_state_has_unpreservable_side_entries_for_property,
    inline_style_declaration_state_for_handle, inline_style_declaration_state_from_css_text,
    refresh_inline_state_entries_after_pdb_mutation, style_entry_is_preservable_for_pdb_property,
    style_entry_is_replaceable_by_pdb_property,
};
use super::pdb_compat::{
    css_value_uses_unresolved_cssom_storage, cssom_empty_specified_placeholder_property,
    cssom_style_property_uses_preferred_pdb_supplemental_entries,
    cssom_style_property_write_uses_pdb, inline_style_property_write_can_use_pdb_storage,
    parse_style_property_entries_with_pdb, set_pdb_block_property_collecting_entries,
    style_entry_affects_property_query, style_entry_is_pdb_supplemental_side_entry,
    style_property_mutation_affected_names_with_pdb,
    style_property_mutation_cleanup_names_with_pdb,
};
use super::*;

pub(crate) fn parse_inline_css_text_with_base(
    style_text: &str,
    base_url: Option<&url::Url>,
) -> Vec<StyleEntry> {
    let mut entries: Vec<StyleEntry> = Vec::new();
    for declaration in parse_css_declaration_list(style_text) {
        let raw_name = declaration.name.trim();
        let name = if raw_name.starts_with("--") {
            raw_name.to_owned()
        } else {
            // CSS declaration names are case-insensitive, unlike IDL accessor
            // names such as cssFloat. Normalize before resolving CSS aliases.
            canonical_style_property_name(&raw_name.to_ascii_lowercase())
        };
        if declaration.value.is_empty() && cssom_empty_specified_placeholder_property(&name) {
            entries.push(StyleEntry {
                name,
                value: String::new(),
                priority: declaration.priority,
            });
            continue;
        }
        if name.is_empty() {
            continue;
        }
        if cssom_style_property_write_uses_pdb(&name, &declaration.value) {
            if let Some(parsed) = parse_style_property_entries_with_pdb(
                &name,
                &declaration.value,
                declaration.priority,
            ) {
                entries.extend(parsed.entries);
            }
            continue;
        }
        if let Some(parsed) = parse_style_property_entries_with_base(
            &name,
            &declaration.value,
            declaration.priority,
            base_url,
        ) {
            entries.extend(parsed.entries);
        }
    }
    entries
}

pub(crate) fn parse_style_property_entries_for_cssom_write(
    name: &str,
    value: &str,
    priority: bool,
    base_url: Option<&url::Url>,
) -> Option<ParsedStylePropertyEntries> {
    let name = canonical_style_property_name(name);
    if moli_css_parse::is_cssom_custom_property_name(&name) {
        return (!value.is_empty())
            .then(|| parse_style_property_entries_with_pdb(&name, value, priority))
            .flatten();
    }
    if name.starts_with("--") {
        return None;
    }
    if cssom_style_property_write_uses_pdb(&name, value) {
        return parse_style_property_entries_with_pdb(&name, value, priority);
    }
    parse_style_property_entries_with_base(&name, value, priority, base_url)
}

pub(in crate::native_bridge::element::styles) fn parse_style_property_entries_for_cssom_fallback_write(
    entries: &[StyleEntry],
    name: &str,
    value: &str,
    priority: bool,
    base_url: Option<&url::Url>,
) -> Option<ParsedStylePropertyEntries> {
    if cssom_style_property_write_uses_pdb(name, value) {
        let parsed = parse_style_property_entries_for_cssom_write(name, value, priority, base_url)?;
        if let Some(affected_names) = style_property_mutation_affected_names_with_pdb(name)
            && entries.iter().any(|entry| {
                style_entry_affects_property_query(entry, name, &affected_names)
                    && !style_entry_is_replaceable_by_pdb_property(entry, name, &affected_names)
                    && !style_entry_is_preservable_for_pdb_property(entry, name)
            })
        {
            return parse_style_property_entries_with_base(name, value, priority, base_url);
        }
        return Some(parsed);
    }
    parse_style_property_entries_for_cssom_write(name, value, priority, base_url)
}

pub(in crate::native_bridge::element::styles) fn set_inline_style_property_with_pdb_storage(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    value: &str,
    priority: bool,
    numeric: Option<&moli_css_parse::CssDeclaredNumericValue>,
) -> Option<bool> {
    if name != "all" && !inline_style_property_write_can_use_pdb_storage(name, value) {
        return None;
    }
    let runtime = unsafe { &*runtime_ptr };
    let existing_inline_base_url = runtime.existing_element_inline_style_base_url(handle);
    let mut state = inline_style_declaration_state_for_handle(
        runtime,
        handle,
        existing_inline_base_url.as_ref(),
    );
    let update_inline_style_base = name == "background-image" && !value.is_empty();
    let inline_base_url = if update_inline_style_base {
        Some(runtime.element_inline_style_base_url(handle))
    } else {
        existing_inline_base_url
    };
    let affected_names = style_property_mutation_affected_names_with_pdb(name)?;
    if inline_state_has_unpreservable_side_entries_for_property(&state, name, &affected_names) {
        return None;
    }
    let has_replaceable_side_entry =
        inline_state_has_replaceable_side_entries_for_property(&state, name, &affected_names);
    let new_entries = if value.is_empty() {
        let mut removed_from_block = false;
        for affected in &affected_names {
            removed_from_block |= state.block.remove_property(affected).changed;
        }
        if !removed_from_block && !has_replaceable_side_entry {
            return Some(false);
        }
        (Vec::new(), Vec::new())
    } else if name == "all" {
        let parsed = parse_style_property_entries_with_base(name, value, priority, None)?;
        if state.block.set_property(name, value, priority)
            == moli_css_parse::CssSetResult::ParseError
        {
            return None;
        }
        let block_entries = state
            .block
            .entries()
            .into_iter()
            .map(StyleEntry::from)
            .collect::<Vec<_>>();
        if block_entries.iter().any(|entry| entry.name == "all") {
            (block_entries, Vec::new())
        } else {
            (parsed.entries, Vec::new())
        }
    } else {
        let parsed = parse_style_property_entries_with_pdb(name, value, priority)?;
        if parsed
            .entries
            .iter()
            .all(style_entry_is_pdb_supplemental_side_entry)
        {
            for affected in &affected_names {
                let _ = state.block.remove_property(affected);
            }
            (parsed.entries.clone(), parsed.entries)
        } else {
            let supplemental_entries = parsed
                .entries
                .iter()
                .filter(|entry| style_entry_is_pdb_supplemental_side_entry(entry))
                .cloned()
                .collect::<Vec<_>>();
            let uses_preferred_supplemental_entries =
                cssom_style_property_uses_preferred_pdb_supplemental_entries(name, value, priority);
            let mut entries = set_pdb_block_property_collecting_entries(
                &mut state.block,
                name,
                value,
                priority,
                &parsed,
                uses_preferred_supplemental_entries,
            )?;
            for affected in style_property_mutation_cleanup_names_with_pdb(name) {
                let _ = state.block.remove_property(&affected);
            }
            if uses_preferred_supplemental_entries {
                entries = parsed.entries.clone();
            } else if css_value_uses_unresolved_cssom_storage(value)
                && (entries.is_empty() || entries.iter().any(|entry| entry.value.is_empty()))
            {
                entries = parsed
                    .entries
                    .iter()
                    .filter(|entry| !style_entry_is_pdb_supplemental_side_entry(entry))
                    .cloned()
                    .collect();
            }
            if entries.is_empty() {
                entries =
                    inline_state_block_entries_for_property_mutation(&state, name, &affected_names);
            }
            if entries.is_empty() {
                (supplemental_entries.clone(), supplemental_entries)
            } else {
                entries.extend(supplemental_entries.iter().cloned());
                (entries, supplemental_entries)
            }
        }
    };
    let (new_entries, new_side_entries) = new_entries;
    refresh_inline_state_entries_after_pdb_mutation(
        &mut state,
        name,
        &affected_names,
        new_entries,
        new_side_entries,
    );
    if let Some(numeric) = numeric {
        state
            .block
            .retain_typed_numeric_value(name, numeric.clone());
    }
    let css_text = state.css_text();
    let resolution_text = state.style_resolution_text();
    if update_inline_style_base && let Some(inline_base_url) = &inline_base_url {
        unsafe { &mut *runtime_ptr }
            .set_element_inline_style_base_url(handle, inline_base_url.clone());
    }
    if style_string(runtime, handle) == css_text {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime.set_element_inline_style_resolution_text(handle, resolution_text);
        runtime.set_element_inline_style_declaration_state(handle, state);
        return Some(false);
    }
    set_reflected_style_attribute_with_inline_base_url(
        scope,
        runtime_ptr,
        handle,
        &css_text,
        inline_base_url.as_ref(),
        state,
        resolution_text,
    );
    Some(true)
}

pub(in crate::native_bridge::element::styles) fn set_inline_style_css_text_with_pdb_storage(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    css_text: &str,
) {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let mut state = if let Some(state) = inline_css_text_pdb_storage_state(css_text) {
        state
    } else {
        let runtime = unsafe { &*runtime_ptr };
        let base_url = runtime.element_inline_style_base_url(handle);
        inline_style_declaration_state_from_css_text(css_text, Some(&base_url))
    };
    state.refresh_pdb_entries();
    let css_text = state.css_text();
    let resolution_text = state.style_resolution_text();
    set_reflected_style_attribute_with_inline_base_url(
        scope,
        runtime_ptr,
        handle,
        &css_text,
        None,
        state,
        resolution_text,
    );
}
