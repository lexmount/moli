use moli_web_mime::{is_dom_parser_xml_mime, is_html_document_mime};
use moli_webapi_declare::WebApiFunctionTemplate;
use url::Url;

use crate::{
    dom::native::{DomHost, NativeDom, NativeNodeId},
    parser::{HtmlParser, XmlParser},
    webidl,
};

use super::{
    native_bridge::document::{
        build_detached_document_object_from_dom_host,
        build_detached_document_object_from_dom_host_with_content_type,
    },
    util::{context_host_ptr_from_global_bridge, get_private_object, throw_type_error},
};

pub(crate) const DOM_PARSER_FOREIGN_NODE_SLOT: &str = "__moliDomParserForeignNode";
const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const PARSER_ERROR_STYLE: &str = "display: block; white-space: pre; border: 2px solid #c77; padding: 0 1em 0 1em; margin: 1em; background-color: #fdd; color: black";
const PARSER_ERROR_DETAIL_STYLE: &str = "font-family:monospace;font-size:12px";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DetachedDocumentKind {
    Html,
    Xml,
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "SupportedType")]
enum DomParserSupportedType {
    #[webidl(token = "text/html")]
    Html,
    #[webidl(token = "text/xml")]
    TextXml,
    #[webidl(token = "application/xml")]
    ApplicationXml,
    #[webidl(token = "application/xhtml+xml")]
    ApplicationXhtmlXml,
    #[webidl(token = "image/svg+xml")]
    ImageSvgXml,
}

impl DomParserSupportedType {
    fn as_mime(self) -> &'static str {
        match self {
            Self::Html => "text/html",
            Self::TextXml => "text/xml",
            Self::ApplicationXml => "application/xml",
            Self::ApplicationXhtmlXml => "application/xhtml+xml",
            Self::ImageSvgXml => "image/svg+xml",
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMParser.parseFromString")]
struct DomParserParseFromStringArgs {
    #[webidl(required, with = dom_parser_source_arg)]
    source: DomParserSource,
    #[webidl(required, converter = "enum")]
    mime: DomParserSupportedType,
}

enum DomParserSource {
    TrustedHtml(String),
    String(String),
}

fn dom_parser_source_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<DomParserSource, webidl::WebIdlError> {
    if args.length() <= index {
        return Err(webidl::WebIdlError::custom_message(
            "Failed to execute 'parseFromString' on 'DOMParser': 2 arguments required, but only 0 present.",
        ));
    }
    let value = args.get(index);
    if let Some(value) = crate::context_bootstrap::trusted_html_value_string(scope, value) {
        return Ok(DomParserSource::TrustedHtml(value));
    }
    webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::argument("DOMParser.parseFromString", (index + 1) as usize),
    )
    .map(|value| DomParserSource::String(value.0))
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMParser", enumerable)]
struct DomParserPrototypeMethodsDeclaration {
    #[webapi(method, length = 2, callback = dom_parser_parse_from_string_callback)]
    parse_from_string: (),
}

pub(super) fn dom_parser_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "DOMParser constructor must be called with new");
        return;
    }
    rv.set(args.this().into());
}

pub(super) fn dom_parser_parse_from_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DomParserParseFromStringArgs>(scope, &args) else {
        return;
    };
    let source = match parsed.source {
        DomParserSource::TrustedHtml(source) => source,
        DomParserSource::String(source) => {
            let requirements = context_host_ptr_from_global_bridge(scope)
                .map(|host_ptr| unsafe { &*host_ptr }.trusted_types_for_script_requirements(scope))
                .unwrap_or_default();
            let Some(value) = crate::util::v8_string(scope, &source) else {
                return;
            };
            let Some(source) = crate::context_bootstrap::trusted_html_string_or_throw(
                scope,
                value.into(),
                requirements,
                "DOMParser parseFromString",
                "parseFromString",
            ) else {
                return;
            };
            source
        }
    };
    let Some(obj) = parse_detached_document_from_string(scope, &source, parsed.mime.as_mime())
    else {
        rv.set(v8::null(scope).into());
        return;
    };
    rv.set(obj.into());
}

