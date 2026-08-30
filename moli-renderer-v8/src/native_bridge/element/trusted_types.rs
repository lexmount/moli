use super::JsContextHost;
use crate::document_runtime::DomHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native_bridge) enum TrustedAttributeSetter {
    SetAttribute,
    SetAttributeNs,
    SetAttributeNode,
    AttrValue,
    SvgAnimatedStringBaseVal,
}

impl TrustedAttributeSetter {
    fn api_name(self) -> &'static str {
        match self {
            Self::SetAttribute => "setAttribute",
            Self::SetAttributeNs => "setAttributeNS",
            Self::SetAttributeNode => "setAttributeNode",
            Self::AttrValue => "value",
            Self::SvgAnimatedStringBaseVal => "baseVal",
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
            Self::SvgAnimatedStringBaseVal => {
                crate::webidl::Context::member("SVGAnimatedString", "baseVal")
            }
        }
    }
}

enum TrustedAttributeSink {
    Html(&'static str),
    Script(String),
    ScriptUrl(&'static str),
}

impl TrustedAttributeSink {
    fn type_name(&self) -> &'static str {
        match self {
            Self::Html(_) => "TrustedHTML",
            Self::Script(_) => "TrustedScript",
            Self::ScriptUrl(_) => "TrustedScriptURL",
        }
    }
}

fn trusted_attribute_sink_for_names(
    element_namespace: &str,
    element_local_name: &str,
    attribute_namespace: Option<&str>,
    attribute_local_name: &str,
) -> Option<TrustedAttributeSink> {
    if attribute_namespace.is_none()
        && matches!(
            element_namespace,
            "http://www.w3.org/1999/xhtml"
                | "http://www.w3.org/2000/svg"
                | "http://www.w3.org/1998/Math/MathML"
        )
        && super::event_handlers::is_element_event_handler_content_attribute_name(
            attribute_local_name,
        )
    {
        return Some(TrustedAttributeSink::Script(format!(
            "Element {attribute_local_name}"
        )));
    }

    match (
        element_namespace,
        element_local_name,
        attribute_namespace,
        attribute_local_name,
    ) {
        ("http://www.w3.org/1999/xhtml", "iframe", None, "srcdoc") => {
            Some(TrustedAttributeSink::Html("HTMLIFrameElement srcdoc"))
        }
        ("http://www.w3.org/1999/xhtml", "script", None, "src") => {
            Some(TrustedAttributeSink::ScriptUrl("HTMLScriptElement src"))
        }
        (
            "http://www.w3.org/2000/svg",
            "script",
            None | Some("http://www.w3.org/1999/xlink"),
            "href",
        ) => Some(TrustedAttributeSink::ScriptUrl("SVGScriptElement href")),
        _ => None,
    }
}

pub(crate) fn trusted_attribute_type_name_for_names(
    element_namespace: &str,
    element_local_name: &str,
    attribute_namespace: Option<&str>,
    attribute_local_name: &str,
) -> Option<&'static str> {
    trusted_attribute_sink_for_names(
        element_namespace,
        element_local_name,
        attribute_namespace,
        attribute_local_name,
    )
    .map(|sink| sink.type_name())
}

pub(crate) fn trusted_property_type_name_for_names(
    element_namespace: &str,
    element_local_name: &str,
    property_name: &str,
) -> Option<&'static str> {
    if matches!(property_name, "innerHTML" | "outerHTML") {
        return Some("TrustedHTML");
    }
    match (element_namespace, element_local_name, property_name) {
        ("http://www.w3.org/1999/xhtml", "iframe", "srcdoc") => Some("TrustedHTML"),
        ("http://www.w3.org/1999/xhtml", "script", "innerText" | "text" | "textContent") => {
            Some("TrustedScript")
        }
        ("http://www.w3.org/1999/xhtml", "script", "src") => Some("TrustedScriptURL"),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native_bridge::element) enum TrustedHtmlSink {
    ElementInnerHtml,
    ShadowRootInnerHtml,
    ElementOuterHtml,
    IframeSrcdoc,
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
            Self::IframeSrcdoc => "HTMLIFrameElement srcdoc",
            Self::ElementSetHtmlUnsafe => "Element setHTMLUnsafe",
            Self::ShadowRootSetHtmlUnsafe => "ShadowRoot setHTMLUnsafe",
            Self::ElementInsertAdjacentHtml => "Element insertAdjacentHTML",
        }
    }

    fn api_name(self) -> &'static str {
        match self {
            Self::ElementInnerHtml | Self::ShadowRootInnerHtml => "innerHTML",
            Self::ElementOuterHtml => "outerHTML",
            Self::IframeSrcdoc => "srcdoc",
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
        let sink = trusted_attribute_sink_for_names(
            element.namespace(),
            element.local_name(),
            attribute_namespace,
            local_name,
        )?;
        Some((runtime_ptr, sink))
    });

    if let Some((runtime_ptr, sink)) = sink {
        let requirements = unsafe { &*runtime_ptr }.trusted_types_for_script_requirements(scope);
        return match sink {
            TrustedAttributeSink::Html(sink) => {
                crate::context_bootstrap::trusted_html_string_or_throw(
                    scope,
                    value,
                    requirements,
                    sink,
                    setter.api_name(),
                )
            }
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
    let namespace =
        svg_animated_string_attribute_namespace(unsafe { &*runtime_ptr }, handle, attribute);
    let value = trusted_attribute_value_string(
        scope,
        Some((runtime_ptr, handle)),
        namespace,
        attribute,
        value,
        TrustedAttributeSetter::SvgAnimatedStringBaseVal,
    )?;
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

pub(crate) fn prepare_trusted_script_text(
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
    let source = crate::context_bootstrap::trusted_script_string_for_script_element_execution(
        scope, source, sink,
    )?;
    // The Trusted Types integration updates the script-text slot before the
    // HTML prepare algorithm reads the source and determines the script type.
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_script_text_internal_slot(handle, &source);
    Some(source)
}
