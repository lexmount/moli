mod attributes;
mod document;
mod element;
mod host;
mod html_serialization;
mod node;
mod queries;
mod scripts;

use std::sync::Arc;

pub use document::{
    Document, DocumentFragment, DocumentReadyState, DocumentTitleSetterTarget, DocumentType,
};
pub use element::{
    Attribute, CustomElementState, Element, SelectedFile, html_element_interface_name,
    svg_element_interface_name,
};
use host::StylesheetCandidateRegistries;
pub use host::{
    ConnectedShadowRootSnapshot, DomAttributeMutation, DomAttributeMutationOutcome,
    DomChildListMutation, DomHost, DomMutationEffects, DomMutationRecord, DomMutationRecordBatch,
    DomMutationRecordKind, DomScriptMutationEffects, DomSlotAssignmentChange,
    DomSlotMutationEffects, DomStyleInvalidationInputs, DomStylesheetOwnerChange,
    DomStylesheetOwnerChangeKind, DomStylesheetOwnerTransitions, DomStylesheetOwnerTreeScopes,
    DomTreeMutationEffects, ScriptPrepareTrigger, ScriptPrepareTriggerKind,
    StylesheetCandidateTreeScopeSnapshots,
};
pub use host::{
    HostElementSnapshot, ShadowRootBindingSnapshot, ShadowRootInclusion, ShadowRootInit,
    ShadowRootRegistryAttributePolicy,
};
pub use html_serialization::HtmlSerializationLimitExceeded;
pub use node::{
    CDataSection, Comment, LiveDomNodeMetadata, NativeNodeId, Node, NodeData, NodeFlags, NodeType,
    ProcessingInstruction, Text,
};

// Node IDs remain dense indexes, while immutable page snapshots share complete
// chunks. A mutation detaches only its 256-node chunk, bounding copy-on-write
// work without introducing a pointer per node or changing tree identity.
const NATIVE_DOM_NODE_CHUNK_CAPACITY: usize = 256;

#[derive(Debug, Clone)]
struct NativeNodeStorage {
    chunks: Arc<Vec<Arc<Vec<Node>>>>,
    len: usize,
}

impl NativeNodeStorage {
    fn from_node(node: Node) -> Self {
        Self {
            chunks: Arc::new(vec![Arc::new(vec![node])]),
            len: 1,
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn get(&self, index: usize) -> Option<&Node> {
        let chunk = self.chunks.get(index / NATIVE_DOM_NODE_CHUNK_CAPACITY)?;
        chunk.get(index % NATIVE_DOM_NODE_CHUNK_CAPACITY)
    }

    fn get_mut(&mut self, index: usize) -> Option<&mut Node> {
        let chunks = Arc::make_mut(&mut self.chunks);
        let chunk = chunks.get_mut(index / NATIVE_DOM_NODE_CHUNK_CAPACITY)?;
        Arc::make_mut(chunk).get_mut(index % NATIVE_DOM_NODE_CHUNK_CAPACITY)
    }

    fn push(&mut self, node: Node) {
        let chunks = Arc::make_mut(&mut self.chunks);
        match chunks.last_mut() {
            Some(chunk) if chunk.len() < NATIVE_DOM_NODE_CHUNK_CAPACITY => {
                Arc::make_mut(chunk).push(node);
            }
            _ => chunks.push(Arc::new(vec![node])),
        }
        self.len += 1;
    }

    fn iter(&self) -> NativeDomNodes<'_> {
        NativeDomNodes {
            storage: self,
            front: 0,
            back: self.len,
        }
    }
}

#[derive(Clone)]
pub struct NativeDomNodes<'a> {
    storage: &'a NativeNodeStorage,
    front: usize,
    back: usize,
}

impl<'a> NativeDomNodes<'a> {
    pub fn iter(&self) -> Self {
        self.clone()
    }
}

impl<'a> Iterator for NativeDomNodes<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let index = self.front;
        self.front += 1;
        self.storage.get(index)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.back - self.front;
        (remaining, Some(remaining))
    }
}

impl<'a> DoubleEndedIterator for NativeDomNodes<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.storage.get(self.back)
    }
}

impl ExactSizeIterator for NativeDomNodes<'_> {}
impl std::iter::FusedIterator for NativeDomNodes<'_> {}

#[derive(Debug, Clone)]
pub struct NativeDom {
    nodes: NativeNodeStorage,
    document_node_id: NativeNodeId,
    stylesheet_candidate_registries: StylesheetCandidateRegistries,
    parse_errors: Vec<String>,
}

impl AsRef<NativeDom> for NativeDom {
    fn as_ref(&self) -> &NativeDom {
        self
    }
}

impl NativeDom {
    pub fn new(final_url: url::Url) -> Self {
        Self::new_html(final_url)
    }

    pub fn new_html(final_url: url::Url) -> Self {
        let document_node_id = NativeNodeId::new(0);
        let document_node = Node::new(
            document_node_id,
            None,
            None,
            NodeFlags::new(true),
            NodeData::Document(Box::new(Document::new_html(final_url))),
        );
        Self {
            nodes: NativeNodeStorage::from_node(document_node),
            document_node_id,
            stylesheet_candidate_registries: StylesheetCandidateRegistries::default(),
            parse_errors: Vec::new(),
        }
    }

    pub fn new_xml(final_url: url::Url) -> Self {
        let document_node_id = NativeNodeId::new(0);
        let document_node = Node::new(
            document_node_id,
            None,
            None,
            NodeFlags::new(true),
            NodeData::Document(Box::new(Document::new_xml(final_url))),
        );
        Self {
            nodes: NativeNodeStorage::from_node(document_node),
            document_node_id,
            stylesheet_candidate_registries: StylesheetCandidateRegistries::default(),
            parse_errors: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn nodes(&self) -> NativeDomNodes<'_> {
        self.nodes.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn document_node_id(&self) -> NativeNodeId {
        self.document_node_id
    }

    pub fn final_url(&self) -> Option<&url::Url> {
        self.document().map(Document::url)
    }

    pub fn parse_errors(&self) -> &[String] {
        &self.parse_errors
    }

    pub fn push_parse_error(&mut self, error: String) {
        self.parse_errors.push(error);
    }

    pub fn node(&self, node_id: NativeNodeId) -> Option<&Node> {
        self.nodes.get(node_id.index())
    }

    pub fn node_mut(&mut self, node_id: NativeNodeId) -> Option<&mut Node> {
        self.nodes.get_mut(node_id.index())
    }

    pub fn document(&self) -> Option<&Document> {
        self.node(self.document_node_id).and_then(Node::as_document)
    }

    pub fn set_document_content_type(&mut self, content_type: impl Into<String>) -> bool {
        let content_type = content_type.into();
        let document = self
            .node_mut(self.document_node_id)
            .and_then(|node| node.data_mut().as_document_mut())
            .expect("NativeDom must retain its document node");
        if document.content_type() == content_type {
            return false;
        }
        document.set_content_type(content_type);
        true
    }

    pub fn element(&self, node_id: NativeNodeId) -> Option<&Element> {
        self.node(node_id).and_then(Node::as_element)
    }

    pub fn child_ids(&self, node_id: NativeNodeId) -> impl Iterator<Item = NativeNodeId> + '_ {
        let mut next = self.first_child(node_id);
        std::iter::from_fn(move || {
            let child = next?;
            next = self.next_sibling(child);
            Some(child)
        })
    }

    pub fn child_ids_reversed(
        &self,
        node_id: NativeNodeId,
    ) -> impl Iterator<Item = NativeNodeId> + '_ {
        let mut previous = self.last_child(node_id);
        std::iter::from_fn(move || {
            let child = previous?;
            previous = self.previous_sibling(child);
            Some(child)
        })
    }

    pub fn find_child(
        &self,
        node_id: NativeNodeId,
        mut predicate: impl FnMut(NativeNodeId) -> bool,
    ) -> Option<NativeNodeId> {
        self.child_ids(node_id).find(|child| predicate(*child))
    }

    pub fn nth_child(&self, node_id: NativeNodeId, index: usize) -> Option<NativeNodeId> {
        self.child_ids(node_id).nth(index)
    }

    pub fn child_index(&self, parent: NativeNodeId, child: NativeNodeId) -> Option<usize> {
        if self.parent_node(child)? != parent {
            return None;
        }
        let mut index = 0;
        let mut current = child;
        while let Some(previous) = self.previous_sibling(current) {
            index += 1;
            current = previous;
        }
        Some(index)
    }

    pub fn parent_node(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.node(node_id).and_then(Node::parent_node)
    }

    pub fn first_child(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.node(node_id).and_then(Node::first_child)
    }

    pub fn last_child(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.node(node_id).and_then(Node::last_child)
    }

    pub fn next_sibling(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.node(node_id).and_then(Node::next_sibling)
    }

    pub fn previous_sibling(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.node(node_id).and_then(Node::prev_sibling)
    }

    pub fn child_nodes(&self, node_id: NativeNodeId) -> Option<Vec<NativeNodeId>> {
        self.node(node_id)?;
        Some(self.child_ids(node_id).collect())
    }

    pub fn create_node(
        &mut self,
        data: NodeData,
        owner_document: Option<NativeNodeId>,
        connected: bool,
        in_document_tree: bool,
    ) -> NativeNodeId {
        let node_id = NativeNodeId::new(self.nodes.len());
        let mut flags = NodeFlags::new(in_document_tree);
        flags.set_connected(connected);
        self.nodes
            .push(Node::new(node_id, None, owner_document, flags, data));
        node_id
    }

    pub fn create_element(&mut self, local_name: &str) -> NativeNodeId {
        let handle = self.create_node(
            NodeData::Element(Element::new_html(local_name)),
            Some(self.document_node_id),
            false,
            false,
        );
        if local_name.eq_ignore_ascii_case("template") {
            let fragment = self.create_template_contents_fragment();
            if let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            {
                element.set_template_contents(Some(fragment));
            }
        }
        handle
    }

    pub fn create_element_ns(
        &mut self,
        namespace: Option<&str>,
        qualified_name: &str,
    ) -> Option<NativeNodeId> {
        let (prefix, local_name) = split_qualified_name(qualified_name)?;
        Some(self.create_element_with_parts(namespace, prefix, local_name))
    }

    pub fn create_element_with_parts(
        &mut self,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
    ) -> NativeNodeId {
        let namespace = namespace.unwrap_or_default().to_owned();
        let handle = self.create_node(
            NodeData::Element(Element::new(
                local_name.to_owned(),
                namespace,
                prefix.map(str::to_owned),
                Vec::new(),
            )),
            Some(self.document_node_id),
            false,
            false,
        );
        if local_name.eq_ignore_ascii_case("template") {
            let fragment = self.create_template_contents_fragment();
            if let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            {
                element.set_template_contents(Some(fragment));
            }
        }
        handle
    }

    pub fn create_text_node(&mut self, data: &str) -> NativeNodeId {
        self.create_text_node_for_document(self.document_node_id, data)
    }

    pub fn create_text_node_for_document(
        &mut self,
        owner_document: NativeNodeId,
        data: &str,
    ) -> NativeNodeId {
        self.create_node(
            NodeData::Text(Text::new(data.to_owned())),
            Some(owner_document),
            false,
            false,
        )
    }

    pub fn create_cdata_section(&mut self, data: &str) -> NativeNodeId {
        self.create_cdata_section_for_document(self.document_node_id, data)
    }

    pub fn create_cdata_section_for_document(
        &mut self,
        owner_document: NativeNodeId,
        data: &str,
    ) -> NativeNodeId {
        self.create_node(
            NodeData::CDataSection(CDataSection::new(data.to_owned())),
            Some(owner_document),
            false,
            false,
        )
    }

    pub fn create_comment(&mut self, data: &str) -> NativeNodeId {
        self.create_comment_for_document(self.document_node_id, data)
    }

    pub fn create_comment_for_document(
        &mut self,
        owner_document: NativeNodeId,
        data: &str,
    ) -> NativeNodeId {
        self.create_node(
            NodeData::Comment(Comment::new(data.to_owned())),
            Some(owner_document),
            false,
            false,
        )
    }

    pub fn create_document_type(
        &mut self,
        name: String,
        public_id: String,
        system_id: String,
    ) -> NativeNodeId {
        self.create_document_type_for_document(self.document_node_id, name, public_id, system_id)
    }

