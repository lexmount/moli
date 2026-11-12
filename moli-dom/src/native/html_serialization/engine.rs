use super::super::NativeDom;
use super::super::node::{NativeNodeId, Node, NodeData};
use super::{
    HtmlEmbeddedDocument, HtmlEmbeddedDocumentProvider, HtmlInsertionPoint, HtmlSerializationError,
    HtmlSerializationInsertion, HtmlSerializationLimitExceeded, HtmlSerializationPolicy,
};
use std::collections::HashSet;

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
    // This is node-scoped: one traversal can enter inert template contents
    // whose owner Document has different scripting state from the outer tree.
    scripting_enabled_for_node: &'a dyn Fn(NativeNodeId) -> bool,
    shadow_root_provider: Option<&'a dyn HtmlShadowRootProvider>,
    policy: Option<&'a dyn HtmlSerializationPolicy>,
    insertion: Option<HtmlSerializationInsertion<'a>>,
    embedded_documents: Option<&'a dyn HtmlEmbeddedDocumentProvider>,
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
            policy: None,
            insertion: None,
            embedded_documents: None,
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
            '\u{00A0}' => out.push_str("&nbsp;"),
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

// There is one fixed writer. Depth is data, never another generic sink type.
struct HtmlEscapingWriter<'a, S, const EMBEDDED: bool> {
    sink: &'a mut S,
    attribute_depth: usize,
}

impl<S: HtmlSerializationSink, const EMBEDDED: bool> HtmlSerializationSink
    for HtmlEscapingWriter<'_, S, EMBEDDED>
{
    fn push_str(&mut self, value: &str) {
        if !EMBEDDED || self.attribute_depth == 0 {
            self.sink.push_str(value);
            return;
        }
        let mut start = 0;
        for (index, ch) in value.char_indices() {
            if matches!(ch, '&' | '<' | '>' | '"' | '\u{a0}') {
                self.sink.push_str(&value[start..index]);
                self.push(ch);
                start = index + ch.len_utf8();
                if self.limit_exceeded() {
                    return;
                }
            }
        }
        self.sink.push_str(&value[start..]);
    }
    fn push(&mut self, value: char) {
        if !EMBEDDED || self.attribute_depth == 0 {
            self.sink.push(value);
            return;
        }
        let (suffix, amps) = match value {
            '&' => ("", self.attribute_depth),
            '<' => ("lt;", self.attribute_depth - 1),
            '>' => ("gt;", self.attribute_depth - 1),
            '"' => ("quot;", self.attribute_depth - 1),
            '\u{a0}' => ("nbsp;", self.attribute_depth - 1),
            _ => {
                self.sink.push(value);
                return;
            }
        };
        self.sink.push('&');
        for _ in 0..amps {
            if self.limit_exceeded() {
                return;
            }
            self.sink.push_str("amp;");
        }
        self.sink.push_str(suffix);
    }
    fn limit_exceeded(&self) -> bool {
        self.sink.limit_exceeded()
    }
}

enum HtmlSerializationFrame<'a> {
    Node(NativeNodeId),
    Children(NativeNodeId),
    ShadowRootTemplate(HtmlSerializedShadowRoot),
    CloseElement(&'a str),
    CloseShadowRootTemplate,
    Markup(&'a str),
    EmbeddedAttributes,
    EndEmbeddedDocument(NativeNodeId),
}

struct EmbeddedAttributes<'a> {
    owner: NativeNodeId,
    embedded: HtmlEmbeddedDocument<'a>,
    next_attribute: usize,
    wrote_srcdoc: bool,
    wrote_url: bool,
}

struct HtmlSerializer<'dom, 'sink, S, const EMBEDDED: bool, const POLICY: bool> {
    dom: &'dom NativeDom,
    options: HtmlSerializationOptions<'dom>,
    out: HtmlEscapingWriter<'sink, S, EMBEDDED>,
    stack: Vec<HtmlSerializationFrame<'dom>>,
    embedded_attributes: Vec<EmbeddedAttributes<'dom>>,
    active_documents: HashSet<NativeNodeId>,
    root_document: Option<NativeNodeId>,
}

