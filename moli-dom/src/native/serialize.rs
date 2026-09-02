use super::NativeDom;
use super::node::{NativeNodeId, Node, NodeData};

/// A bounded serialization stopped before appending bytes past its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HtmlSerializationLimitExceeded {
    pub max_bytes: usize,
}

impl std::fmt::Display for HtmlSerializationLimitExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "serialized HTML exceeds the {}-byte output limit",
            self.max_bytes
        )
    }
}

impl std::error::Error for HtmlSerializationLimitExceeded {}

pub(super) trait HtmlSerializationSink {
    fn push_str(&mut self, value: &str);
    fn push(&mut self, value: char);
    fn limit_exceeded(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HtmlSerializationTarget {
    IncludeNode,
    ChildrenOnly,
}

pub(super) struct HtmlSerializedShadowRoot {
    root: NativeNodeId,
    template_start: String,
}

impl HtmlSerializedShadowRoot {
    pub(super) fn new(root: NativeNodeId, template_start: String) -> Self {
        Self {
            root,
            template_start,
        }
    }
}

pub(super) trait HtmlShadowRootProvider {
    fn serialized_shadow_root_for_host(
        &self,
        host: NativeNodeId,
    ) -> Option<HtmlSerializedShadowRoot>;
}

impl<F> HtmlShadowRootProvider for F
where
    F: Fn(NativeNodeId) -> Option<HtmlSerializedShadowRoot>,
{
    fn serialized_shadow_root_for_host(
        &self,
        host: NativeNodeId,
    ) -> Option<HtmlSerializedShadowRoot> {
        self(host)
    }
}

#[derive(Clone, Copy)]
pub(super) struct HtmlSerializationOptions<'a> {
    target: HtmlSerializationTarget,
    scripting_enabled: bool,
    shadow_root_provider: Option<&'a dyn HtmlShadowRootProvider>,
}

impl<'a> HtmlSerializationOptions<'a> {
    pub(super) const fn new(target: HtmlSerializationTarget, scripting_enabled: bool) -> Self {
        Self {
            target,
            scripting_enabled,
            shadow_root_provider: None,
        }
    }

    pub(super) const fn with_shadow_root_provider(
        mut self,
        shadow_root_provider: &'a dyn HtmlShadowRootProvider,
    ) -> Self {
        self.shadow_root_provider = Some(shadow_root_provider);
        self
    }
}

impl HtmlSerializationSink for String {
    fn push_str(&mut self, value: &str) {
        String::push_str(self, value);
    }

    fn push(&mut self, value: char) {
        String::push(self, value);
    }