    pub fn create_document_type_for_document(
        &mut self,
        owner_document: NativeNodeId,
        name: String,
        public_id: String,
        system_id: String,
    ) -> NativeNodeId {
        self.create_node(
            NodeData::DocumentType(DocumentType::new(name, public_id, system_id)),
            Some(owner_document),
            false,
            false,
        )
    }

    pub fn create_document(&mut self, url: url::Url) -> NativeNodeId {
        self.create_node(
            NodeData::Document(Box::new(Document::new_html(url))),
            None,
            false,
            false,
        )
    }

    pub fn create_document_fragment(&mut self) -> NativeNodeId {
        self.create_document_fragment_for_document(self.document_node_id)
    }

    pub fn create_document_fragment_for_document(
        &mut self,
        owner_document: NativeNodeId,
    ) -> NativeNodeId {
        self.create_node(
            NodeData::DocumentFragment(DocumentFragment),
            Some(owner_document),
            false,
            false,
        )
    }

    pub fn create_template_contents_fragment(&mut self) -> NativeNodeId {
        self.create_template_contents_fragment_for_document(self.document_node_id)
    }

    pub fn create_template_contents_fragment_for_document(
        &mut self,
        document_handle: NativeNodeId,
    ) -> NativeNodeId {
        let url = self
            .node(document_handle)
            .and_then(Node::as_document)
            .map(|document| document.url().clone())
            .unwrap_or_else(|| url::Url::parse("about:blank").expect("about:blank is valid"));
        let owner_document = self.create_node(
            NodeData::Document(Box::new(Document::new_html(url))),
            None,
            false,
            false,
        );
        self.create_document_fragment_for_document(owner_document)
    }

    pub fn create_processing_instruction(&mut self, target: &str, data: &str) -> NativeNodeId {
        self.create_processing_instruction_for_document(self.document_node_id, target, data)
    }

    pub fn create_processing_instruction_for_document(
        &mut self,
        owner_document: NativeNodeId,
        target: &str,
        data: &str,
    ) -> NativeNodeId {
        self.create_node(
            NodeData::ProcessingInstruction(ProcessingInstruction::new(
                target.to_owned(),
                data.to_owned(),
            )),
            Some(owner_document),
            false,
            false,
        )
    }

    pub fn can_have_children(&self, node_id: NativeNodeId) -> bool {
        self.node(node_id).is_some_and(Node::can_have_children)
    }

    pub fn contains(&self, parent: NativeNodeId, child: NativeNodeId) -> bool {
        self.node(parent)
            .is_some_and(|node| node.contains(self, child))
    }

    pub fn is_ancestor(&self, candidate_ancestor: NativeNodeId, node_id: NativeNodeId) -> bool {
        let mut current = self.parent_node(node_id);
        while let Some(parent) = current {
            if parent == candidate_ancestor {
                return true;
            }
            current = self.parent_node(parent);
        }
        false
    }

    pub fn detach_from_parent(&mut self, child: NativeNodeId) {
        let _ = self.detach_from_parent_with_stylesheet_candidate_changes(child);
    }

    pub fn mark_subtree_tree_scope(
        &mut self,
        node_id: NativeNodeId,
        owner_document: Option<NativeNodeId>,
        connected: bool,
        in_document_tree: bool,
    ) {
        self.mark_subtree_tree_scope_preserving_stylesheet_candidates(
            node_id,
            owner_document,
            connected,
            in_document_tree,
        );
    }

    pub fn append_child(&mut self, parent: NativeNodeId, child: NativeNodeId) -> bool {
        self.insert_before(parent, child, None)
    }

    pub fn remove_child(&mut self, parent: NativeNodeId, child: NativeNodeId) -> bool {
        self.remove_child_with_stylesheet_candidate_changes(parent, child)
            .is_some()
    }

    pub fn insert_before(
        &mut self,
        parent: NativeNodeId,
        child: NativeNodeId,
        reference_child: Option<NativeNodeId>,
    ) -> bool {
        self.insert_before_with_stylesheet_candidate_changes(parent, child, reference_child)
            .is_some()
    }

    fn can_insert_child_type(parent_type: NodeType, child_type: NodeType) -> bool {
        match parent_type {
            NodeType::Document => !matches!(
                child_type,
                NodeType::Document | NodeType::Text | NodeType::CDataSection
            ),
            NodeType::DocumentFragment | NodeType::Element => {
                !matches!(child_type, NodeType::Document | NodeType::DocumentType)
            }
            NodeType::DocumentType
            | NodeType::Text
            | NodeType::CDataSection
            | NodeType::Comment
            | NodeType::ProcessingInstruction => false,
        }
    }

    pub fn document_element_handle(&self) -> Option<NativeNodeId> {
        self.document_element_handle_for_document(self.document_node_id)
    }

    pub fn document_element_handle_for_document(
        &self,
        document_node_id: NativeNodeId,
    ) -> Option<NativeNodeId> {
        self.node(document_node_id)
            .and_then(Node::as_document)
            .and_then(|document| document.document_element_handle(self, document_node_id))
    }

    pub fn document_element_node_id(&self) -> Option<NativeNodeId> {
        self.document_element_handle()
    }

    pub fn document_head_handle(&self) -> Option<NativeNodeId> {
        self.document_head_handle_for_document(self.document_node_id)
    }

    pub fn document_head_handle_for_document(
        &self,
        document_node_id: NativeNodeId,
    ) -> Option<NativeNodeId> {
        self.node(document_node_id)
            .and_then(Node::as_document)
            .and_then(|document| document.head_handle(self, document_node_id))
    }

    pub fn document_title(&self) -> String {
        self.document_title_for_document(self.document_node_id)
    }

    pub fn document_title_for_document(&self, document_node_id: NativeNodeId) -> String {
        let raw_title = self
            .document_title_element_for_document(document_node_id)
            .map(|title| self.child_text_content(title))
            .unwrap_or_default();
        strip_and_collapse_html_ascii_whitespace(&raw_title)
    }

    pub fn document_title_element_for_document(
        &self,
        document_node_id: NativeNodeId,
    ) -> Option<NativeNodeId> {
        let root = self.document_element_handle_for_document(document_node_id)?;
        if self.is_svg_svg_element(root) {
            return self.find_child(root, |handle| self.is_svg_title_element(handle));
        }
        self.find_first_html_element_in_tree_order(root, "title")
    }

    pub fn document_title_setter_target_for_document(
        &self,
        document_node_id: NativeNodeId,
    ) -> Option<DocumentTitleSetterTarget> {
        let root = self.document_element_handle_for_document(document_node_id)?;
        if self.is_svg_svg_element(root) {
            return Some(
                self.find_child(root, |handle| self.is_svg_title_element(handle))
                    .map(DocumentTitleSetterTarget::ExistingTitle)
                    .unwrap_or(DocumentTitleSetterTarget::PrependToSvgRoot(root)),
            );
        }

        let root_element = self.node(root).and_then(Node::as_element)?;
        if root_element.namespace() != "http://www.w3.org/1999/xhtml" {
            return None;
        }
        if let Some(title) = self.find_first_html_element_in_tree_order(root, "title") {
            return Some(DocumentTitleSetterTarget::ExistingTitle(title));
        }
        if !root_element.is_html_element("html") {
            return None;
        }
        let head = self.find_child(root, |handle| {
            self.node(handle)
                .is_some_and(|node| node.is_html_element_named("head"))
        })?;
        Some(DocumentTitleSetterTarget::AppendToHtmlHead(head))
    }

    /// Returns the first HTML `<title>` element found in tree order under the
    /// document element. Spec: HTML §"the title element of the document".
    pub fn first_html_title_in_tree_order(&self) -> Option<NativeNodeId> {
        self.first_html_title_in_tree_order_for_document(self.document_node_id)
    }

    pub fn first_html_title_in_tree_order_for_document(
        &self,
        document_node_id: NativeNodeId,
    ) -> Option<NativeNodeId> {
        let root = self.document_element_handle_for_document(document_node_id)?;
        self.find_first_html_element_in_tree_order(root, "title")
    }

    fn find_first_html_element_in_tree_order(
        &self,
        root: NativeNodeId,
        local_name: &str,
    ) -> Option<NativeNodeId> {
        let mut stack = vec![root];
        while let Some(node_id) = stack.pop() {
            if self
                .node(node_id)
                .and_then(Node::as_element)
                .is_some_and(|element| element.is_html_element(local_name))
            {
                return Some(node_id);
            }
            let mut child = self.last_child(node_id);
            while let Some(child_id) = child {
                child = self.previous_sibling(child_id);
                stack.push(child_id);
            }
        }
        None
    }

    fn is_svg_svg_element(&self, handle: NativeNodeId) -> bool {
        self.node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.namespace() == "http://www.w3.org/2000/svg" && element.local_name() == "svg"
            })
    }

    fn is_svg_title_element(&self, handle: NativeNodeId) -> bool {
        self.node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.namespace() == "http://www.w3.org/2000/svg"
                    && element.local_name() == "title"
            })
    }

    fn child_text_content(&self, handle: NativeNodeId) -> String {
        self.child_ids(handle)
            .filter_map(|child| match self.node(child)?.data() {
                NodeData::Text(text) => Some(text.data()),
                NodeData::CDataSection(cdata) => Some(cdata.data()),
                _ => None,
            })
            .fold(String::new(), |mut value, text| {
                value.push_str(text);
                value
            })
    }

    pub fn head_node_id(&self) -> Option<NativeNodeId> {
        self.document_head_handle()
    }

    pub fn document_body_handle(&self) -> Option<NativeNodeId> {
        self.document_body_handle_for_document(self.document_node_id)
    }

    pub fn document_body_handle_for_document(
        &self,
        document_node_id: NativeNodeId,
    ) -> Option<NativeNodeId> {
        self.node(document_node_id)
            .and_then(Node::as_document)
            .and_then(|document| document.body_handle(self, document_node_id))
    }

    pub fn body_node_id(&self) -> Option<NativeNodeId> {
        self.document_body_handle()
    }

    pub fn text_content(&self, node_id: NativeNodeId) -> Option<String> {
        self.node(node_id).map(|node| node.text_content(self))
    }

    pub fn direct_text_content(&self, node_id: NativeNodeId) -> Option<String> {
        self.node(node_id)
            .map(|node| node.direct_text_content(self))
    }

    pub fn node_metadata(&self, node_id: NativeNodeId) -> Option<LiveDomNodeMetadata> {
        self.node(node_id).map(Node::metadata)
    }

    pub fn wrapper_prototype_name(&self, node_id: NativeNodeId) -> &'static str {
        self.node(node_id)
            .map(Node::wrapper_prototype_name)
            .unwrap_or("Node")
    }
}

fn strip_and_collapse_html_ascii_whitespace(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut pending_space = false;
    for character in value.chars() {
        if matches!(
            character,
            '\u{0009}' | '\u{000A}' | '\u{000C}' | '\u{000D}' | ' '
        ) {
            pending_space = !result.is_empty();
        } else {
            if pending_space {
                result.push(' ');
                pending_space = false;
            }
            result.push(character);
        }
    }
    result
}