impl<'dom, S: HtmlSerializationSink, const EMBEDDED: bool, const POLICY: bool>
    HtmlSerializer<'dom, '_, S, EMBEDDED, POLICY>
{
    #[inline(always)]
    fn run(&mut self) -> Result<(), HtmlSerializationError> {
        while !self.out.limit_exceeded() {
            let Some(frame) = self.stack.pop() else {
                break;
            };
            match frame {
                HtmlSerializationFrame::Node(node) => self.node(node)?,
                HtmlSerializationFrame::Children(node) => {
                    let insertion = self.options.insertion.filter(|i| POLICY && i.node == node);
                    if let Some(insertion) = insertion
                        && insertion.point == HtmlInsertionPoint::LastChild
                    {
                        self.stack
                            .push(HtmlSerializationFrame::Markup(insertion.markup));
                    }
                    let children_root = self
                        .dom
                        .node(node)
                        .and_then(Node::as_element)
                        .and_then(|e| e.template_contents())
                        .unwrap_or(node);
                    self.stack.extend(
                        self.dom
                            .child_ids_reversed(children_root)
                            .map(HtmlSerializationFrame::Node),
                    );
                    if let Some(insertion) = insertion
                        && insertion.point == HtmlInsertionPoint::FirstChild
                    {
                        self.out.push_str(insertion.markup);
                    }
                }
                HtmlSerializationFrame::ShadowRootTemplate(root) => {
                    self.out.push_str(&root.template_start);
                    self.stack
                        .push(HtmlSerializationFrame::CloseShadowRootTemplate);
                    self.stack.push(HtmlSerializationFrame::Children(root.root));
                }
                HtmlSerializationFrame::CloseElement(name) => {
                    self.out.push_str("</");
                    self.out.push_str(name);
                    self.out.push('>');
                }
                HtmlSerializationFrame::CloseShadowRootTemplate => self.out.push_str("</template>"),
                HtmlSerializationFrame::Markup(markup) => self.out.push_str(markup),
                HtmlSerializationFrame::EmbeddedAttributes => self.embedded_attributes()?,
                HtmlSerializationFrame::EndEmbeddedDocument(document) => {
                    self.active_documents.remove(&document);
                    self.out.attribute_depth -= 1;
                    self.out.push('"');
                }
            }
        }
        Ok(())
    }

    // Keep node dispatch in the traversal loop so ordinary output can
    // eliminate frame-only work along with the depth-aware writer.
    #[inline(always)]
    fn node(&mut self, id: NativeNodeId) -> Result<(), HtmlSerializationError> {
        let Some(node) = self.dom.node(id) else {
            return Ok(());
        };
        if let Some(insertion) = self.options.insertion
            && POLICY
            && insertion.node == id
            && insertion.point == HtmlInsertionPoint::BeforeNode
        {
            self.out.push_str(insertion.markup);
        }
        if self
            .options
            .policy
            .is_some_and(|p| POLICY && !p.include_node(node))
        {
            return Ok(());
        }
        match node.data() {
            NodeData::Document(_) | NodeData::DocumentFragment(_) => {
                self.stack.push(HtmlSerializationFrame::Children(id))
            }
            NodeData::DocumentType(document_type) => {
                self.out.push_str("<!DOCTYPE ");
                self.out.push_str(document_type.name());
                if !document_type.public_id().is_empty() || !document_type.system_id().is_empty() {
                    self.out.push_str(" PUBLIC \"");
                    self.out.push_str(document_type.public_id());
                    self.out.push_str("\" \"");
                    self.out.push_str(document_type.system_id());
                    self.out.push('"');
                }
                self.out.push('>');
            }
            NodeData::Element(element) => {
                let overrides = self
                    .options
                    .policy
                    .filter(|_| POLICY)
                    .map_or(&[][..], |p| p.attribute_overrides(id));
                self.out.push('<');
                self.out.push_str(element.local_name());
                if let Some(name) = element.custom_element_is_name()
                    && !element.has_attribute("is")
                    && !overrides
                        .iter()
                        .any(|a| a.namespace().is_empty() && a.local_name() == "is")
                {
                    self.out.push_str(" is=\"");
                    escape_html_attribute(name, &mut self.out);
                    self.out.push('"');
                }
                // Filtering happens before lookup; excluded ancestors never
                // consult frame/shadow providers or build derived child output.
                if let Some(embedded) = self
                    .options
                    .embedded_documents
                    .filter(|_| EMBEDDED)
                    .and_then(|p| p.embedded_document(id))
                {
                    self.embedded_attributes.push(EmbeddedAttributes {
                        owner: id,
                        embedded,
                        next_attribute: 0,
                        wrote_srcdoc: false,
                        wrote_url: false,
                    });
                    self.stack.push(HtmlSerializationFrame::EmbeddedAttributes);
                } else {
                    for attribute in element.attributes() {
                        if self.out.limit_exceeded() {
                            return Ok(());
                        }
                        let output = replacement_attribute(attribute, overrides);
                        if self
                            .options
                            .policy
                            .is_none_or(|p| !POLICY || p.include_attribute(id, output))
                        {
                            serialize_attribute(output, &mut self.out);
                        }
                    }
                    for attribute in overrides {
                        if !element
                            .attributes()
                            .iter()
                            .any(|a| attribute_key_eq(a, attribute))
                            && self
                                .options
                                .policy
                                .is_none_or(|p| !POLICY || p.include_attribute(id, attribute))
                        {
                            serialize_attribute(attribute, &mut self.out);
                        }
                    }
                    self.end_open_element(id);
                }
            }
            NodeData::Text(text) => {
                if text_data_serializes_literally(
                    self.dom,
                    id,
                    self.options.scripting_enabled_for_node,
                ) {
                    self.out.push_str(text.data());
                } else {
                    escape_html_text(text.data(), &mut self.out);
                }
            }
            NodeData::CDataSection(cdata) => serialize_cdata_section(
                cdata.data(),
                &mut self.out,
                text_data_serializes_literally(
                    self.dom,
                    id,
                    self.options.scripting_enabled_for_node,
                ),
                self.dom.node_document_is_html_document(id).unwrap_or(false),
            ),
            NodeData::Comment(comment) => {
                self.out.push_str("<!--");
                self.out.push_str(comment.data());
                self.out.push_str("-->");
            }
            NodeData::ProcessingInstruction(pi) => {
                self.out.push_str("<?");
                self.out.push_str(pi.target());
                if !pi.data().is_empty() {
                    self.out.push(' ');
                    self.out.push_str(pi.data());
                }
                self.out.push_str("?>");
            }
        }
        Ok(())
    }

    #[inline]
    fn end_open_element(&mut self, id: NativeNodeId) {
        self.out.push('>');
        let element = self
            .dom
            .node(id)
            .and_then(Node::as_element)
            .expect("visited element");
        if !is_void_html_element(element.namespace(), element.local_name()) {
            self.stack
                .push(HtmlSerializationFrame::CloseElement(element.local_name()));
            self.stack.push(HtmlSerializationFrame::Children(id));
            self.shadow_root(id);
        }
    }

    fn shadow_root(&mut self, host: NativeNodeId) {
        if let Some(root) = self
            .options
            .shadow_root_provider
            .and_then(|p| p.serialized_shadow_root_for_host(host))
        {
            self.stack
                .push(HtmlSerializationFrame::ShadowRootTemplate(root));
        }
    }

    fn embedded_attributes(&mut self) -> Result<(), HtmlSerializationError> {
        let mut state = self
            .embedded_attributes
            .pop()
            .expect("pending owner attributes");
        let element = self
            .dom
            .node(state.owner)
            .and_then(Node::as_element)
            .expect("visited owner");
        let overrides = self
            .options
            .policy
            .filter(|_| POLICY)
            .map_or(&[][..], |p| p.attribute_overrides(state.owner));
        while state.next_attribute < element.attributes().len() + overrides.len() {
            let index = state.next_attribute;
            state.next_attribute += 1;
            let attribute = if let Some(attribute) = element.attributes().get(index) {
                replacement_attribute(attribute, overrides)
            } else {
                let attribute = &overrides[index - element.attributes().len()];
                if element
                    .attributes()
                    .iter()
                    .any(|a| attribute_key_eq(a, attribute))
                {
                    continue;
                }
                attribute
            };
            if attribute.name_matches("srcdoc") {
                state.wrote_srcdoc = true;
                self.start_embedded(state)?;
                return Ok(());
            }
            if attribute.name_matches("data-moli-frame-url") {
                state.wrote_url = true;
                self.out.push_str(" data-moli-frame-url=\"");
                escape_html_attribute(&state.embedded.url, &mut self.out);
                self.out.push('"');
            } else if self
                .options
                .policy
                .is_none_or(|p| !POLICY || p.include_attribute(state.owner, attribute))
            {
                serialize_attribute(attribute, &mut self.out);
            }
        }
        if !state.wrote_srcdoc {
            state.wrote_srcdoc = true;
            self.start_embedded(state)?;
            return Ok(());
        }
        if !state.wrote_url {
            self.out.push_str(" data-moli-frame-url=\"");
            escape_html_attribute(&state.embedded.url, &mut self.out);
            self.out.push('"');
        }
        self.end_open_element(state.owner);
        Ok(())
    }

    fn start_embedded(
        &mut self,
        state: EmbeddedAttributes<'dom>,
    ) -> Result<(), HtmlSerializationError> {
        let document = state.embedded.document;
        if !self.dom.node(document).is_some_and(Node::is_document) {
            return Err(HtmlSerializationError::InvalidNode { node: document });
        }
        if self.root_document == Some(document) || !self.active_documents.insert(document) {
            return Err(HtmlSerializationError::CyclicEmbeddedDocument { document });
        }
        self.out.push_str(" srcdoc=\"");
        self.out.attribute_depth += 1;
        self.embedded_attributes.push(state);
        self.stack.push(HtmlSerializationFrame::EmbeddedAttributes);
        self.stack
            .push(HtmlSerializationFrame::EndEmbeddedDocument(document));
        self.stack.push(HtmlSerializationFrame::Children(document));
        Ok(())
    }
}

