use crate::{
    context_bootstrap::{
        adopted_style_sheet_installations_from_value,
        clear_css_style_sheet_shadow_root_adopted_owner_tracking,
        initialize_css_module_style_sheet_object, new_css_style_sheet_object,
        new_style_sheet_list_object, set_style_sheet_list_contents,
        sync_constructed_css_style_sheet_rules_from_text,
        sync_css_style_sheet_shadow_root_adopted_owner_tracking,
    },
    native_bridge::document::{
        AdoptedStyleSheetsArrayOwner, detached_native_handle_for_runtime,
        detached_shadow_root_active_element_value, detached_shadow_root_for_host,
        install_adopted_style_sheets_array_mutation_methods,
        normalize_adopted_style_sheets_assignment,
    },
    native_bridge::{DomHandle, set_wrapped_handle_or_null, wrapped_handle_value},
    util::{
        get_private_value, new_null_prototype_object, node_wrapper_from_handle, set_private_value,
        v8_string, v8str,
    },
};
use moli_encoding::decode_text_for_legacy_web;
use moli_v8_util::throw_type_error;
use moli_web_mime::{
    MimeSniffingContext, data_url_body_and_computed_mime_type, is_css_mime, mime_charset,
};
use std::collections::{HashMap, HashSet};

use super::super::super::node::{
    node_runtime_and_handle_from_args, node_runtime_and_handle_from_object,
    node_runtime_and_handle_from_object_or_detached,
};
use super::super::property_string_value;

const SHADOW_ROOT_ADOPTED_STYLE_SHEETS_SLOT: &str = "__moliAdoptedStyleSheets";
const SHADOW_ROOT_STYLE_SHEETS_SLOT: &str = "__moliShadowRootStyleSheets";
const CSS_MODULE_SHEET_CACHE_SLOT: &str = "__moliCssModuleSheetCache";
const CSS_MODULE_SHEET_LOADED_SLOT: &str = "__moliCssModuleSheetLoaded";

fn shadow_root_style_sheet_handles_in_tree_order(
    runtime: &crate::native_bridge::JsContextHost,
    root: crate::document_runtime::DomHandle,
) -> Vec<crate::document_runtime::DomHandle> {
    let mut handles = Vec::new();
    let mut stack = Vec::new();
    let mut current = Some(root);
    while let Some(handle) = current {
        let Some(node) = runtime.dom_host().node(handle) else {
            current = stack.pop();
            continue;
        };
        if matches!(node.local_name(), Some("style" | "link")) {
            handles.push(handle);
        }
        if let Some(sibling) = runtime.dom_host().next_sibling(handle) {
            stack.push(sibling);
        }
        current = runtime
            .dom_host()
            .first_child(handle)
            .or_else(|| stack.pop());
    }
    handles
}

fn set_array_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    values: &[v8::Local<'s, v8::Value>],
) {
    for (index, value) in values.iter().enumerate() {
        let _ = array.set_index(scope, index as u32, *value);
    }
    let mut index = values.len() as u32;
    while index < array.length() {
        let _ = array.delete_index(scope, index);
        index += 1;
    }
    let _ = array.set(
        scope,
        v8str(scope, "length").into(),
        v8::Integer::new(scope, values.len() as i32).into(),
    );
}

fn sync_shadow_root_style_sheets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    runtime: &crate::native_bridge::JsContextHost,
    root: crate::document_runtime::DomHandle,
) {
    let mut sheets = Vec::new();
    for sheet_handle in shadow_root_style_sheet_handles_in_tree_order(runtime, root) {
        let Some(element) = runtime
            .dom_host()
            .node(sheet_handle)
            .and_then(crate::dom::native::Node::as_element)
        else {
            continue;
        };
        if !runtime.dom_host().is_connected(sheet_handle)
            || !crate::style_engine::stylesheet_owner_can_have_sheet(element)
        {
            continue;
        }
        let Some(wrapper) = node_wrapper_from_handle(scope, sheet_handle) else {
            continue;
        };
        let Some(sheet) = wrapper.get(scope, v8str(scope, "sheet").into()) else {
            continue;
        };
        if !sheet.is_null_or_undefined() {
            sheets.push(sheet);
        }
    }
    set_style_sheet_list_contents(scope, list, &sheets);
}

