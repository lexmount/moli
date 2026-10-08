use super::super::node::{NativeNodeId, Node, NodeData};
use super::super::{DomStringValue, NativeDom};
use super::HtmlSerializationLimitExceeded;

pub(in crate::native) trait HtmlSerializationSink {
    const PRESERVES_UTF16: bool = false;

    fn push_str(&mut self, value: &str);
    fn push(&mut self, value: char);
    fn limit_exceeded(&self) -> bool;

    // UTF-8 consumers deliberately use the scalar view. Web API consumers
    // override this boundary to preserve every DOMString code unit.
    fn push_utf16(&mut self, value: &[u16]) {
        self.push_str(&String::from_utf16_lossy(value));
    }

    fn push_dom_string(&mut self, value: &DomStringValue) {
        self.push_str(value.as_str_lossy());
    }
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
    // This is node-scoped: one traversal can enter inert template contents
    // whose owner Document has different scripting state from the outer tree.
    scripting_enabled_for_node: &'a dyn Fn(NativeNodeId) -> bool,
    shadow_root_provider: Option<&'a dyn HtmlShadowRootProvider>,
}

impl<'a> HtmlSerializationOptions<'a> {
    const fn new(
        target: HtmlSerializationTarget,
        scripting_enabled_for_node: &'a dyn Fn(NativeNodeId) -> bool,
    ) -> Self {
        Self {
            target,
            scripting_enabled_for_node,
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

impl HtmlSerializationSink for Vec<u16> {
    const PRESERVES_UTF16: bool = true;

    fn push_str(&mut self, value: &str) {
        self.extend(value.encode_utf16());
    }

    fn push(&mut self, value: char) {
        self.extend_from_slice(value.encode_utf16(&mut [0; 2]));
    }

    fn push_utf16(&mut self, value: &[u16]) {
        self.extend_from_slice(value);
    }

    fn push_dom_string(&mut self, value: &DomStringValue) {
        value.append_utf16_units_to(self);
    }

    fn limit_exceeded(&self) -> bool {
        false
    }
}

fn escaped_html_character(value: u32, attribute: bool) -> Option<&'static str> {
    match value {
        0x26 => Some("&amp;"),
        0x3c => Some("&lt;"),
        0x3e => Some("&gt;"),
        0x22 if attribute => Some("&quot;"),
        0xa0 => Some("&nbsp;"),
        _ => None,
    }
}

fn escape_html<S>(value: &str, units: Option<&[u16]>, attribute: bool, out: &mut S)
where
    S: HtmlSerializationSink + ?Sized,
{
    if S::PRESERVES_UTF16
        && let Some(units) = units
    {
        let mut start = 0;
        for (index, &unit) in units.iter().enumerate() {
            if out.limit_exceeded() {
                return;
            }
            if let Some(escaped) = escaped_html_character(u32::from(unit), attribute) {
                out.push_utf16(&units[start..index]);
                out.push_str(escaped);
                start = index + 1;
            }
        }
        out.push_utf16(&units[start..]);
        return;
    }
    for ch in value.chars() {
        if out.limit_exceeded() {
            return;
        }
        if let Some(escaped) = escaped_html_character(ch as u32, attribute) {
            out.push_str(escaped);
        } else {
            out.push(ch);
        }
    }
}

fn escape_html_dom_string<S>(value: &DomStringValue, attribute: bool, out: &mut S)
where
    S: HtmlSerializationSink + ?Sized,
{
    let units = value.as_str().is_none().then(|| value.utf16_units());
    escape_html(value.as_str_lossy(), units.as_deref(), attribute, out);
}

fn serialize_cdata_section<S>(
    value: &DomStringValue,
    out: &mut S,
    raw_text_parent: bool,
    html_document: bool,
) where
    S: HtmlSerializationSink + ?Sized,
{
    if html_document {
        if raw_text_parent {
            out.push_dom_string(value);
        } else {
            escape_html_dom_string(value, false, out);
        }
    } else {
        out.push_str("<![CDATA[");
        out.push_dom_string(value);
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
            out.push_dom_string(document_type.name_value());
            if !document_type.public_id().is_empty() || !document_type.system_id().is_empty() {
                out.push_str(" PUBLIC \"");
                out.push_dom_string(document_type.public_id_value());
                out.push_str("\" \"");
                out.push_dom_string(document_type.system_id_value());
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
                escape_html_dom_string(is_name, true, out);
                out.push('"');
            }
            for attribute in element.attributes() {
                if out.limit_exceeded() {
                    return;
                }
                out.push(' ');
                attribute.push_html_serialized_name(|part| out.push_str(part));
                out.push_str("=\"");
                escape_html(
                    attribute.value(),
                    element.attribute_value_utf16_units(attribute),
                    true,
                    out,
                );
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
            if text_data_serializes_literally(dom, node_id, options.scripting_enabled_for_node) {
                out.push_dom_string(text.value());
            } else {
                escape_html_dom_string(text.value(), false, out);
            }
        }
        NodeData::CDataSection(cdata) => {
            serialize_cdata_section(
                cdata.value(),
                out,
                text_data_serializes_literally(dom, node_id, options.scripting_enabled_for_node),
                dom.node_document_is_html_document(node_id).unwrap_or(false),
            );
        }
        NodeData::Comment(comment) => {
            out.push_str("<!--");
            out.push_dom_string(comment.value());
            out.push_str("-->");
        }
        NodeData::ProcessingInstruction(processing_instruction) => {
            out.push_str("<?");
            out.push_str(processing_instruction.target());
            if !processing_instruction.data().is_empty() {
                out.push(' ');
                out.push_dom_string(processing_instruction.value());
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
    scripting_enabled_for_node: &dyn Fn(NativeNodeId) -> bool,
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
    ) || parent.local_name() == "noscript" && scripting_enabled_for_node(node_id)
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
    let scripting_enabled_for_node = |_: NativeNodeId| scripting_enabled;
    let mut html = String::new();
    serialize_html_into_sink(
        dom,
        node_id,
        HtmlSerializationOptions::new(target, &scripting_enabled_for_node),
        &mut html,
    )
    .then_some(html)
}

pub(super) fn serialize_html_with_stored_scripting_state(
    dom: &NativeDom,
    node_id: NativeNodeId,
    target: HtmlSerializationTarget,
) -> Option<String> {
    let scripting_enabled_for_node =
        |node| dom.node_document_scripting_enabled(node).unwrap_or(false);
    let mut html = String::new();
    serialize_html_into_sink(
        dom,
        node_id,
        HtmlSerializationOptions::new(target, &scripting_enabled_for_node),
        &mut html,
    )
    .then_some(html)
}

pub(in crate::native) fn serialize_html_with_shadow_root_provider<F, S>(
    dom: &NativeDom,
    node_id: NativeNodeId,
    target: HtmlSerializationTarget,
    scripting_enabled_for_node: &dyn Fn(NativeNodeId) -> bool,
    shadow_root_provider: &F,
) -> Option<S>
where
    F: Fn(NativeNodeId) -> Option<HtmlSerializedShadowRoot>,
    S: HtmlSerializationSink + Default,
{
    let options = HtmlSerializationOptions::new(target, scripting_enabled_for_node)
        .with_shadow_root_provider(shadow_root_provider);
    let mut html = S::default();
    serialize_html_into_sink(dom, node_id, options, &mut html).then_some(html)
}

pub(in crate::native) fn escape_html_attribute_into_string(value: &str, out: &mut String) {
    escape_html(value, None, true, out);
}

pub(super) fn serialize_html_with_limit(
    dom: &NativeDom,
    node_id: NativeNodeId,
    max_bytes: usize,
) -> Result<Option<String>, HtmlSerializationLimitExceeded> {
    let scripting_enabled_for_node =
        |node| dom.node_document_scripting_enabled(node).unwrap_or(false);
    let mut out = BoundedHtmlSerialization::new(max_bytes);
    if !serialize_html_into_sink(
        dom,
        node_id,
        HtmlSerializationOptions::new(
            HtmlSerializationTarget::IncludeNode,
            &scripting_enabled_for_node,
        ),
        &mut out,
    ) {
        return Ok(None);
    }
    out.finish().map(Some)
}
