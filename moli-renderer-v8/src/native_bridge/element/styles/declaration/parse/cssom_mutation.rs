use super::declaration_parser::parse_style_property_entries_with_base;
use super::inline_state::{
    inline_css_text_pdb_storage_state, inline_state_block_entries_for_property_mutation,
    inline_state_has_replaceable_side_entries_for_property,
    inline_state_has_unpreservable_side_entries_for_property,
    inline_style_declaration_state_from_css_text, refresh_inline_state_entries_after_pdb_mutation,
    style_entry_is_preservable_for_pdb_property, style_entry_is_replaceable_by_pdb_property,
};
use super::pdb_compat::{
    css_value_uses_unresolved_cssom_storage, cssom_empty_specified_placeholder_property,
    cssom_style_property_uses_preferred_pdb_supplemental_entries,
    cssom_style_property_write_uses_pdb, inline_style_property_write_can_use_pdb_storage,
    parse_pdb_style_property, parse_style_property_entries_with_pdb,
    set_pdb_block_property_collecting_entries, style_entry_affects_property_query,
    style_entry_is_pdb_supplemental_side_entry, style_property_mutation_affected_names_with_pdb,
    style_property_mutation_cleanup_names_with_pdb,
};
use super::*;
use crate::css_style::{CssomStyleAttribute, InlineStyleParseContext};

pub(crate) fn parse_inline_css_text_with_base(
    style_text: &str,
    base_url: Option<&url::Url>,
) -> Vec<StyleEntry> {
    parse_inline_declaration_entries(&parse_css_declaration_list(style_text), base_url)
}