fn attribute_key_eq(a: &super::super::Attribute, b: &super::super::Attribute) -> bool {
    a.namespace() == b.namespace() && a.local_name() == b.local_name()
}
fn replacement_attribute<'a>(
    attribute: &'a super::super::Attribute,
    overrides: &'a [super::super::Attribute],
) -> &'a super::super::Attribute {
    overrides
        .iter()
        .find(|a| attribute_key_eq(a, attribute))
        .unwrap_or(attribute)
}
fn serialize_attribute<S: HtmlSerializationSink>(attribute: &super::super::Attribute, out: &mut S) {
    out.push(' ');
    attribute.push_html_serialized_name(|part| out.push_str(part));
    out.push_str("=\"");
    escape_html_attribute(attribute.value(), out);
    out.push('"');
}

fn serialize_html_into_sink<'a, S: HtmlSerializationSink>(
    dom: &'a NativeDom,
    node_id: NativeNodeId,
    options: HtmlSerializationOptions<'a>,
    out: &mut S,
) -> Result<bool, HtmlSerializationError> {
    // Ordinary Web output has neither export policy nor embedded Documents.
    // Select once, so those optional checks disappear from its hot traversal.
    match (
        options.embedded_documents.is_some(),
        options.policy.is_some(),
    ) {
        (false, false) => serialize_html_in_mode::<S, false, false>(dom, node_id, options, out),
        (false, true) => serialize_html_in_mode::<S, false, true>(dom, node_id, options, out),
        (true, false) => serialize_html_in_mode::<S, true, false>(dom, node_id, options, out),
        (true, true) => serialize_html_in_mode::<S, true, true>(dom, node_id, options, out),
    }
}

