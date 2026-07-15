use super::JsContextHost;
use crate::document_runtime::DomHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native_bridge) enum TrustedAttributeSetter {
    SetAttribute,
    SetAttributeNs,
    SetAttributeNode,
    AttrValue,
}

impl TrustedAttributeSetter {
    fn api_name(self) -> &'static str {
        match self {
            Self::SetAttribute => "setAttribute",
            Self::SetAttributeNs => "setAttributeNS",
            Self::SetAttributeNode => "setAttributeNode",
            Self::AttrValue => "value",
        }
    }

    fn conversion_context(self) -> crate::webidl::Context {
        match self {
            Self::SetAttribute => crate::webidl::Context::argument("Element setAttribute", 2),
            Self::SetAttributeNs => crate::webidl::Context::argument("Element setAttributeNS", 3),
            Self::SetAttributeNode => {
                crate::webidl::Context::argument("Element setAttributeNode", 1)
            }
            Self::AttrValue => crate::webidl::Context::member("Attr", "value"),
        }
    }
}

enum TrustedAttributeSink {
    Script(String),
    ScriptUrl(&'static str),
}

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

pub(in crate::native_bridge) fn trusted_attribute_value_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_and_handle: Option<(*mut JsContextHost, DomHandle)>,
    attribute_namespace: Option<&str>,
    local_name: &str,
    value: v8::Local<'s, v8::Value>,
    setter: TrustedAttributeSetter,
) -> Option<String> {
    let sink = runtime_and_handle.and_then(|(runtime_ptr, handle)| {
        let element = unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .and_then(|node| node.as_element())?;
        let element_namespace = element.namespace();

        if attribute_namespace.is_none()
            && matches!(
                element_namespace,
                "http://www.w3.org/1999/xhtml"
                    | "http://www.w3.org/2000/svg"
                    | "http://www.w3.org/1998/Math/MathML"
            )
            && super::event_handlers::is_element_event_handler_content_attribute_name(local_name)
        {
            return Some((
                runtime_ptr,
                TrustedAttributeSink::Script(format!("Element {local_name}")),
            ));
        }

        let sink = match (
            element_namespace,
            element.local_name(),
            attribute_namespace,
            local_name,
        ) {
            ("http://www.w3.org/1999/xhtml", "script", None, "src") => {
                Some(TrustedAttributeSink::ScriptUrl("HTMLScriptElement src"))
            }
            ("http://www.w3.org/1999/xhtml", "embed", None, "src") => {
                Some(TrustedAttributeSink::ScriptUrl("HTMLEmbedElement src"))
            }
            ("http://www.w3.org/1999/xhtml", "object", None, "data") => {
                Some(TrustedAttributeSink::ScriptUrl("HTMLObjectElement data"))
            }
            ("http://www.w3.org/1999/xhtml", "object", None, "codebase") => Some(
                TrustedAttributeSink::ScriptUrl("HTMLObjectElement codebase"),
            ),
            (
                "http://www.w3.org/2000/svg",
                "script",
                None | Some("http://www.w3.org/1999/xlink"),
                "href",
            ) => Some(TrustedAttributeSink::ScriptUrl("SVGScriptElement href")),
            _ => None,
        }?;
        Some((runtime_ptr, sink))
    });

    if let Some((runtime_ptr, sink)) = sink {
        let requirements = unsafe { &*runtime_ptr }.trusted_types_for_script_requirements(scope);
        return match sink {
            TrustedAttributeSink::Script(sink) => {
                crate::context_bootstrap::trusted_script_string_or_type_error(
                    scope,
                    value,
                    requirements,
                    &sink,
                    setter.api_name(),
                )
            }
            TrustedAttributeSink::ScriptUrl(sink) => {
                crate::context_bootstrap::trusted_script_url_string_or_throw(
                    scope,
                    value,
                    requirements,
                    sink,
                    setter.api_name(),
                )
            }
        };
    }

    match crate::webidl::convert::<crate::webidl::DomString>(
        scope,
        value,
        setter.conversion_context(),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            crate::webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(in crate::native_bridge) fn trusted_attribute_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_and_handle: Option<(*mut JsContextHost, DomHandle)>,
    attribute_namespace: Option<&str>,
    local_name: &str,
    value: &str,
    setter: TrustedAttributeSetter,
) -> Option<String> {
    let value = crate::util::v8_string(scope, value)?;
    trusted_attribute_value_string(
        scope,
        runtime_and_handle,
        attribute_namespace,
        local_name,
        value.into(),
        setter,
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
