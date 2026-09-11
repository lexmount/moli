use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::HashMap,
    ops::{Deref, DerefMut},
    rc::Rc,
};

use url::Url;
use xml5ever::{
    Attribute as XmlAttribute, ExpandedName as XmlExpandedName, QualName as XmlQualName,
    driver::{XmlParseOpts, parse_document as parse_xml_document},
    interface::{ElementFlags as XmlElementFlags, QuirksMode as XmlQuirksMode},
    tendril::{StrTendril as XmlStrTendril, TendrilSink as XmlTendrilSink},
    tree_builder::{NodeOrText as XmlNodeOrText, TreeSink as XmlTreeSinkBase, XmlTreeSink},
};
use xmlparser::{
    ElementEnd as XmlElementEnd, Token as XmlTokenizerToken, Tokenizer as XmlTokenizer,
};

use super::{
    html_chunks, xml_preprocess::prepare_xml_for_xml5ever,
    xml_tree_viewer::transform_document_to_xml_tree_view,
};
use moli_dom::native::{Attribute as NativeAttribute, DomHost, NativeDom, NativeNodeId, Node};

#[derive(Debug, Clone, Default)]
pub struct XmlParser;

struct XmlPrepassError {
    detail: String,
    discard_partial_tree: bool,
}

impl XmlPrepassError {
    fn tree(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            discard_partial_tree: false,
        }
    }

    fn before_document_element(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            discard_partial_tree: true,
        }
    }

    fn namespace(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            discard_partial_tree: true,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct XmlParseHandle {
    pub(super) node_id: NativeNodeId,
    pub(super) element_name: Option<Rc<XmlQualName>>,
}

struct XmlDocumentSink<'a> {
    target: RefCell<XmlLiveTreeSinkTarget<'a>>,
    quirks_mode: Cell<XmlQuirksMode>,
}

enum XmlDomHost<'a> {
    Owned(Box<DomHost>),
    Borrowed(&'a mut DomHost),
}

impl Deref for XmlDomHost<'_> {
    type Target = DomHost;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(dom_host) => dom_host.as_ref(),
            Self::Borrowed(dom_host) => dom_host,
        }
    }
}

impl DerefMut for XmlDomHost<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Owned(dom_host) => dom_host.as_mut(),
            Self::Borrowed(dom_host) => dom_host,
        }
    }
}

struct XmlLiveTreeSinkTarget<'a> {
    dom_host: XmlDomHost<'a>,
    document_handle: NativeNodeId,
}

impl XmlParser {
    pub fn parse(&self, final_url: Url, xml: String) -> NativeDom {
        self.parse_with_presentation(final_url, xml, false)
    }

    /// Parses a top-level XML navigation and applies Chromium's source-tree
    /// presentation when the document has no associated style information.
    /// DOMParser and child-document callers must use the raw parser entry point.
    pub fn parse_top_level_document(&self, final_url: Url, xml: String) -> NativeDom {
        self.parse_with_presentation(final_url, xml, true)
    }

    fn parse_with_presentation(
        &self,
        final_url: Url,
        xml: String,
        present_unstyled_xml: bool,
    ) -> NativeDom {
        let xml = prepare_xml_for_xml5ever(&xml);
        let prepass_error = xml_element_stack_error(&xml);
        let sink = XmlDocumentSink::new(XmlLiveTreeSinkTarget::new_owned(final_url));
        let mut parser = parse_xml_document(sink, XmlParseOpts::default());
        for chunk in html_chunks(&xml) {
            parser.process(XmlStrTendril::from(chunk));
        }
        let mut document = parser.finish().finish_document();
        // xml5ever recovers from some mismatched element stacks. DOMParser must
        // expose an XML parsererror document instead of that recovered tree.
        if let Some(error) = prepass_error {
            document.push_parse_error(error.detail);
            if error.discard_partial_tree {
                let mut host = DomHost::from_dom(document);
                let document_handle = host.document_handle();
                for child in host.child_handles(document_handle).collect::<Vec<_>>() {
                    let _ = host.remove_child(document_handle, child);
                }
                document = host.snapshot_document();
            }
        }
        if present_unstyled_xml {
            document = transform_document_to_xml_tree_view(document);
        }
        document
    }

