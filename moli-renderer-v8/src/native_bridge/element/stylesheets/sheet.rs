use crate::context_bootstrap::{
    bind_css_style_sheet_to_live_stylesheet, css_style_sheet_id,
    initialize_css_style_sheet_rules_from_text, new_css_style_sheet_object,
    set_css_style_sheet_href, set_css_style_sheet_origin_clean, set_css_style_sheet_owner_node,
    sync_css_style_sheet_media_list_from_owner,
};
use crate::style_engine::link_rel_qualifies_as_stylesheet;
use crate::util::{
    context_host_ptr_from_global_bridge, get_private_value, set_private_value, throw_type_error,
};
use moli_web_mime::is_css_stylesheet_type_hint;

use super::super::super::node::node_runtime_and_handle_from_object;

pub(super) const STYLE_SHEET_CACHE_SLOT: &str = "__moliStyleSheet";

fn sync_sheet_from_linked_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    runtime: &crate::native_bridge::JsContextHost,
    source: &crate::style_engine::StyloStylesheetSource,
) -> bool {
    let Some(stylesheet) = source
        .live_stylesheet_id()
        .and_then(|id| runtime.live_stylesheet(id))
    else {
        return false;
    };
    bind_css_style_sheet_to_live_stylesheet(scope, sheet, stylesheet);
    set_css_style_sheet_href(scope, sheet, source.sheet_url());
    set_css_style_sheet_origin_clean(scope, sheet, source.origin_clean());
    true
}

fn sync_csp_blocked_link_sheet_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) {
    if let Some(disposition) = crate::stylesheet_blocking::stylesheet_link_disposition(
        runtime.dom_host(),
        moli_dom::NodeId::new(handle.index()),
    ) {
        set_css_style_sheet_href(scope, sheet, disposition.url());
    }
    set_css_style_sheet_origin_clean(scope, sheet, true);
}

fn linked_stylesheet_source(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<crate::style_engine::StyloStylesheetSource> {
    runtime.linked_stylesheet_source_for_owner(handle)
}

fn sync_linked_sheet_media_from_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) {
    let media = runtime
        .dom_host()
        .get_attribute(handle, "media")
        .unwrap_or_default();
    sync_css_style_sheet_media_list_from_owner(scope, sheet, &media);
}

pub(crate) fn style_sheet_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !style_sheet_receiver_is_supported(scope, args.this()) {
        throw_type_error(
            scope,
            "Failed to get 'sheet' on 'LinkStyle': Illegal invocation.",
        );
        return;
    }
    match style_sheet_for_element(scope, args.this()) {
        Some(sheet) => rv.set(sheet.into()),
        None => rv.set_null(),
    }
}

fn style_sheet_receiver_is_supported<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    if node_runtime_and_handle_from_object(scope, receiver).is_ok() {
        return true;
    }
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    crate::native_bridge::document::detached_native_handle_for_runtime(scope, runtime_ptr, receiver)
        .is_some()
}