fn sync_detached_shadow_root_style_sheets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    runtime: &crate::native_bridge::JsContextHost,
    root: crate::document_runtime::DomHandle,
) {
    let mut sheets = Vec::new();
    for sheet_handle in shadow_root_style_sheet_handles_in_tree_order(runtime, root) {
        let Some(element) = runtime
            .dom_host()
            .node(sheet_handle)
            .and_then(crate::dom::native::Node::as_element)
        else {
            continue;
        };
        if !crate::style_engine::stylesheet_owner_can_have_sheet(element) {
            continue;
        }
        if element.local_name().eq_ignore_ascii_case("link")
            && !runtime
                .dom_host()
                .get_attribute(sheet_handle, "rel")
                .unwrap_or_default()
                .split_ascii_whitespace()
                .any(|token| token.eq_ignore_ascii_case("stylesheet"))
        {
            continue;
        }
        if let Some(sheet) = wrapped_handle_value(scope, runtime_ptr, sheet_handle) {
            sheets.push(sheet);
        }
    }
    set_style_sheet_list_contents(scope, list, &sheets);
}

pub(in crate::native_bridge) fn shadow_root_host_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(host_handle) = runtime.dom_host().shadow_root_host(handle) else {
        rv.set_null();
        return;
    };
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, Some(host_handle));
}

pub(crate) fn shadow_root_adopted_style_sheets_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    get_shadow_root_adopted_style_sheets_for_receiver(scope, args.this(), &mut rv);
}

pub(crate) fn shadow_root_adopted_style_sheets_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_shadow_root_adopted_style_sheets_for_receiver(scope, args.this(), args.get(0));
    rv.set_undefined();
}

fn get_shadow_root_adopted_style_sheets_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_shadow_root_style_sheets_illegal_invocation(scope, "adoptedStyleSheets");
        rv.set_undefined();
        return;
    };
    if !unsafe { &*runtime_ptr }.dom_host().is_shadow_root(handle) {
        throw_shadow_root_style_sheets_illegal_invocation(scope, "adoptedStyleSheets");
        rv.set_undefined();
        return;
    }
    let holder = receiver;
    ensure_shadow_root_adopted_style_sheets_initialized(
        scope,
        unsafe { &mut *runtime_ptr },
        handle,
        holder,
    );
    if let Some(existing) = get_private_value(scope, holder, SHADOW_ROOT_ADOPTED_STYLE_SHEETS_SLOT)
        && !existing.is_undefined()
    {
        rv.set(existing);
        return;
    }
    rv.set_undefined();
}

pub(crate) fn ensure_shadow_root_adopted_style_sheets_initialized<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    holder: v8::Local<'s, v8::Object>,
) {
    let array = v8::Array::new(scope, 0);
    if let Some(existing) = get_private_value(scope, holder, SHADOW_ROOT_ADOPTED_STYLE_SHEETS_SLOT)
        && !existing.is_undefined()
    {
        if !shadow_root_has_declarative_adopted_style_sheet_specifiers(runtime, handle) {
            // JS assignment and installed array mutation methods update style sources
            // synchronously. Without declarative specifiers, the getter has nothing to
            // refresh after the wrapper has been initialized.
            return;
        }
        refresh_declarative_css_module_sheets(scope, runtime, handle);
        let installations = adopted_style_sheet_installations_from_value(scope, existing);
        runtime.set_shadow_root_adopted_style_sheet_installations(handle, installations);
        if let Ok(array) = v8::Local::<v8::Object>::try_from(existing) {
            sync_css_style_sheet_shadow_root_adopted_owner_tracking(scope, array, handle);
        }
        return;
    }
    install_adopted_style_sheets_array_mutation_methods(
        scope,
        array,
        AdoptedStyleSheetsArrayOwner::ShadowRoot(handle),
    );
    populate_declarative_adopted_style_sheets(scope, runtime, handle, array);
    set_private_value(
        scope,
        holder,
        SHADOW_ROOT_ADOPTED_STYLE_SHEETS_SLOT,
        array.into(),
    );
    sync_css_style_sheet_shadow_root_adopted_owner_tracking(scope, array.into(), handle);
}