    /// Parses an inert XML tree directly into an empty XML Document that
    /// already belongs to `dom_host`.
    ///
    /// The mutable borrow is retained by the tree sink for exactly this
    /// synchronous parser call. Script elements remain inert; executable
    /// top-level and child Documents must use `XmlDocumentStream` instead.
    pub fn parse_inert_tree_into_document(
        &self,
        dom_host: &mut DomHost,
        document_handle: NativeNodeId,
        xml: &str,
    ) -> Option<()> {
        let target = XmlLiveTreeSinkTarget::new_borrowed(dom_host, document_handle)?;
        let sink = XmlDocumentSink::new(target);
        let mut parser = parse_xml_document(sink, XmlParseOpts::default());
        let xml = prepare_xml_for_xml5ever(xml);
        for chunk in html_chunks(&xml) {
            parser.process(XmlStrTendril::from(chunk));
        }
        parser.finish().finish_live_tree();
        Some(())
    }
}

fn xml_element_stack_error(xml: &str) -> Option<XmlPrepassError> {
    let mut pending_start_name = None;
    let mut pending_used_prefixes = Vec::new();
    let mut pending_declared_prefixes = Vec::new();
    let mut open_elements = Vec::new();
    let mut active_prefixes = HashMap::from([("xml".to_owned(), 1usize)]);
    let mut saw_document_element = false;

    for token in XmlTokenizer::from(xml) {
        match token {
            Ok(XmlTokenizerToken::ElementStart { prefix, local, .. }) => {
                saw_document_element = true;
                let prefix = prefix.as_str();
                if !prefix.is_empty() {
                    pending_used_prefixes.push(prefix.to_owned());
                }
                pending_start_name = Some(qualified_xml_name(prefix, local.as_str()));
            }
            Ok(XmlTokenizerToken::Attribute {
                prefix,
                local,
                value,
                ..
            }) => {
                let prefix = prefix.as_str();
                if prefix == "xmlns" {
                    if !value.as_str().is_empty() {
                        pending_declared_prefixes.push(local.as_str().to_owned());
                    }
                } else if !prefix.is_empty() {
                    pending_used_prefixes.push(prefix.to_owned());
                }
            }
            Ok(XmlTokenizerToken::ElementEnd { end, .. }) => match end {
                XmlElementEnd::Open => {
                    let Some(name) = pending_start_name.take() else {
                        return Some(XmlPrepassError::tree("XML start tag is missing a name"));
                    };
                    for prefix in &pending_declared_prefixes {
                        *active_prefixes.entry(prefix.clone()).or_default() += 1;
                    }
                    if let Some(prefix) = pending_used_prefixes
                        .iter()
                        .find(|prefix| !active_prefixes.contains_key(*prefix))
                    {
                        return Some(XmlPrepassError::namespace(format!(
                            "XML namespace prefix `{prefix}` is not declared"
                        )));
                    }
                    pending_used_prefixes.clear();
                    open_elements.push((name, std::mem::take(&mut pending_declared_prefixes)));
                }
                XmlElementEnd::Empty => {
                    if pending_start_name.take().is_none() {
                        return Some(XmlPrepassError::tree(
                            "XML empty-element tag is missing a name",
                        ));
                    }
                    for prefix in &pending_declared_prefixes {
                        *active_prefixes.entry(prefix.clone()).or_default() += 1;
                    }
                    if let Some(prefix) = pending_used_prefixes
                        .iter()
                        .find(|prefix| !active_prefixes.contains_key(*prefix))
                    {
                        return Some(XmlPrepassError::namespace(format!(
                            "XML namespace prefix `{prefix}` is not declared"
                        )));
                    }
                    pending_used_prefixes.clear();
                    for prefix in pending_declared_prefixes.drain(..) {
                        decrement_xml_prefix_count(&mut active_prefixes, &prefix);
                    }
                }
                XmlElementEnd::Close(prefix, local) => {
                    let closing_name = qualified_xml_name(prefix.as_str(), local.as_str());
                    let Some((open_name, declared_prefixes)) = open_elements.pop() else {
                        return Some(XmlPrepassError::tree(format!(
                            "XML end tag </{closing_name}> has no matching start tag"
                        )));
                    };
                    if open_name != closing_name {
                        return Some(XmlPrepassError::tree(format!(
                            "XML end tag </{closing_name}> does not match <{open_name}>"
                        )));
                    }
                    for prefix in declared_prefixes {
                        decrement_xml_prefix_count(&mut active_prefixes, &prefix);
                    }
                }
            },
            Ok(_) => {}
            Err(error) => {
                let detail = error.to_string();
                return Some(if saw_document_element {
                    XmlPrepassError::tree(detail)
                } else {
                    // xml5ever can recover a malformed prologue or doctype and
                    // then expose nodes parsed after the error. DOMParser error
                    // documents must not retain that recovered document tree.
                    XmlPrepassError::before_document_element(detail)
                });
            }
        }
    }

    if let Some(name) = pending_start_name {
        Some(XmlPrepassError::tree(format!(
            "XML start tag <{name}> is incomplete"
        )))
    } else {
        open_elements
            .last()
            .map(|(name, _)| XmlPrepassError::tree(format!("XML element <{name}> is not closed")))
    }
}