pub(crate) fn style_sheet_for_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let detached_runtime_ptr = context_host_ptr_from_global_bridge(scope);
    let detached_owner = detached_runtime_ptr.and_then(|runtime_ptr| {
        crate::native_bridge::document::detached_native_handle_for_runtime(
            scope,
            runtime_ptr,
            receiver,
        )
        .map(|handle| (runtime_ptr, handle))
    });
    let (runtime_ptr, handle, detached_owner) = if let Some((runtime_ptr, handle)) = detached_owner
    {
        (runtime_ptr, handle, true)
    } else {
        let (runtime_ptr, handle) = node_runtime_and_handle_from_object(scope, receiver).ok()?;
        (runtime_ptr, handle, false)
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let node = runtime.dom_host().node(handle)?;
    if let Some(processing_instruction) = node.as_processing_instruction() {
        let target = processing_instruction.target().to_owned();
        let data = processing_instruction.data().to_owned();
        return style_sheet_for_processing_instruction(
            scope, receiver, runtime, handle, &target, &data,
        );
    }
    let element = node.as_element()?;
    let is_style = element.is_inline_style_element();
    let is_css_link = element.is_html_element("link")
        && link_rel_qualifies_as_stylesheet(element.attribute("rel"), element.attribute("title"));
    if (!is_style && !is_css_link)
        || !crate::style_engine::stylesheet_owner_type_is_supported(element)
        || (is_css_link
            && runtime
                .dom_host()
                .get_attribute(handle, "disabled")
                .is_some())
    {
        return None;
    }
    let owner_is_connected = if detached_owner {
        crate::native_bridge::document::detached_node_is_connected(scope, receiver)
    } else {
        runtime.dom_host().is_connected(handle)
    };
    if !owner_is_connected {
        return None;
    }
    if is_style && runtime.owner_style_sheet_source(handle).is_none() {
        return None;
    }
    let linked_source = is_css_link
        .then(|| linked_stylesheet_source(runtime, handle))
        .flatten();
    let link_csp_blocked = is_css_link && runtime.stylesheet_owner_is_csp_blocked(handle);
    if is_css_link && linked_source.is_none() && !link_csp_blocked {
        return None;
    }
    let owner_live_stylesheet = is_style
        .then(|| runtime.owner_live_stylesheet(handle))
        .flatten();

    if let Some(existing) = get_private_value(scope, receiver, STYLE_SHEET_CACHE_SLOT)
        && let Ok(existing) = v8::Local::<v8::Object>::try_from(existing)
    {
        if is_style {
            if let Some(stylesheet) = owner_live_stylesheet.as_ref() {
                if css_style_sheet_id(scope, existing) == Some(stylesheet.id()) {
                    set_css_style_sheet_owner_node(scope, existing, receiver);
                    set_css_style_sheet_origin_clean(scope, existing, true);
                    return Some(existing);
                }
                crate::context_bootstrap::clear_css_style_sheet_owner_node(scope, existing);
                let undefined = v8::undefined(scope);
                set_private_value(scope, receiver, STYLE_SHEET_CACHE_SLOT, undefined.into());
            } else {
                return None;
            }
        } else if let Some(source) = linked_source.as_ref() {
            if !sync_sheet_from_linked_source(scope, existing, runtime, source) {
                return None;
            }
            set_css_style_sheet_owner_node(scope, existing, receiver);
            sync_linked_sheet_media_from_owner(scope, existing, runtime, handle);
        } else if link_csp_blocked {
            sync_csp_blocked_link_sheet_metadata(scope, existing, runtime, handle);
            set_css_style_sheet_owner_node(scope, existing, receiver);
        } else {
            set_css_style_sheet_owner_node(scope, existing, receiver);
        }
        if !is_style {
            return Some(existing);
        }
    }

    let sheet = new_css_style_sheet_object(scope);
    set_css_style_sheet_owner_node(scope, sheet, receiver);
    if is_style {
        if let Some(stylesheet) = owner_live_stylesheet {
            bind_css_style_sheet_to_live_stylesheet(scope, sheet, stylesheet);
        } else {
            return None;
        }
        set_css_style_sheet_origin_clean(scope, sheet, true);
    } else if let Some(source) = linked_source.as_ref() {
        if !sync_sheet_from_linked_source(scope, sheet, runtime, source) {
            return None;
        }
        sync_linked_sheet_media_from_owner(scope, sheet, runtime, handle);
    } else if link_csp_blocked {
        initialize_css_style_sheet_rules_from_text(scope, sheet, "");
        sync_csp_blocked_link_sheet_metadata(scope, sheet, runtime, handle);
    }
    set_private_value(scope, receiver, STYLE_SHEET_CACHE_SLOT, sheet.into());
    Some(sheet)
}

