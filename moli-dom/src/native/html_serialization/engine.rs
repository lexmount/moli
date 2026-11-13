use super::super::node::{NativeNodeId, Node, NodeData};
use super::super::{NativeDom, ShadowRootInit};
use super::attributes::{EffectiveAttributes, EmbeddedAttribute};
use super::{
    HtmlEmbeddedDocument, HtmlEmbeddedDocumentProvider, HtmlInsertionPoint, HtmlSerializationError,
    HtmlSerializationInsertion, HtmlSerializationLimitExceeded, HtmlSerializationPolicy,
};
use std::collections::HashSet;

trait HtmlSerializationSink {
    // Propagate through escaping wrappers so budgeted output keeps its early
    // exit instead of scanning an entire unchanged run first.
    const BOUNDED: bool = false;

    fn push_str(&mut self, value: &str);
    // The current escape stage leaves this run intact. An enclosing writer
    // still applies its own encoding before the final String receives it.
    #[inline]
    fn unchanged_run(&mut self, value: &str) {
        self.push_str(value);
    }
    fn attribute_value(&mut self, value: &str) {
        escape_html_attribute(value, self);
    }
    fn push(&mut self, value: char);
    fn limit_exceeded(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::native) enum HtmlSerializationTarget {
    IncludeNode,
    ChildrenOnly,
}

pub(in crate::native) struct HtmlSerializedShadowRoot<'a> {
    pub root: NativeNodeId,
    pub init: &'a ShadowRootInit,
    pub web_api: bool,
    pub registry_attribute: bool,
    pub include_style_sheets: bool,
}

pub(in crate::native) trait HtmlShadowRootProvider<'a> {
    fn serialized_shadow_root_for_host(
        &self,
        host: NativeNodeId,
    ) -> Option<HtmlSerializedShadowRoot<'a>>;
}

impl<'a, F> HtmlShadowRootProvider<'a> for F
where
    F: Fn(NativeNodeId) -> Option<HtmlSerializedShadowRoot<'a>>,
{
    fn serialized_shadow_root_for_host(
        &self,
        host: NativeNodeId,
    ) -> Option<HtmlSerializedShadowRoot<'a>> {
        self(host)
    }
}

#[derive(Clone, Copy)]
pub(in crate::native) struct HtmlSerializationOptions<'a> {
    pub target: HtmlSerializationTarget,
    // This is node-scoped: one traversal can enter inert template contents
    // whose owner Document has different scripting state from the outer tree.
    pub scripting_enabled_for_node: &'a dyn Fn(NativeNodeId) -> bool,
    pub shadow_root_provider: Option<&'a dyn HtmlShadowRootProvider<'a>>,
    pub policy: Option<&'a dyn HtmlSerializationPolicy>,
    pub embedded_documents: Option<&'a dyn HtmlEmbeddedDocumentProvider>,
    pub max_bytes: Option<usize>,
}

impl<'a> HtmlSerializationOptions<'a> {
    pub(in crate::native) const fn new(
        target: HtmlSerializationTarget,
        scripting_enabled_for_node: &'a dyn Fn(NativeNodeId) -> bool,
    ) -> Self {
        Self {
            target,
            scripting_enabled_for_node,
            shadow_root_provider: None,
            policy: None,
            embedded_documents: None,
            max_bytes: None,
        }
    }
}

impl HtmlSerializationSink for String {
    fn push_str(&mut self, value: &str) {
        String::push_str(self, value);
    }

    #[inline]
    fn unchanged_run(&mut self, value: &str) {
        if value.len() <= MIN_HTML_ESCAPE_RUN {
            String::push_str(self, value);
            return;
        }
        if let Some(needed) = self.len().checked_add(value.len())
            && needed > self.capacity()
        {
            grow_html_output_for_run(self, needed);
        }
        String::push_str(self, value);
    }

    fn push(&mut self, value: char) {
        String::push(self, value);
    }

    fn limit_exceeded(&self) -> bool {
        false
    }
}

#[cold]
fn grow_html_output_for_run(out: &mut String, needed: usize) {
    // Reserve at geometric capacity once, without leaving a large append
    // exactly full and doubling it again for the following closing tag.
    let mut capacity = out.capacity().max(8);
    while capacity < needed {
        capacity = capacity
            .checked_mul(2)
            .filter(|capacity| *capacity <= isize::MAX as usize)
            .unwrap_or(needed);
    }
    out.reserve(capacity - out.len());
}