fn shadow_root_has_declarative_adopted_style_sheet_specifiers(
    runtime: &crate::native_bridge::JsContextHost,
    shadow_root: crate::document_runtime::DomHandle,
) -> bool {
    runtime
        .dom_host()
        .shadow_root_adopted_style_sheets(shadow_root)
        .and_then(|value| value)
        .is_some_and(|value| value.split_ascii_whitespace().next().is_some())
}

pub(crate) fn clear_shadow_root_adopted_style_sheets(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) {
    runtime.set_shadow_root_adopted_style_sheet_installations(handle, Vec::new());
    let Some(holder) = node_wrapper_from_handle(scope, handle) else {
        return;
    };
    if let Some(existing) = get_private_value(scope, holder, SHADOW_ROOT_ADOPTED_STYLE_SHEETS_SLOT)
        && let Ok(array) = v8::Local::<v8::Array>::try_from(existing)
    {
        set_array_contents(scope, array, &[]);
        install_adopted_style_sheets_array_mutation_methods(
            scope,
            array,
            AdoptedStyleSheetsArrayOwner::ShadowRoot(handle),
        );
        sync_css_style_sheet_shadow_root_adopted_owner_tracking(scope, array.into(), handle);
        return;
    }
    let array = v8::Array::new(scope, 0);
    install_adopted_style_sheets_array_mutation_methods(
        scope,
        array,
        AdoptedStyleSheetsArrayOwner::ShadowRoot(handle),
    );
    set_private_value(
        scope,
        holder,
        SHADOW_ROOT_ADOPTED_STYLE_SHEETS_SLOT,
        array.into(),
    );
    sync_css_style_sheet_shadow_root_adopted_owner_tracking(scope, array.into(), handle);
}

fn refresh_declarative_css_module_sheets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &mut crate::native_bridge::JsContextHost,
    shadow_root: crate::document_runtime::DomHandle,
) {
    let Some(Some(value)) = runtime
        .dom_host()
        .shadow_root_adopted_style_sheets(shadow_root)
    else {
        return;
    };
    let mut declarative_modules = None;
    let mut declarative_import_map = None;
    let mut seen = HashSet::new();
    for specifier in value.split_ascii_whitespace() {
        if !seen.insert(specifier) {
            continue;
        }
        refresh_declarative_css_module_sheet(
            scope,
            runtime,
            shadow_root,
            specifier,
            &mut declarative_import_map,
            &mut declarative_modules,
        );
    }
}

fn refresh_declarative_css_module_sheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &mut crate::native_bridge::JsContextHost,
    shadow_root: crate::document_runtime::DomHandle,
    specifier: &str,
    declarative_import_map: &mut Option<HashMap<String, String>>,
    declarative_modules: &mut Option<HashMap<String, DeclarativeCssModule>>,
) {
    let _ = declarative_css_module_sheet_for_specifier(
        scope,
        runtime,
        shadow_root,
        specifier,
        declarative_import_map,
        declarative_modules,
    );
}

fn populate_declarative_adopted_style_sheets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &mut crate::native_bridge::JsContextHost,
    shadow_root: crate::document_runtime::DomHandle,
    array: v8::Local<'s, v8::Array>,
) {
    let Some(Some(value)) = runtime
        .dom_host()
        .shadow_root_adopted_style_sheets(shadow_root)
    else {
        return;
    };
    let mut sheets = Vec::new();
    let mut declarative_modules = None;
    let mut declarative_import_map = None;
    let mut seen = HashSet::new();
    for specifier in value.split_ascii_whitespace() {
        if !seen.insert(specifier) {
            continue;
        }
        let sheet = declarative_css_module_sheet_for_specifier(
            scope,
            runtime,
            shadow_root,
            specifier,
            &mut declarative_import_map,
            &mut declarative_modules,
        );
        if let Some(sheet) = sheet {
            sheets.push(sheet.into());
        }
    }
    set_array_contents(scope, array, &sheets);
    let installations = adopted_style_sheet_installations_from_value(scope, array.into());
    runtime.set_shadow_root_adopted_style_sheet_installations(shadow_root, installations);
}