pub(crate) fn install_dom_parser_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "DOMParser" {
        return;
    }
    DomParserPrototypeMethodsDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub(super) fn parse_detached_document_from_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: &str,
    mime: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let is_html = is_html_document_mime(mime);
    let is_xml = is_dom_parser_xml_mime(mime);
    if !is_html && !is_xml {
        return None;
    }

    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let runtime = unsafe { &*host_ptr };
    if is_html {
        return parse_detached_html_document_from_source(
            scope,
            runtime.document_url().clone(),
            source,
        );
    }

    let parser = XmlParser;
    let parsed = parser.parse(runtime.document_url().clone(), source.to_owned());
    let parsed = if parsed.parse_errors().is_empty() && native_document_has_element_child(&parsed) {
        parsed
    } else {
        materialize_xml_parser_error_document(parsed)
    };
    build_detached_document(scope, parsed, DetachedDocumentKind::Xml, false)
}

fn materialize_xml_parser_error_document(parsed: NativeDom) -> NativeDom {
    let error_detail = parsed
        .parse_errors()
        .first()
        .cloned()
        .unwrap_or_else(|| "XML document has no document element".to_owned());
    let mut host = DomHost::from_dom(parsed);
    let document = host.document_handle();
    let document_element = host
        .child_handles(document)
        .find(|handle| host.node(*handle).is_some_and(|node| node.is_element()));

    let parser_error = create_dom_parser_error_element(&mut host, document, &error_detail);
    if let Some(document_element) = document_element {
        let first_child = host
            .node(document_element)
            .and_then(|node| node.first_child());
        let _ = host.insert_before(document_element, parser_error, first_child);
        return host.snapshot_document();
    }

    for child in host.child_handles(document).collect::<Vec<_>>() {
        let _ = host.remove_child(document, child);
    }
    let html = host.create_parser_element_without_attributes_for_document(
        document,
        "html".to_owned(),
        HTML_NAMESPACE.to_owned(),
        None,
    );
    let body = host.create_parser_element_without_attributes_for_document(
        document,
        "body".to_owned(),
        HTML_NAMESPACE.to_owned(),
        None,
    );
    let _ = host.append_child(document, html);
    let _ = host.append_child(html, body);
    let _ = host.append_child(body, parser_error);
    host.snapshot_document()
}

fn create_dom_parser_error_element(
    host: &mut DomHost,
    document: NativeNodeId,
    error_detail: &str,
) -> NativeNodeId {
    let parser_error = host.create_parser_element_without_attributes_for_document(
        document,
        "parsererror".to_owned(),
        HTML_NAMESPACE.to_owned(),
        None,
    );
    let _ = host.set_attribute(parser_error, "style", PARSER_ERROR_STYLE);

    let heading = create_dom_parser_error_child(host, document, "h3", None);
    append_dom_parser_error_text(
        host,
        document,
        heading,
        "This page contains the following errors:",
    );
    let detail =
        create_dom_parser_error_child(host, document, "div", Some(PARSER_ERROR_DETAIL_STYLE));
    append_dom_parser_error_text(host, document, detail, error_detail);
    let footer = create_dom_parser_error_child(host, document, "h3", None);
    append_dom_parser_error_text(
        host,
        document,
        footer,
        "Below is a rendering of the page up to the first error.",
    );
    let _ = host.append_child(parser_error, heading);
    let _ = host.append_child(parser_error, detail);
    let _ = host.append_child(parser_error, footer);
    parser_error
}

fn create_dom_parser_error_child(
    host: &mut DomHost,
    document: NativeNodeId,
    local_name: &str,
    style: Option<&str>,
) -> NativeNodeId {
    let element = host.create_parser_element_without_attributes_for_document(
        document,
        local_name.to_owned(),
        HTML_NAMESPACE.to_owned(),
        None,
    );
    if let Some(style) = style {
        let _ = host.set_attribute(element, "style", style);
    }
    element
}