pub(crate) fn detach_cached_style_sheet_for_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) {
    let Some(sheet) = get_private_value(scope, receiver, STYLE_SHEET_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    crate::context_bootstrap::clear_css_style_sheet_owner_node(scope, sheet);
    let undefined = v8::undefined(scope);
    set_private_value(scope, receiver, STYLE_SHEET_CACHE_SLOT, undefined.into());
}

pub(crate) fn detach_cached_style_sheet_if_live_stylesheet_changed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    expected_id: crate::live_stylesheet::StylesheetId,
) -> bool {
    let Some(sheet) = get_private_value(scope, receiver, STYLE_SHEET_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    if css_style_sheet_id(scope, sheet) == Some(expected_id) {
        return false;
    }
    detach_cached_style_sheet_for_element(scope, receiver);
    true
}

pub(crate) fn sync_cached_style_sheet_media_from_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    media_text: &str,
) {
    let Some(sheet) = get_private_value(scope, receiver, STYLE_SHEET_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    sync_css_style_sheet_media_list_from_owner(scope, sheet, media_text);
}

fn style_sheet_for_processing_instruction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    runtime: &mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    target: &str,
    data: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    if !target.eq_ignore_ascii_case("xml-stylesheet") || !runtime.dom_host().is_connected(handle) {
        return None;
    }
    let type_attr = processing_instruction_pseudo_attribute(data, "type");
    if !is_css_stylesheet_type_hint(type_attr.as_deref()) {
        return None;
    }
    processing_instruction_pseudo_attribute(data, "href")?;

    if let Some(existing) = get_private_value(scope, receiver, STYLE_SHEET_CACHE_SLOT)
        && let Ok(existing) = v8::Local::<v8::Object>::try_from(existing)
    {
        let source = linked_stylesheet_source(runtime, handle)?;
        sync_sheet_from_linked_source(scope, existing, runtime, &source).then_some(())?;
        set_css_style_sheet_owner_node(scope, existing, receiver);
        return Some(existing);
    }

    let sheet = new_css_style_sheet_object(scope);
    set_css_style_sheet_owner_node(scope, sheet, receiver);
    let source = linked_stylesheet_source(runtime, handle)?;
    sync_sheet_from_linked_source(scope, sheet, runtime, &source).then_some(())?;
    set_private_value(scope, receiver, STYLE_SHEET_CACHE_SLOT, sheet.into());
    Some(sheet)
}

fn processing_instruction_pseudo_attribute(data: &str, name: &str) -> Option<String> {
    let mut input = data.trim_start();
    while !input.is_empty() {
        let name_end = input
            .find(|ch: char| ch.is_ascii_whitespace() || ch == '=')
            .unwrap_or(input.len());
        let attribute_name = &input[..name_end];
        input = input[name_end..].trim_start();
        if !input.starts_with('=') {
            continue;
        }
        input = input[1..].trim_start();
        let (value, rest) = match input.chars().next()? {
            quote @ ('"' | '\'') => {
                let value_start = quote.len_utf8();
                let value_end = input[value_start..].find(quote)? + value_start;
                (
                    &input[value_start..value_end],
                    &input[value_end + quote.len_utf8()..],
                )
            }
            _ => {
                let value_end = input.find(char::is_whitespace).unwrap_or(input.len());
                (&input[..value_end], &input[value_end..])
            }
        };
        if attribute_name == name {
            return Some(value.to_owned());
        }
        input = rest.trim_start();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::processing_instruction_pseudo_attribute;

    #[test]
    fn processing_instruction_attribute_parser_preserves_attributes_after_flags() {
        assert_eq!(
            processing_instruction_pseudo_attribute(
                r#"alternate href="style.css" type="text/css""#,
                "href",
            )
            .as_deref(),
            Some("style.css")
        );
        assert_eq!(
            processing_instruction_pseudo_attribute(
                r#"alternate href="style.css" type="text/css""#,
                "type",
            )
            .as_deref(),
            Some("text/css")
        );
    }
}