#[inline]
fn escape_html_text<S>(value: &str, out: &mut S)
where
    S: HtmlSerializationSink + ?Sized,
{
    if S::BOUNDED || value.len() <= MIN_HTML_ESCAPE_RUN {
        escape_html_scalar::<S, false>(value, out);
    } else {
        escape_long_html_text(value, out);
    }
}

// Long payloads amortize one call; share this loop across traversal modes.
#[inline(never)]
fn escape_long_html_text<S: HtmlSerializationSink + ?Sized>(value: &str, out: &mut S) {
    escape_html_value::<S, false>(value, out);
}

#[inline(always)]
fn escape_html_attribute<S>(value: &str, out: &mut S)
where
    S: HtmlSerializationSink + ?Sized,
{
    escape_html_value::<S, true>(value, out);
}

#[inline(always)]
fn html_escape<const ATTRIBUTE: bool>(ch: char) -> Option<&'static str> {
    match ch {
        '&' => Some("&amp;"),
        '"' if ATTRIBUTE => Some("&quot;"),
        '<' => Some("&lt;"),
        '>' => Some("&gt;"),
        '\u{00A0}' => Some("&nbsp;"),
        _ => None,
    }
}

// Small runs do not amortize slicing and memcpy. Once escapes become dense,
// retain the scalar writer instead of issuing a separate copy for each byte.
const MIN_HTML_ESCAPE_RUN: usize = 16;

#[inline(always)]
fn escape_html_scalar<S: HtmlSerializationSink + ?Sized, const ATTRIBUTE: bool>(
    value: &str,
    out: &mut S,
) {
    for ch in value.chars() {
        if out.limit_exceeded() {
            return;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '"' if ATTRIBUTE => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{00A0}' => out.push_str("&nbsp;"),
            _ => out.push(ch),
        }
    }
}

#[inline(always)]
fn escape_html_value<S: HtmlSerializationSink + ?Sized, const ATTRIBUTE: bool>(
    value: &str,
    out: &mut S,
) {
    // Bounded consumers must stop at their budget rather than scan a long
    // unchanged run before discovering overflow. Short text also keeps the
    // scalar path; neither case benefits from slicing/copying runs.
    if S::BOUNDED || value.len() <= if ATTRIBUTE { 1 } else { MIN_HTML_ESCAPE_RUN } {
        escape_html_scalar::<S, ATTRIBUTE>(value, out);
        return;
    }
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        let Some(escaped) = html_escape::<ATTRIBUTE>(ch) else {
            continue;
        };
        if index - start < MIN_HTML_ESCAPE_RUN {
            escape_html_scalar::<S, ATTRIBUTE>(&value[start..], out);
            return;
        }
        if start != index {
            out.unchanged_run(&value[start..index]);
        }
        out.push_str(escaped);
        start = index + ch.len_utf8();
    }
    if start != value.len() {
        out.unchanged_run(&value[start..]);
    }
}