fn append_dom_parser_error_text(
    host: &mut DomHost,
    document: NativeNodeId,
    parent: NativeNodeId,
    text: &str,
) {
    let text = host.create_text_node_for_document(document, text);
    let _ = host.append_child(parent, text);
}

/// Builds a detached HTML document wrapper from raw markup and an explicit document URL.
///
/// This helper exists so non-DOMParser callers can materialize a queryable `Document`
/// snapshot without inventing a fake live browsing context. The returned object behaves
/// like other detached DOMParser documents:
/// - it is queryable (`getElementById`, `querySelector`, `body`, ...)
/// - it is *not* a live page VM
/// - scripts inside the markup do not execute
///
/// That boundary is intentional. Live child-frame surfaces such as `iframe.contentDocument`
/// must wrap the frame's current native document instead of materializing this detached
/// snapshot helper.
pub(crate) fn parse_detached_html_document_from_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_url: Url,
    source: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let parsed = HtmlParser::with_scripting_enabled(false)
        .parse_without_declarative_shadow_roots(document_url, source.to_owned());
    build_detached_document(scope, parsed, DetachedDocumentKind::Html, false)
}

pub(crate) fn parse_detached_html_document_from_source_with_declarative_shadow_roots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_url: Url,
    source: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let parsed =
        HtmlParser::with_scripting_enabled(false).parse_dom_host(document_url, source.to_owned());
    build_detached_document_from_dom_host(scope, parsed, DetachedDocumentKind::Html, true)
}

/// Builds the DOM projection used for a non-top browsing-context document.
///
/// The projection never executes scripts itself. `html_parser` carries the
/// target Document's scripting state solely for parser behavior such as
/// `<noscript>` tokenization; every caller must select that state explicitly.
pub(crate) fn parse_browsing_context_document_projection_from_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_url: Url,
    source: &str,
    content_type: Option<&str>,
    character_set: Option<&str>,
    html_parser: HtmlParser,
) -> Option<v8::Local<'s, v8::Object>> {
    let source = preserve_decoded_bom_only_browsing_context_body(source, content_type);
    let (parsed, kind) =
        parse_browsing_context_document_snapshot(document_url, &source, content_type, html_parser);
    build_detached_document_from_dom_host_with_content_type(
        scope,
        parsed,
        kind,
        true,
        content_type,
        character_set,
    )
}

pub(crate) fn preserve_decoded_bom_only_browsing_context_body<'a>(
    source: &'a str,
    content_type: Option<&str>,
) -> std::borrow::Cow<'a, str> {
    if source == "\u{feff}" && !content_type.is_some_and(is_dom_parser_xml_mime) {
        std::borrow::Cow::Borrowed("<body>\u{feff}</body>")
    } else {
        std::borrow::Cow::Borrowed(source)
    }
}

fn parse_browsing_context_document_snapshot(
    document_url: Url,
    source: &str,
    content_type: Option<&str>,
    html_parser: HtmlParser,
) -> (DomHost, DetachedDocumentKind) {
    if content_type.is_some_and(is_dom_parser_xml_mime)
        || child_document_url_is_xml_like(&document_url)
    {
        let parser = XmlParser;
        return (
            DomHost::from_dom(parser.parse(document_url, source.to_owned())),
            DetachedDocumentKind::Xml,
        );
    }
    (
        html_parser.parse_dom_host(document_url, source.to_owned()),
        DetachedDocumentKind::Html,
    )
}

fn child_document_url_is_xml_like(url: &Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    path.ends_with(".xml") || path.ends_with(".xhtml") || path.ends_with(".svg")
}

fn build_detached_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: NativeDom,
    kind: DetachedDocumentKind,
    expose_declarative_shadow_roots: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    build_detached_document_from_dom_host(
        scope,
        DomHost::from_dom(parsed),
        kind,
        expose_declarative_shadow_roots,
    )
}