fn decrement_xml_prefix_count(active_prefixes: &mut HashMap<String, usize>, prefix: &str) {
    let Some(count) = active_prefixes.get_mut(prefix) else {
        return;
    };
    *count -= 1;
    if *count == 0 {
        active_prefixes.remove(prefix);
    }
}

fn qualified_xml_name(prefix: &str, local_name: &str) -> String {
    if prefix.is_empty() {
        local_name.to_owned()
    } else {
        format!("{prefix}:{local_name}")
    }
}

impl XmlParseHandle {
    pub(super) fn new(node_id: NativeNodeId, element_name: Option<Rc<XmlQualName>>) -> Self {
        Self {
            node_id,
            element_name,
        }
    }
}

impl PartialEq for XmlParseHandle {
    fn eq(&self, other: &Self) -> bool {
        self.node_id == other.node_id
    }
}

impl Eq for XmlParseHandle {}

impl<'a> XmlDocumentSink<'a> {
    fn new(target: XmlLiveTreeSinkTarget<'a>) -> Self {
        Self {
            target: RefCell::new(target),
            quirks_mode: Cell::new(XmlQuirksMode::NoQuirks),
        }
    }
}

impl XmlLiveTreeSinkTarget<'static> {
    fn new_owned(final_url: Url) -> Self {
        let dom_host = DomHost::from_dom(NativeDom::new_xml(final_url));
        let document_handle = dom_host.document_handle();
        Self {
            dom_host: XmlDomHost::Owned(Box::new(dom_host)),
            document_handle,
        }
    }

    fn finish_document(self) -> NativeDom {
        let XmlDomHost::Owned(dom_host) = self.dom_host else {
            unreachable!("owned XML parsing must finish with an owned DOM host")
        };
        dom_host.snapshot_document()
    }
}

impl<'a> XmlLiveTreeSinkTarget<'a> {
    fn new_borrowed(dom_host: &'a mut DomHost, document_handle: NativeNodeId) -> Option<Self> {
        let is_empty_xml_document = dom_host
            .node(document_handle)
            .and_then(Node::as_document)
            .is_some_and(|document| !document.is_html_document())
            && dom_host.child_handles(document_handle).next().is_none();
        if !is_empty_xml_document {
            return None;
        }
        Some(Self {
            dom_host: XmlDomHost::Borrowed(dom_host),
            document_handle,
        })
    }

    fn finish_live_tree(self) {}

    fn document_handle(&self) -> XmlParseHandle {
        XmlParseHandle::new(self.document_handle, None)
    }

    fn create_element(&mut self, name: XmlQualName, attrs: Vec<XmlAttribute>) -> XmlParseHandle {
        let element_name = Rc::new(name.clone());
        let attributes = attrs
            .into_iter()
            .map(|attribute| {
                NativeAttribute::new(
                    attribute.name.local.to_string(),
                    attribute.name.ns.to_string(),
                    attribute
                        .name
                        .prefix
                        .as_ref()
                        .map(|prefix| prefix.to_string()),
                    attribute.value.to_string(),
                )
            })
            .collect::<Vec<_>>();
        let node_id = self
            .dom_host
            .create_parser_element_without_attributes_for_document(
                self.document_handle,
                name.local.to_string(),
                name.ns.to_string(),
                name.prefix.as_ref().map(|prefix| prefix.to_string()),
            );
        self.dom_host
            .add_attrs_if_missing_for_parser(node_id, attributes);
        XmlParseHandle::new(node_id, Some(element_name))
    }

    fn create_comment(&mut self, text: String) -> XmlParseHandle {
        XmlParseHandle::new(
            self.dom_host
                .create_comment_for_document(self.document_handle, &text),
            None,
        )
    }

    fn create_cdata(&mut self, text: String) -> XmlParseHandle {
        XmlParseHandle::new(
            self.dom_host
                .create_cdata_section_for_document(self.document_handle, &text),
            None,
        )
    }

    fn create_processing_instruction(&mut self, target: String, data: String) -> XmlParseHandle {
        XmlParseHandle::new(
            self.dom_host.create_processing_instruction_for_document(
                self.document_handle,
                &target,
                &data,
            ),
            None,
        )
    }

    fn append_text(&mut self, parent_id: NativeNodeId, text: String) {
        if parent_id == self.document_handle && text.trim().is_empty() {
            return;
        }
        self.insert_text_node(parent_id, None, text);
    }