fn serialize_html_in_mode<
    'a,
    S: HtmlSerializationSink,
    const EMBEDDED: bool,
    const POLICY: bool,
>(
    dom: &'a NativeDom,
    node_id: NativeNodeId,
    options: HtmlSerializationOptions<'a>,
    out: &mut S,
) -> Result<bool, HtmlSerializationError> {
    let Some(node) = dom.node(node_id) else {
        return Ok(false);
    };
    if options.target == HtmlSerializationTarget::ChildrenOnly
        && node
            .as_element()
            .is_some_and(|e| is_void_html_element(e.namespace(), e.local_name()))
    {
        return Ok(true);
    }
    let root_document = if node.is_document() {
        Some(node_id)
    } else {
        node.owner_document()
    };
    let stack = match options.target {
        HtmlSerializationTarget::IncludeNode => vec![HtmlSerializationFrame::Node(node_id)],
        HtmlSerializationTarget::ChildrenOnly => vec![HtmlSerializationFrame::Children(node_id)],
    };
    let mut serializer = HtmlSerializer::<S, EMBEDDED, POLICY> {
        dom,
        options,
        out: HtmlEscapingWriter {
            sink: out,
            attribute_depth: 0,
        },
        stack,
        embedded_attributes: Vec::new(),
        active_documents: HashSet::new(),
        root_document,
    };
    if options.target == HtmlSerializationTarget::ChildrenOnly {
        serializer.shadow_root(node_id);
    }
    serializer.run()?;
    Ok(true)
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
    .expect("standard node serialization cannot enter embedded documents")
    .then_some(html)
}

