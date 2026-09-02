use super::super::NativeDom;
use super::super::node::{NativeNodeId, Node, NodeData};
use super::HtmlSerializationLimitExceeded;

trait HtmlSerializationSink {
    fn push_str(&mut self, value: &str);
    fn push(&mut self, value: char);
    fn limit_exceeded(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::native) enum HtmlSerializationTarget {
    IncludeNode,
    ChildrenOnly,
}

pub(in crate::native) struct HtmlSerializedShadowRoot {
    root: NativeNodeId,
    template_start: String,
}

impl HtmlSerializedShadowRoot {
    pub(in crate::native) fn new(root: NativeNodeId, template_start: String) -> Self {
        Self {
            root,
            template_start,
        }
    }
}

trait HtmlShadowRootProvider {
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
struct HtmlSerializationOptions<'a> {
    target: HtmlSerializationTarget,
    scripting_enabled: bool,
    shadow_root_provider: Option<&'a dyn HtmlShadowRootProvider>,
}

impl<'a> HtmlSerializationOptions<'a> {
    const fn new(target: HtmlSerializationTarget, scripting_enabled: bool) -> Self {
        Self {
            target,
            scripting_enabled,
            shadow_root_provider: None,
        }
    }

    const fn with_shadow_root_provider(
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

fn escape_html_text<S>(value: &str, out: &mut S)
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

fn escape_html_attribute<S>(value: &str, out: &mut S)
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

fn serialize_cdata_section<S>(value: &str, out: &mut S, raw_text_parent: bool, html_document: bool)
where
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

fn is_void_html_element(namespace: &str, local_name: &str) -> bool {
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

fn serialize_html_into_sink<S>(
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

pub(super) fn serialize_html(
    dom: &NativeDom,
    node_id: NativeNodeId,
    target: HtmlSerializationTarget,
    scripting_enabled: bool,
) -> Option<String> {
    let mut html = String::new();
    serialize_html_into_sink(
        dom,
        node_id,
        HtmlSerializationOptions::new(target, scripting_enabled),
        &mut html,
    )
    .then_some(html)
}

pub(in crate::native) fn serialize_html_with_shadow_root_provider<F>(
    dom: &NativeDom,
    node_id: NativeNodeId,
    target: HtmlSerializationTarget,
    scripting_enabled: bool,
    shadow_root_provider: &F,
) -> Option<String>
where
    F: Fn(NativeNodeId) -> Option<HtmlSerializedShadowRoot>,
{
    let options = HtmlSerializationOptions::new(target, scripting_enabled)
        .with_shadow_root_provider(shadow_root_provider);
    let mut html = String::new();
    serialize_html_into_sink(dom, node_id, options, &mut html).then_some(html)
}

pub(in crate::native) fn escape_html_attribute_into_string(value: &str, out: &mut String) {
    escape_html_attribute(value, out);
}

pub(super) fn serialize_html_with_limit(
    dom: &NativeDom,
    node_id: NativeNodeId,
    max_bytes: usize,
) -> Result<Option<String>, HtmlSerializationLimitExceeded> {
    let mut out = BoundedHtmlSerialization::new(max_bytes);
    if !serialize_html_into_sink(
        dom,
        node_id,
        HtmlSerializationOptions::new(HtmlSerializationTarget::IncludeNode, true),
        &mut out,
    ) {
        return Ok(None);
    }
    out.finish().map(Some)
}