#[derive(Clone)]
struct DeclarativeCssModule {
    cache_key: String,
    css_text: String,
}

fn declarative_css_module_sheet_for_specifier<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &mut crate::native_bridge::JsContextHost,
    shadow_root: crate::document_runtime::DomHandle,
    specifier: &str,
    declarative_import_map: &mut Option<HashMap<String, String>>,
    declarative_modules: &mut Option<HashMap<String, DeclarativeCssModule>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let owner_document = shadow_root_owner_document(runtime, shadow_root);
    let base_url = shadow_root_adopted_style_base_url(runtime, shadow_root);
    if let Some(owner_document) = owner_document {
        let import_map = declarative_import_map
            .get_or_insert_with(|| collect_declarative_import_map_entries(runtime, owner_document));
        if let Some(mapped) = import_map.get(specifier)
            && let Some(sheet) =
                css_module_sheet_for_mapped_specifier(scope, runtime, mapped, &base_url)
        {
            return Some(sheet);
        }
        if owner_document == runtime.document_handle()
            && let Ok(resolved_url) =
                runtime.resolve_module_specifier_with_base(specifier, &base_url)
        {
            return css_module_sheet_for_resolved_url(scope, runtime, &resolved_url);
        }
    }
    if css_module_specifier_is_url_like(specifier)
        && let Some(sheet) =
            css_module_sheet_for_mapped_specifier(scope, runtime, specifier, &base_url)
    {
        return Some(sheet);
    }
    let owner_document = owner_document?;
    let modules = declarative_modules.get_or_insert_with(|| {
        collect_declarative_css_module_entries_for_document(runtime, owner_document)
    });
    modules.get(specifier).and_then(|module| {
        css_module_sheet_for_url(scope, &module.cache_key, Some(&module.css_text))
    })
}

fn css_module_sheet_for_mapped_specifier<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &crate::native_bridge::JsContextHost,
    specifier: &str,
    base_url: &url::Url,
) -> Option<v8::Local<'s, v8::Object>> {
    let resolved_url = url::Url::options()
        .base_url(Some(base_url))
        .parse(specifier)
        .ok()?;
    css_module_sheet_for_resolved_url(scope, runtime, &resolved_url)
}

fn css_module_sheet_for_resolved_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &crate::native_bridge::JsContextHost,
    resolved_url: &url::Url,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(css_text) = decoded_css_data_url_text(resolved_url)
        .or_else(|| runtime.css_module_text_for_url(resolved_url))
    {
        return css_module_sheet_for_url(scope, resolved_url.as_str(), Some(css_text.as_str()));
    }
    if runtime.css_module_failed_for_url(resolved_url) {
        return None;
    }
    css_module_sheet_for_url(scope, resolved_url.as_str(), None)
}

fn shadow_root_owner_document(
    runtime: &crate::native_bridge::JsContextHost,
    shadow_root: crate::document_runtime::DomHandle,
) -> Option<crate::document_runtime::DomHandle> {
    runtime
        .dom_host()
        .node(shadow_root)
        .and_then(crate::dom::native::Node::owner_document)
}

fn collect_declarative_import_map_entries(
    runtime: &crate::native_bridge::JsContextHost,
    document: crate::document_runtime::DomHandle,
) -> HashMap<String, String> {
    let mut stack = vec![document];
    while let Some(handle) = stack.pop() {
        if let Some(node) = runtime.dom_host().node(handle)
            && node.as_element().is_some_and(|element| {
                element.is_html_element("script")
                    && runtime
                        .dom_host()
                        .get_attribute(handle, "type")
                        .is_some_and(|value| value.eq_ignore_ascii_case("importmap"))
            })
        {
            let mut imports = HashMap::new();
            if let Some(text) = runtime.dom_host().text_content(handle)
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                && let Some(entries) = value.get("imports").and_then(serde_json::Value::as_object)
            {
                for (specifier, mapped) in entries {
                    if let Some(mapped) = mapped.as_str() {
                        imports.insert(specifier.clone(), mapped.to_owned());
                    }
                }
            }
            return imports;
        }
        stack.extend(runtime.dom_host().child_handles_reversed(handle));
    }
    HashMap::new()
}