pub(in crate::native) fn serialize_html_with_shadow_root_provider<F>(
    dom: &NativeDom,
    node_id: NativeNodeId,
    target: HtmlSerializationTarget,
    scripting_enabled_for_node: &dyn Fn(NativeNodeId) -> bool,
    shadow_root_provider: Option<&F>,
    policy: Option<&dyn HtmlSerializationPolicy>,
    embedded_documents: Option<&dyn HtmlEmbeddedDocumentProvider>,
    max_bytes: Option<usize>,
) -> Result<Option<String>, HtmlSerializationError>
where
    F: Fn(NativeNodeId) -> Option<HtmlSerializedShadowRoot>,
{
    let mut options = HtmlSerializationOptions::new(target, scripting_enabled_for_node);
    if let Some(provider) = shadow_root_provider {
        options = options.with_shadow_root_provider(provider);
    }
    options.policy = policy;
    options.insertion = policy.and_then(|p| p.insertion());
    options.embedded_documents = embedded_documents;
    if let Some(limit) = max_bytes {
        let mut out = BoundedHtmlSerialization::new(limit);
        if !serialize_html_into_sink(dom, node_id, options, &mut out)? {
            return Ok(None);
        }
        out.finish()
            .map(Some)
            .map_err(|e| HtmlSerializationError::OutputLimitExceeded {
                max_bytes: e.max_bytes,
            })
    } else {
        let mut html = String::new();
        let found = serialize_html_into_sink(dom, node_id, options, &mut html)?;
        Ok(found.then_some(html))
    }
}

pub fn escape_html_attribute_into_string(value: &str, out: &mut String) {
    escape_html_attribute(value, out);
}

pub(super) fn serialize_html_with_limit(
    dom: &NativeDom,
    node_id: NativeNodeId,
    max_bytes: usize,
) -> Result<Option<String>, HtmlSerializationLimitExceeded> {
    let scripting_enabled_for_node = |_: NativeNodeId| true;
    let mut out = BoundedHtmlSerialization::new(max_bytes);
    if !serialize_html_into_sink(
        dom,
        node_id,
        HtmlSerializationOptions::new(
            HtmlSerializationTarget::IncludeNode,
            &scripting_enabled_for_node,
        ),
        &mut out,
    )
    .expect("bounded node serialization cannot enter embedded documents")
    {
        return Ok(None);
    }
    out.finish().map(Some)
}

#[cfg(test)]
mod escaping_tests {
    use super::*;

    #[test]
    fn fixed_depth_writer_matches_repeated_attribute_escaping_for_unicode_and_token_splits() {
        let mut seed = 1_u64;
        for depth in 0..10 {
            for _ in 0..128 {
                let mut input = String::from("&<>\"\u{a0}雪\n\r\t");
                for _ in 0..64 {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    if let Some(ch) = char::from_u32((seed >> 32) as u32 % 0x110000) {
                        input.push(ch);
                    }
                }
                let mut expected = input.clone();
                for _ in 0..depth {
                    let mut next = String::new();
                    escape_html_attribute(&expected, &mut next);
                    expected = next;
                }
                let mut output = String::new();
                let mut writer = HtmlEscapingWriter::<_, true> {
                    sink: &mut output,
                    attribute_depth: depth,
                };
                writer.push_str(&input);
                assert_eq!(output, expected);
                output.clear();
                let mut writer = HtmlEscapingWriter::<_, true> {
                    sink: &mut output,
                    attribute_depth: depth,
                };
                for ch in input.chars() {
                    writer.push(ch);
                }
                assert_eq!(output, expected);
            }
        }
    }
}