pub(super) fn parse_inline_declaration_entries(
    declarations: &[StyleEntry],
    base_url: Option<&url::Url>,
) -> Vec<StyleEntry> {
    let mut entries: Vec<StyleEntry> = Vec::new();
    for declaration in declarations {
        let name = canonical_style_property_name(declaration.name.trim());
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

pub(in crate::native_bridge::element::styles) enum InlineStylePropertyWrite {
    NeedsCompatibilityEntries,
    NoAttributeWrite,
    AttributeWritten,
}

impl InlineStylePropertyWrite {
    fn from_attribute_write(written: bool) -> Self {
        if written {
            Self::AttributeWritten
        } else {
            Self::NoAttributeWrite
        }
    }
}

pub(in crate::native_bridge::element::styles) fn set_inline_style_property_with_pdb_storage(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    name: &str,
    value: &str,
    priority: bool,
) -> InlineStylePropertyWrite {
    let custom = moli_css_parse::is_cssom_custom_property_name(name);
    if name != "all" && !custom && !inline_style_property_write_can_use_pdb_storage(name, value) {
        return InlineStylePropertyWrite::NeedsCompatibilityEntries;
    }
    let runtime = unsafe { &*runtime_ptr };
    let existing_inline_base_url = runtime.existing_element_inline_style_base_url(handle);
    let blocked = runtime.element_inline_style_csp_state(handle)
        == crate::style_engine::InlineStyleCspState::BlockedAttribute;
    let cached = runtime
        .element_inline_style_declaration_state(handle)
        .filter(|_| !blocked);
    let initial = cached.is_none().then(|| {
        if blocked {
            CssInlineStyleDeclarationState::default()
        } else {
            let mut state = inline_style_declaration_state_from_css_text(
                &style_string(runtime, handle),
                existing_inline_base_url.as_ref(),
            );
            state.refresh_pdb_entries();
            state
        }
    });
    let update_inline_style_base = name == "background-image" && !value.is_empty();
    let inline_base_url = if update_inline_style_base {
        Some(runtime.element_inline_style_base_url(handle))
    } else {
        existing_inline_base_url
    };
    let current = cached
        .as_deref()
        .or(initial.as_ref())
        .expect("inline style declaration state");
    if current.compatibility.is_none() && name != "all" {
        let parsed = (!value.is_empty()
            && (custom || !css_value_uses_unresolved_cssom_storage(value))
            && !cssom_style_property_uses_preferred_pdb_supplemental_entries(
                name, value, priority,
            ))
        .then(|| {
            let document = runtime.dom_host().owner_document_handle(handle)?;
            let base = inline_base_url
                .clone()
                .unwrap_or_else(|| runtime.element_inline_style_base_url(handle));
            // CSSOM parses in standards mode. Rendering uses the document mode
            // and may consume this block when both contexts agree.
            let mode = QuirksMode::NoQuirks;
            let matching_context = current
                .parse_context
                .as_ref()
                .filter(|previous| previous.matches(document, &base, mode));
            let publishable = matching_context.is_some() || current.block.is_empty();
            let context = matching_context
                .cloned()
                .unwrap_or_else(|| InlineStyleParseContext {
                    document,
                    url_data: UrlExtraData::from(base),
                    quirks_mode: mode,
                });
            moli_css_parse::parse_cssom_property(name, value, priority, &context.url_data, mode)
                .filter(moli_css_parse::ParsedCssomProperty::is_ordinary_declaration_list)
                .map(|declarations| (declarations, publishable.then_some(context)))
        })
        .flatten();
        if value.is_empty() || parsed.is_some() {
            let removal_names = if value.is_empty() {
                let Some(names) = style_property_mutation_affected_names_with_pdb(name) else {
                    return InlineStylePropertyWrite::NeedsCompatibilityEntries;
                };
                if !names
                    .iter()
                    .any(|property| current.block.property_is_declared(property))
                {
                    return InlineStylePropertyWrite::NoAttributeWrite;
                }
                names
            } else {
                Vec::new()
            };
            let cleanup_names = style_property_mutation_cleanup_names_with_pdb(name);
            let compare_longhand = cleanup_names.is_empty() && !update_inline_style_base;
            let can_defer = !blocked
                && compare_longhand
                && parsed.as_ref().map_or(current.parse_context.is_some(), |(property, context)| {
                    property.is_single_declaration() && context.is_some()
                })
                // Raw attribute text can have different spelling even when its
                // declarations equal the newly serialized CSSOM block.
                && (current.parse_context.is_some()
                    || !runtime.dom_host().dom().has_attribute(handle, "style"))
                && runtime.can_defer_cssom_attribute(handle);
            let mutate = |state: &mut CssInlineStyleDeclarationState| {
                if let Some((parsed, context)) = parsed {
                    if !state.block.apply_parsed_property(parsed, compare_longhand) {
                        return false;
                    }
                    state.parse_context = context;
                    for property in cleanup_names {
                        state.block.remove_property(&property);
                    }
                } else {
                    for property in &removal_names {
                        state.block.remove_property(property);
                    }
                }
                true
            };
            if can_defer
                && !crate::custom_elements::observes_attribute_change(runtime_ptr, handle, "style")
                && !crate::observer_runtime::observes_attribute_old_value(
                    runtime_ptr,
                    handle,
                    "style",
                )
            {
                drop(cached);
                return InlineStylePropertyWrite::from_attribute_write(
                    commit_deferred_inline_property(scope, runtime_ptr, handle, initial, mutate),
                );
            }
            // The committed attribute supplies the previous text until the
            // exact attribute mutation has been applied.
            let mut state = initial
                .or_else(|| cached.as_deref().cloned())
                .expect("inline style declaration state");
            drop(cached);
            if !mutate(&mut state) {
                return InlineStylePropertyWrite::NoAttributeWrite;
            }
            return InlineStylePropertyWrite::from_attribute_write(commit_inline_property_state(
                scope,
                runtime_ptr,
                handle,
                state,
                inline_base_url.as_ref(),
                update_inline_style_base,
            ));
        }
    }
    let Some(affected_names) = style_property_mutation_affected_names_with_pdb(name) else {
        return InlineStylePropertyWrite::NeedsCompatibilityEntries;
    };
    let mut state = initial
        .or_else(|| cached.as_deref().cloned())
        .expect("compatibility inline style declaration state");
    drop(cached);
    state.parse_context = None;
    if inline_state_has_unpreservable_side_entries_for_property(&state, name, &affected_names) {
        return InlineStylePropertyWrite::NeedsCompatibilityEntries;
    }
    let has_replaceable_side_entry =
        inline_state_has_replaceable_side_entries_for_property(&state, name, &affected_names);
    let new_entries = if value.is_empty() {
        let mut removed_from_block = false;
        for affected in &affected_names {
            removed_from_block |= state.block.remove_property(affected).changed;
        }
        if !removed_from_block && !has_replaceable_side_entry {
            return InlineStylePropertyWrite::NoAttributeWrite;
        }
        (Vec::new(), Vec::new())
    } else if name == "all" {
        let Some(parsed) = parse_style_property_entries_with_base(name, value, priority, None)
        else {
            return InlineStylePropertyWrite::NeedsCompatibilityEntries;
        };
        if state.block.set_property(name, value, priority)
            == moli_css_parse::CssSetResult::ParseError
        {
            return InlineStylePropertyWrite::NeedsCompatibilityEntries;
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
        let Some((parsed, native)) = parse_pdb_style_property(name, value, priority) else {
            return InlineStylePropertyWrite::NeedsCompatibilityEntries;
        };
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
            let mut entries = if let Some(native) = native {
                state.block.apply_parsed_property(native, false);
                parsed.entries.clone()
            } else {
                let Some(entries) = set_pdb_block_property_collecting_entries(
                    &mut state.block,
                    name,
                    value,
                    priority,
                    &parsed,
                    uses_preferred_supplemental_entries,
                ) else {
                    return InlineStylePropertyWrite::NeedsCompatibilityEntries;
                };
                entries
            };
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
    InlineStylePropertyWrite::from_attribute_write(commit_inline_property_state(
        scope,
        runtime_ptr,
        handle,
        state,
        inline_base_url.as_ref(),
        update_inline_style_base,
    ))
}

fn commit_deferred_inline_property(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    initial: Option<CssInlineStyleDeclarationState>,
    mutate: impl FnOnce(&mut CssInlineStyleDeclarationState) -> bool,
) -> bool {
    let runtime = unsafe { &mut *runtime_ptr };
    let effects = if runtime.element_cssom_style(handle).is_some() {
        runtime
            .dom_host_mut()
            .edit_cssom_style_attribute(handle, mutate)
    } else {
        let mut state = initial.expect("uncached inline style declaration state");
        if !mutate(&mut state) {
            return false;
        }
        runtime
            .dom_host_mut()
            .set_cssom_style_attribute(handle, Box::new(CssomStyleAttribute::new(state)), None)
            .into_effects()
    };
    let style_input_changed = effects.did_change();
    runtime.apply_cssom_style_attribute_effects(
        scope,
        runtime_ptr,
        handle,
        effects,
        style_input_changed,
    )
}

fn commit_inline_property_state(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    state: CssInlineStyleDeclarationState,
    base_url: Option<&url::Url>,
    update_base_url: bool,
) -> bool {
    let (css_text, override_text) = state.css_text_and_style_resolution_override();
    let attribute_changed = style_string(unsafe { &*runtime_ptr }, handle) != css_text;
    commit_inline_style_declaration(
        scope,
        runtime_ptr,
        handle,
        state,
        attribute_changed.then_some(css_text.as_str()),
        override_text,
        base_url.filter(|_| attribute_changed || update_base_url),
    );
    attribute_changed
}

pub(in crate::native_bridge::element::styles) fn set_inline_style_css_text_with_pdb_storage(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    css_text: &str,
    base_url: &url::Url,
) {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let (declarations, source_preserved) = crate::css_style::scan_inline_css_text(css_text);
    let mut state = if let Some(state) =
        inline_css_text_pdb_storage_state(css_text, &declarations, source_preserved)
    {
        state
    } else {
        let entries = parse_inline_declaration_entries(&declarations, Some(base_url));
        super::inline_state::inline_style_declaration_state_from_entries(&entries)
    };
    state.refresh_pdb_entries();
    let (css_text, override_text) = state.css_text_and_style_resolution_override();
    commit_inline_style_declaration(
        scope,
        runtime_ptr,
        handle,
        state,
        Some(&css_text),
        override_text,
        Some(base_url),
    );
}

pub(super) fn commit_inline_style_declaration(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    state: CssInlineStyleDeclarationState,
    attribute_text: Option<&str>,
    override_text: Option<String>,
    base_url: Option<&url::Url>,
) {
    let publish_base_url = |runtime: &mut PageScriptHost| {
        if let Some(base_url) = base_url {
            runtime.set_element_inline_style_base_url(handle, base_url.clone());
        }
    };
    let Some(attribute_text) = attribute_text else {
        let runtime = unsafe { &mut *runtime_ptr };
        publish_base_url(runtime);
        runtime.set_element_inline_style_declaration_state(handle, state, override_text);
        return;
    };
    crate::custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let source =
            CssomStyleAttribute::with_text(state, attribute_text.to_owned(), override_text, true);
        let base_url_changed = base_url.is_some_and(|base_url| {
            runtime
                .existing_element_inline_style_base_url(handle)
                .unwrap_or_else(|| runtime.element_inline_style_base_url(handle))
                != *base_url
        });
        runtime.set_style_attribute_from_cssom_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            source,
            attribute_text,
            base_url_changed,
        );
        // Publish the CSSOM and rendering views before the reaction scope
        // invokes attributeChangedCallback and permits another mutation.
        publish_base_url(runtime);
    });
}