    fn limit_exceeded(&self) -> bool {
        false
    }
}

pub(super) fn escape_html_text<S>(value: &str, out: &mut S)
where
    S: HtmlSerializationSink + ?Sized,
{
    for ch in value.chars() {
        if out.limit_exceeded() {
            return;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{00A0}' => out.push_str("&nbsp;"),
            _ => out.push(ch),
        }
    }
}

pub(super) fn escape_html_attribute<S>(value: &str, out: &mut S)
where
    S: HtmlSerializationSink + ?Sized,
{
    for ch in value.chars() {
        if out.limit_exceeded() {
            return;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

pub(super) fn serialize_cdata_section<S>(
    value: &str,
    out: &mut S,
    raw_text_parent: bool,
    html_document: bool,
) where
    S: HtmlSerializationSink + ?Sized,
{
    if html_document {
        if raw_text_parent {
            out.push_str(value);
        } else {
            escape_html_text(value, out);
        }
    } else {
        out.push_str("<![CDATA[");
        out.push_str(value);
        out.push_str("]]>");
    }
}

pub(super) fn is_void_html_element(namespace: &str, local_name: &str) -> bool {
    namespace == "http://www.w3.org/1999/xhtml"
        && matches!(
            local_name,
            "area"
                | "base"
                | "basefont"
                | "bgsound"
                | "br"
                | "col"
                | "embed"
                | "frame"
                | "hr"
                | "img"
                | "input"
                | "keygen"
                | "link"
                | "meta"
                | "param"
                | "source"
                | "track"
                | "wbr"
        )
}

enum HtmlSerializationFrame<'a> {
    Node(NativeNodeId),
    Children(NativeNodeId),
    ShadowRootTemplate(HtmlSerializedShadowRoot),
    CloseElement(&'a str),
    CloseShadowRootTemplate,
}

pub(super) fn serialize_html_into_sink<S>(
    dom: &NativeDom,
    node_id: NativeNodeId,
    options: HtmlSerializationOptions<'_>,
    out: &mut S,
) -> bool
where
    S: HtmlSerializationSink,
{
    let Some(node) = dom.node(node_id) else {
        return false;
    };
    if options.target == HtmlSerializationTarget::ChildrenOnly
        && node
            .as_element()
            .is_some_and(|element| is_void_html_element(element.namespace(), element.local_name()))
    {
        return true;
    }

    let mut stack = match options.target {
        HtmlSerializationTarget::IncludeNode => vec![HtmlSerializationFrame::Node(node_id)],
        HtmlSerializationTarget::ChildrenOnly => {
            let mut stack = vec![HtmlSerializationFrame::Children(node_id)];
            push_shadow_root_frame(node_id, options.shadow_root_provider, &mut stack);
            stack
        }
    };

    while !out.limit_exceeded() {
        let Some(frame) = stack.pop() else {
            break;
        };
        match frame {
            HtmlSerializationFrame::Node(node_id) => {
                serialize_html_node_frame(dom, node_id, options, out, &mut stack);
            }
            HtmlSerializationFrame::Children(node_id) => {
                push_child_html_serialization_frames(dom, node_id, &mut stack);
            }
            HtmlSerializationFrame::ShadowRootTemplate(shadow_root) => {
                out.push_str(&shadow_root.template_start);
                stack.push(HtmlSerializationFrame::CloseShadowRootTemplate);
                stack.push(HtmlSerializationFrame::Children(shadow_root.root));
            }
            HtmlSerializationFrame::CloseElement(local_name) => {
                out.push_str("</");
                out.push_str(local_name);
                out.push('>');
            }
            HtmlSerializationFrame::CloseShadowRootTemplate => {
                out.push_str("</template>");
            }
        }
    }
    true
}

fn serialize_html_node_frame<'a, S>(
    dom: &'a NativeDom,
    node_id: NativeNodeId,
    options: HtmlSerializationOptions<'_>,
    out: &mut S,
    stack: &mut Vec<HtmlSerializationFrame<'a>>,
) where
    S: HtmlSerializationSink,
{
    let Some(node) = dom.node(node_id) else {
        return;
    };
    match node.data() {
        NodeData::Document(_) | NodeData::DocumentFragment(_) => {
            stack.push(HtmlSerializationFrame::Children(node_id));
        }
        NodeData::DocumentType(document_type) => {
            out.push_str("<!DOCTYPE ");
            out.push_str(document_type.name());
            if !document_type.public_id().is_empty() || !document_type.system_id().is_empty() {
                out.push_str(" PUBLIC \"");
                out.push_str(document_type.public_id());
                out.push_str("\" \"");
                out.push_str(document_type.system_id());
                out.push('"');
            }
            out.push('>');
        }
        NodeData::Element(element) => {
            out.push('<');
            out.push_str(element.local_name());
            if let Some(is_name) = element.custom_element_is_name()
                && !element.has_attribute("is")
            {
                out.push_str(" is=\"");
                escape_html_attribute(is_name, out);
                out.push('"');
            }
            for attribute in element.attributes() {
                if out.limit_exceeded() {
                    return;
                }
                out.push(' ');
                attribute.push_html_serialized_name(|part| out.push_str(part));
                out.push_str("=\"");
                escape_html_attribute(attribute.value(), out);
                out.push('"');
            }
            out.push('>');

            if !is_void_html_element(element.namespace(), element.local_name()) {
                stack.push(HtmlSerializationFrame::CloseElement(element.local_name()));
                stack.push(HtmlSerializationFrame::Children(node_id));
                push_shadow_root_frame(node_id, options.shadow_root_provider, stack);
            }
        }
        NodeData::Text(text) => {
            if text_data_serializes_literally(dom, node_id, options.scripting_enabled) {
                out.push_str(text.data());
            } else {
                escape_html_text(text.data(), out);
            }
        }
        NodeData::CDataSection(cdata) => {
            serialize_cdata_section(
                cdata.data(),
                out,
                text_data_serializes_literally(dom, node_id, options.scripting_enabled),
                dom.node_document_is_html_document(node_id).unwrap_or(false),
            );
        }
        NodeData::Comment(comment) => {
            out.push_str("<!--");
            out.push_str(comment.data());
            out.push_str("-->");
        }
        NodeData::ProcessingInstruction(processing_instruction) => {
            out.push_str("<?");
            out.push_str(processing_instruction.target());
            if !processing_instruction.data().is_empty() {
                out.push(' ');
                out.push_str(processing_instruction.data());
            }
            out.push_str("?>");
        }
    }
}

fn push_child_html_serialization_frames<'a>(
    dom: &NativeDom,
    node_id: NativeNodeId,
    stack: &mut Vec<HtmlSerializationFrame<'a>>,
) {
    let children_root = dom
        .node(node_id)
        .and_then(Node::as_element)
        .and_then(|element| element.template_contents())
        .unwrap_or(node_id);
    stack.extend(
        dom.child_ids_reversed(children_root)
            .map(HtmlSerializationFrame::Node),
    );
}

fn push_shadow_root_frame(
    host: NativeNodeId,
    shadow_root_provider: Option<&dyn HtmlShadowRootProvider>,
    stack: &mut Vec<HtmlSerializationFrame<'_>>,
) {
    let Some(shadow_root) =
        shadow_root_provider.and_then(|provider| provider.serialized_shadow_root_for_host(host))
    else {
        return;
    };
    stack.push(HtmlSerializationFrame::ShadowRootTemplate(shadow_root));
}

fn text_data_serializes_literally(
    dom: &NativeDom,
    node_id: NativeNodeId,
    scripting_enabled: bool,
) -> bool {
    if !dom.node_document_is_html_document(node_id).unwrap_or(false) {
        return false;
    }
    let Some(parent) = dom
        .parent_node(node_id)
        .and_then(|parent| dom.node(parent))
        .and_then(Node::as_element)
    else {
        return false;
    };
    if parent.namespace() != "http://www.w3.org/1999/xhtml" {
        return false;
    }
    matches!(
        parent.local_name(),
        "style" | "script" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext"
    ) || scripting_enabled && parent.local_name() == "noscript"
}

struct BoundedHtmlSerialization {
    output: String,
    max_bytes: usize,
    exceeded: bool,
}

impl BoundedHtmlSerialization {
    fn new(max_bytes: usize) -> Self {
        Self {
            output: String::new(),
            max_bytes,
            exceeded: false,
        }
    }

    fn finish(self) -> Result<String, HtmlSerializationLimitExceeded> {
        if self.exceeded {
            Err(HtmlSerializationLimitExceeded {
                max_bytes: self.max_bytes,
            })
        } else {
            Ok(self.output)
        }
    }
}

impl HtmlSerializationSink for BoundedHtmlSerialization {
    fn push_str(&mut self, value: &str) {
        if self.exceeded {
            return;
        }
        if self
            .output
            .len()
            .checked_add(value.len())
            .is_none_or(|length| length > self.max_bytes)
        {
            self.exceeded = true;
            return;
        }
        self.output.push_str(value);
    }

    fn push(&mut self, value: char) {
        if self.exceeded {
            return;
        }
        if self
            .output
            .len()
            .checked_add(value.len_utf8())
            .is_none_or(|length| length > self.max_bytes)
        {
            self.exceeded = true;
            return;
        }
        self.output.push(value);
    }

    fn limit_exceeded(&self) -> bool {
        self.exceeded
    }
}

impl NativeDom {
    pub fn serialize_document(&self) -> String {
        self.serialize_document_with_scripting_enabled(true)
    }

    pub fn serialize_document_with_scripting_enabled(&self, scripting_enabled: bool) -> String {
        let mut html = String::new();
        serialize_html_into_sink(
            self,
            self.document_node_id,
            HtmlSerializationOptions::new(HtmlSerializationTarget::ChildrenOnly, scripting_enabled),
            &mut html,
        );
        html
    }

    pub fn is_html_element_named(&self, node_id: NativeNodeId, local_name: &str) -> bool {
        self.node(node_id)
            .is_some_and(|node| node.is_html_element_named(local_name))
    }

    pub fn option_value(&self, node_id: NativeNodeId) -> Option<String> {
        let element = self.node(node_id).and_then(Node::as_element)?;
        if !element.is_html_option() {
            return None;
        }
        Some(element.option_value(self, node_id))
    }

    pub fn outer_html(&self, node_id: NativeNodeId) -> Option<String> {
        self.outer_html_with_scripting_enabled(node_id, true)
    }

    pub fn outer_html_with_scripting_enabled(
        &self,
        node_id: NativeNodeId,
        scripting_enabled: bool,
    ) -> Option<String> {
        let mut html = String::new();
        serialize_html_into_sink(
            self,
            node_id,
            HtmlSerializationOptions::new(HtmlSerializationTarget::IncludeNode, scripting_enabled),
            &mut html,
        )
        .then_some(html)
    }

    /// Serializes one subtree without ever growing the output beyond `max_bytes`.
    ///
    /// This is intended for bounded derived consumers such as a fresh inline
    /// SVG image parse. Web-exposed `outerHTML` continues to use the unbounded
    /// serializer because truncation would violate its string contract.
    pub fn outer_html_with_limit(
        &self,
        node_id: NativeNodeId,
        max_bytes: usize,
    ) -> Result<Option<String>, HtmlSerializationLimitExceeded> {
        let mut out = BoundedHtmlSerialization::new(max_bytes);
        if !serialize_html_into_sink(
            self,
            node_id,
            HtmlSerializationOptions::new(HtmlSerializationTarget::IncludeNode, true),
            &mut out,
        ) {
            return Ok(None);
        }
        out.finish().map(Some)
    }

    pub fn inner_html(&self, node_id: NativeNodeId) -> Option<String> {
        self.inner_html_with_scripting_enabled(node_id, true)
    }

    pub fn inner_html_with_scripting_enabled(
        &self,
        node_id: NativeNodeId,
        scripting_enabled: bool,
    ) -> Option<String> {
        let mut html = String::new();
        serialize_html_into_sink(
            self,
            node_id,
            HtmlSerializationOptions::new(HtmlSerializationTarget::ChildrenOnly, scripting_enabled),
            &mut html,
        )
        .then_some(html)
    }

    pub fn script_handles(&self) -> Vec<NativeNodeId> {
        self.nodes
            .iter()
            .filter_map(|node| node.is_script_element().then_some(node.id()))
            .collect()
    }

    pub fn script_node_ids(&self) -> Vec<NativeNodeId> {
        self.script_handles()
    }

    pub fn document_order_script_handles(&self) -> Vec<NativeNodeId> {
        let mut script_handles = Vec::new();
        let mut stack = vec![self.document_node_id];
        while let Some(node_id) = stack.pop() {
            let Some(node) = self.node(node_id) else {
                continue;
            };
            if node.is_script_element() {
                script_handles.push(node_id);
            }
            stack.extend(self.child_ids_reversed(node_id));
        }
        script_handles
    }

    pub fn document_order_script_node_ids(&self) -> Vec<NativeNodeId> {
        self.document_order_script_handles()
    }

    pub fn script_src(&self, node_id: NativeNodeId) -> Option<&str> {
        self.node(node_id)?.as_element()?.script_source_attribute()
    }

    pub fn script_text(&self, node_id: NativeNodeId) -> Option<String> {
        let script_node = self.node(node_id)?;
        let element = script_node.as_element()?;
        if !element.is_script_element() {
            return None;
        }

        let mut script_text = String::new();
        for child_id in script_node.child_ids(self) {
            let Some(child) = self.node(child_id) else {
                continue;
            };

            if let Some(text) = child.as_text() {
                script_text.push_str(text.data());
            }
        }

        (!script_text.is_empty()).then_some(script_text)
    }

    pub fn push_parse_error(&mut self, error: String) {
        self.parse_errors.push(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::DomHost;

    fn test_url() -> url::Url {
        url::Url::parse("https://serialization.test/").expect("test URL")
    }

    #[test]
    fn html_serializers_share_the_complete_void_element_set() {
        let mut dom = NativeDom::new_html(test_url());
        let container = dom.create_element("div");
        let mut expected = String::new();
        let mut void_elements = Vec::new();
        for local_name in [
            "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img",
            "input", "keygen", "link", "meta", "param", "source", "track", "wbr",
        ] {
            let element = dom.create_element(local_name);
            let ignored_child = dom.create_element("span");
            assert!(dom.append_child(element, ignored_child));
            assert_eq!(dom.outer_html(element), Some(format!("<{local_name}>")));
            assert_eq!(dom.inner_html(element).as_deref(), Some(""));
            assert!(dom.append_child(container, element));
            expected.push_str(&format!("<{local_name}>"));
            void_elements.push(element);
        }
        assert_eq!(
            dom.inner_html(container).as_deref(),
            Some(expected.as_str())
        );

        let foreign_param = dom
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "param")
            .expect("SVG element");
        assert_eq!(
            dom.outer_html(foreign_param).as_deref(),
            Some("<param></param>")
        );

        let mut host = DomHost::from_dom(dom);
        let param = host.create_element("param");
        assert!(host.append_child(container, param));
        assert_eq!(
            host.get_html(container, true, false, &[]).as_deref(),
            Some(format!("{expected}<param>").as_str())
        );
        for element in void_elements {
            assert_eq!(
                host.get_html(element, true, false, &[]).as_deref(),
                Some("")
            );
        }
    }

    #[test]
    fn html_text_serialization_uses_the_actual_parent_and_scripting_mode() {
        let mut dom = NativeDom::new_html(test_url());
        let container = dom.create_element("main");
        let mut cases = Vec::new();
        for (local_name, literal_with_scripting, literal_without_scripting) in [
            ("style", true, true),
            ("script", true, true),
            ("xmp", true, true),
            ("iframe", true, true),
            ("noembed", true, true),
            ("noframes", true, true),
            ("plaintext", true, true),
            ("noscript", true, false),
            ("textarea", false, false),
            ("title", false, false),
            ("div", false, false),
        ] {
            let element = dom.create_element(local_name);
            let text = dom.create_text_node("<&");
            assert!(dom.append_child(element, text));
            assert!(dom.append_child(container, element));
            cases.push((
                element,
                local_name,
                literal_with_scripting,
                literal_without_scripting,
            ));
        }

        for &(element, _, literal_with_scripting, literal_without_scripting) in &cases {
            assert_eq!(
                dom.inner_html_with_scripting_enabled(element, true)
                    .as_deref(),
                Some(if literal_with_scripting {
                    "<&"
                } else {
                    "&lt;&amp;"
                })
            );
            assert_eq!(
                dom.inner_html_with_scripting_enabled(element, false)
                    .as_deref(),
                Some(if literal_without_scripting {
                    "<&"
                } else {
                    "&lt;&amp;"
                })
            );
        }

        let host = DomHost::from_dom(dom);
        for &(element, local_name, literal_with_scripting, literal_without_scripting) in &cases {
            for (scripting_enabled, literal) in [
                (true, literal_with_scripting),
                (false, literal_without_scripting),
            ] {
                let contents = if literal { "<&" } else { "&lt;&amp;" };
                assert_eq!(
                    host.get_html_with_shadow_root_registry_attribute_policy(
                        element,
                        scripting_enabled,
                        false,
                        &[],
                        None,
                    )
                    .as_deref(),
                    Some(contents),
                    "children-only serialization diverged for {local_name}"
                );
                assert_eq!(
                    host.outer_html_with_shadow_roots(
                        element,
                        scripting_enabled,
                        crate::native::ShadowRootInclusion::None,
                        None,
                    ),
                    Some(format!("<{local_name}>{contents}</{local_name}>")),
                    "node serialization diverged for {local_name}"
                );
            }
        }
    }

    #[test]
    fn bounded_outer_html_stops_before_exceeding_the_output_limit() {
        let mut dom = NativeDom::new_html(test_url());
        let element = dom.create_element("div");
        let expected = "<div></div>";

        assert_eq!(
            dom.outer_html_with_limit(element, expected.len()),
            Ok(Some(expected.to_owned()))
        );
        assert_eq!(
            dom.outer_html_with_limit(element, expected.len() - 1),
            Err(HtmlSerializationLimitExceeded {
                max_bytes: expected.len() - 1,
            })
        );
    }

    #[test]
    fn html_serializers_apply_attribute_serialized_name_rules() {
        let mut dom = NativeDom::new_html(test_url());
        let container = dom.create_element("section");
        let element = dom
            .create_element_ns(Some("urn:element"), "div")
            .expect("namespaced element");
        assert!(dom.set_attribute_ns(
            element,
            Some("http://www.w3.org/XML/1998/namespace"),
            Some("alternate"),
            "lang",
            "en-us",
        ));
        assert!(dom.set_attribute_ns(
            element,
            Some("http://www.w3.org/2000/xmlns/"),
            None,
            "binding",
            "urn:binding",
        ));
        assert!(dom.set_attribute_ns(
            element,
            Some("http://www.w3.org/2000/xmlns/"),
            None,
            "xmlns",
            "urn:default",
        ));
        assert!(dom.set_attribute_ns(
            element,
            Some("http://www.w3.org/1999/xlink"),
            Some("alternate"),
            "href",
            "target",
        ));
        assert!(dom.set_attribute_ns(element, Some("urn:custom"), Some("p"), "attr", "value",));
        assert!(dom.append_child(container, element));

        let expected = concat!(
            "<div xml:lang=\"en-us\" xmlns:binding=\"urn:binding\" ",
            "xmlns=\"urn:default\" xlink:href=\"target\" p:attr=\"value\"></div>"
        );
        assert_eq!(dom.inner_html(container).as_deref(), Some(expected));

        let host = DomHost::from_dom(dom);
        assert_eq!(
            host.get_html(container, true, false, &[]).as_deref(),
            Some(expected)
        );
    }

    #[test]
    fn html_serializers_escape_adopted_cdata_as_text() {
        const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
        const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

        let mut xml_dom = NativeDom::new_xml(test_url());
        let xml_svg = xml_dom
            .create_element_ns(Some(SVG_NAMESPACE), "svg")
            .expect("XML SVG element");
        assert!(xml_dom.set_attribute_ns(
            xml_svg,
            Some(XMLNS_NAMESPACE),
            None,
            "xmlns",
            SVG_NAMESPACE,
        ));
        let xml_cdata = xml_dom.create_cdata_section("<img>&");
        assert!(xml_dom.append_child(xml_svg, xml_cdata));
        assert_eq!(
            xml_dom.outer_html(xml_svg).as_deref(),
            Some(r#"<svg xmlns="http://www.w3.org/2000/svg"><![CDATA[<img>&]]></svg>"#)
        );

        let mut html_dom = NativeDom::new_html(test_url());
        let html_svg = html_dom
            .create_element_ns(Some(SVG_NAMESPACE), "svg")
            .expect("HTML-document SVG element");
        assert!(html_dom.set_attribute_ns(
            html_svg,
            Some(XMLNS_NAMESPACE),
            None,
            "xmlns",
            SVG_NAMESPACE,
        ));
        let adopted_cdata = html_dom.create_cdata_section("<img>&");
        assert!(html_dom.append_child(html_svg, adopted_cdata));
        assert_eq!(
            html_dom.outer_html(html_svg).as_deref(),
            Some(r#"<svg xmlns="http://www.w3.org/2000/svg">&lt;img&gt;&amp;</svg>"#)
        );

        let host = DomHost::from_dom(html_dom);
        assert_eq!(
            host.get_html(html_svg, true, false, &[]).as_deref(),
            Some("&lt;img&gt;&amp;")
        );
    }
}
