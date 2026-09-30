use super::JsContextHost;
use crate::document_runtime::DomHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native_bridge::element) enum TrustedHtmlSink {
    ElementInnerHtml,
    ShadowRootInnerHtml,
    ElementOuterHtml,
    ElementSetHtmlUnsafe,
    ShadowRootSetHtmlUnsafe,
    ElementInsertAdjacentHtml,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native_bridge::element) enum TrustedScriptElementSink {
    InnerText,
    TextContent,
    Text,
}

impl TrustedScriptElementSink {
    fn name(self) -> &'static str {
        match self {
            Self::InnerText => "HTMLScriptElement innerText",
            Self::TextContent => "HTMLScriptElement textContent",
            Self::Text => "HTMLScriptElement text",
        }
    }

    pub(super) fn api_name(self) -> &'static str {
        match self {
            Self::InnerText => "innerText",
            Self::TextContent => "textContent",
            Self::Text => "text",
        }
    }

    fn null_is_empty(self) -> bool {
        !matches!(self, Self::Text)
    }
}

impl TrustedHtmlSink {
    fn name(self) -> &'static str {
        match self {
            Self::ElementInnerHtml => "Element innerHTML",
            Self::ShadowRootInnerHtml => "ShadowRoot innerHTML",
            Self::ElementOuterHtml => "Element outerHTML",
            Self::ElementSetHtmlUnsafe => "Element setHTMLUnsafe",
            Self::ShadowRootSetHtmlUnsafe => "ShadowRoot setHTMLUnsafe",
            Self::ElementInsertAdjacentHtml => "Element insertAdjacentHTML",
        }
    }

    fn api_name(self) -> &'static str {
        match self {
            Self::ElementInnerHtml | Self::ShadowRootInnerHtml => "innerHTML",
            Self::ElementOuterHtml => "outerHTML",
            Self::ElementSetHtmlUnsafe | Self::ShadowRootSetHtmlUnsafe => "setHTMLUnsafe",
            Self::ElementInsertAdjacentHtml => "insertAdjacentHTML",
        }
    }

    fn uses_legacy_null_to_empty_string(self) -> bool {
        matches!(
            self,
            Self::ElementInnerHtml | Self::ShadowRootInnerHtml | Self::ElementOuterHtml
        )
    }
}

pub(in crate::native_bridge::element) fn trusted_html_sink_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'s, v8::Value>,
    sink: TrustedHtmlSink,
) -> Option<String> {
    let value = if sink.uses_legacy_null_to_empty_string() && value.is_null() {
        v8::String::empty(scope).into()
    } else {
        value
    };
    let requirements = unsafe { &*runtime_ptr }.trusted_types_for_script_requirements(scope);
    crate::context_bootstrap::trusted_html_string_or_throw(
        scope,
        value,
        requirements,
        sink.name(),
        sink.api_name(),
    )
}

pub(in crate::native_bridge::element) fn trusted_script_element_sink_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'s, v8::Value>,
    sink: TrustedScriptElementSink,
) -> Option<String> {
    let value = if sink.null_is_empty() && value.is_null_or_undefined() {
        v8::String::empty(scope).into()
    } else {
        value
    };
    let requirements = unsafe { &*runtime_ptr }.trusted_types_for_script_requirements(scope);
    crate::context_bootstrap::trusted_script_string_or_type_error(
        scope,
        value,
        requirements,
        sink.name(),
        sink.api_name(),
    )
}

pub(in crate::native_bridge::element) fn trusted_script_url_sink_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    let requirements = unsafe { &*runtime_ptr }.trusted_types_for_script_requirements(scope);
    crate::context_bootstrap::trusted_script_url_string_or_throw(
        scope,
        value,
        requirements,
        "HTMLScriptElement src",
        "src",
    )
}

pub(crate) fn trusted_script_source_for_execution(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    source: &str,
) -> Option<String> {
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.requires_trusted_types_for_script(scope) {
        return Some(source.to_owned());
    }
    let (trusted_source, sink) = runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .filter(|element| element.is_script_element())
        .map(|element| {
            let sink = if element.namespace() == "http://www.w3.org/2000/svg" {
                "SVGScriptElement text"
            } else {
                "HTMLScriptElement text"
            };
            (element.script_text_internal_slot().to_owned(), sink)
        })?;
    if source == trusted_source {
        return Some(source.to_owned());
    }
    crate::context_bootstrap::trusted_script_string_for_script_element_execution(
        scope, source, sink,
    )
}

fn svg_animated_string_attribute_namespace(
    runtime: &JsContextHost,
    handle: DomHandle,
    attribute: &str,
) -> Option<&'static str> {
    (attribute == "href"
        && !runtime.dom_host().has_attribute_ns(handle, None, attribute)
        && runtime.dom_host().has_attribute_ns(
            handle,
            Some(crate::native_bridge::document::XLINK_NS),
            attribute,
        ))
    .then_some(crate::native_bridge::document::XLINK_NS)
}

pub(crate) fn set_svg_animated_string_base_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    let (runtime_ptr, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, owner).ok()?;
    let script_href = {
        let element = unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)?
            .as_element()?;
        element.namespace() == "http://www.w3.org/2000/svg"
            && element.local_name() == "script"
            && attribute == "href"
    };
    let value = if script_href {
        let requirements = unsafe { &*runtime_ptr }.trusted_types_for_script_requirements(scope);
        crate::context_bootstrap::trusted_script_url_string_or_throw(
            scope,
            value,
            requirements,
            "SVGScriptElement href",
            "baseVal",
        )?
    } else {
        match crate::webidl::convert::<crate::webidl::DomString>(
            scope,
            value,
            crate::webidl::Context::member("SVGAnimatedString", "baseVal"),
        ) {
            Ok(value) => value.0,
            Err(error) => {
                crate::webidl::throw_error(scope, &error);
                return None;
            }
        }
    };
    let (runtime_ptr, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, owner).ok()?;
    let namespace =
        svg_animated_string_attribute_namespace(unsafe { &*runtime_ptr }, handle, attribute);
    if attribute == "href" {
        let _ = unsafe { &mut *runtime_ptr }.set_attribute_ns(
            scope,
            runtime_ptr,
            handle,
            namespace,
            namespace.map(|_| "xlink"),
            attribute,
            if namespace.is_some() {
                "xlink:href"
            } else {
                attribute
            },
            &value,
        );
    } else {
        let _ = unsafe { &mut *runtime_ptr }.set_attribute(
            scope,
            runtime_ptr,
            handle,
            attribute,
            &value,
        );
    }
    Some(value)
}