fn split_qualified_name(qualified_name: &str) -> Option<(Option<&str>, &str)> {
    if qualified_name.is_empty() {
        return None;
    }
    match qualified_name.split_once(':') {
        Some((prefix, local_name)) if !prefix.is_empty() && !local_name.is_empty() => {
            Some((Some(prefix), local_name))
        }
        Some(_) => None,
        None => Some((None, qualified_name)),
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::*;

    fn test_url() -> url::Url {
        url::Url::parse("https://example.test/").expect("valid test url")
    }

    #[test]
    fn mutation_observer_and_devtools_recording_interests_are_independent() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let element = host.create_element("div");

        let effects = host.set_attribute_effects(element, "data-none", "0");
        assert!(effects.observer_records().records().is_empty());

        host.set_devtools_mutation_records_enabled(true);
        let effects = host.set_attribute_effects(element, "data-devtools", "1");
        assert_eq!(effects.observer_records().records().len(), 1);

        host.set_mutation_observer_records_enabled(true);
        host.set_devtools_mutation_records_enabled(false);
        let effects = host.set_attribute_effects(element, "data-observer", "2");
        assert_eq!(effects.observer_records().records().len(), 1);

        host.set_mutation_observer_records_enabled(false);
        let effects = host.set_attribute_effects(element, "data-disabled", "3");
        assert!(effects.observer_records().records().is_empty());
    }

    #[test]
    fn insert_before_self_reference_reports_remove_and_reinsert() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");
        let first = host.create_element("first");
        let second = host.create_element("second");
        assert!(host.append_child(body, first));
        assert!(host.append_child(body, second));
        host.set_devtools_mutation_records_enabled(true);

        let effects = host.insert_before_effects(body, first, Some(first));

        assert!(effects.did_change());
        assert_eq!(
            host.child_handles(body).collect::<Vec<_>>(),
            [first, second]
        );
        let records = effects.observer_records().records();
        assert_eq!(records.len(), 2);
        let DomMutationRecordKind::ChildList(removal) = records[0].kind() else {
            panic!("expected removal record");
        };
        assert_eq!(removal.removed_nodes(), &[first]);
        assert!(removal.added_nodes().is_empty());
        let DomMutationRecordKind::ChildList(insertion) = records[1].kind() else {
            panic!("expected insertion record");
        };
        assert_eq!(insertion.added_nodes(), &[first]);
        assert!(insertion.removed_nodes().is_empty());
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn common_node_layout_stays_compact() {
        assert_eq!(size_of::<Option<NativeNodeId>>(), size_of::<NativeNodeId>());
        assert_eq!(size_of::<NativeNodeId>(), size_of::<u32>());
        assert!(
            size_of::<DocumentType>() <= 48,
            "DocumentType grew to {} bytes",
            size_of::<DocumentType>()
        );
        assert_eq!(size_of::<Text>(), 16);
        assert_eq!(size_of::<CDataSection>(), 16);
        assert_eq!(size_of::<Comment>(), 16);
        assert_eq!(size_of::<ProcessingInstruction>(), 32);
        assert!(
            size_of::<Attribute>() <= 40,
            "Attribute common record grew to {} bytes",
            size_of::<Attribute>()
        );
        assert!(
            size_of::<Element>() <= 40,
            "Element common record grew to {} bytes",
            size_of::<Element>()
        );
        assert!(
            size_of::<NodeData>() <= 56,
            "NodeData grew to {} bytes",
            size_of::<NodeData>()
        );
        assert!(
            size_of::<Node>() <= 88,
            "Node grew to {} bytes",
            size_of::<Node>()
        );
    }

    #[test]
    fn document_ready_state_has_only_the_web_observable_states() {
        let mut document = Document::new_html(test_url());
        assert_eq!(document.ready_state(), DocumentReadyState::Complete);
        assert_eq!(document.ready_state().as_str(), "complete");

        document.set_ready_state(DocumentReadyState::Loading);
        assert_eq!(document.ready_state().as_str(), "loading");
        document.set_ready_state(DocumentReadyState::Interactive);
        assert_eq!(document.ready_state().as_str(), "interactive");
    }

    #[test]
    fn native_node_id_keeps_dense_zero_based_indexes_with_a_nonzero_niche() {
        for index in [0, 1, 255, 65_535, (u32::MAX - 1) as usize] {
            let id = NativeNodeId::new(index);
            assert_eq!(id.index(), index);
            assert_eq!(id.index_u32(), index as u32);
            assert_eq!(id.encoded(), index as u32 + 1);
            assert_eq!(format!("{id:?}"), format!("NativeNodeId({index})"));
        }
        assert_eq!(size_of::<Option<NativeNodeId>>(), size_of::<u32>());
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    #[should_panic(expected = "native DOM exceeds the u32 node limit")]
    fn native_node_id_rejects_indexes_outside_the_u32_encoding() {
        let _ = NativeNodeId::new(u32::MAX as usize);
    }

    #[test]
    fn element_node_name_preserves_namespace_and_prefix_rules() {
        let mut dom = NativeDom::new_html(test_url());

        let html = dom.create_element("span");
        assert_eq!(dom.node(html).expect("html element").node_name(), "SPAN");

        let svg = dom
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg:linearGradient")
            .expect("svg element");
        assert_eq!(
            dom.node(svg).expect("svg element").node_name(),
            "svg:linearGradient"
        );

        let foreign = dom
            .create_element_ns(Some("urn:moli:test"), "lm:node")
            .expect("foreign element");
        assert_eq!(
            dom.node(foreign).expect("foreign element").node_name(),
            "lm:node"
        );
    }

    #[test]
    fn document_structure_distinguishes_body_from_body_or_frameset() {
        let mut non_html_root_dom = NativeDom::new_html(test_url());
        let non_html_document = non_html_root_dom.document_node_id();
        let test_root = non_html_root_dom.create_element("test");
        assert!(non_html_root_dom.append_child(non_html_document, test_root));
        assert_eq!(non_html_root_dom.document_element_handle(), Some(test_root));
        assert_eq!(non_html_root_dom.document_body_handle(), None);

        let mut dom = NativeDom::new_html(test_url());
        let document = dom.document_node_id();
        let html = dom.create_element("html");
        let nested_container = dom.create_element("x");
        let nested_frameset = dom.create_element("frameset");
        let direct_frameset = dom.create_element("frameset");
        let trailing_body = dom.create_element("body");
        assert!(dom.append_child(document, html));
        assert!(dom.append_child(html, nested_container));
        assert!(dom.append_child(nested_container, nested_frameset));
        assert!(dom.append_child(html, direct_frameset));
        assert!(dom.append_child(html, trailing_body));

        let document_node = dom
            .node(document)
            .and_then(Node::as_document)
            .expect("document node");
        assert_eq!(
            document_node.body_handle(&dom, document),
            Some(trailing_body)
        );
        assert_eq!(
            document_node.body_or_frameset_handle(&dom, document),
            Some(direct_frameset)
        );
    }

    #[test]
    fn document_title_selects_html_and_svg_targets_and_normalizes_ascii_whitespace() {
        let mut html_dom = NativeDom::new_html(test_url());
        let html_document = html_dom.document_node_id();
        let html = html_dom.create_element("html");
        let head = html_dom.create_element("head");
        let title = html_dom.create_element("title");
        let title_text = html_dom.create_text_node(" \tone\n\n two\u{000B}three\r ");
        assert!(html_dom.append_child(html_document, html));
        assert!(html_dom.append_child(html, head));
        assert!(html_dom.append_child(head, title));
        assert!(html_dom.append_child(title, title_text));

        assert_eq!(html_dom.document_title(), "one two\u{000B}three");
        assert_eq!(
            html_dom.document_title_setter_target_for_document(html_document),
            Some(DocumentTitleSetterTarget::ExistingTitle(title))
        );

        let mut empty_html_dom = NativeDom::new_html(test_url());
        let empty_document = empty_html_dom.document_node_id();
        let empty_html = empty_html_dom.create_element("html");
        let empty_head = empty_html_dom.create_element("head");
        assert!(empty_html_dom.append_child(empty_document, empty_html));
        assert!(empty_html_dom.append_child(empty_html, empty_head));
        assert_eq!(
            empty_html_dom.document_title_setter_target_for_document(empty_document),
            Some(DocumentTitleSetterTarget::AppendToHtmlHead(empty_head))
        );

        let mut svg_dom = NativeDom::new_xml(test_url());
        let svg_document = svg_dom.document_node_id();
        let svg = svg_dom
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
            .expect("SVG root");
        let nested = svg_dom
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "g")
            .expect("SVG group");
        let nested_title = svg_dom
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "title")
            .expect("nested SVG title");
        assert!(svg_dom.append_child(svg_document, svg));
        assert!(svg_dom.append_child(svg, nested));
        assert!(svg_dom.append_child(nested, nested_title));
        assert_eq!(svg_dom.document_title(), "");
        assert_eq!(
            svg_dom.document_title_setter_target_for_document(svg_document),
            Some(DocumentTitleSetterTarget::PrependToSvgRoot(svg))
        );

        let direct_title = svg_dom
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "title")
            .expect("direct SVG title");
        let direct_text = svg_dom.create_text_node(" svg\t title ");
        assert!(svg_dom.append_child(direct_title, direct_text));
        assert!(svg_dom.insert_before(svg, direct_title, Some(nested)));
        assert_eq!(svg_dom.document_title(), "svg title");
        assert_eq!(
            svg_dom.document_title_setter_target_for_document(svg_document),
            Some(DocumentTitleSetterTarget::ExistingTitle(direct_title))
        );

        let mut xml_dom = NativeDom::new_xml(test_url());
        let xml_document = xml_dom.document_node_id();
        let xml_root = xml_dom.create_element_ns(None, "root").expect("XML root");
        let html_title = xml_dom
            .create_element_ns(Some("http://www.w3.org/1999/xhtml"), "title")
            .expect("HTML title");
        let html_title_text = xml_dom.create_text_node("unchanged");
        assert!(xml_dom.append_child(xml_document, xml_root));
        assert!(xml_dom.append_child(xml_root, html_title));
        assert!(xml_dom.append_child(html_title, html_title_text));
        assert_eq!(xml_dom.document_title(), "unchanged");
        assert_eq!(
            xml_dom.document_title_setter_target_for_document(xml_document),
            None
        );
    }

    #[test]
    fn document_fragment_insertion_preflights_all_children_before_mutating_tree() {
        let mut dom = NativeDom::new_html(test_url());
        let document = dom.document_node_id();
        let fragment = dom.create_document_fragment();
        let style = dom.create_element("style");
        let invalid_text = dom.create_text_node("not allowed under Document");

        assert!(dom.append_child(fragment, style));
        assert!(dom.append_child(fragment, invalid_text));
        let document_children_before = dom.child_ids(document).collect::<Vec<_>>();
        let document_candidates_before = dom.stylesheet_candidate_handles_for_tree_scope(document);

        assert!(!dom.append_child(document, fragment));
        assert_eq!(
            dom.child_ids(document).collect::<Vec<_>>(),
            document_children_before
        );
        assert_eq!(
            dom.child_ids(fragment).collect::<Vec<_>>(),
            vec![style, invalid_text]
        );
        assert_eq!(dom.parent_node(style), Some(fragment));
        assert_eq!(dom.parent_node(invalid_text), Some(fragment));
        assert_eq!(
            dom.stylesheet_candidate_handles_for_tree_scope(document),
            document_candidates_before,
        );
    }

    #[test]
    fn child_browsing_context_candidate_index_restores_and_tracks_document_order() {
        let mut source = DomHost::from_dom(NativeDom::new_html(test_url()));
        source.reset_html_document_shell();
        let body = source.document_body_handle().expect("document body");
        let first_created = source.create_element("iframe");
        let second_created = source.create_element("frame");
        let embed = source.create_element("embed");
        let object = source.create_element("object");
        assert!(source.append_child(body, second_created));
        assert!(source.append_child(body, embed));
        assert!(source.append_child(body, first_created));
        assert!(source.append_child(body, object));

        let mut host = DomHost::from_dom(source.into_dom());
        let document = host.document_handle();
        assert_eq!(
            host.child_browsing_context_host_candidate_handles_in_subtree_in_document_order(
                document
            ),
            vec![second_created, embed, first_created, object]
        );

        assert!(host.remove_child(body, second_created));
        assert_eq!(
            host.child_browsing_context_host_candidate_handles_in_subtree_in_document_order(
                document
            ),
            vec![embed, first_created, object]
        );
        assert_eq!(
            host.child_browsing_context_host_candidate_handles_in_subtree_in_document_order(
                second_created
            ),
            vec![second_created]
        );

        assert!(host.append_child(body, second_created));
        assert_eq!(
            host.child_browsing_context_host_candidate_handles_in_subtree_in_document_order(
                document
            ),
            vec![embed, first_created, object, second_created]
        );
    }

    #[test]
    fn option_value_collects_nested_text_without_script_content() {
        let mut dom = NativeDom::new_html(test_url());
        let option = dom.create_element("option");
        let leading = dom.create_text_node("  Alpha\n");
        let span = dom.create_element("span");
        let nested = dom.create_text_node(" Beta ");
        let script = dom.create_element("script");
        let script_text = dom.create_text_node("ignored");

        assert!(dom.append_child(option, leading));
        assert!(dom.append_child(option, span));
        assert!(dom.append_child(span, nested));
        assert!(dom.append_child(option, script));
        assert!(dom.append_child(script, script_text));

        assert_eq!(dom.option_value(option).as_deref(), Some("Alpha Beta"));
    }

    #[test]
    fn option_value_walk_handles_deep_trees_iteratively() {
        const DEPTH: usize = 4096;

        let mut dom = NativeDom::new_html(test_url());
        let option = dom.create_element("option");
        assert!(dom.append_child(dom.document_node_id(), option));

        let mut parent = option;
        for _ in 0..DEPTH {
            let child = dom.create_element("span");
            assert!(dom.append_child(parent, child));
            parent = child;
        }
        let text = dom.create_text_node("deep value");
        let script = dom.create_element("script");
        let script_text = dom.create_text_node("ignored");
        assert!(dom.append_child(parent, text));
        assert!(dom.append_child(parent, script));
        assert!(dom.append_child(script, script_text));

        assert_eq!(dom.option_value(option).as_deref(), Some("deep value"));
    }

    #[test]
    fn radio_groups_include_disconnected_tree_roots_and_follow_form_owners() {
        fn radio(host: &mut DomHost, name: &str) -> NativeNodeId {
            let handle = host.create_element("input");
            assert!(host.set_attribute(handle, "type", "radio"));
            assert!(host.set_attribute(handle, "name", name));
            handle
        }

        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));

        let unnamed = host.create_element("input");
        assert!(host.set_attribute(unnamed, "type", "radio"));
        assert!(host.radio_group_members(unnamed).is_empty());

        let root_radio = radio(&mut host, "root-group");
        let child_radio = radio(&mut host, "root-group");
        assert!(host.set_checked_state(root_radio, true));
        assert!(host.set_checked_state(child_radio, true));
        assert!(host.append_child(root_radio, child_radio));
        assert_eq!(
            host.radio_group_members(root_radio),
            vec![root_radio, child_radio]
        );
        assert!(
            host.node(root_radio)
                .and_then(Node::as_element)
                .unwrap()
                .checked()
        );
        assert!(
            host.node(child_radio)
                .and_then(Node::as_element)
                .unwrap()
                .checked()
        );
        assert!(host.set_checked_state(root_radio, true));
        assert!(
            !host
                .node(child_radio)
                .and_then(Node::as_element)
                .unwrap()
                .checked()
        );

        let container = host.create_element("div");
        let loose_first = radio(&mut host, "loose-group");
        let loose_second = radio(&mut host, "loose-group");
        assert!(host.set_checked_state(loose_first, true));
        assert!(host.set_checked_state(loose_second, true));
        assert!(host.append_child(container, loose_first));
        assert!(host.append_child(container, loose_second));
        assert!(
            host.node(loose_first)
                .and_then(Node::as_element)
                .unwrap()
                .checked()
        );
        assert!(
            host.node(loose_second)
                .and_then(Node::as_element)
                .unwrap()
                .checked()
        );

        let form = host.create_element("form");
        let form_first = radio(&mut host, "form-group");
        let form_second = radio(&mut host, "form-group");
        assert!(host.set_checked_state(form_first, true));
        assert!(host.set_checked_state(form_second, true));
        assert!(host.append_child(form, form_first));
        assert!(host.append_child(form, form_second));
        assert!(
            !host
                .node(form_first)
                .and_then(Node::as_element)
                .unwrap()
                .checked()
        );
        assert!(
            host.node(form_second)
                .and_then(Node::as_element)
                .unwrap()
                .checked()
        );
    }

    #[test]
    fn native_dom_select_queries_use_effective_selectedness() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();

        let single = host.create_element("select");
        let single_first = host.create_element("option");
        let single_second = host.create_element("option");
        assert!(host.append_child(document, single));
        assert!(host.append_child(single, single_first));
        assert!(host.append_child(single, single_second));
        assert_eq!(
            host.dom().select_selected_option_elements(single),
            vec![single_first]
        );
        assert!(host.dom().option_effectively_selected(single_first));
        assert!(!host.dom().option_effectively_selected(single_second));

        let explicit = host.create_element("select");
        let explicit_first = host.create_element("option");
        let explicit_second = host.create_element("option");
        assert!(host.set_attribute(explicit_first, "selected", ""));
        assert!(host.set_attribute(explicit_second, "selected", ""));
        assert!(host.append_child(document, explicit));
        assert!(host.append_child(explicit, explicit_first));
        assert!(host.append_child(explicit, explicit_second));
        assert_eq!(
            host.dom().select_selected_option_elements(explicit),
            vec![explicit_second]
        );

        let multiple = host.create_element("select");
        let multiple_first = host.create_element("option");
        let multiple_second = host.create_element("option");
        assert!(host.set_attribute(multiple, "multiple", ""));
        assert!(host.set_attribute(multiple_second, "selected", ""));
        assert!(host.append_child(document, multiple));
        assert!(host.append_child(multiple, multiple_first));
        assert!(host.append_child(multiple, multiple_second));
        assert_eq!(
            host.dom().select_selected_option_elements(multiple),
            vec![multiple_second]
        );
        assert!(!host.dom().option_effectively_selected(multiple_first));

        let disabled_fallback = host.create_element("select");
        let disabled_first = host.create_element("option");
        let enabled_second = host.create_element("option");
        assert!(host.set_attribute(disabled_first, "disabled", ""));
        assert!(host.append_child(document, disabled_fallback));
        assert!(host.append_child(disabled_fallback, disabled_first));
        assert!(host.append_child(disabled_fallback, enabled_second));
        assert_eq!(
            host.dom()
                .select_selected_option_elements(disabled_fallback),
            vec![enabled_second]
        );

        let listbox = host.create_element("select");
        let listbox_first = host.create_element("option");
        let listbox_second = host.create_element("option");
        assert!(host.set_attribute(listbox, "size", "2"));
        assert!(host.append_child(document, listbox));
        assert!(host.append_child(listbox, listbox_first));
        assert!(host.append_child(listbox, listbox_second));
        assert!(
            host.dom()
                .select_selected_option_elements(listbox)
                .is_empty()
        );
    }

    #[test]
    fn selected_option_insertion_deselects_single_select_peers_without_dirtying_them() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();
        let select = host.create_element("select");
        let first = host.create_element("option");
        let existing_selected = host.create_element("option");
        let inserted_selected = host.create_element("option");

        assert!(host.set_attribute(existing_selected, "selected", ""));
        assert!(host.append_child(document, select));
        assert!(host.append_child(select, first));
        assert!(host.append_child(select, existing_selected));
        assert!(host.set_selected_state(inserted_selected, true));
        assert!(host.insert_before(select, inserted_selected, Some(first)));

        assert_eq!(
            host.select_selected_option_elements(select),
            vec![inserted_selected]
        );
        let existing = host
            .node(existing_selected)
            .and_then(Node::as_element)
            .unwrap();
        assert!(!existing.selected());
        assert!(!existing.selected_dirty());
        let inserted = host
            .node(inserted_selected)
            .and_then(Node::as_element)
            .unwrap();
        assert!(inserted.selected());
        assert!(inserted.selected_dirty());

        let fragment_select = host.create_element("select");
        let fragment = host.create_document_fragment();
        let fragment_first = host.create_element("option");
        let fragment_second = host.create_element("option");
        assert!(host.set_selected_state(fragment_first, true));
        assert!(host.set_selected_state(fragment_second, true));
        assert!(host.append_child(fragment, fragment_first));
        assert!(host.append_child(fragment, fragment_second));
        assert!(host.append_child(document, fragment_select));
        assert!(host.append_child(fragment_select, fragment));
        assert_eq!(
            host.select_selected_option_elements(fragment_select),
            vec![fragment_second]
        );
    }

    #[test]
    fn manual_slot_assignment_reports_previous_and_current_assigned_nodes() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let shadow_host = host.create_element("section");
        let mut init = ShadowRootInit::new("open");
        init.set_slot_assignment("manual");
        let shadow_root = host
            .attach_shadow_root_with_init(shadow_host, init)
            .expect("shadow root");
        let first_slot = host.create_element("slot");
        let second_slot = host.create_element("slot");
        let first = host.create_element("span");
        let second = host.create_element("span");

        assert!(host.append_child(host.document_node_id(), shadow_host));
        assert!(host.append_child(shadow_root, first_slot));
        assert!(host.append_child(shadow_root, second_slot));
        assert!(host.append_child(shadow_host, first));
        assert!(host.append_child(shadow_host, second));

        let initial = host.assign_nodes_to_slot(first_slot, vec![first]);
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].slot(), first_slot);
        assert_eq!(initial[0].previous_assigned_nodes(), &[]);
        assert_eq!(initial[0].assigned_nodes(), &[first]);

        let changed = host.assign_nodes_to_slot(second_slot, vec![first, second]);
        assert_eq!(changed.len(), 2);
        assert_eq!(changed[0].slot(), first_slot);
        assert_eq!(changed[0].previous_assigned_nodes(), &[first]);
        assert_eq!(changed[0].assigned_nodes(), &[]);
        assert_eq!(changed[1].slot(), second_slot);
        assert_eq!(changed[1].previous_assigned_nodes(), &[]);
        assert_eq!(changed[1].assigned_nodes(), &[first, second]);
    }

    #[test]
    fn shadow_root_reference_target_changes_invalidate_query_state() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let shadow_host = host.create_element("section");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let target = host.create_element("input");
        assert!(host.set_attribute(target, "id", "forwarded"));
        assert!(host.append_child(host.document_node_id(), shadow_host));
        assert!(host.append_child(shadow_root, target));

        let before = host.query_version();
        assert!(host.set_shadow_root_reference_target(shadow_root, Some("forwarded".to_owned())));
        assert!(host.query_version() > before);
        assert_eq!(
            host.resolve_reference_target_chain(shadow_host),
            Some(target)
        );

        let after_change = host.query_version();
        assert!(!host.set_shadow_root_reference_target(shadow_root, Some("forwarded".to_owned())));
        assert_eq!(host.query_version(), after_change);

        assert!(host.set_shadow_root_reference_target(shadow_root, None));
        assert!(host.query_version() > after_change);
        assert_eq!(
            host.resolve_reference_target_chain(shadow_host),
            Some(shadow_host)
        );
    }

    #[test]
    fn host_child_mutations_report_slot_assignment_snapshots() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let shadow_host = host.create_element("section");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let slot = host.create_element("slot");
        let child = host.create_element("span");

        assert!(host.set_attribute(slot, "name", "content"));
        assert!(host.set_attribute(child, "slot", "content"));
        assert!(host.append_child(host.document_node_id(), shadow_host));
        assert!(host.append_child(shadow_root, slot));

        let inserted = host.append_child_effects(shadow_host, child);
        assert_eq!(inserted.slots().assignment_changes().len(), 1);
        assert_eq!(inserted.slots().assignment_changes()[0].slot(), slot);
        assert_eq!(
            inserted.slots().assignment_changes()[0].previous_assigned_nodes(),
            &[]
        );
        assert_eq!(
            inserted.slots().assignment_changes()[0].assigned_nodes(),
            &[child]
        );

        let removed = host.remove_child_effects(shadow_host, child);
        assert_eq!(removed.slots().assignment_changes().len(), 1);
        assert_eq!(removed.slots().assignment_changes()[0].slot(), slot);
        assert_eq!(
            removed.slots().assignment_changes()[0].previous_assigned_nodes(),
            &[child]
        );
        assert_eq!(
            removed.slots().assignment_changes()[0].assigned_nodes(),
            &[]
        );
    }

    #[test]
    fn host_child_slot_attribute_mutation_reports_slot_assignment_snapshots() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let shadow_host = host.create_element("section");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let old_slot = host.create_element("slot");
        let new_slot = host.create_element("slot");
        let child = host.create_element("span");

        assert!(host.set_attribute(old_slot, "name", "old"));
        assert!(host.set_attribute(new_slot, "name", "new"));
        assert!(host.set_attribute(child, "slot", "old"));
        assert!(host.append_child(host.document_node_id(), shadow_host));
        assert!(host.append_child(shadow_root, old_slot));
        assert!(host.append_child(shadow_root, new_slot));
        assert!(host.append_child(shadow_host, child));

        let changed = host.set_attribute_effects(child, "slot", "new");
        assert_eq!(changed.slots().assignment_changes().len(), 2);
        assert_eq!(changed.slots().assignment_changes()[0].slot(), old_slot);
        assert_eq!(
            changed.slots().assignment_changes()[0].previous_assigned_nodes(),
            &[child]
        );
        assert_eq!(
            changed.slots().assignment_changes()[0].assigned_nodes(),
            &[]
        );
        assert_eq!(changed.slots().assignment_changes()[1].slot(), new_slot);
        assert_eq!(
            changed.slots().assignment_changes()[1].previous_assigned_nodes(),
            &[]
        );
        assert_eq!(
            changed.slots().assignment_changes()[1].assigned_nodes(),
            &[child]
        );
    }

    #[test]
    fn host_child_slot_snapshot_batch_preserves_duplicate_slotchange_order() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let shadow_host = host.create_element("section");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let old_first = host.create_element("slot");
        let old_second = host.create_element("slot");
        let new_first = host.create_element("slot");
        let new_second = host.create_element("slot");
        for slot in [old_first, old_second] {
            assert!(host.set_attribute(slot, "name", "old"));
            assert!(host.append_child(shadow_root, slot));
        }
        for slot in [new_first, new_second] {
            assert!(host.set_attribute(slot, "name", "new"));
            assert!(host.append_child(shadow_root, slot));
        }
        let child = host.create_element("span");
        assert!(host.set_attribute(child, "slot", "old"));
        assert!(host.append_child(shadow_host, child));

        let changed = host.set_attribute_effects(child, "slot", "new");
        assert_eq!(
            changed.slots().changed_slots(),
            &[old_first, new_first, old_second, new_second]
        );
        assert_eq!(changed.slots().assignment_changes().len(), 2);
        assert_eq!(changed.slots().assignment_changes()[0].slot(), old_first);
        assert_eq!(changed.slots().assignment_changes()[1].slot(), new_first);
    }

    #[test]
    fn shadow_slot_name_index_reuses_light_mutations_and_tracks_shadow_tree_order() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let shadow_host = host.create_element("section");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let slot_a = host.create_element("slot");
        let slot_b = host.create_element("slot");
        assert!(host.set_attribute(slot_a, "name", "a"));
        assert!(host.set_attribute(slot_b, "name", "b"));
        assert!(host.append_child(shadow_root, slot_a));
        assert!(host.append_child(shadow_root, slot_b));

        let child = host.create_element("span");
        assert!(host.set_attribute(child, "slot", "a"));
        let builds_after_shadow_setup = host.shadow_slot_name_index_build_count_for_test();
        assert!(builds_after_shadow_setup > 0);

        assert!(host.append_child(shadow_host, child));
        assert!(host.set_attribute(child, "slot", "b"));
        assert!(host.remove_child(shadow_host, child));
        assert!(host.append_child(shadow_host, child));
        assert_eq!(
            host.shadow_slot_name_index_build_count_for_test(),
            builds_after_shadow_setup,
            "light-tree assignment mutations must reuse the shadow slot index"
        );
        assert_eq!(host.assigned_slot_for_node(child), Some(slot_b));

        let wrapper = host.create_element("div");
        assert!(host.append_child(shadow_root, wrapper));
        assert_eq!(
            host.shadow_slot_name_index_build_count_for_test(),
            builds_after_shadow_setup,
            "invalidation stays lazy until the index is queried again"
        );
        assert_eq!(host.assigned_slot_for_node(child), Some(slot_b));
        let builds_after_shadow_insertion = host.shadow_slot_name_index_build_count_for_test();
        assert_eq!(builds_after_shadow_insertion, builds_after_shadow_setup + 1);

        assert!(host.set_attribute(slot_a, "name", "b"));
        assert_eq!(host.assigned_slot_for_node(child), Some(slot_a));
        let builds_after_name_change = host.shadow_slot_name_index_build_count_for_test();
        assert_eq!(builds_after_name_change, builds_after_shadow_insertion + 1);

        assert!(host.insert_before(shadow_root, slot_b, Some(slot_a)));
        assert_eq!(host.assigned_slot_for_node(child), Some(slot_b));
        assert_eq!(
            host.shadow_slot_name_index_build_count_for_test(),
            builds_after_name_change + 1
        );
    }

    #[test]
    fn shadow_slot_name_indexes_are_isolated_between_nested_shadow_roots() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let outer_host = host.create_element("section");
        let outer_root = host
            .attach_shadow_root(outer_host, "open")
            .expect("outer shadow root");
        let outer_slot = host.create_element("slot");
        assert!(host.set_attribute(outer_slot, "name", "content"));
        assert!(host.append_child(outer_root, outer_slot));

        let inner_host = host.create_element("article");
        let inner_root = host
            .attach_shadow_root(inner_host, "open")
            .expect("inner shadow root");
        let inner_slot = host.create_element("slot");
        assert!(host.set_attribute(inner_slot, "name", "content"));
        assert!(host.append_child(inner_root, inner_slot));
        assert!(host.append_child(outer_root, inner_host));

        let outer_child = host.create_element("span");
        assert!(host.set_attribute(outer_child, "slot", "content"));
        assert!(host.append_child(outer_host, outer_child));
        let inner_child = host.create_element("span");
        assert!(host.set_attribute(inner_child, "slot", "content"));
        assert!(host.append_child(inner_host, inner_child));

        assert_eq!(host.assigned_slot_for_node(outer_child), Some(outer_slot));
        assert_eq!(host.assigned_slot_for_node(inner_child), Some(inner_slot));
        assert_eq!(
            host.assigned_nodes_for_slot_with_options(outer_slot, false),
            vec![outer_child]
        );
        assert_eq!(
            host.assigned_nodes_for_slot_with_options(inner_slot, false),
            vec![inner_child]
        );
    }

    #[test]
    fn shadow_slot_tree_mutations_report_retargeted_assignment_snapshots() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let shadow_host = host.create_element("section");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let old_slot = host.create_element("slot");
        let new_slot = host.create_element("slot");
        let child = host.create_element("span");

        assert!(host.set_attribute(old_slot, "name", "content"));
        assert!(host.set_attribute(new_slot, "name", "content"));
        assert!(host.set_attribute(child, "slot", "content"));
        assert!(host.append_child(host.document_node_id(), shadow_host));
        assert!(host.append_child(shadow_root, old_slot));
        assert!(host.append_child(shadow_host, child));
        assert_eq!(host.assigned_slot_for_node(child), Some(old_slot));

        let inserted = host.insert_before_effects(shadow_root, new_slot, Some(old_slot));
        assert_eq!(inserted.slots().assignment_changes().len(), 2);
        let old_slot_change = inserted
            .slots()
            .assignment_changes()
            .iter()
            .find(|change| change.slot() == old_slot)
            .expect("old slot assignment change");
        assert_eq!(old_slot_change.previous_assigned_nodes(), &[child]);
        assert_eq!(old_slot_change.assigned_nodes(), &[]);
        let new_slot_change = inserted
            .slots()
            .assignment_changes()
            .iter()
            .find(|change| change.slot() == new_slot)
            .expect("new slot assignment change");
        assert_eq!(new_slot_change.previous_assigned_nodes(), &[]);
        assert_eq!(new_slot_change.assigned_nodes(), &[child]);
        assert_eq!(host.assigned_slot_for_node(child), Some(new_slot));

        let removed = host.remove_child_effects(shadow_root, new_slot);
        assert_eq!(removed.slots().assignment_changes().len(), 2);
        let removed_new_slot_change = removed
            .slots()
            .assignment_changes()
            .iter()
            .find(|change| change.slot() == new_slot)
            .expect("removed slot assignment change");
        assert_eq!(removed_new_slot_change.previous_assigned_nodes(), &[child]);
        assert_eq!(removed_new_slot_change.assigned_nodes(), &[]);
        let restored_old_slot_change = removed
            .slots()
            .assignment_changes()
            .iter()
            .find(|change| change.slot() == old_slot)
            .expect("restored slot assignment change");
        assert_eq!(restored_old_slot_change.previous_assigned_nodes(), &[]);
        assert_eq!(restored_old_slot_change.assigned_nodes(), &[child]);
        assert_eq!(host.assigned_slot_for_node(child), Some(old_slot));
    }

    #[test]
    fn connected_shadow_roots_snapshot_tracks_attachment_and_connection_changes() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();
        let connected_host = host.create_element("section");
        assert!(host.append_child(document, connected_host));

        assert!(host.snapshot_connected_shadow_roots().is_empty());
        assert!(host.snapshot_connected_shadow_root_bindings().is_empty());
        let connected_root = host
            .attach_shadow_root(connected_host, "open")
            .expect("connected shadow root");
        assert_eq!(host.snapshot_connected_shadow_roots(), vec![connected_root]);
        assert_eq!(
            host.snapshot_connected_shadow_root_bindings_for_test(),
            vec![ConnectedShadowRootSnapshot {
                host: connected_host,
                root: connected_root
            }]
        );

        let detached_host = host.create_element("article");
        let detached_root = host
            .attach_shadow_root(detached_host, "open")
            .expect("detached shadow root");
        assert_eq!(host.snapshot_connected_shadow_roots(), vec![connected_root]);
        assert_eq!(
            host.snapshot_connected_shadow_root_bindings_for_test(),
            vec![ConnectedShadowRootSnapshot {
                host: connected_host,
                root: connected_root
            }]
        );

        assert!(host.append_child(document, detached_host));
        let roots = host.snapshot_connected_shadow_roots_for_test();
        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&connected_root));
        assert!(roots.contains(&detached_root));
        assert_eq!(
            host.snapshot_connected_shadow_root_bindings_for_test(),
            vec![
                ConnectedShadowRootSnapshot {
                    host: connected_host,
                    root: connected_root
                },
                ConnectedShadowRootSnapshot {
                    host: detached_host,
                    root: detached_root
                }
            ]
        );

        assert!(host.remove_child(document, connected_host));
        assert_eq!(host.snapshot_connected_shadow_roots(), vec![detached_root]);
        assert_eq!(
            host.snapshot_connected_shadow_root_bindings_for_test(),
            vec![ConnectedShadowRootSnapshot {
                host: detached_host,
                root: detached_root
            }]
        );
    }

    #[test]
    fn tree_scope_versions_are_isolated_per_document() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let parent_document = host.document_node_id();
        let child_document = host.create_detached_html_document();
        let child_host = host.create_parser_element_without_attributes_for_document(
            child_document,
            "section".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(host.append_child(child_document, child_host));
        host.attach_shadow_root(child_host, "open")
            .expect("detached child-document ShadowRoot");
        assert_eq!(host.document_tree_scope_version(parent_document), 0);
        assert_eq!(host.document_tree_scope_version(child_document), 0);

        host.mark_subtree_connected_preserving_owner_document(child_document);
        assert_eq!(
            host.document_tree_scope_version(child_document),
            1,
            "connecting the child Document must advance only its TreeScope universe"
        );
        assert_eq!(host.document_tree_scope_version(parent_document), 0);

        let parent_host = host.create_element("article");
        assert!(host.append_child(parent_document, parent_host));
        host.attach_shadow_root(parent_host, "open")
            .expect("connected parent-document ShadowRoot");
        assert_eq!(host.document_tree_scope_version(parent_document), 1);
        assert_eq!(
            host.document_tree_scope_version(child_document),
            1,
            "a parent-document binding must not invalidate the child Document"
        );
    }

    #[test]
    fn implicit_move_to_detached_parent_invalidates_old_document_tree_scopes() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();
        let shadow_host = host.create_element("section");
        assert!(host.append_child(document, shadow_host));
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("connected shadow root");
        let version_before_move = host.document_tree_scope_version(document);
        assert_eq!(host.snapshot_connected_shadow_roots(), vec![shadow_root]);

        let detached_parent = host.create_element("main");
        assert!(host.append_child(detached_parent, shadow_host));

        assert_eq!(
            host.document_tree_scope_version(document),
            version_before_move + 1,
            "the implicit removal must invalidate the old Document TreeScope universe",
        );
        assert_eq!(host.connected_shadow_roots_cache_versions_for_test(), None);
        assert!(host.snapshot_connected_shadow_roots().is_empty());
    }

    #[test]
    fn implicit_move_of_nested_shadow_tree_invalidates_old_document_once() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();
        let outer_host = host.create_element("section");
        assert!(host.append_child(document, outer_host));
        let outer_root = host
            .attach_shadow_root(outer_host, "open")
            .expect("outer connected shadow root");
        let inner_host = host.create_element("article");
        assert!(host.append_child(outer_root, inner_host));
        let inner_root = host
            .attach_shadow_root(inner_host, "open")
            .expect("nested connected shadow root");
        let version_before_move = host.document_tree_scope_version(document);
        let roots = host.snapshot_connected_shadow_roots();
        assert!(roots.contains(&outer_root));
        assert!(roots.contains(&inner_root));

        let detached_parent = host.create_element("main");
        assert!(host.append_child(detached_parent, outer_host));

        assert_eq!(
            host.document_tree_scope_version(document),
            version_before_move + 1,
            "one subtree move must advance the old Document only once",
        );
        assert!(!host.is_connected(outer_root));
        assert!(!host.is_connected(inner_root));
        assert!(host.snapshot_connected_shadow_roots().is_empty());
    }

    #[test]
    fn implicit_cross_document_move_invalidates_both_tree_scope_universes() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let old_document = host.document_node_id();
        let new_document = host.create_detached_html_document();
        let new_parent = host.create_parser_element_without_attributes_for_document(
            new_document,
            "main".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(host.append_child(new_document, new_parent));
        host.mark_subtree_connected_preserving_owner_document(new_document);

        let shadow_host = host.create_element("section");
        assert!(host.append_child(old_document, shadow_host));
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("old-document shadow root");
        let old_version_before_move = host.document_tree_scope_version(old_document);
        let new_version_before_move = host.document_tree_scope_version(new_document);

        assert!(host.append_child(new_parent, shadow_host));

        assert_eq!(host.owner_document_handle(shadow_host), Some(new_document));
        assert!(host.is_connected(shadow_root));
        assert_eq!(
            host.document_tree_scope_version(old_document),
            old_version_before_move + 1,
            "the implicit removal must invalidate the departed Document",
        );
        assert_eq!(
            host.document_tree_scope_version(new_document),
            new_version_before_move + 1,
            "the insertion must invalidate the destination Document",
        );
    }

    #[test]
    fn connected_shadow_roots_snapshot_cache_uses_shadow_connection_version() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();
        let connected_host = host.create_element("section");
        assert!(host.append_child(document, connected_host));
        let connected_root = host
            .attach_shadow_root(connected_host, "open")
            .expect("connected shadow root");
        let detached_host = host.create_element("article");
        let detached_root = host
            .attach_shadow_root(detached_host, "open")
            .expect("detached shadow root");

        assert_eq!(
            host.snapshot_connected_shadow_root_bindings(),
            vec![ConnectedShadowRootSnapshot {
                host: connected_host,
                root: connected_root
            }]
        );
        let cached_versions = host
            .connected_shadow_roots_cache_versions_for_test()
            .expect("connected shadow root snapshot cache should be populated");

        let plain = host.create_element("main");
        assert!(host.append_child(document, plain));
        assert!(host.set_attribute(plain, "data-state", "active"));
        assert_eq!(
            host.connected_shadow_roots_cache_versions_for_test(),
            Some(cached_versions)
        );
        assert_eq!(host.snapshot_connected_shadow_roots(), vec![connected_root]);
        assert_eq!(
            host.snapshot_connected_shadow_root_bindings(),
            vec![ConnectedShadowRootSnapshot {
                host: connected_host,
                root: connected_root
            }]
        );
        assert_eq!(
            host.connected_shadow_roots_cache_versions_for_test(),
            Some(cached_versions)
        );

        assert!(host.append_child(document, detached_host));
        assert_eq!(host.connected_shadow_roots_cache_versions_for_test(), None);
        let roots = host.snapshot_connected_shadow_roots_for_test();
        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&connected_root));
        assert!(roots.contains(&detached_root));
        assert_eq!(
            host.snapshot_connected_shadow_root_bindings_for_test(),
            vec![
                ConnectedShadowRootSnapshot {
                    host: connected_host,
                    root: connected_root
                },
                ConnectedShadowRootSnapshot {
                    host: detached_host,
                    root: detached_root
                }
            ]
        );
        assert_ne!(
            host.connected_shadow_roots_cache_versions_for_test(),
            Some(cached_versions)
        );
    }

    #[test]
    fn preserving_owner_document_connection_updates_connected_shadow_roots_snapshot() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();
        let active_host = host.create_element("section");
        assert!(host.append_child(document, active_host));
        let active_root = host
            .attach_shadow_root(active_host, "open")
            .expect("active shadow root");

        assert_eq!(host.snapshot_connected_shadow_roots(), vec![active_root]);
        assert!(
            host.connected_shadow_roots_cache_versions_for_test()
                .is_some()
        );

        let child_document = host.create_detached_html_document();
        let child_host = host.create_parser_element_without_attributes_for_document(
            child_document,
            "article".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(host.append_child(child_document, child_host));
        let child_root = host
            .attach_shadow_root(child_host, "open")
            .expect("child shadow root");

        assert_eq!(
            host.snapshot_connected_shadow_roots(),
            vec![active_root],
            "disconnected child-document shadow roots must not enter the snapshot"
        );
        assert!(
            host.connected_shadow_roots_cache_versions_for_test()
                .is_some()
        );
        host.mark_subtree_connected_preserving_owner_document(child_document);
        assert_eq!(host.connected_shadow_roots_cache_versions_for_test(), None);
        assert!(host.is_connected(child_root));
        let roots = host.snapshot_connected_shadow_roots_for_test();
        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&active_root));
        assert!(roots.contains(&child_root));

        host.mark_subtree_disconnected_preserving_owner_document(child_document);
        assert_eq!(host.connected_shadow_roots_cache_versions_for_test(), None);
        assert!(!host.is_connected(child_root));
        assert_eq!(host.snapshot_connected_shadow_roots(), vec![active_root]);
    }

    #[test]
    fn connected_shadow_roots_related_lookup_is_empty_without_shadow_roots() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();
        let root = host.create_element("main");
        let child = host.create_element("span");
        assert!(host.append_child(document, root));
        assert!(host.append_child(root, child));

        assert!(
            host.snapshot_connected_shadow_root_bindings_related_to_light_tree_handle_for_test(
                root,
            )
            .is_empty()
        );
        assert_eq!(
            host.connected_shadow_roots_cache_versions_for_test(),
            None,
            "the no-shadow fast path must not materialize a document-wide snapshot"
        );
    }

    #[test]
    fn connected_shadow_roots_related_to_light_tree_handle_use_dom_membership() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();
        let ancestor = host.create_element("section");
        let descendant_host = host.create_element("article");
        let light_child = host.create_element("span");
        let sibling_host = host.create_element("aside");
        let detached_host = host.create_element("nav");

        let ancestor_root = host
            .attach_shadow_root(ancestor, "open")
            .expect("ancestor shadow root");
        let descendant_root = host
            .attach_shadow_root(descendant_host, "open")
            .expect("descendant shadow root");
        let sibling_root = host
            .attach_shadow_root(sibling_host, "open")
            .expect("sibling shadow root");
        let detached_root = host
            .attach_shadow_root(detached_host, "open")
            .expect("detached shadow root");
        let shadow_child = host.create_element("b");

        assert!(host.append_child(document, ancestor));
        assert!(host.append_child(ancestor, descendant_host));
        assert!(host.append_child(descendant_host, light_child));
        assert!(host.append_child(document, sibling_host));
        assert!(host.append_child(ancestor_root, shadow_child));

        let document_bindings = host
            .snapshot_connected_shadow_root_bindings_related_to_light_tree_handle_for_test(
                document,
            );
        assert_eq!(
            document_bindings,
            vec![
                ConnectedShadowRootSnapshot {
                    host: ancestor,
                    root: ancestor_root
                },
                ConnectedShadowRootSnapshot {
                    host: descendant_host,
                    root: descendant_root
                },
                ConnectedShadowRootSnapshot {
                    host: sibling_host,
                    root: sibling_root
                }
            ]
        );

        assert_eq!(
            host.snapshot_connected_shadow_root_bindings_related_to_light_tree_handle_for_test(
                light_child,
            ),
            vec![
                ConnectedShadowRootSnapshot {
                    host: ancestor,
                    root: ancestor_root
                },
                ConnectedShadowRootSnapshot {
                    host: descendant_host,
                    root: descendant_root
                }
            ]
        );
        assert_eq!(
            host.snapshot_connected_shadow_root_bindings_related_to_light_tree_handle_for_test(
                ancestor,
            ),
            vec![
                ConnectedShadowRootSnapshot {
                    host: ancestor,
                    root: ancestor_root
                },
                ConnectedShadowRootSnapshot {
                    host: descendant_host,
                    root: descendant_root
                }
            ]
        );
        assert_eq!(
            host.snapshot_connected_shadow_root_bindings_related_to_light_tree_handle_for_test(
                shadow_child,
            ),
            Vec::<ConnectedShadowRootSnapshot>::new()
        );
        assert!(
            !host
                .snapshot_connected_shadow_root_bindings_related_to_light_tree_handle_for_test(
                    light_child,
                )
                .contains(&ConnectedShadowRootSnapshot {
                    host: sibling_host,
                    root: sibling_root
                })
        );
        assert!(
            !host
                .snapshot_connected_shadow_root_bindings_related_to_light_tree_handle_for_test(
                    light_child,
                )
                .contains(&ConnectedShadowRootSnapshot {
                    host: detached_host,
                    root: detached_root
                })
        );
    }

    #[test]
    fn connected_shadow_roots_related_lookup_uses_indexed_cache_without_forcing_snapshot() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_node_id();
        let shadow_host = host.create_element("section");
        let light_child = host.create_element("span");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");

        assert!(host.append_child(document, shadow_host));
        assert!(host.append_child(shadow_host, light_child));
        assert_eq!(host.connected_shadow_roots_cache_versions_for_test(), None);

        assert_eq!(
            host.snapshot_connected_shadow_root_bindings_related_to_light_tree_handle_for_test(
                light_child,
            ),
            vec![ConnectedShadowRootSnapshot {
                host: shadow_host,
                root: shadow_root
            }]
        );
        assert_eq!(
            host.connected_shadow_roots_cache_versions_for_test(),
            None,
            "per-handle related lookup should not force a full connected-shadow snapshot"
        );

        assert_eq!(
            host.snapshot_connected_shadow_root_bindings(),
            vec![ConnectedShadowRootSnapshot {
                host: shadow_host,
                root: shadow_root
            }]
        );
        assert_eq!(
            host.connected_shadow_roots_cache_binding_counts_for_test(),
            Some((1, 1)),
            "document-wide snapshot cache should index connected shadow bindings by host"
        );
        assert_eq!(
            host.snapshot_connected_shadow_root_bindings_related_to_light_tree_handle_for_test(
                light_child,
            ),
            vec![ConnectedShadowRootSnapshot {
                host: shadow_host,
                root: shadow_root
            }]
        );
    }

    #[test]
    fn template_content_uses_separate_owner_document_for_adoption() {
        let mut dom = NativeDom::new_html(test_url());
        let template = dom.create_element("template");
        let content = dom
            .node(template)
            .and_then(Node::as_element)
            .and_then(Element::template_contents)
            .expect("template content");
        let content_owner = dom
            .node(content)
            .and_then(Node::owner_document)
            .expect("template content owner document");
        assert_ne!(content_owner, dom.document_node_id());

        let child = dom.create_element("span");
        assert_eq!(
            dom.node(child).and_then(Node::owner_document),
            Some(dom.document_node_id())
        );

        assert!(dom.append_child(content, child));
        assert_eq!(
            dom.node(child).and_then(Node::owner_document),
            Some(content_owner)
        );

        let body = dom.create_element("body");
        assert!(dom.append_child(dom.document_node_id(), body));
        assert!(dom.append_child(body, child));
        assert_eq!(
            dom.node(child).and_then(Node::owner_document),
            Some(dom.document_node_id())
        );
    }

    #[test]
    fn cloned_template_content_keeps_template_owner_document() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let template = host.create_element("template");
        let content = host
            .node(template)
            .and_then(Node::as_element)
            .and_then(Element::template_contents)
            .expect("template content");
        let child = host.create_element("span");
        assert!(host.append_child(content, child));

        let clone = host.clone_node(template, true).expect("template clone");
        let cloned_content = host
            .node(clone)
            .and_then(Node::as_element)
            .and_then(Element::template_contents)
            .expect("cloned template content");
        let cloned_content_owner = host
            .node(cloned_content)
            .and_then(Node::owner_document)
            .expect("cloned template content owner document");
        let cloned_child = host
            .node(cloned_content)
            .and_then(Node::first_child)
            .expect("cloned template content child");

        assert_ne!(cloned_content_owner, host.document_handle());
        assert_eq!(
            host.node(cloned_child).and_then(Node::owner_document),
            Some(cloned_content_owner)
        );
    }

    #[test]
    fn cloned_element_preserves_cryptographic_nonce_separately_from_content_attribute() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let element = host.create_element("div");
        assert!(host.set_attribute(element, "nonce", ""));
        assert!(host.set_cryptographic_nonce(element, Some("secret".to_owned())));

        let clone = host.clone_node(element, false).expect("element clone");
        let cloned_element = host
            .node(clone)
            .and_then(Node::as_element)
            .expect("cloned element");

        assert_eq!(cloned_element.attribute("nonce"), Some(""));
        assert_eq!(cloned_element.cryptographic_nonce(), Some("secret"));
    }

    #[test]
    fn deep_cloned_document_owns_its_cloned_descendants() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_handle();
        let html = host.create_element("html");
        let body = host.create_element("body");
        assert!(host.append_child(document, html));
        assert!(host.append_child(html, body));

        let cloned_document = host.clone_node(document, true).expect("document clone");
        let cloned_html = host
            .node(cloned_document)
            .and_then(Node::first_child)
            .expect("cloned document element");
        let cloned_body = host
            .node(cloned_html)
            .and_then(Node::first_child)
            .expect("cloned body");

        assert_eq!(
            host.node(cloned_document).and_then(Node::owner_document),
            None
        );
        assert_eq!(
            host.node(cloned_html).and_then(Node::owner_document),
            Some(cloned_document)
        );
        assert_eq!(
            host.node(cloned_body).and_then(Node::owner_document),
            Some(cloned_document)
        );
        assert_eq!(host.root_node_handle(cloned_body), Some(cloned_document));
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(
                cloned_document,
                "body",
                false,
                true,
            ),
            vec![cloned_body]
        );
    }

    #[test]
    fn shallow_cloned_template_content_is_empty() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let template = host.create_element("template");
        let content = host
            .node(template)
            .and_then(Node::as_element)
            .and_then(Element::template_contents)
            .expect("template content");
        let child = host.create_element("span");
        assert!(host.append_child(content, child));

        let clone = host.clone_node(template, false).expect("template clone");
        let cloned_content = host
            .node(clone)
            .and_then(Node::as_element)
            .and_then(Element::template_contents)
            .expect("cloned template content");

        assert_eq!(host.node(cloned_content).and_then(Node::first_child), None);
    }

    #[test]
    fn import_foreign_node_with_shadow_roots_records_cloned_handles() {
        let mut source = DomHost::from_dom(NativeDom::new_html(test_url()));
        source.reset_html_document_shell();
        let body = source.document_body_handle().expect("source body");
        let wrapper = source.create_element("div");
        let script = source.create_element("script");
        assert!(source.append_child(wrapper, script));
        assert!(source.append_child(body, wrapper));

        let mut target = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://target.test/").expect("target url"),
        ));
        let target_document = target.document_handle();
        let mut cloned_handles = std::collections::HashMap::new();
        let cloned_wrapper = target
            .import_foreign_node_with_shadow_roots_and_handle_map(
                target_document,
                &source,
                wrapper,
                true,
                &mut cloned_handles,
            )
            .expect("foreign wrapper should import");
        let cloned_script = cloned_handles
            .get(&script)
            .copied()
            .expect("script handle should be mapped");

        assert_eq!(cloned_handles.get(&wrapper).copied(), Some(cloned_wrapper));
        assert!(target.is_html_element_named(cloned_script, "script"));
        assert_eq!(
            target.node(cloned_script).and_then(Node::owner_document),
            Some(target_document)
        );
    }

    #[test]
    fn option_text_streams_ascii_whitespace_across_descendant_boundaries() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let empty_option = host.create_element("option");
        let whitespace_option = host.create_element("option");
        let whitespace = host.create_text_node(" \t\n\u{000c}\r");
        assert!(host.append_child(whitespace_option, whitespace));
        for option in [empty_option, whitespace_option] {
            let element = host
                .node(option)
                .and_then(Node::as_element)
                .expect("option element");
            assert_eq!(element.option_text(host.dom(), option), "");
        }

        let option = host.create_element("option");
        let prefix = host.create_text_node(" \tbe");
        assert!(host.append_child(option, prefix));

        let span = host.create_element("span");
        let suffix = host.create_text_node("fore");
        assert!(host.append_child(span, suffix));
        assert!(host.append_child(option, span));

        let comment = host.create_comment(" ignored ");
        assert!(host.append_child(option, comment));
        for namespace in ["http://www.w3.org/1999/xhtml", "http://www.w3.org/2000/svg"] {
            let script = host
                .create_element_ns(Some(namespace), "script")
                .expect("script element");
            let ignored = host.create_text_node(" ignored ");
            assert!(host.append_child(script, ignored));
            assert!(host.append_child(option, script));
        }

        let math_script = host
            .create_element_ns(Some("http://www.w3.org/1998/Math/MathML"), "script")
            .expect("MathML script element");
        let math_text = host.create_text_node(" \nmath ");
        assert!(host.append_child(math_script, math_text));
        assert!(host.append_child(option, math_script));
        let cdata = host.create_cdata_section("\u{a0}tail \r");
        assert!(host.append_child(option, cdata));

        let option_element = host
            .node(option)
            .and_then(Node::as_element)
            .expect("option element");
        assert_eq!(
            option_element.option_text(host.dom(), option),
            "before math \u{a0}tail"
        );
        assert_eq!(
            option_element.option_value(host.dom(), option),
            "before math \u{a0}tail"
        );
        assert_eq!(
            option_element.option_label(host.dom(), option),
            "before math \u{a0}tail"
        );
    }

    #[test]
    fn document_base_url_uses_first_supported_base_href() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/path/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let head = host.document_head_handle().expect("document head");

        let blocked = host.create_element("base");
        host.set_attribute(blocked, "href", "data:/,ignored");
        host.append_child(head, blocked);
        let second = host.create_element("base");
        host.set_attribute(second, "href", "https://cdn.example/assets/");
        host.append_child(head, second);

        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/page.html"
        );

        host.remove_child(head, blocked);
        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://cdn.example/assets/"
        );
    }

    #[test]
    fn document_base_url_treats_empty_base_href_as_document_url() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/path/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let head = host.document_head_handle().expect("document head");

        let empty = host.create_element("base");
        host.set_attribute(empty, "href", "");
        host.append_child(head, empty);
        let second = host.create_element("base");
        host.set_attribute(second, "href", "https://cdn.example/assets/");
        host.append_child(head, second);

        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/page.html"
        );
    }

    #[test]
    fn empty_base_href_does_not_override_a_document_base_url_override() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/path/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let head = host.document_head_handle().expect("document head");
        host.node_mut(host.document_handle())
            .and_then(|node| node.data_mut().as_document_mut())
            .expect("document")
            .set_base_url_override(Some(
                url::Url::parse("https://override.test/root/").unwrap(),
            ));

        let base = host.create_element("base");
        assert!(host.set_attribute(base, "href", " \n\t "));
        assert!(host.append_child(head, base));

        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://override.test/root/"
        );
    }

    #[test]
    fn srcdoc_document_keeps_url_separate_from_fallback_base_url() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("about:srcdoc").unwrap(),
        ));
        let document = host.document_handle();
        assert!(host.set_document_fallback_base_url_for_handle(
            document,
            Some(url::Url::parse("https://parent.test/path/page.html").unwrap()),
        ));
        host.reset_html_document_shell();

        assert_eq!(
            host.document_url().expect("document URL").as_str(),
            "about:srcdoc"
        );
        assert_eq!(
            host.document_base_url()
                .expect("fallback base URL")
                .as_str(),
            "https://parent.test/path/page.html"
        );

        let head = host.document_head_handle().expect("document head");
        let base = host.create_element("base");
        host.set_attribute(base, "href", "assets/");
        host.append_child(head, base);
        assert_eq!(
            host.document_base_url().expect("element base URL").as_str(),
            "https://parent.test/path/assets/"
        );
    }

    #[test]
    fn document_base_url_uses_body_base_href_in_tree_order() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/path/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");

        let base = host.create_element("base");
        host.set_attribute(base, "href", "scripts/foo/");
        host.append_child(body, base);

        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/scripts/foo/"
        );
    }

    #[test]
    fn document_base_href_and_target_have_independent_tree_order_winners() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/path/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let head = host.document_head_handle().expect("document head");

        let href_base = host.create_element("base");
        assert!(host.set_attribute(href_base, "href", "assets/"));
        assert!(host.append_child(head, href_base));

        let target_base = host.create_element("base");
        assert!(host.set_attribute(target_base, "target", "_blank"));
        let insertion = host.append_child_effects(head, target_base);

        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/assets/"
        );
        assert_eq!(host.document_base_target(), Some("_blank"));
        assert!(insertion.did_change());

        let target_change = host.set_attribute_effects(href_base, "target", "_self");
        assert_eq!(host.document_base_target(), Some("_self"));
        assert!(target_change.did_change());
    }

    #[test]
    fn base_processing_tracks_tree_order_reparenting_and_attribute_removal() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/path/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let head = host.document_head_handle().expect("document head");

        let first = host.create_element("base");
        assert!(host.set_attribute(first, "href", "first/"));
        assert!(host.set_attribute(first, "target", "first"));
        assert!(host.append_child(head, first));
        let second = host.create_element("base");
        assert!(host.set_attribute(second, "href", "second/"));
        assert!(host.set_attribute(second, "target", "second"));
        assert!(host.append_child(head, second));

        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/first/"
        );
        assert_eq!(host.document_base_target(), Some("first"));

        assert!(host.insert_before(head, second, Some(first)));
        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/second/"
        );
        assert_eq!(host.document_base_target(), Some("second"));

        assert!(host.remove_attribute(second, "href"));
        assert!(host.remove_attribute(second, "target"));
        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/first/"
        );
        assert_eq!(host.document_base_target(), Some("first"));
    }

    #[test]
    fn base_state_is_isolated_from_shadow_trees_and_between_documents() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://main.test/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");

        let shadow_host = host.create_element("section");
        assert!(host.append_child(body, shadow_host));
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let shadow_base = host.create_element("base");
        assert!(host.set_attribute(shadow_base, "href", "https://shadow.test/"));
        let shadow_insertion = host.append_child_effects(shadow_root, shadow_base);
        assert!(shadow_insertion.did_change());
        assert_eq!(
            host.document_base_url().expect("main base URL").as_str(),
            "https://main.test/page.html"
        );

        let detached_document = host.create_detached_html_document_with_url(
            url::Url::parse("https://detached.test/path/page.html").unwrap(),
        );
        let detached_base = host.create_element("base");
        assert!(host.set_attribute(detached_base, "href", "assets/"));
        let detached_insertion = host.append_child_effects(detached_document, detached_base);
        assert!(detached_insertion.did_change());
        assert_eq!(
            host.document_base_url_for_handle(detached_document)
                .expect("detached base URL")
                .as_str(),
            "https://detached.test/path/assets/"
        );
        assert_eq!(
            host.document_base_url().expect("main base URL").as_str(),
            "https://main.test/page.html"
        );
    }

    #[test]
    fn namespaced_href_does_not_participate_in_document_base_state() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/page.html").unwrap(),
        ));
        let base = host.create_element("base");
        assert!(host.set_attribute(base, "href", "/accepted/"));
        assert!(host.append_child(host.document_handle(), base));

        let effects = host.set_attribute_ns_effects(
            base,
            Some("urn:test"),
            None,
            "href",
            "href",
            "/ignored/",
        );

        assert!(effects.did_change());
        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/accepted/"
        );
    }

    #[test]
    fn document_base_url_is_synchronous_without_cross_owner_followup() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/path/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let head = host.document_head_handle().expect("document head");
        let body = host.document_body_handle().expect("document body");
        let base = host.create_element("base");
        host.set_attribute(base, "href", "scripts/");
        let insertion = host.append_child_effects(head, base);

        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/scripts/"
        );
        assert!(insertion.did_change());

        let unrelated_attribute = host.set_attribute_effects(body, "class", "updated");
        assert!(unrelated_attribute.did_change());

        let unrelated = host.create_element("section");
        let unrelated_insertion = host.append_child_effects(body, unrelated);
        assert!(unrelated_insertion.did_change());
        let unrelated_removal = host.remove_child_effects(body, unrelated);
        assert!(unrelated_removal.did_change());
        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/scripts/"
        );

        let href_change = host.set_attribute_effects(base, "href", "assets/");
        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/assets/"
        );
        assert!(href_change.did_change());
    }

    #[test]
    fn document_base_url_processes_base_in_inserted_wrapper_subtree() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/path/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");
        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/page.html"
        );

        let wrapper = host.create_element("section");
        let base = host.create_element("base");
        assert!(host.set_attribute(base, "href", "assets/"));
        assert!(host.append_child(wrapper, base));
        let effects = host.append_child_effects(body, wrapper);

        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/assets/"
        );
        assert!(effects.did_change());
    }

    #[test]
    fn document_fragment_base_owner_lifecycle_updates_on_insert_and_removal() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/path/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");
        let fragment = host.create_document_fragment();
        let ordinary = host.create_element("section");
        let wrapper = host.create_element("section");
        let base = host.create_element("base");
        assert!(host.set_attribute(base, "href", "assets/"));
        assert!(host.append_child(wrapper, base));
        assert!(host.append_child(fragment, ordinary));
        assert!(host.append_child(fragment, wrapper));

        assert!(host.append_child(body, fragment));
        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/assets/"
        );

        assert!(host.remove_child(body, wrapper));
        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/path/page.html"
        );
    }

    #[test]
    fn document_base_url_recomputes_when_document_url_changes() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/first/page.html").unwrap(),
        ));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");
        let base = host.create_element("base");
        host.set_attribute(base, "href", "assets/");
        host.append_child(body, base);

        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/first/assets/"
        );
        assert!(
            host.set_document_url(
                url::Url::parse("https://example.test/second/page.html").unwrap()
            )
        );
        assert_eq!(
            host.document_base_url().expect("base URL").as_str(),
            "https://example.test/second/assets/"
        );
    }

    #[test]
    fn detached_document_base_url_cache_tracks_base_tree_mutations() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.create_detached_html_document_with_url(
            url::Url::parse("about:blank").expect("valid synthetic document URL"),
        );
        let html = host.create_parser_element_without_attributes_for_document(
            document,
            "html".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        let head = host.create_parser_element_without_attributes_for_document(
            document,
            "head".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(host.append_child(document, html));
        assert!(host.append_child(html, head));
        assert_eq!(
            host.document_base_url_for_handle(document)
                .expect("synthetic document base URL")
                .as_str(),
            "about:blank"
        );

        let base = host.create_parser_element_without_attributes_for_document(
            document,
            "base".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(host.set_attribute(base, "href", "https://first.example/"));
        assert!(host.append_child(head, base));
        assert_eq!(
            host.document_base_url_for_handle(document)
                .expect("inserted base URL")
                .as_str(),
            "https://first.example/"
        );

        assert!(host.set_attribute(base, "href", "https://second.example/"));
        assert_eq!(
            host.document_base_url_for_handle(document)
                .expect("mutated base URL")
                .as_str(),
            "https://second.example/"
        );

        assert!(host.remove_child(head, base));
        assert_eq!(
            host.document_base_url_for_handle(document)
                .expect("removed base URL")
                .as_str(),
            "about:blank"
        );
    }

    #[test]
    fn ensure_html_document_body_synthesizes_missing_body_without_replacing_frameset() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let html = host.create_parser_element(
            "html".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
            Vec::new(),
        );
        assert!(host.append_child(host.document_handle(), html));

        let body = host
            .ensure_html_document_body()
            .expect("missing body should be synthesized");
        assert!(host.is_html_element_named(body, "body"));
        assert_eq!(host.node(body).and_then(Node::parent_node), Some(html));
        assert!(
            host.node(body)
                .is_some_and(|node| node.flags().parser_created())
        );
        assert_eq!(host.ensure_html_document_body(), Some(body));

        let mut frameset_host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let frameset_html = frameset_host.create_parser_element(
            "html".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
            Vec::new(),
        );
        let frameset = frameset_host.create_parser_element(
            "frameset".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
            Vec::new(),
        );
        assert!(frameset_host.append_child(frameset_host.document_handle(), frameset_html));
        assert!(frameset_host.append_child(frameset_html, frameset));

        assert_eq!(frameset_host.ensure_html_document_body(), None);
    }

    #[test]
    fn ensure_html_document_shell_synthesizes_missing_html_head_and_body() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let doctype = host.create_document_type("html", "", "");
        assert!(host.append_child(host.document_handle(), doctype));

        let body = host
            .ensure_html_document_shell()
            .expect("missing HTML shell should be synthesized");
        let html = host
            .document_element_handle()
            .expect("document element should be synthesized");
        let head = host
            .document_head_handle()
            .expect("document head should be synthesized");

        assert!(host.is_html_element_named(html, "html"));
        assert!(host.is_html_element_named(head, "head"));
        assert!(host.is_html_element_named(body, "body"));
        assert_eq!(
            host.node(doctype).and_then(Node::parent_node),
            Some(host.document_handle())
        );
        assert_eq!(
            host.node(html).and_then(Node::parent_node),
            Some(host.document_handle())
        );
        assert_eq!(host.node(head).and_then(Node::parent_node), Some(html));
        assert_eq!(host.node(body).and_then(Node::parent_node), Some(html));
        assert_eq!(host.nth_child(host.document_handle(), 0), Some(doctype));
        assert_eq!(host.nth_child(host.document_handle(), 1), Some(html));
    }

    #[test]
    fn ensure_html_document_shell_inserts_missing_head_before_body() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let html = host.create_parser_element(
            "html".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
            Vec::new(),
        );
        let body = host.create_parser_element(
            "body".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
            Vec::new(),
        );
        assert!(host.append_child(host.document_handle(), html));
        assert!(host.append_child(html, body));

        assert_eq!(host.ensure_html_document_shell(), Some(body));
        let head = host
            .document_head_handle()
            .expect("head should be synthesized");
        assert_eq!(host.nth_child(html, 0), Some(head));
        assert_eq!(host.nth_child(html, 1), Some(body));
    }

    #[test]
    fn moving_subtree_root_preserves_children() {
        let mut dom = NativeDom::new_html(test_url());
        let old_parent = dom.create_element("div");
        let new_parent = dom.create_element("section");
        let root = dom.create_element("article");
        let child = dom.create_element("span");
        let text = dom.create_text_node("nested");

        assert!(dom.append_child(old_parent, root));
        assert!(dom.append_child(root, child));
        assert!(dom.append_child(child, text));

        assert!(dom.append_child(new_parent, root));

        assert_eq!(dom.parent_node(root), Some(new_parent));
        assert_eq!(dom.child_ids(root).collect::<Vec<_>>(), vec![child]);
        assert_eq!(dom.child_ids(child).collect::<Vec<_>>(), vec![text]);
        assert_eq!(dom.text_content(root).as_deref(), Some("nested"));
    }

    #[test]
    fn child_index_and_nth_child_follow_sibling_links() {
        let mut dom = NativeDom::new_html(test_url());
        let parent = dom.create_element("section");
        let first = dom.create_element("div");
        let second = dom.create_element("span");
        let third = dom.create_element("button");
        let unrelated_parent = dom.create_element("article");

        assert!(dom.append_child(parent, first));
        assert!(dom.append_child(parent, second));
        assert!(dom.append_child(parent, third));

        assert_eq!(dom.nth_child(parent, 0), Some(first));
        assert_eq!(dom.nth_child(parent, 1), Some(second));
        assert_eq!(dom.nth_child(parent, 2), Some(third));
        assert_eq!(dom.nth_child(parent, 3), None);
        assert_eq!(
            dom.child_ids(parent).collect::<Vec<_>>(),
            vec![first, second, third]
        );
        assert_eq!(
            dom.find_child(parent, |child| child == second),
            Some(second)
        );
        assert_eq!(dom.child_index(parent, first), Some(0));
        assert_eq!(dom.child_index(parent, second), Some(1));
        assert_eq!(dom.child_index(parent, third), Some(2));
        assert_eq!(dom.child_index(unrelated_parent, second), None);
    }

    #[test]
    fn removing_node_preserves_its_descendant_links() {
        let mut dom = NativeDom::new_html(test_url());
        let parent = dom.create_element("section");
        let child = dom.create_element("div");
        let grandchild = dom.create_text_node("kept");

        assert!(dom.append_child(parent, child));
        assert!(dom.append_child(child, grandchild));
        assert!(dom.remove_child(parent, child));

        assert_eq!(dom.parent_node(child), None);
        assert_eq!(dom.first_child(child), Some(grandchild));
        assert_eq!(dom.parent_node(grandchild), Some(child));
        assert_eq!(dom.text_content(child).as_deref(), Some("kept"));
    }

    #[test]
    fn native_dom_text_html_and_script_walks_handle_deep_trees_iteratively() {
        const DEPTH: usize = 4096;

        let mut dom = NativeDom::new_html(test_url());
        let root = dom.create_element("div");
        assert!(dom.append_child(dom.document_node_id(), root));

        let mut parent = root;
        for _ in 0..DEPTH {
            let child = dom.create_element("div");
            assert!(dom.append_child(parent, child));
            parent = child;
        }

        let text = dom.create_text_node("leaf");
        let script = dom.create_element("script");
        let script_text = dom.create_text_node("code");
        assert!(dom.append_child(parent, text));
        assert!(dom.append_child(parent, script));
        assert!(dom.append_child(script, script_text));

        assert_eq!(dom.text_content(root).as_deref(), Some("leafcode"));
        assert_eq!(dom.document_order_script_handles(), vec![script]);
        assert_eq!(
            dom.connected_script_handles(dom.document_node_id()),
            vec![script]
        );

        let outer = dom.outer_html(root).expect("root outerHTML");
        assert_eq!(outer.matches("<div").count(), DEPTH + 1);
        assert!(outer.contains("leaf<script>code</script></div>"));
        assert!(outer.ends_with("</div>"));

        assert_eq!(
            dom.inner_html(parent).as_deref(),
            Some("leaf<script>code</script>")
        );
        assert!(dom.serialize_document().contains("<script>code</script>"));
    }

    #[test]
    fn dom_host_html_and_script_walks_handle_deep_shadow_including_trees_iteratively() {
        const DEPTH: usize = 4096;

        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let root = host.create_element("section");
        assert!(host.append_child(host.document_node_id(), root));

        let mut parent = root;
        for _ in 0..DEPTH {
            let child = host.create_element("div");
            assert!(host.append_child(parent, child));
            parent = child;
        }

        let shadow_host = host.create_element("article");
        assert!(host.append_child(parent, shadow_host));
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let mut shadow_parent = shadow_root;
        for _ in 0..DEPTH {
            let child = host.create_element("span");
            assert!(host.append_child(shadow_parent, child));
            shadow_parent = child;
        }

        let shadow_script = host.create_element("script");
        let light_script = host.create_element("script");
        assert!(host.append_child(shadow_parent, shadow_script));
        assert!(host.append_child(parent, light_script));

        assert_eq!(
            host.script_handles_in_subtree(root),
            vec![shadow_script, light_script]
        );
        assert_eq!(
            host.connected_script_handles(root),
            vec![shadow_script, light_script]
        );

        let html = host
            .get_html(root, true, false, &[shadow_root])
            .expect("shadow-including HTML");
        assert!(html.contains("<template shadowrootmode=\"open\">"));
        assert!(html.contains("</template></article><script></script>"));
    }

    #[test]
    fn cloned_dom_detaches_only_the_mutated_node_chunk() {
        let mut dom = NativeDom::new_html(test_url());
        let mut handles = Vec::new();
        for index in 0..(NATIVE_DOM_NODE_CHUNK_CAPACITY * 2) {
            handles.push(dom.create_element(&format!("x-{index}")));
        }
        let snapshot = dom.clone();
        assert!(Arc::ptr_eq(&dom.nodes.chunks, &snapshot.nodes.chunks));

        let changed = handles[NATIVE_DOM_NODE_CHUNK_CAPACITY + 7];
        let changed_chunk = changed.index() / NATIVE_DOM_NODE_CHUNK_CAPACITY;
        assert!(dom.set_attribute(changed, "data-state", "changed"));

        assert!(!Arc::ptr_eq(&dom.nodes.chunks, &snapshot.nodes.chunks));
        for (index, (live, frozen)) in dom
            .nodes
            .chunks
            .iter()
            .zip(snapshot.nodes.chunks.iter())
            .enumerate()
        {
            assert_eq!(
                Arc::ptr_eq(live, frozen),
                index != changed_chunk,
                "only the chunk containing the changed node should detach"
            );
        }
        assert_eq!(
            dom.get_attribute(changed, "data-state").as_deref(),
            Some("changed")
        );
        assert_eq!(snapshot.get_attribute(changed, "data-state"), None);
    }
}