fn build_detached_document_from_dom_host<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: DomHost,
    kind: DetachedDocumentKind,
    expose_declarative_shadow_roots: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let _ = expose_declarative_shadow_roots;
    let kind = match kind {
        DetachedDocumentKind::Html => "html",
        DetachedDocumentKind::Xml => "xml",
    };
    build_detached_document_object_from_dom_host(scope, kind, parsed)
}

fn build_detached_document_from_dom_host_with_content_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: DomHost,
    kind: DetachedDocumentKind,
    expose_declarative_shadow_roots: bool,
    content_type: Option<&str>,
    character_set: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let _ = expose_declarative_shadow_roots;
    let kind = match kind {
        DetachedDocumentKind::Html => "html",
        DetachedDocumentKind::Xml => "xml",
    };
    build_detached_document_object_from_dom_host_with_content_type(
        scope,
        kind,
        parsed,
        content_type,
        character_set,
    )
}

fn dom_parser_foreign_wrapper_for_live_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, object, DOM_PARSER_FOREIGN_NODE_SLOT)
}

pub(crate) fn map_live_value_to_foreign<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return value;
    };
    dom_parser_foreign_wrapper_for_live_object(scope, object)
        .map(Into::into)
        .unwrap_or(value)
}

fn native_document_has_element_child(dom: &NativeDom) -> bool {
    dom.child_ids(dom.document_node_id()).any(|handle| {
        dom.node(handle)
            .and_then(|node| node.as_element())
            .is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::native::NativeNodeId;

    fn first_document_element(dom: &NativeDom) -> NativeNodeId {
        dom.find_child(dom.document_node_id(), |handle| {
            dom.node(handle)
                .and_then(|node| node.as_element())
                .is_some()
        })
        .expect("document element")
    }

    #[test]
    fn child_document_snapshot_uses_xml_parser_for_xml_like_urls() {
        let (xml, xml_kind) = parse_browsing_context_document_snapshot(
            Url::parse("https://example.test/common/dummy.xml").unwrap(),
            "<foo>Dummy XML document</foo>\n",
            None,
            HtmlParser::SCRIPTING_ENABLED,
        );
        let xml_root = first_document_element(&xml);
        assert_eq!(xml_kind, DetachedDocumentKind::Xml);
        assert_eq!(
            xml.text_content(xml_root).as_deref(),
            Some("Dummy XML document")
        );

        let (xhtml, xhtml_kind) = parse_browsing_context_document_snapshot(
            Url::parse("https://example.test/common/dummy.xhtml").unwrap(),
            r#"<!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml"><head><title>Dummy XHTML document</title></head><body /></html>
"#,
            None,
            HtmlParser::SCRIPTING_ENABLED,
        );
        let xhtml_root = first_document_element(&xhtml);
        assert_eq!(xhtml_kind, DetachedDocumentKind::Xml);
        assert_eq!(
            xhtml.text_content(xhtml_root).as_deref(),
            Some("Dummy XHTML document")
        );

        let (html, html_kind) = parse_browsing_context_document_snapshot(
            Url::parse("https://example.test/common/dummy.html").unwrap(),
            "<p>Dummy HTML document</p>\n",
            None,
            HtmlParser::SCRIPTING_ENABLED,
        );
        let html_root = html.document_element_handle().expect("html root");
        assert_eq!(html_kind, DetachedDocumentKind::Html);
        assert_eq!(
            html.text_content(html_root).as_deref(),
            Some("Dummy HTML document\n")
        );
    }

    #[test]
    fn browsing_context_document_projection_uses_explicit_scripting_mode() {
        let parse = |parser| {
            parse_browsing_context_document_snapshot(
                Url::parse("https://example.test/child.html").expect("test URL"),
                "<noscript><span id='fallback'></span></noscript>",
                Some("text/html"),
                parser,
            )
            .0
        };

        let enabled = parse(HtmlParser::SCRIPTING_ENABLED);
        assert!(enabled.element_handle_by_id("fallback").is_none());

        let disabled = parse(HtmlParser::SCRIPTING_DISABLED);
        assert!(disabled.element_handle_by_id("fallback").is_some());
    }
}