    fn insert_text_node(
        &mut self,
        parent_id: NativeNodeId,
        reference_child: Option<NativeNodeId>,
        text: String,
    ) {
        if parent_id == self.document_handle && text.trim().is_empty() {
            return;
        }
        if text.is_empty() || self.dom_host.node(parent_id).is_none() {
            return;
        }

        if let Some(reference_child) = reference_child {
            if self
                .dom_host
                .node(reference_child)
                .and_then(Node::as_text)
                .is_some()
            {
                if let Some(text_node) = self
                    .dom_host
                    .node_mut(reference_child)
                    .and_then(|node| node.data_mut().as_text_mut())
                {
                    let mut merged = text;
                    merged.push_str(text_node.data());
                    text_node.set_data(merged);
                }
                return;
            }

            if let Some(previous) = self
                .dom_host
                .node(reference_child)
                .and_then(Node::prev_sibling)
                && self
                    .dom_host
                    .node(previous)
                    .and_then(Node::as_text)
                    .is_some()
            {
                if let Some(text_node) = self
                    .dom_host
                    .node_mut(previous)
                    .and_then(|node| node.data_mut().as_text_mut())
                {
                    let mut merged = text_node.data().to_owned();
                    merged.push_str(&text);
                    text_node.set_data(merged);
                }
                return;
            }
        } else if let Some(last_child) = self.dom_host.node(parent_id).and_then(Node::last_child)
            && self
                .dom_host
                .node(last_child)
                .and_then(Node::as_text)
                .is_some()
        {
            if let Some(text_node) = self
                .dom_host
                .node_mut(last_child)
                .and_then(|node| node.data_mut().as_text_mut())
            {
                let mut merged = text_node.data().to_owned();
                merged.push_str(&text);
                text_node.set_data(merged);
            }
            return;
        }

        let text_node = self
            .dom_host
            .create_text_node_for_document(self.document_handle, &text);
        if let Some(reference_child) = reference_child {
            let Some(parent) = self
                .dom_host
                .node(reference_child)
                .and_then(Node::parent_node)
            else {
                return;
            };
            let _ = self
                .dom_host
                .insert_before(parent, text_node, Some(reference_child));
        } else {
            let _ = self.dom_host.append_child(parent_id, text_node);
        }
    }

    fn append(&mut self, parent_id: NativeNodeId, child: XmlNodeOrText<XmlParseHandle>) {
        let parent_id = self.template_contents_id(parent_id).unwrap_or(parent_id);
        match child {
            XmlNodeOrText::AppendNode(handle) => {
                if self.is_xml_declaration_processing_instruction(handle.node_id) {
                    return;
                }
                let _ = self.dom_host.append_child(parent_id, handle.node_id);
            }
            XmlNodeOrText::AppendText(text) => self.append_text(parent_id, text.to_string()),
        }
    }

    fn append_before_sibling(
        &mut self,
        sibling_id: NativeNodeId,
        child: XmlNodeOrText<XmlParseHandle>,
    ) {
        let Some(parent_id) = self.dom_host.node(sibling_id).and_then(Node::parent_node) else {
            return;
        };
        match child {
            XmlNodeOrText::AppendNode(handle) => {
                if self.is_xml_declaration_processing_instruction(handle.node_id) {
                    return;
                }
                let _ = self
                    .dom_host
                    .insert_before(parent_id, handle.node_id, Some(sibling_id));
            }
            XmlNodeOrText::AppendText(text) => {
                self.insert_text_node(parent_id, Some(sibling_id), text.to_string());
            }
        }
    }

    fn append_based_on_parent_node(
        &mut self,
        element_id: NativeNodeId,
        prev_element_id: NativeNodeId,
        child: XmlNodeOrText<XmlParseHandle>,
    ) {
        if self
            .dom_host
            .node(element_id)
            .and_then(Node::parent_node)
            .is_some()
        {
            self.append_before_sibling(element_id, child);
        } else {
            self.append(prev_element_id, child);
        }
    }

    fn append_doctype(&mut self, name: String, public_id: String, system_id: String) {
        let doctype = self.dom_host.create_document_type_for_document(
            self.document_handle,
            &name,
            &public_id,
            &system_id,
        );
        let _ = self.dom_host.append_child(self.document_handle, doctype);
    }

    fn is_xml_declaration_processing_instruction(&self, node_id: NativeNodeId) -> bool {
        self.dom_host
            .node(node_id)
            .and_then(Node::as_processing_instruction)
            .is_some_and(|pi| pi.target().eq_ignore_ascii_case("xml"))
    }

    fn template_contents_handle(&self, node_id: NativeNodeId) -> Option<XmlParseHandle> {
        self.dom_host
            .parser_template_contents_handle(node_id)
            .map(|handle| XmlParseHandle::new(handle, None))
    }

