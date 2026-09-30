use super::cssom_mutation::parse_inline_css_text_with_base;
use super::inline_state::{
    inline_style_declaration_state_from_entries,
    inline_style_declaration_state_from_serialized_entries,
};
use super::*;

pub(in crate::native_bridge::element::styles) fn style_entries(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Vec<StyleEntry> {
    if runtime.element_inline_style_csp_state(handle)
        == crate::style_engine::InlineStyleCspState::BlockedAttribute
    {
        return Vec::new();
    }
    if let Some(state) = runtime.element_inline_style_declaration_state(handle) {
        return state.entries();
    }
    parse_inline_css_text_with_base(&style_string(runtime, handle), None)
}

pub(super) fn style_entries_with_base(
    runtime: &JsContextHost,
    handle: DomHandle,
    base_url: Option<&url::Url>,
) -> Vec<StyleEntry> {
    if runtime.element_inline_style_csp_state(handle)
        == crate::style_engine::InlineStyleCspState::BlockedAttribute
    {
        return Vec::new();
    }
    if let Some(state) = runtime.element_inline_style_declaration_state(handle) {
        return state.entries();
    }
    parse_inline_css_text_with_base(&style_string(runtime, handle), base_url)
}

pub(in crate::native_bridge::element::styles) fn style_entries_for_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    runtime: &JsContextHost,
    handle: DomHandle,
) -> StyleObjectEntries {
    let base_url = get_private_value(scope, style, STYLE_DECLARATION_BASE_URL_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .and_then(|value| url::Url::parse(&value.to_rust_string_lossy(scope)).ok());
    let entries = style_entries_with_base(runtime, handle, base_url.as_ref());
    StyleObjectEntries { entries, base_url }
}

pub(in crate::native_bridge::element::styles) fn set_style_entries_with_inline_base_url(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    entries: &[StyleEntry],
    inline_base_url: Option<&url::Url>,
    property: &str,
) {
    let state = inline_style_declaration_state_from_entries(entries);
    let css_text = state.css_text();
    let resolution_text = state.style_resolution_text();
    let mut state =
        inline_style_declaration_state_from_serialized_entries(entries, &css_text, inline_base_url);
    if let Some(previous) = unsafe { &*runtime_ptr }.element_inline_style_declaration_state(handle)
    {
        state
            .block
            .retain_unaffected_typed_numeric_values(&previous.block, property);
    }
    set_reflected_style_attribute_with_inline_base_url(
        scope,
        runtime_ptr,
        handle,
        &css_text,
        inline_base_url,
        state,
        resolution_text,
    );
}

pub(in crate::native_bridge::element::styles) fn set_style_entries_if_changed_with_inline_base_url(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    entries: &[StyleEntry],
    inline_base_url: Option<&url::Url>,
    property: &str,
    numeric: Option<moli_css_parse::CssDeclaredNumericValue>,
) -> bool {
    let state = inline_style_declaration_state_from_entries(entries);
    let css_text = state.css_text();
    let resolution_text = state.style_resolution_text();
    let mut state =
        inline_style_declaration_state_from_serialized_entries(entries, &css_text, inline_base_url);
    if let Some(previous) = unsafe { &*runtime_ptr }.element_inline_style_declaration_state(handle)
    {
        state
            .block
            .retain_unaffected_typed_numeric_values(&previous.block, property);
    }
    if let Some(numeric) = numeric {
        state.block.retain_typed_numeric_value(property, numeric);
    }
    if style_string(unsafe { &*runtime_ptr }, handle) == css_text {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime.set_element_inline_style_resolution_text(handle, resolution_text);
        runtime.set_element_inline_style_declaration_state(handle, state);
        return false;
    }
    set_reflected_style_attribute_with_inline_base_url(
        scope,
        runtime_ptr,
        handle,
        &css_text,
        inline_base_url,
        state,
        resolution_text,
    );
    true
}
