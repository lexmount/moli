use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use moli_encoding::{encode_url_query_for_legacy_web, form_output_encoding_for_label};
use url::Url;

use super::super::super::JsContextHost;
use super::super::reflected_attribute;
use super::super::set_reflected_attribute;

pub(in crate::native_bridge::element) fn resolve_url_like_attribute(
    runtime: &JsContextHost,
    handle: DomHandle,
    name: &str,
) -> String {
    if name == "href"
        && runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| element.is_html_element("base"))
    {
        let base = url_base_for_handle(runtime, handle);
        let Some(value) = reflected_attribute(runtime, handle, name) else {
            return base.to_string();
        };
        return parse_url_with_document_query_encoding(runtime, handle, &base, &value)
            .map(|url| url.to_string())
            .unwrap_or(value);
    }

    let Some(value) = reflected_attribute(runtime, handle, name) else {
        return String::new();
    };
    let base = url_base_for_handle(runtime, handle);
    parse_url_with_document_query_encoding(runtime, handle, &base, &value)
        .map(|url| url.to_string())
        .unwrap_or(value)
}

pub(in crate::native_bridge::element) fn parsed_url_like_attribute(
    runtime: &JsContextHost,
    handle: DomHandle,
    name: &str,
) -> Option<Url> {
    let value = reflected_attribute(runtime, handle, name).or_else(|| {
        if name != "href" || !is_svg_anchor(runtime, handle) {
            return None;
        }
        runtime.dom_host().get_attribute_ns(
            handle,
            Some(crate::native_bridge::document::XLINK_NS),
            name,
        )
    })?;
    let base = url_base_for_handle(runtime, handle);
    parse_url_with_document_query_encoding(runtime, handle, &base, &value).ok()
}

/// Reconstructs Chromium's URL-parser flag after `url::Url` has removed raw
/// URL whitespace and percent-encoded the `<` in its serialized result.
pub(in crate::native_bridge::element) fn should_block_dangling_markup_subresource(
    request_url: &Url,
    original_input: &str,
) -> bool {
    matches!(request_url.scheme(), "http" | "https")
        && original_input.as_bytes().contains(&b'<')
        && original_input
            .as_bytes()
            .iter()
            .any(|byte| matches!(byte, b'\r' | b'\n' | b'\t'))
}

pub(crate) fn parse_url_with_document_query_encoding(
    runtime: &JsContextHost,
    handle: DomHandle,
    base: &Url,
    value: &str,
) -> Result<Url, url::ParseError> {
    let Some(encoding) = document_query_encoding_for_handle(runtime, handle) else {
        return Url::options().base_url(Some(base)).parse(value);
    };
    let value = encode_url_query_for_legacy_web(value, encoding);
    Url::options().base_url(Some(base)).parse(value.as_ref())
}

fn document_query_encoding_for_handle(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<&'static encoding_rs::Encoding> {
    let document_handle = document_handle_for_url_context(runtime, handle)?;
    let character_set = runtime.document_character_set_for_handle(document_handle)?;
    form_output_encoding_for_label(character_set).filter(|encoding| *encoding != encoding_rs::UTF_8)
}

fn url_base_for_handle(runtime: &JsContextHost, handle: DomHandle) -> Url {
    let document_handle = document_handle_for_url_context(runtime, handle);

    document_handle
        .map(|document_handle| {
            if document_handle == runtime.dom_host().document_handle() {
                runtime
                    .dom_host()
                    .document_base_url()
                    .unwrap_or_else(|| runtime.host_document().url().clone())
            } else {
                runtime
                    .dom_host()
                    .node(document_handle)
                    .and_then(Node::as_document)
                    .map(|document| document.base_url().clone())
                    .unwrap_or_else(|| runtime.host_document().url().clone())
            }
        })
        .unwrap_or_else(|| runtime.host_document().url().clone())
}

fn document_handle_for_url_context(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    runtime.dom_host().owner_document_handle(handle)
}

pub(in crate::native_bridge::element) fn set_resolved_url_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    url: &Url,
) {
    // SVG2 HyperlinkElementUtils reads href first, but update-href writes the
    // namespaced attribute whenever xlink:href is present, even if both exist.
    let runtime = unsafe { &*runtime_ptr };
    if name == "href"
        && is_svg_anchor(runtime, handle)
        && runtime
            .dom_host()
            .get_attribute_ns(handle, Some(crate::native_bridge::document::XLINK_NS), name)
            .is_some()
    {
        crate::custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let _ = super::super::set_live_element_attribute_ns_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                handle,
                Some(crate::native_bridge::document::XLINK_NS),
                Some("xlink"),
                name,
                "xlink:href",
                url.as_ref(),
            );
        });
        return;
    }
    set_reflected_attribute(scope, runtime_ptr, handle, name, url.as_ref());
}

fn is_svg_anchor(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.is_svg_element("a"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangling_markup_subresource_gate_preserves_url_parser_flag_semantics() {
        let http = Url::parse("https://example.test/resource").unwrap();
        assert!(should_block_dangling_markup_subresource(
            &http,
            "resource?\n<"
        ));
        assert!(should_block_dangling_markup_subresource(
            &http,
            "resource?<\tkey"
        ));
        assert!(!should_block_dangling_markup_subresource(
            &http,
            "resource?<"
        ));
        assert!(!should_block_dangling_markup_subresource(
            &http,
            "resource?\r%3C"
        ));

        let data = Url::parse("data:text/plain,resource").unwrap();
        assert!(!should_block_dangling_markup_subresource(
            &data,
            "data:text/plain,resource\n<"
        ));
    }
}