    fn template_contents_id(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.dom_host.parser_template_contents_handle(node_id)
    }

    fn add_attrs_if_missing(&mut self, node_id: NativeNodeId, attrs: Vec<XmlAttribute>) {
        let Some(node) = self.dom_host.node_mut(node_id) else {
            return;
        };
        let Some(element) = node.data_mut().as_element_mut() else {
            return;
        };

        for attribute in attrs {
            let already_exists = element.attributes().iter().any(|existing| {
                existing.namespace() == attribute.name.ns.as_ref()
                    && existing.local_name() == attribute.name.local.as_ref()
                    && existing.prefix()
                        == attribute.name.prefix.as_ref().map(|prefix| prefix.as_ref())
            });

            if !already_exists {
                element.set_attribute(
                    attribute.name.local.to_string(),
                    attribute.name.ns.to_string(),
                    attribute.name.prefix.map(|prefix| prefix.to_string()),
                    attribute.value.to_string(),
                );
            }
        }
    }

    fn remove_from_parent(&mut self, node_id: NativeNodeId) {
        let Some(parent_id) = self.dom_host.node(node_id).and_then(Node::parent_node) else {
            return;
        };
        let _ = self.dom_host.remove_child(parent_id, node_id);
    }

    fn reparent_children(&mut self, node_id: NativeNodeId, new_parent_id: NativeNodeId) {
        let child_ids = self.dom_host.child_handles(node_id).collect::<Vec<_>>();
        for child_id in child_ids {
            let _ = self.dom_host.append_child(new_parent_id, child_id);
        }
    }
}

impl<'host> XmlTreeSinkBase for XmlDocumentSink<'host> {
    type Handle = XmlParseHandle;
    type Output = XmlLiveTreeSinkTarget<'host>;
    type ElemName<'a>
        = XmlExpandedName<'a>
    where
        Self: 'a;

    fn finish(self) -> Self::Output {
        self.target.into_inner()
    }

    fn parse_error(&self, err: Cow<'static, str>) {
        self.target
            .borrow_mut()
            .dom_host
            .push_parse_error(err.into_owned());
    }

    fn get_document(&self) -> Self::Handle {
        self.target.borrow().document_handle()
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        target
            .element_name
            .as_deref()
            .expect("xml5ever requested the name of a non-element node")
            .expanded()
    }

    fn create_element(
        &self,
        name: XmlQualName,
        attrs: Vec<XmlAttribute>,
        _flags: XmlElementFlags,
    ) -> Self::Handle {
        self.target.borrow_mut().create_element(name, attrs)
    }

    fn create_comment(&self, text: XmlStrTendril) -> Self::Handle {
        self.target.borrow_mut().create_comment(text.to_string())
    }

    fn create_pi(&self, target: XmlStrTendril, data: XmlStrTendril) -> Self::Handle {
        self.target
            .borrow_mut()
            .create_processing_instruction(target.to_string(), data.to_string())
    }