fn collect_declarative_css_module_entries_for_document(
    runtime: &crate::native_bridge::JsContextHost,
    document: crate::document_runtime::DomHandle,
) -> HashMap<String, DeclarativeCssModule> {
    let mut entries = HashMap::new();
    let mut stack = vec![document];
    while let Some(handle) = stack.pop() {
        if let Some(node) = runtime.dom_host().node(handle)
            && node.as_element().is_some_and(|element| {
                element.is_html_element("style")
                    && runtime
                        .dom_host()
                        .get_attribute(handle, "type")
                        .is_some_and(|value| value.eq_ignore_ascii_case("module"))
            })
            && runtime.dom_host().is_connected(handle)
            && let Some(specifier) = runtime.dom_host().get_attribute(handle, "specifier")
            && let Some(css_text) = runtime.dom_host().text_content(handle)
        {
            entries.insert(
                specifier,
                DeclarativeCssModule {
                    cache_key: format!("declarative-css-module:{}", handle.index()),
                    css_text,
                },
            );
        }
        stack.extend(runtime.dom_host().child_handles_reversed(handle));
    }
    entries
}

fn decoded_css_data_url_text(url: &url::Url) -> Option<String> {
    if url.scheme() != "data" {
        return None;
    }
    let (body, mime_type) =
        data_url_body_and_computed_mime_type(url.as_str(), MimeSniffingContext::Browsing)?;
    if !is_css_mime(&mime_type) {
        return None;
    }
    Some(decode_text_for_legacy_web(
        &body,
        mime_charset(&mime_type).as_deref(),
    ))
}

fn css_module_specifier_is_url_like(specifier: &str) -> bool {
    specifier.starts_with('/')
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || url::Url::parse(specifier).is_ok()
}

pub(crate) fn css_module_sheet_for_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    url: &str,
    css_text: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let cache_key = v8str(scope, CSS_MODULE_SHEET_CACHE_SLOT);
    let cache = global
        .get(scope, cache_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or_else(|| {
            let cache = new_null_prototype_object(scope);
            let _ = global.define_own_property(
                scope,
                cache_key.into(),
                cache.into(),
                v8::PropertyAttribute::DONT_ENUM
                    | v8::PropertyAttribute::DONT_DELETE
                    | v8::PropertyAttribute::READ_ONLY,
            );
            cache
        });
    let url_key = v8_string(scope, url)?;
    if let Some(existing) = cache.get(scope, url_key.into())
        && let Ok(sheet) = v8::Local::<v8::Object>::try_from(existing)
    {
        if let Some(css_text) = css_text
            && !css_module_sheet_is_loaded(scope, sheet)
        {
            sync_constructed_css_style_sheet_rules_from_text(scope, sheet, css_text);
            set_css_module_sheet_loaded(scope, sheet, true);
        }
        return Some(sheet);
    }
    let sheet = new_css_style_sheet_object(scope);
    initialize_css_module_style_sheet_object(scope, sheet, url);
    if let Some(css_text) = css_text {
        sync_constructed_css_style_sheet_rules_from_text(scope, sheet, css_text);
        set_css_module_sheet_loaded(scope, sheet, true);
    } else {
        set_css_module_sheet_loaded(scope, sheet, false);
    }
    let _ = cache.set(scope, url_key.into(), sheet.into());
    Some(sheet)
}

fn css_module_sheet_is_loaded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, sheet, CSS_MODULE_SHEET_LOADED_SLOT)
        .is_some_and(|value| value.is_true())
}