fn serialize_cdata_section<S: HtmlSerializationSink, const EMBEDDED: bool>(
    value: &str,
    out: &mut HtmlEscapingWriter<'_, S, EMBEDDED>,
    raw_text_parent: bool,
    html_document: bool,
) {
    if html_document {
        if raw_text_parent {
            out.push_str(value);
        } else {
            out.text(value);
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

// There is one fixed writer. Depth is data, never another generic sink type.
struct HtmlEscapingWriter<'a, S, const EMBEDDED: bool> {
    sink: &'a mut S,
    attribute_depth: usize,
}

impl<S: HtmlSerializationSink, const EMBEDDED: bool> HtmlEscapingWriter<'_, S, EMBEDDED> {
    // Shared across traversal policies; ordinary text keeps its specialized writer.
    #[inline(never)]
    fn shadow_template_start(&mut self, root: &HtmlSerializedShadowRoot<'_>) {
        self.push_str("<template shadowrootmode=\"");
        self.attribute_value(root.init.mode());
        self.push('"');
        if root.init.delegates_focus() {
            self.push_str(" shadowrootdelegatesfocus=\"\"");
        }
        if root.init.serializable() {
            self.push_str(" shadowrootserializable=\"\"");
        }
        if root.web_api && root.init.slot_assignment() != "named" {
            self.push_str(" shadowrootslotassignment=\"");
            self.attribute_value(root.init.slot_assignment());
            self.push('"');
        }
        if root.init.clonable() {
            self.push_str(" shadowrootclonable=\"\"");
        }
        if root.registry_attribute {
            self.push_str(" shadowrootcustomelementregistry=\"\"");
        }
        if root.web_api
            && let Some(reference_target) = root.init.reference_target()
        {
            self.push_str(" shadowrootreferencetarget=\"");
            self.attribute_value(reference_target);
            self.push('"');
        }
        if root.web_api
            && root.include_style_sheets
            && let Some(sheets) = root.init.adopted_style_sheets()
        {
            self.push_str(" shadowrootadoptedstylesheets=\"");
            self.attribute_value(sheets);
            self.push('"');
        }
        self.push('>');
    }

    // Choose once per value. Ordinary text must not pay a frame-depth branch
    // for every character just because another node embeds a Document.
    #[inline]
    fn text(&mut self, value: &str) {
        if let [byte] = value.as_bytes() {
            match byte {
                b'&' => self.push_str("&amp;"),
                b'<' => self.push_str("&lt;"),
                b'>' => self.push_str("&gt;"),
                b'"' => self.push('"'),
                _ => self.sink.push(char::from(*byte)),
            }
            return;
        }
        if EMBEDDED && self.attribute_depth == 0 {
            escape_html_text(value, self.sink);
        } else {
            escape_html_text(value, self);
        }
    }

    #[inline]
    fn attribute(&mut self, attribute: &super::super::Attribute) {
        if EMBEDDED && self.attribute_depth == 0 {
            serialize_attribute(attribute, self.sink);
        } else {
            serialize_attribute(attribute, self);
        }
    }

    #[inline]
    fn start_tag(&mut self, name: &str) {
        if EMBEDDED && self.attribute_depth == 0 {
            self.sink.push('<');
            self.sink.push_str(name);
        } else {
            self.push('<');
            self.push_str(name);
        }
    }

    #[inline]
    fn close_tag(&mut self, name: &str) {
        if EMBEDDED && self.attribute_depth == 0 {
            self.sink.push_str("</");
            self.sink.push_str(name);
            self.sink.push('>');
        } else {
            self.push_str("</");
            self.push_str(name);
            self.push('>');
        }
    }

    #[inline(always)]
    fn scalar(&mut self, value: &str) {
        for ch in value.chars() {
            if self.limit_exceeded() {
                return;
            }
            self.push(ch);
        }
    }

    fn encoded_str<const ADAPTIVE: bool>(&mut self, value: &str) {
        if !EMBEDDED || self.attribute_depth == 0 {
            self.sink.push_str(value);
            return;
        }
        if S::BOUNDED {
            self.scalar(value);
            return;
        }
        let mut start = 0;
        for (index, ch) in value.char_indices() {
            if matches!(ch, '&' | '<' | '>' | '"' | '\u{a0}') {
                if ADAPTIVE && start != 0 && index - start < MIN_HTML_ESCAPE_RUN {
                    self.scalar(&value[start..]);
                    return;
                }
                if start != index {
                    self.sink.unchanged_run(&value[start..index]);
                }
                self.push(ch);
                start = index + ch.len_utf8();
            }
        }
        if start != value.len() {
            self.sink.unchanged_run(&value[start..]);
        }
    }
}

impl<S: HtmlSerializationSink, const EMBEDDED: bool> HtmlSerializationSink
    for HtmlEscapingWriter<'_, S, EMBEDDED>
{
    const BOUNDED: bool = S::BOUNDED;

    #[inline]
    fn unchanged_run(&mut self, value: &str) {
        if !EMBEDDED || self.attribute_depth == 0 {
            self.sink.unchanged_run(value);
        } else {
            self.push_str(value);
        }
    }

    #[inline]
    fn attribute_value(&mut self, value: &str) {
        if EMBEDDED && self.attribute_depth == 0 {
            escape_html_attribute(value, self.sink);
        } else if EMBEDDED {
            // An attribute value has one more encoding layer than surrounding
            // markup. Compose those layers instead of rescanning each entity.
            let mut writer = HtmlEscapingWriter::<_, true> {
                sink: &mut *self.sink,
                attribute_depth: self.attribute_depth + 1,
            };
            writer.encoded_str::<true>(value);
        } else {
            escape_html_attribute(value, self);
        }
    }
    fn push_str(&mut self, value: &str) {
        // Raw script/markup and already escaped entity tokens retain the
        // chunk writer. Only original attribute payloads use dense fallback.
        self.encoded_str::<false>(value);
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
    Siblings(NativeNodeId),
    Children(NativeNodeId),
    ShadowRootTemplate(HtmlSerializedShadowRoot<'a>),
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
    scripting_enabled_for_node: &'dom dyn Fn(NativeNodeId) -> bool,
    shadow_root_provider: Option<&'dom dyn HtmlShadowRootProvider<'dom>>,
    policy: Option<&'dom dyn HtmlSerializationPolicy>,
    filtering: bool,
    insertion: Option<HtmlSerializationInsertion<'dom>>,
    embedded_documents: Option<&'dom dyn HtmlEmbeddedDocumentProvider>,
    sole_embedded_owner: Option<NativeNodeId>,
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
                HtmlSerializationFrame::Node(id) => {
                    if let Some(node) = self.dom.node(id) {
                        self.node(id, node)?;
                    }
                }
                HtmlSerializationFrame::Siblings(id) => {
                    if let Some(node) = self.dom.node(id) {
                        if let Some(next) = node.next_sibling() {
                            self.stack.push(HtmlSerializationFrame::Siblings(next));
                        }
                        self.node(id, node)?;
                    }
                }
                HtmlSerializationFrame::Children(node) => {
                    let insertion = self.insertion.filter(|i| POLICY && i.node == node);
                    if let Some(insertion) = insertion
                        && insertion.point == HtmlInsertionPoint::LastChild
                    {
                        self.stack
                            .push(HtmlSerializationFrame::Markup(insertion.markup));
                    }
                    if let Some(first) = self
                        .dom
                        .html_serialization_children_root(node)
                        .and_then(|root| self.dom.first_child(root))
                    {
                        self.stack.push(HtmlSerializationFrame::Siblings(first));
                    }
                    if let Some(insertion) = insertion
                        && insertion.point == HtmlInsertionPoint::FirstChild
                    {
                        self.out.push_str(insertion.markup);
                    }
                }
                HtmlSerializationFrame::ShadowRootTemplate(root) => {
                    self.out.shadow_template_start(&root);
                    self.stack
                        .push(HtmlSerializationFrame::CloseShadowRootTemplate);
                    self.stack.push(HtmlSerializationFrame::Children(root.root));
                }
                HtmlSerializationFrame::CloseElement(name) => {
                    self.out.close_tag(name);
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
    fn node(&mut self, id: NativeNodeId, node: &'dom Node) -> Result<(), HtmlSerializationError> {
        if let Some(insertion) = self.insertion
            && POLICY
            && insertion.node == id
            && insertion.point == HtmlInsertionPoint::BeforeNode
        {
            self.out.push_str(insertion.markup);
        }
        if self
            .policy
            .is_some_and(|p| POLICY && self.filtering && !p.include_node(node))
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
                    .policy
                    .filter(|_| POLICY)
                    .map_or(&[][..], |p| p.attribute_overrides(id));
                self.out.start_tag(element.local_name());
                if let Some(name) = element.custom_element_is_name()
                    && !element.has_attribute("is")
                    && !overrides
                        .iter()
                        .any(|a| a.namespace().is_empty() && a.local_name() == "is")
                {
                    self.out.push_str(" is=\"");
                    self.out.attribute_value(name);
                    self.out.push('"');
                }
                // Filtering happens before lookup; excluded ancestors never
                // consult frame/shadow providers or build derived child output.
                if let Some(embedded) = self
                    .embedded_documents
                    .filter(|_| EMBEDDED)
                    .filter(|_| self.sole_embedded_owner.is_none_or(|owner| owner == id))
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
                    let filter = self.policy.filter(|_| POLICY && self.filtering);
                    if overrides.is_empty() && filter.is_none() {
                        for attribute in element.attributes() {
                            if self.out.limit_exceeded() {
                                return Ok(());
                            }
                            self.out.attribute(attribute);
                        }
                    } else if !element.attributes().is_empty() || !overrides.is_empty() {
                        let attributes =
                            EffectiveAttributes::new(id, element.attributes(), overrides, filter);
                        let mut index = 0;
                        loop {
                            if self.out.limit_exceeded() {
                                return Ok(());
                            }
                            let Some((attribute, _)) = attributes.next::<false>(&mut index) else {
                                break;
                            };
                            self.out.attribute(attribute);
                        }
                    }
                    self.end_open_element(id, element);
                }
            }
            NodeData::Text(text) => {
                if text_data_serializes_literally(self.dom, id, self.scripting_enabled_for_node) {
                    self.out.push_str(text.data());
                } else {
                    self.out.text(text.data());
                }
            }
            NodeData::CDataSection(cdata) => serialize_cdata_section(
                cdata.data(),
                &mut self.out,
                text_data_serializes_literally(self.dom, id, self.scripting_enabled_for_node),
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
    fn end_open_element(
        &mut self,
        id: NativeNodeId,
        element: &'dom super::super::element::Element,
    ) {
        self.out.push('>');
        if !is_void_html_element(element.namespace(), element.local_name()) {
            self.stack
                .push(HtmlSerializationFrame::CloseElement(element.local_name()));
            self.stack.push(HtmlSerializationFrame::Children(id));
            self.shadow_root(id);
        }
    }

    fn shadow_root(&mut self, host: NativeNodeId) {
        if self.out.limit_exceeded() {
            return;
        }
        if let Some(root) = self
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
            .policy
            .filter(|_| POLICY)
            .map_or(&[][..], |p| p.attribute_overrides(state.owner));
        let filter = self.policy.filter(|_| POLICY && self.filtering);
        let attributes =
            EffectiveAttributes::new(state.owner, element.attributes(), overrides, filter);
        loop {
            if self.out.limit_exceeded() {
                return Ok(());
            }
            let Some((attribute, embedded)) = attributes.next::<true>(&mut state.next_attribute)
            else {
                break;
            };
            match embedded {
                Some(EmbeddedAttribute::Srcdoc) => {
                    state.wrote_srcdoc = true;
                    self.start_embedded(state)?;
                    return Ok(());
                }
                Some(EmbeddedAttribute::FrameUrl) => {
                    state.wrote_url = true;
                    self.out.push_str(" data-moli-frame-url=\"");
                    self.out.attribute_value(&state.embedded.url);
                    self.out.push('"');
                }
                None => self.out.attribute(attribute),
            }
        }
        if !state.wrote_srcdoc {
            state.wrote_srcdoc = true;
            self.start_embedded(state)?;
            return Ok(());
        }
        if !state.wrote_url {
            self.out.push_str(" data-moli-frame-url=\"");
            self.out.attribute_value(&state.embedded.url);
            self.out.push('"');
        }
        self.end_open_element(state.owner, element);
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

#[inline(always)]
fn serialize_attribute<S: HtmlSerializationSink>(attribute: &super::super::Attribute, out: &mut S) {
    out.push(' ');
    attribute.push_html_serialized_name(|part| out.push_str(part));
    out.push_str("=\"");
    out.attribute_value(attribute.value());
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
        scripting_enabled_for_node: options.scripting_enabled_for_node,
        shadow_root_provider: options.shadow_root_provider,
        policy: options.policy,
        filtering: POLICY && options.policy.is_some_and(|p| p.has_filters()),
        insertion: options
            .policy
            .filter(|_| POLICY)
            .and_then(|p| p.insertion()),
        embedded_documents: options.embedded_documents,
        sole_embedded_owner: options.embedded_documents.and_then(|p| p.sole_owner()),
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
    const BOUNDED: bool = true;

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

pub(in crate::native) fn serialize_html_with_options<'a>(
    dom: &'a NativeDom,
    node_id: NativeNodeId,
    options: HtmlSerializationOptions<'a>,
) -> Result<Option<String>, HtmlSerializationError> {
    if let Some(limit) = options.max_bytes {
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
    fn text_and_attribute_runs_match_replacement_reference_and_byte_limits() {
        let mut values = vec![
            String::new(),
            String::from("&<>\"'\u{a0}雪🙂\0\r\n"),
            (0..=127).map(char::from).collect(),
            format!(
                "{}\u{a0}{}&雪<🙂>\"tail",
                "plain ".repeat(1024),
                "é".repeat(1024)
            ),
        ];
        let mut seed = 1_u64;
        for _ in 0..128 {
            let mut value = String::from("&<>\"\u{a0}");
            for _ in 0..64 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                if let Some(ch) = char::from_u32((seed >> 32) as u32 % 0x110000) {
                    value.push(ch);
                }
            }
            values.push(value);
        }
        for value in values {
            let text = value
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\u{a0}', "&nbsp;");
            let attribute = text.replace('"', "&quot;");
            for (is_attribute, expected) in [(false, text), (true, attribute)] {
                let mut actual = String::new();
                if is_attribute {
                    escape_html_attribute(&value, &mut actual);
                } else {
                    escape_html_text(&value, &mut actual);
                }
                assert_eq!(actual, expected);
                for limit in [0, 1, expected.len().saturating_sub(1), expected.len()] {
                    let mut bounded = BoundedHtmlSerialization::new(limit);
                    if is_attribute {
                        escape_html_attribute(&value, &mut bounded);
                    } else {
                        escape_html_text(&value, &mut bounded);
                    }
                    if expected.len() > limit {
                        assert_eq!(
                            bounded.finish(),
                            Err(HtmlSerializationLimitExceeded { max_bytes: limit })
                        );
                    } else {
                        assert_eq!(bounded.finish().unwrap(), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn single_byte_text_keeps_text_and_nested_attribute_escaping() {
        for byte in 0..=127 {
            let value = char::from(byte).to_string();
            let mut expected = String::new();
            escape_html_text(&value, &mut expected);
            let mut plain = String::new();
            HtmlEscapingWriter::<_, false> {
                sink: &mut plain,
                attribute_depth: 0,
            }
            .text(&value);
            assert_eq!(plain, expected, "ordinary byte={byte}");
            for depth in 0..8 {
                let mut output = String::new();
                HtmlEscapingWriter::<_, true> {
                    sink: &mut output,
                    attribute_depth: depth,
                }
                .text(&value);
                assert_eq!(output, expected, "byte={byte}, depth={depth}");
                let mut next = String::new();
                escape_html_attribute(&expected, &mut next);
                expected = next;
            }
        }
    }

    #[test]
    fn value_escaping_preserves_rules_and_limits_at_each_embedding_depth() {
        let value = "prefix<&>\"\u{a0}雪🙂tail";
        let mut expected_text = "prefix&lt;&amp;&gt;\"&nbsp;雪🙂tail".to_owned();
        let mut expected_attribute = "prefix&lt;&amp;&gt;&quot;&nbsp;雪🙂tail".to_owned();
        for depth in 0..8 {
            let expected = format!("{expected_text}{expected_attribute}");
            let mut output = String::new();
            let mut writer = HtmlEscapingWriter::<_, true> {
                sink: &mut output,
                attribute_depth: depth,
            };
            writer.text(value);
            writer.attribute_value(value);
            assert_eq!(output, expected, "depth={depth}");
            for limit in [0, expected.len() - 1, expected.len()] {
                let mut output = BoundedHtmlSerialization::new(limit);
                let mut writer = HtmlEscapingWriter::<_, true> {
                    sink: &mut output,
                    attribute_depth: depth,
                };
                writer.text(value);
                writer.attribute_value(value);
                if limit < expected.len() {
                    assert_eq!(
                        output.finish(),
                        Err(HtmlSerializationLimitExceeded { max_bytes: limit })
                    );
                } else {
                    assert_eq!(output.finish().unwrap(), expected);
                }
            }
            for value in [&mut expected_text, &mut expected_attribute] {
                *value = value
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
                    .replace('\u{a0}', "&nbsp;");
            }
        }
    }

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
                let mut expected_attribute = String::new();
                escape_html_attribute(&expected, &mut expected_attribute);
                output.clear();
                HtmlEscapingWriter::<_, true> {
                    sink: &mut output,
                    attribute_depth: depth,
                }
                .attribute_value(&input);
                assert_eq!(output, expected_attribute);
            }
        }
    }
}