    fn append(&self, parent: &Self::Handle, child: XmlNodeOrText<Self::Handle>) {
        self.target.borrow_mut().append(parent.node_id, child);
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, child: XmlNodeOrText<Self::Handle>) {
        self.target
            .borrow_mut()
            .append_before_sibling(sibling.node_id, child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: XmlNodeOrText<Self::Handle>,
    ) {
        self.target.borrow_mut().append_based_on_parent_node(
            element.node_id,
            prev_element.node_id,
            child,
        );
    }

    fn append_doctype_to_document(
        &self,
        name: XmlStrTendril,
        public_id: XmlStrTendril,
        system_id: XmlStrTendril,
    ) {
        self.target.borrow_mut().append_doctype(
            name.to_string(),
            public_id.to_string(),
            system_id.to_string(),
        );
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        self.target
            .borrow()
            .template_contents_handle(target.node_id)
            .unwrap_or_else(|| target.clone())
    }

    fn same_node(&self, left: &Self::Handle, right: &Self::Handle) -> bool {
        left == right
    }

    fn set_quirks_mode(&self, mode: XmlQuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<XmlAttribute>) {
        self.target
            .borrow_mut()
            .add_attrs_if_missing(target.node_id, attrs);
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        self.target.borrow_mut().remove_from_parent(target.node_id);
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        self.target
            .borrow_mut()
            .reparent_children(node.node_id, new_parent.node_id);
    }
}

impl XmlTreeSink for XmlDocumentSink<'_> {
    fn create_cdata(&self, text: XmlStrTendril) -> Self::Handle {
        self.target.borrow_mut().create_cdata(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::XmlParser;
    use moli_dom::native::{DomHost, NativeDom, NativeNodeId, Node, NodeType};
    use url::Url;

    #[test]
    fn xml_parser_records_unclosed_and_mismatched_elements() {
        for (index, source) in [
            "<root>",
            "<root><child></root>",
            "<root></other>",
            "</root>",
        ]
        .into_iter()
        .enumerate()
        {
            let dom = XmlParser.parse(
                Url::parse(&format!("https://example.test/invalid-{index}.xml")).unwrap(),
                source.to_owned(),
            );
            assert!(
                !dom.parse_errors().is_empty(),
                "source `{source}` should be rejected"
            );
        }

        for (index, source) in ["<root/>", "<root><child/></root>"].into_iter().enumerate() {
            let dom = XmlParser.parse(
                Url::parse(&format!("https://example.test/valid-{index}.xml")).unwrap(),
                source.to_owned(),
            );
            assert!(
                dom.parse_errors().is_empty(),
                "source `{source}`: {:?}",
                dom.parse_errors()
            );
        }

        let declared = XmlParser.parse(
            Url::parse("https://example.test/declared-prefix.xml").unwrap(),
            "<x:root xmlns:x='urn:test' x:value='ok'/>".to_owned(),
        );
        assert!(declared.parse_errors().is_empty());

        let undeclared = XmlParser.parse(
            Url::parse("https://example.test/undeclared-prefix.xml").unwrap(),
            "<root x:value='invalid'/>".to_owned(),
        );
        assert!(!undeclared.parse_errors().is_empty());
        assert!(
            undeclared
                .child_ids(undeclared.document_node_id())
                .all(|child| !undeclared.node(child).is_some_and(Node::is_element))
        );
    }

    #[test]
    fn xml_parser_discards_recovered_tree_after_doctype_error() {
        let url = Url::parse("https://example.test/doctype.xml").unwrap();
        let invalid = XmlParser.parse(
            url.clone(),
            concat!(
                "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\">",
                "<html><div id=\"test\"/></html>"
            )
            .to_owned(),
        );
        assert!(!invalid.parse_errors().is_empty());
        assert!(
            invalid
                .child_ids(invalid.document_node_id())
                .all(|child| !invalid.node(child).is_some_and(Node::is_element))
        );

        for system_id in ["", "x"] {
            let valid = XmlParser.parse(
                url.clone(),
                format!(
                    "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"{system_id}\"><html/>"
                ),
            );
            assert!(
                valid.parse_errors().is_empty(),
                "system ID `{system_id}` should be accepted: {:?}",
                valid.parse_errors()
            );
        }
    }

    #[test]
    fn xml_parser_preserves_namespace_declarations_as_dom_attributes() {
        let dom = XmlParser.parse(
            Url::parse("https://example.test/catalog.xml").unwrap(),
            concat!(
                "<catalog data-before='1' xmlns='urn:catalog&amp;items' ",
                "data-middle='2' xmlns:m='urn:meta' data-after='3'>",
                "<m:item xmlns='urn:nested'/>",
                "</catalog>"
            )
            .to_owned(),
        );
        let root = document_element(&dom);
        let root_attributes = dom
            .node(root)
            .and_then(Node::as_element)
            .expect("catalog element")
            .attributes();

        assert_eq!(
            root_attributes
                .iter()
                .map(|attribute| attribute.name())
                .collect::<Vec<_>>(),
            [
                "data-before",
                "xmlns",
                "data-middle",
                "xmlns:m",
                "data-after"
            ]
        );
        assert_eq!(
            root_attributes[1].namespace(),
            "http://www.w3.org/2000/xmlns/"
        );
        assert_eq!(root_attributes[1].prefix(), None);
        assert_eq!(root_attributes[1].local_name(), "xmlns");
        assert_eq!(root_attributes[1].value(), "urn:catalog&items");
        assert_eq!(
            root_attributes[3].namespace(),
            "http://www.w3.org/2000/xmlns/"
        );
        assert_eq!(root_attributes[3].prefix(), Some("xmlns"));
        assert_eq!(root_attributes[3].local_name(), "m");
        assert_eq!(root_attributes[3].value(), "urn:meta");

        let child = dom
            .child_ids(root)
            .find(|node_id| dom.node(*node_id).is_some_and(Node::is_element))
            .expect("nested item element");
        let child_attributes = dom
            .node(child)
            .and_then(Node::as_element)
            .expect("nested item")
            .attributes();
        assert_eq!(child_attributes.len(), 1);
        assert_eq!(child_attributes[0].name(), "xmlns");
        assert_eq!(child_attributes[0].value(), "urn:nested");
    }

    #[test]
    fn xml_parser_populates_an_existing_xml_document_with_exact_node_owners() {
        let parser = XmlParser;
        let mut host = DomHost::from_dom(NativeDom::new_html(
            Url::parse("https://parent.example.test/").unwrap(),
        ));
        let parent_document = host.document_handle();
        let child_url = Url::parse("https://child.example.test/document.xml").unwrap();
        let child_document = host.create_detached_xml_document_with_url(child_url.clone());

        parser
            .parse_inert_tree_into_document(
                &mut host,
                child_document,
                concat!(
                    "<?before data?>",
                    "<!DOCTYPE root>",
                    "<root xmlns:h='http://www.w3.org/1999/xhtml'>",
                    "<child attr='value'>text<![CDATA[cdata]]><!--comment--></child>",
                    "<h:script />",
                    "</root>"
                ),
            )
            .expect("empty XML document accepts a direct inert parse");

        assert_eq!(host.child_handles(parent_document).count(), 0);
        let mut descendants = Vec::new();
        collect_descendants(&host, child_document, &mut descendants);
        let script_handle = descendants
            .iter()
            .copied()
            .find(|node_id| {
                host.node(*node_id)
                    .is_some_and(|node| node.is_html_element_named("script"))
            })
            .expect("XHTML script element");
        assert_eq!(
            host.node(script_handle).and_then(Node::owner_document),
            Some(child_document)
        );
        assert_eq!(
            host.node(child_document)
                .and_then(Node::as_document)
                .map(|document| document.url()),
            Some(&child_url)
        );

        assert!(
            descendants
                .iter()
                .all(|node_id| host.node(*node_id).and_then(Node::owner_document)
                    == Some(child_document))
        );
        for expected in [
            NodeType::ProcessingInstruction,
            NodeType::DocumentType,
            NodeType::Element,
            NodeType::Text,
            NodeType::CDataSection,
            NodeType::Comment,
        ] {
            assert!(descendants.iter().any(|node_id| {
                host.node(*node_id)
                    .is_some_and(|node| node.node_type() == expected)
            }));
        }
    }

    #[test]
    fn xml_parser_rejects_html_and_nonempty_document_targets() {
        let parser = XmlParser;
        let mut host = DomHost::from_dom(NativeDom::new_html(
            Url::parse("https://example.test/").unwrap(),
        ));
        let html_document = host.document_handle();

        assert!(
            parser
                .parse_inert_tree_into_document(&mut host, html_document, "<root />")
                .is_none()
        );
        assert_eq!(host.child_handles(html_document).count(), 0);

        let xml_document = host.create_detached_xml_document();
        assert!(
            parser
                .parse_inert_tree_into_document(&mut host, xml_document, "<root />")
                .is_some()
        );
        let children_before_retry = host.child_handles(xml_document).collect::<Vec<_>>();
        assert!(
            parser
                .parse_inert_tree_into_document(&mut host, xml_document, "<replacement />")
                .is_none()
        );
        assert_eq!(
            host.child_handles(xml_document).collect::<Vec<_>>(),
            children_before_retry
        );
    }

    #[test]
    fn xml_parser_existing_document_html_template_uses_the_target_document_url() {
        let parser = XmlParser;
        let mut host = DomHost::from_dom(NativeDom::new_html(
            Url::parse("https://parent.example.test/").unwrap(),
        ));
        let child_url = Url::parse("https://child.example.test/document.xml").unwrap();
        let child_document = host.create_detached_xml_document_with_url(child_url.clone());

        parser
            .parse_inert_tree_into_document(
                &mut host,
                child_document,
                concat!(
                    "<root>",
                    "<template xmlns='http://www.w3.org/1999/xhtml'><span /></template>",
                    "</root>"
                ),
            )
            .expect("empty XML document accepts a direct parse");

        let root = host
            .child_handles(child_document)
            .find(|node_id| host.node(*node_id).is_some_and(Node::is_element))
            .expect("document element");
        let template = host
            .child_handles(root)
            .find(|node_id| {
                host.node(*node_id)
                    .is_some_and(|node| node.is_html_element_named("template"))
            })
            .expect("HTML template element");
        let contents = host
            .node(template)
            .and_then(Node::as_element)
            .and_then(|element| element.template_contents())
            .expect("template contents");
        let inert_document = host
            .node(contents)
            .and_then(Node::owner_document)
            .expect("template contents owner document");

        assert_ne!(inert_document, host.document_handle());
        assert_ne!(inert_document, child_document);
        assert_eq!(
            host.node(inert_document)
                .and_then(Node::as_document)
                .map(|document| document.url()),
            Some(&child_url)
        );
    }

    fn collect_descendants(
        host: &DomHost,
        parent: NativeNodeId,
        descendants: &mut Vec<NativeNodeId>,
    ) {
        for child in host.child_handles(parent).collect::<Vec<_>>() {
            descendants.push(child);
            collect_descendants(host, child, descendants);
        }
    }

    #[test]
    fn xml_parser_preserves_cdata_section_nodes() {
        let parser = XmlParser;
        let dom = parser.parse(
            Url::parse("https://example.test/xml").unwrap(),
            "<foo>CDATA section: <![CDATA[ < > & ]]>.</foo>".to_owned(),
        );
        let document = dom.document_node_id();
        let root = dom
            .child_ids(document)
            .find(|node_id| dom.node(*node_id).is_some_and(|node| node.is_element()))
            .expect("document element");
        let children = dom.child_ids(root).collect::<Vec<_>>();

        assert_eq!(children.len(), 3);
        assert_eq!(
            dom.node(children[0]).map(|node| node.node_type()),
            Some(NodeType::Text)
        );
        assert_eq!(
            dom.node(children[1]).map(|node| node.node_type()),
            Some(NodeType::CDataSection)
        );
        assert_eq!(
            dom.node(children[1])
                .and_then(|node| node.as_cdata_section())
                .map(|cdata| cdata.data()),
            Some(" < > & ")
        );
        assert_eq!(
            dom.node(children[2]).map(|node| node.node_type()),
            Some(NodeType::Text)
        );
    }

    #[test]
    fn xml_parser_puts_only_html_template_children_in_template_contents() {
        let parser = XmlParser;
        let html_dom = parser.parse(
            Url::parse("https://example.test/html-template.xml").unwrap(),
            "<template xmlns='http://www.w3.org/1999/xhtml'><test/></template>".to_owned(),
        );
        let html_template = document_element(&html_dom);
        let html_contents = html_dom
            .node(html_template)
            .and_then(Node::as_element)
            .and_then(|element| element.template_contents())
            .expect("HTML template contents");

        assert_eq!(html_dom.child_ids(html_template).count(), 0);
        assert_eq!(
            html_dom
                .child_ids(html_contents)
                .next()
                .and_then(|child| html_dom.node(child))
                .and_then(Node::as_element)
                .map(|element| element.local_name()),
            Some("test")
        );

        for (url, xml) in [
            (
                "https://example.test/no-namespace-template.xml",
                "<template><test/></template>",
            ),
            (
                "https://example.test/svg-template.xml",
                "<template xmlns='http://www.w3.org/2000/svg'><test/></template>",
            ),
        ] {
            let dom = parser.parse(Url::parse(url).unwrap(), xml.to_owned());
            let template = document_element(&dom);
            assert!(
                dom.node(template)
                    .and_then(Node::as_element)
                    .and_then(|element| element.template_contents())
                    .is_none()
            );
            assert_eq!(
                dom.child_ids(template)
                    .next()
                    .and_then(|child| dom.node(child))
                    .and_then(Node::as_element)
                    .map(|element| element.local_name()),
                Some("test")
            );
        }
    }

    #[test]
    fn xml_parser_preserves_cdata_inside_html_template_contents() {
        let parser = XmlParser;
        let dom = parser.parse(
            Url::parse("https://example.test/html-template-cdata.xml").unwrap(),
            concat!(
                "<template xmlns='http://www.w3.org/1999/xhtml'>",
                "<![CDATA[top-level]]>",
                "<test><![CDATA[nested]]></test>",
                "</template>"
            )
            .to_owned(),
        );
        let template = document_element(&dom);
        let contents = dom
            .node(template)
            .and_then(Node::as_element)
            .and_then(|element| element.template_contents())
            .expect("HTML template contents");
        let content_children = dom.child_ids(contents).collect::<Vec<_>>();

        assert_eq!(content_children.len(), 2);
        assert_eq!(
            dom.node(content_children[0])
                .and_then(Node::as_cdata_section)
                .map(|cdata| cdata.data()),
            Some("top-level")
        );
        assert_eq!(
            dom.node(content_children[1])
                .and_then(Node::as_element)
                .map(|element| element.local_name()),
            Some("test")
        );
        assert_eq!(
            dom.child_ids(content_children[1])
                .next()
                .and_then(|child| dom.node(child))
                .and_then(Node::as_cdata_section)
                .map(|cdata| cdata.data()),
            Some("nested")
        );
    }

    fn document_element(dom: &moli_dom::native::NativeDom) -> moli_dom::native::NativeNodeId {
        dom.child_ids(dom.document_node_id())
            .find(|node_id| dom.node(*node_id).is_some_and(|node| node.is_element()))
            .expect("document element")
    }
}