fn set_css_module_sheet_loaded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    loaded: bool,
) {
    let value = v8::Boolean::new(scope, loaded);
    set_private_value(scope, sheet, CSS_MODULE_SHEET_LOADED_SLOT, value.into());
}

fn set_shadow_root_adopted_style_sheets_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_shadow_root_style_sheets_illegal_invocation(scope, "adoptedStyleSheets");
        return;
    };
    if !unsafe { &*runtime_ptr }.dom_host().is_shadow_root(handle) {
        throw_shadow_root_style_sheets_illegal_invocation(scope, "adoptedStyleSheets");
        return;
    }
    let holder = receiver;
    let owner_document = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::owner_document);
    let Some(next_array) = normalize_adopted_style_sheets_assignment(
        scope,
        value,
        AdoptedStyleSheetsArrayOwner::ShadowRoot(handle),
        owner_document,
    ) else {
        return;
    };
    let next_value = next_array.into();
    let installations = adopted_style_sheet_installations_from_value(scope, next_value);
    if let Some(previous) = get_private_value(scope, holder, SHADOW_ROOT_ADOPTED_STYLE_SHEETS_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        clear_css_style_sheet_shadow_root_adopted_owner_tracking(scope, previous, handle);
    }
    set_private_value(
        scope,
        holder,
        SHADOW_ROOT_ADOPTED_STYLE_SHEETS_SLOT,
        next_value,
    );
    unsafe { &mut *runtime_ptr }
        .set_shadow_root_adopted_style_sheet_installations(handle, installations);
    sync_css_style_sheet_shadow_root_adopted_owner_tracking(scope, next_array.into(), handle);
}

fn shadow_root_adopted_style_base_url(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> url::Url {
    runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::owner_document)
        .map(|document| runtime.document_base_url_for_handle(document))
        .unwrap_or_else(|| runtime.document_url().clone())
}

pub(in crate::native_bridge) fn shadow_root_active_element_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let is_live_receiver = node_runtime_and_handle_from_object(scope, args.this()).is_ok();
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    if !unsafe { &*runtime_ptr }.dom_host().is_shadow_root(handle) {
        rv.set_undefined();
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(active_handle) = runtime.active_element_handle() else {
        if !is_live_receiver
            && let Some(value) = detached_shadow_root_active_element_value(scope, args.this())
        {
            rv.set(value);
            return;
        }
        rv.set_null();
        return;
    };
    let Some(active_handle) = shadow_root_retargeted_active_element(runtime, handle, active_handle)
    else {
        rv.set_null();
        return;
    };
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, Some(active_handle));
}

fn shadow_root_retargeted_active_element(
    runtime: &crate::native_bridge::JsContextHost,
    root: DomHandle,
    active: DomHandle,
) -> Option<DomHandle> {
    let mut current = active;
    while let Some(containing_root) = runtime.dom_host().containing_shadow_root(current) {
        if containing_root == root {
            return Some(current);
        }
        current = runtime.dom_host().shadow_root_host(containing_root)?;
    }
    None
}

pub(crate) fn shadow_root_style_sheets_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    get_shadow_root_style_sheets_for_receiver(scope, args.this(), &mut rv);
}

fn get_shadow_root_style_sheets_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let is_live_receiver = node_runtime_and_handle_from_object(scope, receiver).is_ok();
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_shadow_root_style_sheets_illegal_invocation(scope, "styleSheets");
        rv.set_undefined();
        return;
    };
    if !unsafe { &*runtime_ptr }.dom_host().is_shadow_root(handle) {
        throw_shadow_root_style_sheets_illegal_invocation(scope, "styleSheets");
        rv.set_undefined();
        return;
    }
    let holder = receiver;
    let slot = v8str(scope, SHADOW_ROOT_STYLE_SHEETS_SLOT);
    if let Some(existing) = holder.get(scope, slot.into())
        && !existing.is_undefined()
    {
        if let Ok(list) = v8::Local::<v8::Object>::try_from(existing) {
            if is_live_receiver {
                sync_shadow_root_style_sheets(scope, list, unsafe { &*runtime_ptr }, handle);
            } else {
                sync_detached_shadow_root_style_sheets(
                    scope,
                    list,
                    runtime_ptr,
                    unsafe { &*runtime_ptr },
                    handle,
                );
            }
            rv.set(list.into());
            return;
        }
        rv.set(existing);
        return;
    }
    let list = new_style_sheet_list_object(scope);
    let _ = holder.set(scope, slot.into(), list.into());
    if is_live_receiver {
        sync_shadow_root_style_sheets(scope, list, unsafe { &*runtime_ptr }, handle);
    } else {
        sync_detached_shadow_root_style_sheets(
            scope,
            list,
            runtime_ptr,
            unsafe { &*runtime_ptr },
            handle,
        );
    }
    rv.set(list.into());
}

fn throw_shadow_root_style_sheets_illegal_invocation(
    scope: &mut v8::PinScope<'_, '_>,
    member: &str,
) {
    throw_type_error(
        scope,
        &format!("Failed to get '{member}' on 'ShadowRoot': Illegal invocation."),
    );
}

pub(in crate::native_bridge) fn shadow_root_mode_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let Some(mode) = unsafe { &*runtime_ptr }.dom_host().shadow_root_mode(handle) else {
        rv.set_null();
        return;
    };
    if let Some(mode) = v8_string(scope, &mode) {
        rv.set(mode.into());
    } else {
        rv.set_null();
    }
}

pub(in crate::native_bridge) fn shadow_root_delegates_focus_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = unsafe { &*runtime_ptr }
        .dom_host()
        .shadow_root_delegates_focus(handle)
    else {
        rv.set_undefined();
        return;
    };
    rv.set(v8::Boolean::new(scope, value).into());
}

pub(in crate::native_bridge) fn shadow_root_slot_assignment_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = unsafe { &*runtime_ptr }
        .dom_host()
        .shadow_root_slot_assignment(handle)
    else {
        rv.set_undefined();
        return;
    };
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_null();
    }
}

pub(in crate::native_bridge) fn shadow_root_clonable_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = unsafe { &*runtime_ptr }
        .dom_host()
        .shadow_root_clonable(handle)
    else {
        rv.set_undefined();
        return;
    };
    rv.set(v8::Boolean::new(scope, value).into());
}

pub(in crate::native_bridge) fn shadow_root_serializable_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = unsafe { &*runtime_ptr }
        .dom_host()
        .shadow_root_serializable(handle)
    else {
        rv.set_undefined();
        return;
    };
    rv.set(v8::Boolean::new(scope, value).into());
}

pub(in crate::native_bridge) fn shadow_root_reference_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = unsafe { &*runtime_ptr }
        .dom_host()
        .shadow_root_reference_target(handle)
    else {
        rv.set_undefined();
        return;
    };
    match value {
        Some(value) => {
            if let Some(value) = v8_string(scope, &value) {
                rv.set(value.into());
            } else {
                rv.set_null();
            }
        }
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn shadow_root_reference_target_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = args.get(0);
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let reference_target = if value.is_null() {
        None
    } else {
        let Some(value) = property_string_value(scope, value) else {
            rv.set_undefined();
            return;
        };
        Some(value)
    };
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_shadow_root_reference_target(handle, reference_target);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn element_shadow_root_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        match detached_shadow_root_for_host(scope, args.this()) {
            Some(root) => rv.set(root.into()),
            None => rv.set_null(),
        }
        return;
    };
    if detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        match detached_shadow_root_for_host(scope, args.this()) {
            Some(root) => rv.set(root.into()),
            None => rv.set_null(),
        }
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(root_handle) = runtime.dom_host().shadow_root_handle(handle) else {
        rv.set_null();
        return;
    };
    if runtime.dom_host().shadow_root_mode(root_handle).as_deref() != Some("open") {
        rv.set_null();
        return;
    }
    match runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, root_handle)
    {
        Some(root) => {
            ensure_shadow_root_adopted_style_sheets_initialized(scope, runtime, root_handle, root);
            rv.set(root.into());
        }
        None => rv.set_null(),
    }
}
