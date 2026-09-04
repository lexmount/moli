use super::super::{Document, DocumentReadyState};
use super::*;

pub(super) type ShadowTreeSlotSnapshot = (DomHandle, DomHandle, String, Vec<DomHandle>);

#[derive(Default)]
pub(super) struct SubtreeRemovalContext {
    pub(super) shadow_slots: Vec<ShadowTreeSlotSnapshot>,
    pub(super) shadow_hosts: Vec<DomHandle>,
    pub(super) open_popovers: Vec<DomHandle>,
}
use percent_encoding::percent_decode_str;

impl DomHost {
    pub fn create_detached_html_document(&mut self) -> DomHandle {
        let url = self
            .dom
            .final_url()
            .cloned()
            .unwrap_or_else(|| Url::parse("about:blank").expect("static about:blank parses"));
        self.create_detached_html_document_with_url(url)
    }

    pub fn create_detached_html_document_with_url(&mut self, url: Url) -> DomHandle {
        self.dom.create_node(
            NodeData::Document(Box::new(Document::new_html(url))),
            None,
            false,
            false,
        )
    }

    pub fn create_detached_xml_document(&mut self) -> DomHandle {
        let url = self
            .dom
            .final_url()
            .cloned()
            .unwrap_or_else(|| Url::parse("about:blank").expect("static about:blank parses"));
        self.create_detached_xml_document_with_url(url)
    }

    pub fn create_detached_xml_document_with_url(&mut self, url: Url) -> DomHandle {
        self.dom.create_node(
            NodeData::Document(Box::new(Document::new_xml(url))),
            None,
            false,
            false,
        )
    }

    pub fn from_dom(dom: NativeDom) -> Self {
        let mut host = Self {
            dom,
            dom_version: Cell::new(0),
            query_version: Cell::new(0),
            shadow_root_binding_version: Cell::new(0),
            connected_shadow_roots_version: Cell::new(0),
            document_tree_scope_versions: RefCell::new(HashMap::new()),
            id_index: RefCell::new(None),
            name_index: RefCell::new(None),
            element_query_index: RefCell::new(ElementQueryIndex::default()),
            live_collection_cache: RefCell::new(HashMap::new()),
            shadow_roots_by_host: RefCell::new(HashMap::new()),
            shadow_hosts_by_root: RefCell::new(HashMap::new()),
            shadow_slot_name_indexes: RefCell::new(HashMap::new()),
            #[cfg(test)]
            shadow_slot_name_index_build_count: Cell::new(0),
            connected_shadow_roots_cache: RefCell::new(None),
            manual_slot_assignments: RefCell::new(HashMap::new()),
            child_browsing_context_host_candidates: RefCell::new(Vec::new()),
            shadow_disabled_custom_element_definitions: RefCell::new(HashSet::new()),
            active_element: Cell::new(None),
            hovered_elements: RefCell::new(IndexSet::new()),
            mutation_observer_records_enabled: Cell::new(false),
            devtools_mutation_records_enabled: Cell::new(false),
        };
        host.update_all_document_targets_from_url();
        host.rebuild_child_browsing_context_host_candidates();
        host
    }

    pub fn set_mutation_observer_records_enabled(&self, enabled: bool) {
        self.mutation_observer_records_enabled.set(enabled);
    }

    pub fn set_devtools_mutation_records_enabled(&self, enabled: bool) {
        self.devtools_mutation_records_enabled.set(enabled);
    }

    pub fn devtools_mutation_records_enabled(&self) -> bool {
        self.devtools_mutation_records_enabled.get()
    }

    pub fn mutation_records_enabled(&self) -> bool {
        self.mutation_observer_records_enabled.get() || self.devtools_mutation_records_enabled.get()
    }

    pub fn register_shadow_disabled_custom_element_definition(&self, name: &str) {
        self.shadow_disabled_custom_element_definitions
            .borrow_mut()
            .insert(name.to_owned());
    }

    pub fn custom_element_definition_disables_shadow(&self, name: &str) -> bool {
        self.shadow_disabled_custom_element_definitions
            .borrow()
            .contains(name)
    }

    pub fn into_dom(self) -> NativeDom {
        self.dom
    }

    pub fn document_handle(&self) -> DomHandle {
        self.dom.document_node_id()
    }

    pub fn owner_document_handle(&self, handle: DomHandle) -> Option<DomHandle> {
        let node = self.node(handle)?;
        if node.is_document() {
            Some(handle)
        } else {
            node.owner_document()
        }
    }

    /// Resolves associated trees back to the node whose browsing-context
    /// document identity they share.
    ///
    /// Template contents have a separate inert owner `Document`, while shadow
    /// roots are associated with their host. External document-scoped node-id
    /// registries therefore use the outer host identity rather than treating
    /// either associated tree as a detached document.
    pub fn document_identity_handle(&self, handle: DomHandle) -> Option<DomHandle> {
        self.node(handle)?;
        let mut identity = handle;
        for _ in 0..=self.dom.len() {
            if let Some(host) = self.associated_tree_host_for_node(identity) {
                identity = host;
                continue;
            }
            return Some(identity);
        }
        None
    }

    fn associated_tree_host_for_node(&self, handle: DomHandle) -> Option<DomHandle> {
        let mut root = handle;
        while let Some(parent) = self.node(root).and_then(Node::parent_node) {
            root = parent;
        }
        let root_node = self.node(root)?;
        if !root_node.is_document_fragment() {
            return None;
        }
        if let Some(host) = self.shadow_root_host(root).or_else(|| {
            let owner_document = self
                .owner_document_handle(root)
                .unwrap_or_else(|| self.document_handle());
            let mut stack = vec![owner_document];
            while let Some(candidate) = stack.pop() {
                if self.shadow_root_handle(candidate) == Some(root) {
                    return Some(candidate);
                }
                stack.extend(self.child_handles(candidate));
            }
            None
        }) {
            return Some(host);
        }
        self.dom.nodes().find_map(|candidate| {
            candidate
                .as_element()
                .and_then(Element::template_contents)
                .filter(|contents| *contents == root)
                .map(|_| candidate.id())
        })
    }

    pub fn node(&self, handle: DomHandle) -> Option<&Node> {
        self.dom.node(handle)
    }

    pub fn node_mut(&mut self, handle: DomHandle) -> Option<&mut Node> {
        self.dom.node_mut(handle)
    }

    pub fn dom(&self) -> &NativeDom {
        &self.dom
    }

    pub fn query_version(&self) -> u64 {
        self.query_version.get()
    }

    pub fn dom_version(&self) -> u64 {
        self.dom_version.get()
    }

    pub fn shadow_root_binding_version(&self) -> u64 {
        self.shadow_root_binding_version.get()
    }

    /// Monotonic version of connected-shadow-root membership.
    ///
    /// Consumers can compare this together with
    /// [`Self::shadow_root_binding_version`] before cloning the cached binding
    /// snapshot. Ordinary DOM mutations that cannot add, remove, connect, or
    /// disconnect a shadow root leave both versions unchanged.
    pub fn connected_shadow_roots_version(&self) -> u64 {
        self.connected_shadow_roots_version.get()
    }

    /// Monotonic identity of one Document's connected ShadowRoot TreeScopes.
    ///
    /// This is deliberately document-local. The page-wide binding counters
    /// exist only to validate the shared connected-root lookup cache.
    pub fn document_tree_scope_version(&self, document: DomHandle) -> u64 {
        self.document_tree_scope_versions
            .borrow()
            .get(&document)
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn record_mutation(&self, scope: MutationScope) {
        self.dom_version
            .set(self.dom_version.get().saturating_add(1));
        if matches!(scope, MutationScope::QueryState) {
            self.query_version
                .set(self.query_version.get().saturating_add(1));
            self.live_collection_cache.borrow_mut().clear();
        }
    }

    fn record_shadow_root_binding_mutation(&self) {
        self.shadow_root_binding_version
            .set(self.shadow_root_binding_version.get().saturating_add(1));
        self.connected_shadow_roots_cache.borrow_mut().take();
    }

    fn record_connected_shadow_roots_mutation(&self) {
        self.connected_shadow_roots_version
            .set(self.connected_shadow_roots_version.get().saturating_add(1));
        self.connected_shadow_roots_cache.borrow_mut().take();
    }

    fn record_document_tree_scope_mutation(&self, document: DomHandle) {
        let mut versions = self.document_tree_scope_versions.borrow_mut();
        let version = versions.entry(document).or_default();
        *version = version.saturating_add(1);
    }

    fn record_document_tree_scope_mutations(&self, documents: impl IntoIterator<Item = DomHandle>) {
        let mut seen = HashSet::new();
        for document in documents {
            if seen.insert(document) {
                self.record_document_tree_scope_mutation(document);
            }
        }
    }

    pub(super) fn record_query_index_candidates_in_subtree(&self, root: DomHandle) {
        self.record_query_index_candidates_in_subtrees(std::slice::from_ref(&root));
    }

    pub(super) fn record_query_index_candidates_in_subtrees(&self, roots: &[DomHandle]) {
        if self.id_index.borrow().is_none()
            && self.name_index.borrow().is_none()
            && !self.element_query_index.borrow().has_materialized_index()
        {
            return;
        }
        let mut stack = roots.iter().rev().copied().collect::<Vec<_>>();
        while let Some(handle) = stack.pop() {
            let Some(node) = self.node(handle) else {
                continue;
            };
            self.record_named_index_candidate(handle);
            self.record_element_query_index_candidate(handle);
            let mut child = node.first_child();
            while let Some(handle) = child {
                stack.push(handle);
                child = self.next_sibling(handle);
            }
        }
    }

    pub(super) fn record_named_index_candidate(&self, handle: DomHandle) {
        let Some(element) = self.node(handle).and_then(Node::as_element) else {
            return;
        };
        if let Some(index) = self.id_index.borrow_mut().as_mut() {
            Self::update_named_element_index(index, handle, element.id());
        }
        if let Some(index) = self.name_index.borrow_mut().as_mut() {
            Self::update_named_element_index(index, handle, element.name_attribute());
        }
    }

    fn update_named_element_index(
        index: &mut NamedElementIndex,
        handle: DomHandle,
        current_value: Option<&str>,
    ) {
        if index.value_by_handle.get(&handle).map(AsRef::as_ref) == current_value {
            return;
        }
        if let Some(previous_value) = index.value_by_handle.remove(&handle) {
            let remove_key =
                index
                    .handles_by_value
                    .get_mut(&previous_value)
                    .is_some_and(|handles| {
                        handles.swap_remove(&handle);
                        handles.is_empty()
                    });
            if remove_key {
                index.handles_by_value.remove(&previous_value);
            }
        }
        if let Some(current_value) = current_value {
            let current_value = std::sync::Arc::<str>::from(current_value);
            index
                .handles_by_value
                .entry(current_value.clone())
                .or_default()
                .insert(handle);
            index.value_by_handle.insert(handle, current_value);
        }
    }

    pub(super) fn record_element_query_index_candidate(&self, handle: DomHandle) {
        let Some(element) = self.node(handle).and_then(Node::as_element) else {
            return;
        };
        self.element_query_index
            .borrow_mut()
            .record_materialized(handle, element);
    }

    pub(super) fn rekey_qualified_name_query_index_candidate(
        &self,
        handle: DomHandle,
        previous_qualified_name: &str,
        current_qualified_name: &str,
    ) {
        self.element_query_index.borrow_mut().rekey_qualified_name(
            handle,
            previous_qualified_name,
            current_qualified_name,
        );
    }

    pub fn mark_subtree_connected_preserving_owner_document(&mut self, root: DomHandle) {
        self.record_query_index_candidates_in_subtree(root);
        let mut touched_shadow_root = false;
        let mut touched_documents = HashSet::new();
        let mut stylesheet_owners = Vec::new();
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            let owner_document = self.node(handle).and_then(Node::owner_document);
            if let Some(node) = self.node_mut(handle) {
                node.set_tree_scope(owner_document, true, true);
            }
            if self.shadow_root_handle(handle).is_some() {
                if let Some(document) = self.tree_scope_owner_document_for_shadow_host(handle) {
                    touched_documents.insert(document);
                }
                touched_shadow_root |= self.mark_shadow_tree_scope_for_subtree(
                    handle,
                    self.tree_scope_owner_document_for_shadow_host(handle),
                    true,
                    &mut stylesheet_owners,
                );
            }
            let mut children = self.child_handles(handle).collect::<Vec<_>>();
            children.reverse();
            stack.extend(children);
        }
        if touched_shadow_root {
            self.record_connected_shadow_roots_mutation();
            self.record_document_tree_scope_mutations(touched_documents);
        }
        self.record_mutation(MutationScope::QueryState);
    }

    pub fn mark_subtree_disconnected_preserving_owner_document(&mut self, root: DomHandle) {
        let mut touched_shadow_root = false;
        let mut touched_documents = HashSet::new();
        let mut stylesheet_owners = Vec::new();
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            let owner_document = self.node(handle).and_then(Node::owner_document);
            if let Some(node) = self.node_mut(handle) {
                node.set_tree_scope(owner_document, false, false);
            }
            if self.shadow_root_handle(handle).is_some() {
                if let Some(document) = self.tree_scope_owner_document_for_shadow_host(handle) {
                    touched_documents.insert(document);
                }
                touched_shadow_root |= self.mark_shadow_tree_scope_for_subtree(
                    handle,
                    self.tree_scope_owner_document_for_shadow_host(handle),
                    false,
                    &mut stylesheet_owners,
                );
            }
            let mut children = self.child_handles(handle).collect::<Vec<_>>();
            children.reverse();
            stack.extend(children);
        }
        if touched_shadow_root {
            self.record_connected_shadow_roots_mutation();
            self.record_document_tree_scope_mutations(touched_documents);
        }
        self.record_mutation(MutationScope::QueryState);
    }

    fn tree_scope_owner_document_for_shadow_host(&self, handle: DomHandle) -> Option<DomHandle> {
        self.node(handle).and_then(|node| {
            if node.is_document() {
                Some(handle)
            } else {
                node.owner_document()
            }
        })
    }

    fn ensure_id_index(&self) {
        // Build lazily because most pages never use window named access. Keep
        // every current candidate, including detached nodes; lookup validates
        // current attributes, connectivity, and document order. That lets tree
        // moves and removals retain the index without document-wide rebuilds.
        if self.id_index.borrow().is_some() {
            return;
        }

        let mut index = NamedElementIndex::default();
        for node in self.dom.nodes() {
            let Some(id) = node.as_element().and_then(Element::id) else {
                continue;
            };
            Self::update_named_element_index(&mut index, node.id(), Some(id));
        }
        *self.id_index.borrow_mut() = Some(index);
    }

    fn ensure_name_index(&self) {
        // `name` lookup is separate from `id` so a name-only miss does not build
        // the id candidate index and vice versa.
        if self.name_index.borrow().is_some() {
            return;
        }

        let mut index = NamedElementIndex::default();
        for node in self.dom.nodes() {
            let Some(name) = node.as_element().and_then(Element::name_attribute) else {
                continue;
            };
            Self::update_named_element_index(&mut index, node.id(), Some(name));
        }
        *self.name_index.borrow_mut() = Some(index);
    }

    fn first_current_named_candidate(
        &self,
        candidates: &IndexSet<DomHandle>,
        key: &str,
        read_value: impl Fn(&Element) -> Option<&str>,
    ) -> Option<DomHandle> {
        let document_handle = self.document_handle();
        candidates
            .iter()
            .copied()
            .filter(|handle| {
                self.node(*handle).is_some_and(|node| {
                    node.flags().in_document_tree()
                        && node.owner_document() == Some(document_handle)
                        && node
                            .as_element()
                            .and_then(&read_value)
                            .is_some_and(|value| value == key)
                })
            })
            .min_by(|left, right| self.compare_handles_in_document_order(*left, *right))
    }

    pub(super) fn compare_handles_in_document_order(
        &self,
        left: DomHandle,
        right: DomHandle,
    ) -> std::cmp::Ordering {
        let Some(left_node) = self.node(left) else {
            return std::cmp::Ordering::Greater;
        };
        let position = left_node.compare_document_position(&self.dom, right);
        if position & Node::DOCUMENT_POSITION_FOLLOWING != 0 {
            std::cmp::Ordering::Less
        } else if position & Node::DOCUMENT_POSITION_PRECEDING != 0 {
            std::cmp::Ordering::Greater
        } else {
            left.index().cmp(&right.index())
        }
    }

    pub fn is_connected(&self, handle: DomHandle) -> bool {
        self.node(handle)
            .is_some_and(|node| node.flags().connected())
    }

    pub fn active_element_handle(&self) -> Option<DomHandle> {
        self.active_element.get().filter(|handle| {
            self.node(*handle)
                .is_some_and(|node| node.is_element() && node.is_connected())
        })
    }

    pub fn set_active_element_handle(&self, handle: Option<DomHandle>) {
        if self.active_element.get() == handle {
            return;
        }
        self.active_element.set(handle);
        self.record_mutation(MutationScope::QueryState);
    }

    pub fn element_matches_focus(&self, handle: DomHandle) -> bool {
        let Some(active) = self.active_element_handle() else {
            return false;
        };
        if active == handle {
            return true;
        }
        let mut current = active;
        while let Some(root) = self.containing_shadow_root(current) {
            let Some(host) = self.shadow_root_host(root) else {
                return false;
            };
            if host == handle {
                return true;
            }
            current = host;
        }
        false
    }

    pub fn element_matches_focus_within(&self, handle: DomHandle) -> bool {
        let Some(active) = self.active_element_handle() else {
            return false;
        };
        let mut current = Some(active);
        while let Some(candidate) = current {
            if candidate == handle {
                return true;
            }
            let Some(parent) = self.node(candidate).and_then(Node::parent_node) else {
                return false;
            };
            current = if self.is_shadow_root(parent) {
                self.shadow_root_host(parent)
            } else {
                Some(parent)
            };
        }
        false
    }

    pub fn hovered_element_handles(&self) -> Vec<DomHandle> {
        self.hovered_elements
            .borrow()
            .iter()
            .copied()
            .filter(|handle| {
                self.node(*handle)
                    .is_some_and(|node| node.is_element() && node.is_connected())
            })
            .collect()
    }

    pub fn set_hovered_element_handles(&self, handles: Vec<DomHandle>) -> bool {
        let next = handles
            .into_iter()
            .filter(|handle| {
                self.node(*handle)
                    .is_some_and(|node| node.is_element() && node.is_connected())
            })
            .collect::<IndexSet<_>>();
        let mut hovered = self.hovered_elements.borrow_mut();
        if *hovered == next {
            return false;
        }
        *hovered = next;
        drop(hovered);
        self.record_mutation(MutationScope::QueryState);
        true
    }

    pub fn clear_hovered_element_handles(&self) {
        if self.hovered_elements.borrow().is_empty() {
            return;
        }
        self.hovered_elements.borrow_mut().clear();
        self.record_mutation(MutationScope::QueryState);
    }

    pub(super) fn prune_disconnected_hovered_elements(&self) {
        self.hovered_elements
            .borrow_mut()
            .retain(|handle| self.is_connected(*handle));
    }

    pub fn element_matches_hover(&self, handle: DomHandle) -> bool {
        self.is_connected(handle) && self.hovered_elements.borrow().contains(&handle)
    }

    pub fn resolve_node(&self, node_id: NodeId) -> Option<DomHandle> {
        self.node(node_id).map(|_| node_id)
    }

    pub fn document_element_handle(&self) -> Option<DomHandle> {
        self.dom.document_element_handle()
    }

    pub fn document_head_handle(&self) -> Option<DomHandle> {
        self.dom.document_head_handle()
    }

    pub fn document_head_handle_for_document(
        &self,
        document_handle: DomHandle,
    ) -> Option<DomHandle> {
        self.dom.document_head_handle_for_document(document_handle)
    }

    pub fn document_body_handle(&self) -> Option<DomHandle> {
        self.dom.document_body_handle()
    }

    pub fn document_body_handle_for_document(
        &self,
        document_handle: DomHandle,
    ) -> Option<DomHandle> {
        self.dom
            .node(document_handle)?
            .as_document()?
            .body_handle(&self.dom, document_handle)
    }

    pub fn child_handles(&self, handle: DomHandle) -> impl Iterator<Item = DomHandle> + '_ {
        self.dom.child_ids(handle)
    }

    pub fn child_handles_reversed(
        &self,
        handle: DomHandle,
    ) -> impl Iterator<Item = DomHandle> + '_ {
        self.dom.child_ids_reversed(handle)
    }

    pub fn find_child(
        &self,
        handle: DomHandle,
        predicate: impl FnMut(DomHandle) -> bool,
    ) -> Option<DomHandle> {
        self.dom.find_child(handle, predicate)
    }

    pub fn nth_child(&self, handle: DomHandle, index: usize) -> Option<DomHandle> {
        self.child_handles(handle).nth(index)
    }

    pub fn child_index(&self, parent: DomHandle, child: DomHandle) -> Option<usize> {
        self.dom.child_index(parent, child)
    }

    pub fn first_child(&self, handle: DomHandle) -> Option<DomHandle> {
        self.dom.first_child(handle)
    }

    pub fn next_sibling(&self, handle: DomHandle) -> Option<DomHandle> {
        self.dom.next_sibling(handle)
    }

    pub fn clear_document_contents(&mut self) {
        let document_children = self
            .child_handles(self.document_handle())
            .collect::<Vec<_>>();
        for child in document_children {
            self.dom.detach_from_parent(child);
        }
        self.update_document_target_from_url(self.document_handle());
    }

    pub fn reset_html_document_shell(&mut self) {
        // A document-stream replacement removes the old document element from
        // the Document, but it does not dismantle that detached subtree. Old
        // node wrappers must continue to observe their original parent/child
        // relationships, as in Blink's Document::open/SetContent path.
        self.clear_document_contents();

        let document_element = self.create_element("html");
        let head = self.create_element("head");
        let body = self.create_element("body");

        let _ = self.append_child(self.document_handle(), document_element);
        let _ = self.append_child(document_element, head);
        let _ = self.append_child(document_element, body);
    }

    pub fn ensure_html_document_shell(&mut self) -> Option<DomHandle> {
        let document_element = match self.document_element_handle() {
            Some(document_element) => document_element,
            None => {
                let html = self.create_parser_element(
                    "html".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                    Vec::new(),
                );
                let head = self.create_parser_element(
                    "head".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                    Vec::new(),
                );
                let body = self.create_parser_element(
                    "body".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                    Vec::new(),
                );
                let _ = self.append_child(self.document_handle(), html);
                let _ = self.append_child(html, head);
                let _ = self.append_child(html, body);
                return Some(body);
            }
        };

        if !self.is_html_element_named(document_element, "html") {
            return None;
        }

        let body = self.ensure_html_document_body()?;
        if self.document_head_handle().is_none() {
            let head = self.create_parser_element(
                "head".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
                Vec::new(),
            );
            let _ = self.insert_before(document_element, head, Some(body));
        }
        Some(body)
    }

    pub fn ensure_html_document_body(&mut self) -> Option<DomHandle> {
        if let Some(body) = self.document_body_handle() {
            return Some(body);
        }
        let document_element = self.document_element_handle()?;
        if !self.is_html_element_named(document_element, "html") {
            return None;
        }
        if self
            .child_handles(document_element)
            .any(|child| self.is_html_element_named(child, "frameset"))
        {
            return None;
        }

        let body = self.create_parser_element(
            "body".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
            Vec::new(),
        );
        let _ = self.append_child(document_element, body);
        Some(body)
    }

    pub fn element_by_id(&self, id: &str) -> Option<HostElementSnapshot> {
        let handle = self.element_handle_by_id(id)?;
        let NodeData::Element(_) = self.node(handle)?.data() else {
            return None;
        };
        Some(HostElementSnapshot {
            text_content: self.text_content(handle).unwrap_or_default(),
        })
    }

    pub fn element_handle_by_id(&self, id: &str) -> Option<DomHandle> {
        self.ensure_id_index();
        self.id_index
            .borrow()
            .as_ref()
            .and_then(|index| index.handles_by_value.get(id))
            .and_then(|handles| self.first_current_named_candidate(handles, id, Element::id))
    }

    pub fn element_handle_by_name(&self, name: &str) -> Option<DomHandle> {
        self.ensure_name_index();
        self.name_index
            .borrow()
            .as_ref()
            .and_then(|index| index.handles_by_value.get(name))
            .and_then(|handles| {
                self.first_current_named_candidate(handles, name, Element::name_attribute)
            })
    }

    pub fn element_handles_by_id_or_name_matching(
        &self,
        key: &str,
        mut accepts_name: impl FnMut(DomHandle) -> bool,
    ) -> Vec<DomHandle> {
        self.ensure_id_index();
        self.ensure_name_index();

        let mut candidates = IndexSet::new();
        if let Some(handles) = self
            .id_index
            .borrow()
            .as_ref()
            .and_then(|index| index.handles_by_value.get(key))
        {
            candidates.extend(handles.iter().copied());
        }
        if let Some(handles) = self
            .name_index
            .borrow()
            .as_ref()
            .and_then(|index| index.handles_by_value.get(key))
        {
            candidates.extend(handles.iter().copied());
        }

        let document_handle = self.document_handle();
        let mut matches = candidates
            .into_iter()
            .filter(|handle| {
                self.node(*handle).is_some_and(|node| {
                    node.flags().in_document_tree()
                        && node.owner_document() == Some(document_handle)
                        && node.as_element().is_some_and(|element| {
                            element.id() == Some(key)
                                || (element.name_attribute() == Some(key) && accepts_name(*handle))
                        })
                })
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| self.compare_handles_in_document_order(*left, *right));
        matches
    }

    pub fn element_handles_by_id_or_name_matching_in_subtree(
        &self,
        root: DomHandle,
        key: &str,
        mut accepts_name: impl FnMut(DomHandle) -> bool,
    ) -> Vec<DomHandle> {
        if root == self.document_handle() {
            return self.element_handles_by_id_or_name_matching(key, accepts_name);
        }

        let mut matches = Vec::new();
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            if self
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| {
                    element.id() == Some(key)
                        || (element.name_attribute() == Some(key) && accepts_name(handle))
                })
            {
                matches.push(handle);
            }
            let children = self.child_handles(handle).collect::<Vec<_>>();
            stack.extend(children.into_iter().rev());
        }
        matches
    }

    pub fn element_handle_by_id_in_subtree(&self, root: DomHandle, id: &str) -> Option<DomHandle> {
        if root == self.document_handle() {
            return self.element_handle_by_id(id);
        }
        self.element_handle_in_subtree(root, |element| element.id() == Some(id))
    }

    pub fn element_handle_by_name_in_subtree(
        &self,
        root: DomHandle,
        name: &str,
    ) -> Option<DomHandle> {
        if root == self.document_handle() {
            return self.element_handle_by_name(name);
        }
        self.element_handle_in_subtree(root, |element| element.attribute("name") == Some(name))
    }

    fn element_handle_in_subtree(
        &self,
        root: DomHandle,
        mut matches: impl FnMut(&Element) -> bool,
    ) -> Option<DomHandle> {
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            if self
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(&mut matches)
            {
                return Some(handle);
            }
            let children = self.child_handles(handle).collect::<Vec<_>>();
            stack.extend(children.into_iter().rev());
        }
        None
    }

    pub fn document_url(&self) -> Option<&Url> {
        self.document_url_for_handle(self.document_handle())
    }

    pub fn document_url_for_handle(&self, document_handle: DomHandle) -> Option<&Url> {
        self.node(document_handle)
            .and_then(Node::as_document)
            .map(|document| document.url())
    }

    pub fn document_content_type_for_handle(&self, document_handle: DomHandle) -> Option<&str> {
        self.node(document_handle)
            .and_then(Node::as_document)
            .map(|document| document.content_type())
    }

    pub fn document_quirks_mode_for_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<selectors::matching::QuirksMode> {
        self.node(document_handle)
            .and_then(Node::as_document)
            .map(Document::quirks_mode)
    }

    pub fn document_base_url(&self) -> Option<Url> {
        self.document_base_url_for_handle(self.document_handle())
    }

    pub fn document_base_url_for_handle(&self, document_handle: DomHandle) -> Option<Url> {
        self.node(document_handle)
            .and_then(Node::as_document)
            .map(|document| document.base_url().clone())
    }

    pub fn document_base_target_for_handle(&self, document_handle: DomHandle) -> Option<&str> {
        self.node(document_handle)
            .and_then(Node::as_document)
            .and_then(Document::base_target)
    }

    pub fn document_base_target(&self) -> Option<&str> {
        self.document_base_target_for_handle(self.document_handle())
    }

    pub fn document_ready_state(&self) -> Option<DocumentReadyState> {
        self.document_ready_state_for_handle(self.document_handle())
    }

    pub fn document_ready_state_for_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<DocumentReadyState> {
        self.node(document_handle)
            .and_then(Node::as_document)
            .map(|document| document.ready_state())
    }

    pub fn document_default_language_for_handle(&self, document_handle: DomHandle) -> Option<&str> {
        self.node(document_handle)
            .and_then(Node::as_document)
            .and_then(|document| document.default_language())
    }

    pub fn document_source_last_modified_for_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<f64> {
        self.node(document_handle)
            .and_then(Node::as_document)
            .and_then(Document::source_last_modified_ms)
    }

    pub fn document_default_language_for_node(&self, handle: DomHandle) -> Option<String> {
        let document_handle = self.owner_document_handle(handle)?;
        self.document_meta_default_language_for_handle(document_handle)
            .or_else(|| {
                self.document_default_language_for_handle(document_handle)
                    .map(str::to_owned)
            })
    }

    fn document_meta_default_language_for_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<String> {
        let head = self
            .node(document_handle)
            .and_then(Node::as_document)
            .and_then(|document| document.head_handle(self.dom(), document_handle))?;
        for child in self.child_handles(head) {
            let Some(element) = self.node(child).and_then(Node::as_element) else {
                continue;
            };
            if !element.is_html_element("meta") {
                continue;
            }
            let Some(http_equiv) = element.attribute("http-equiv") else {
                continue;
            };
            if !http_equiv.eq_ignore_ascii_case("content-language") {
                continue;
            }
            let Some(content) = element.attribute("content") else {
                continue;
            };
            if let Some(language) = Self::single_content_language_value(content) {
                return Some(language);
            }
        }
        None
    }

    fn single_content_language_value(value: &str) -> Option<String> {
        let value = value.trim();
        if value.is_empty() || value.contains(',') {
            return None;
        }
        value
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .then(|| value.to_ascii_lowercase())
    }

    pub fn set_document_url(&mut self, url: Url) -> bool {
        self.set_document_url_for_handle(self.document_handle(), url)
    }

    pub fn set_document_url_for_handle(&mut self, document_handle: DomHandle, url: Url) -> bool {
        let Some(document) = self
            .node_mut(document_handle)
            .and_then(|node| node.data_mut().as_document_mut())
        else {
            return false;
        };
        if document.url() == &url {
            return false;
        }
        document.set_url(url);
        self.dom.process_base_element(document_handle, true);
        self.record_mutation(MutationScope::LocalState);
        self.update_document_target_from_url(document_handle);
        true
    }

    pub fn set_document_fallback_base_url_for_handle(
        &mut self,
        document_handle: DomHandle,
        fallback_base_url: Option<Url>,
    ) -> bool {
        let Some(document) = self
            .node_mut(document_handle)
            .and_then(|node| node.data_mut().as_document_mut())
        else {
            return false;
        };
        let fallback_base_url = fallback_base_url.unwrap_or_else(|| document.url().clone());
        if document.fallback_base_url() == &fallback_base_url {
            return false;
        }
        document.set_fallback_base_url(fallback_base_url);
        self.dom.process_base_element(document_handle, true);
        self.record_mutation(MutationScope::LocalState);
        true
    }

    pub fn set_document_content_type_for_handle(
        &mut self,
        document_handle: DomHandle,
        content_type: impl Into<String>,
    ) -> bool {
        let Some(document) = self
            .node_mut(document_handle)
            .and_then(|node| node.data_mut().as_document_mut())
        else {
            return false;
        };
        let content_type = content_type.into();
        if document.content_type() == content_type {
            return false;
        }
        document.set_content_type(content_type);
        self.record_mutation(MutationScope::LocalState);
        true
    }

    pub fn set_document_default_language_for_handle(
        &mut self,
        document_handle: DomHandle,
        language: Option<String>,
    ) -> bool {
        let Some(document) = self
            .node_mut(document_handle)
            .and_then(|node| node.data_mut().as_document_mut())
        else {
            return false;
        };
        if document.default_language() == language.as_deref() {
            return false;
        }
        document.set_default_language(language);
        self.record_mutation(MutationScope::QueryState);
        true
    }

    pub fn set_document_source_last_modified_for_handle(
        &mut self,
        document_handle: DomHandle,
        timestamp_ms: Option<f64>,
    ) -> bool {
        let Some(document) = self
            .node_mut(document_handle)
            .and_then(|node| node.data_mut().as_document_mut())
        else {
            return false;
        };
        if document.source_last_modified_ms() == timestamp_ms {
            return false;
        }
        document.set_source_last_modified_ms(timestamp_ms);
        self.record_mutation(MutationScope::QueryState);
        true
    }

    pub fn document_target_element(&self, document_handle: DomHandle) -> Option<DomHandle> {
        let target = self
            .node(document_handle)
            .and_then(Node::as_document)
            .and_then(Document::css_target)?;
        let target_node = self.node(target)?;
        (target_node.is_element() && target_node.owner_document() == Some(document_handle))
            .then_some(target)
    }

    pub fn set_document_target_element(
        &mut self,
        document_handle: DomHandle,
        target: Option<DomHandle>,
    ) -> bool {
        let target = target.filter(|handle| {
            self.node(*handle).is_some_and(|node| {
                node.is_element() && node.owner_document() == Some(document_handle)
            })
        });
        let Some(document) = self
            .node_mut(document_handle)
            .and_then(|node| node.data_mut().as_document_mut())
        else {
            return false;
        };
        let changed = document.set_css_target(target);
        if changed {
            self.record_mutation(MutationScope::QueryState);
        }
        changed
    }

    pub fn element_matches_target(&self, handle: DomHandle) -> bool {
        let Some(node) = self.node(handle).filter(|node| node.is_element()) else {
            return false;
        };
        let Some(document_handle) = node.owner_document() else {
            return false;
        };
        self.document_target_element(document_handle) == Some(handle)
    }

    pub fn update_document_target_from_url(&mut self, document_handle: DomHandle) -> bool {
        let target = self.resolve_document_target_from_url(document_handle);
        self.set_document_target_element(document_handle, target)
    }

    pub fn update_all_document_targets_from_url(&mut self) {
        let documents: Vec<_> = self
            .dom
            .nodes()
            .iter()
            .filter(|node| node.is_document())
            .map(Node::id)
            .collect();
        for document in documents {
            self.update_document_target_from_url(document);
        }
    }

    fn update_owner_document_target_for_node(&mut self, handle: DomHandle) {
        if let Some(document_handle) = self.owner_document_handle(handle) {
            self.update_document_target_from_url(document_handle);
        }
    }

    pub(super) fn update_target_after_indicated_part_mutation(&mut self, handle: DomHandle) {
        self.update_owner_document_target_for_node(handle);
    }

    fn resolve_document_target_from_url(&self, document_handle: DomHandle) -> Option<DomHandle> {
        let fragment = self
            .node(document_handle)
            .and_then(Node::as_document)
            .and_then(|document| document.url().fragment())?;
        if fragment.is_empty() {
            return None;
        }
        if let Some(target) = self.find_document_anchor(document_handle, fragment) {
            return Some(target);
        }
        let decoded = percent_decode_str(fragment)
            .decode_utf8_lossy()
            .into_owned();
        if decoded.eq_ignore_ascii_case("top") {
            return None;
        }
        self.find_document_anchor(document_handle, &decoded)
    }

    fn find_document_anchor(&self, document_handle: DomHandle, name: &str) -> Option<DomHandle> {
        if name.is_empty() || !self.node(document_handle).is_some_and(Node::is_document) {
            return None;
        }
        let elements = self.collect_matching_elements(document_handle, false, |_| true);
        if let Some(handle) = elements.iter().copied().find(|handle| {
            self.node(*handle)
                .and_then(Node::as_element)
                .and_then(Element::id)
                .is_some_and(|id| id == name)
        }) {
            return Some(handle);
        }
        elements.into_iter().find(|handle| {
            self.node(*handle)
                .and_then(Node::as_element)
                .is_some_and(|element| {
                    element.is_html_element("a") && element.name_attribute() == Some(name)
                })
        })
    }

    pub fn set_document_ready_state(&mut self, state: DocumentReadyState) -> bool {
        self.set_document_ready_state_for_handle(self.document_handle(), state)
    }

    pub fn set_document_ready_state_for_handle(
        &mut self,
        document_handle: DomHandle,
        state: DocumentReadyState,
    ) -> bool {
        let Some(document) = self
            .node_mut(document_handle)
            .and_then(|node| node.data_mut().as_document_mut())
        else {
            return false;
        };
        if document.ready_state() == state {
            return false;
        }
        document.set_ready_state(state);
        self.record_mutation(MutationScope::LocalState);
        true
    }

    pub fn child_nodes(&self, handle: DomHandle) -> Option<Vec<DomHandle>> {
        self.node(handle)?;
        Some(self.child_handles(handle).collect())
    }

    pub fn create_element(&mut self, local_name: &str) -> DomHandle {
        let handle = self.dom.create_element(local_name);
        self.record_element_query_index_candidate(handle);
        self.record_child_browsing_context_host_candidate_if_needed(handle, local_name, "");
        handle
    }

    pub fn create_element_ns(
        &mut self,
        namespace: Option<&str>,
        qualified_name: &str,
    ) -> Option<DomHandle> {
        let handle = self.dom.create_element_ns(namespace, qualified_name)?;
        self.record_element_query_index_candidate(handle);
        let (_, local_name) = super::super::split_qualified_name(qualified_name)?;
        self.record_child_browsing_context_host_candidate_if_needed(
            handle,
            local_name,
            namespace.unwrap_or_default(),
        );
        Some(handle)
    }

    pub fn create_element_with_parts(
        &mut self,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
    ) -> DomHandle {
        let handle = self
            .dom
            .create_element_with_parts(namespace, prefix, local_name);
        self.record_element_query_index_candidate(handle);
        self.record_child_browsing_context_host_candidate_if_needed(
            handle,
            local_name,
            namespace.unwrap_or_default(),
        );
        handle
    }

    pub fn child_browsing_context_host_candidate_handles(&self) -> Vec<DomHandle> {
        self.child_browsing_context_host_candidates
            .borrow()
            .iter()
            .copied()
            .filter(|handle| self.node(*handle).is_some())
            .collect()
    }

    pub fn child_browsing_context_host_candidate_handles_in_subtree_in_document_order(
        &self,
        root: DomHandle,
    ) -> Vec<DomHandle> {
        let Some(root_node) = self.node(root) else {
            return Vec::new();
        };
        let mut handles = self
            .child_browsing_context_host_candidates
            .borrow()
            .iter()
            .copied()
            .filter(|handle| {
                self.is_child_browsing_context_host_candidate(*handle)
                    && root_node.contains(&self.dom, *handle)
            })
            .collect::<Vec<_>>();
        handles.sort_unstable_by(|left, right| {
            let Some(left_node) = self.node(*left) else {
                return std::cmp::Ordering::Greater;
            };
            let position = left_node.compare_document_position(&self.dom, *right);
            if position & Node::DOCUMENT_POSITION_FOLLOWING != 0 {
                std::cmp::Ordering::Less
            } else if position & Node::DOCUMENT_POSITION_PRECEDING != 0 {
                std::cmp::Ordering::Greater
            } else {
                left.index().cmp(&right.index())
            }
        });
        handles.dedup();
        handles
    }

    pub fn compact_child_browsing_context_host_candidates(&self) {
        let mut seen = std::collections::HashSet::new();
        self.child_browsing_context_host_candidates
            .borrow_mut()
            .retain(|handle| {
                self.is_connected(*handle)
                    && self.is_child_browsing_context_host_candidate(*handle)
                    && seen.insert(*handle)
            });
    }

    fn rebuild_child_browsing_context_host_candidates(&self) {
        *self.child_browsing_context_host_candidates.borrow_mut() = self
            .dom
            .nodes()
            .iter()
            .map(Node::id)
            .filter(|handle| self.is_child_browsing_context_host_candidate(*handle))
            .collect();
    }

    pub(super) fn record_child_browsing_context_host_candidate_if_needed(
        &self,
        handle: DomHandle,
        local_name: &str,
        namespace: &str,
    ) {
        if is_html_frame_owner_candidate(local_name, namespace) {
            let mut candidates = self.child_browsing_context_host_candidates.borrow_mut();
            if !candidates.contains(&handle) {
                candidates.push(handle);
            }
        }
    }

    pub(super) fn record_inserted_subtree_candidates_in_subtrees(
        &self,
        roots: &[DomHandle],
    ) -> Vec<DomHandle> {
        let record_query_candidates = self.id_index.borrow().is_some()
            || self.name_index.borrow().is_some()
            || self.element_query_index.borrow().has_materialized_index();
        let mut shadow_hosts = Vec::new();
        let mut stack = roots.iter().rev().copied().collect::<Vec<_>>();
        while let Some(handle) = stack.pop() {
            if record_query_candidates {
                self.record_named_index_candidate(handle);
                self.record_element_query_index_candidate(handle);
            }
            if let Some(element) = self.node(handle).and_then(Node::as_element) {
                self.record_child_browsing_context_host_candidate_if_needed(
                    handle,
                    element.local_name(),
                    element.namespace(),
                );
            }
            if self.shadow_root_handle(handle).is_some() {
                shadow_hosts.push(handle);
            }
            let mut child = self.first_child(handle);
            while let Some(handle) = child {
                stack.push(handle);
                child = self.next_sibling(handle);
            }
        }
        shadow_hosts
    }

    fn is_child_browsing_context_host_candidate(&self, handle: DomHandle) -> bool {
        self.node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                is_html_frame_owner_candidate(element.local_name(), element.namespace())
            })
    }

    pub fn create_text_node(&mut self, data: &str) -> DomHandle {
        self.dom.create_text_node(data)
    }

    pub fn create_text_node_for_document(
        &mut self,
        document_handle: DomHandle,
        data: &str,
    ) -> DomHandle {
        self.dom
            .create_text_node_for_document(document_handle, data)
    }

    pub fn create_cdata_section(&mut self, data: &str) -> DomHandle {
        self.dom.create_cdata_section(data)
    }

    pub fn create_cdata_section_for_document(
        &mut self,
        document_handle: DomHandle,
        data: &str,
    ) -> DomHandle {
        self.dom
            .create_cdata_section_for_document(document_handle, data)
    }

    pub fn create_comment(&mut self, data: &str) -> DomHandle {
        self.dom.create_comment(data)
    }

    pub fn create_comment_for_document(
        &mut self,
        document_handle: DomHandle,
        data: &str,
    ) -> DomHandle {
        self.dom.create_comment_for_document(document_handle, data)
    }

    pub fn create_document_type(
        &mut self,
        name: &str,
        public_id: &str,
        system_id: &str,
    ) -> DomHandle {
        self.dom
            .create_document_type(name.to_owned(), public_id.to_owned(), system_id.to_owned())
    }

    pub fn create_document_type_for_document(
        &mut self,
        document_handle: DomHandle,
        name: &str,
        public_id: &str,
        system_id: &str,
    ) -> DomHandle {
        self.dom.create_document_type_for_document(
            document_handle,
            name.to_owned(),
            public_id.to_owned(),
            system_id.to_owned(),
        )
    }

    pub fn create_document(&mut self, url: url::Url) -> DomHandle {
        self.dom.create_document(url)
    }

    pub fn create_document_fragment(&mut self) -> DomHandle {
        self.dom.create_document_fragment()
    }

    pub fn create_document_fragment_for_document(
        &mut self,
        document_handle: DomHandle,
    ) -> DomHandle {
        self.dom
            .create_document_fragment_for_document(document_handle)
    }

    pub fn create_processing_instruction_for_document(
        &mut self,
        document_handle: DomHandle,
        target: &str,
        data: &str,
    ) -> DomHandle {
        self.dom
            .create_processing_instruction_for_document(document_handle, target, data)
    }

    pub fn attach_shadow_root(&mut self, host: DomHandle, mode: &str) -> Option<DomHandle> {
        self.attach_shadow_root_with_init(host, ShadowRootInit::new(mode))
    }

    pub fn attach_shadow_root_with_init(
        &mut self,
        host: DomHandle,
        init: ShadowRootInit,
    ) -> Option<DomHandle> {
        self.attach_shadow_root_with_init_internal(host, init, false)
    }

    pub(super) fn attach_declarative_shadow_root_with_init(
        &mut self,
        host: DomHandle,
        init: ShadowRootInit,
    ) -> Option<DomHandle> {
        self.attach_shadow_root_with_init_internal(host, init, true)
    }

    fn attach_shadow_root_with_init_internal(
        &mut self,
        host: DomHandle,
        init: ShadowRootInit,
        declarative: bool,
    ) -> Option<DomHandle> {
        let element = self.node(host).and_then(Node::as_element)?;
        if !can_host_shadow_root(element) {
            return None;
        }
        let existing_shadow_root = {
            self.shadow_roots_by_host
                .borrow()
                .get(&host)
                .map(|state| (state.handle, state.init.clone(), state.declarative))
        };
        if let Some((existing_root, existing_init, existing_declarative)) = existing_shadow_root {
            if !declarative && existing_declarative && existing_init.mode() == init.mode() {
                for child in self.dom.child_ids(existing_root).collect::<Vec<_>>() {
                    let _ = self.remove_child(existing_root, child);
                }
                if let Some(state) = self.shadow_roots_by_host.borrow_mut().get_mut(&host) {
                    state.declarative = false;
                }
                self.record_shadow_root_binding_mutation();
                return Some(existing_root);
            }
            return None;
        }
        let root = self.create_document_fragment();
        let owner_document = self.node(host).and_then(Node::owner_document);
        let connected = self.is_connected(host);
        self.dom.register_stylesheet_candidate_tree_scope(root);
        self.dom
            .mark_subtree_tree_scope(root, owner_document, connected, false);
        self.shadow_roots_by_host.borrow_mut().insert(
            host,
            ShadowRootState {
                handle: root,
                init,
                declarative,
                available_to_element_internals: declarative,
            },
        );
        self.shadow_hosts_by_root.borrow_mut().insert(root, host);
        self.record_shadow_root_binding_mutation();
        if connected && let Some(document) = owner_document {
            self.record_document_tree_scope_mutation(document);
        }
        Some(root)
    }

    pub fn shadow_root_handle(&self, host: DomHandle) -> Option<DomHandle> {
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.handle)
    }

    pub(super) fn sync_shadow_tree_scopes_for_inserted_subtrees(
        &mut self,
        inserted_roots: &[DomHandle],
        shadow_hosts: &[DomHandle],
        departed_connected_document: Option<DomHandle>,
    ) -> Vec<DomStylesheetOwnerChange> {
        // `insertBefore` performs an implicit removal when its child already
        // has a parent. Preserve that pre-splice membership here so one update
        // invalidates both the old and new Document TreeScope universes.
        let mut membership_documents: HashSet<DomHandle> =
            departed_connected_document.into_iter().collect();
        let mut stylesheet_owners = Vec::new();
        for &root in inserted_roots {
            let Some(node) = self.node(root) else {
                continue;
            };
            if self.containing_shadow_root(root).is_some() {
                self.dom.mark_subtree_tree_scope(
                    root,
                    node.owner_document(),
                    node.flags().connected(),
                    false,
                );
            }
        }
        for &host in shadow_hosts {
            let Some(node) = self.node(host) else {
                continue;
            };
            let connected = node.flags().connected();
            let owner_document = node.owner_document();
            let touched = self.mark_shadow_tree_scope_for_host(
                host,
                owner_document,
                connected,
                &mut stylesheet_owners,
            );
            if touched
                && connected
                && let Some(document) = owner_document
            {
                membership_documents.insert(document);
            }
        }
        if !membership_documents.is_empty() {
            self.record_connected_shadow_roots_mutation();
            self.record_document_tree_scope_mutations(membership_documents);
        }
        stylesheet_owners
            .into_iter()
            .map(|owner| {
                DomStylesheetOwnerChange::tree_connection_changed(
                    owner,
                    self.is_connected(owner),
                    self.dom.stylesheet_candidate_tree_scope_for_node(owner),
                )
            })
            .collect()
    }

    pub(super) fn sync_shadow_tree_scopes_for_removed_subtree(
        &mut self,
        shadow_hosts: &[DomHandle],
        was_connected: bool,
    ) -> Vec<DomStylesheetOwnerChange> {
        let mut membership_documents = HashSet::new();
        let mut stylesheet_owners = Vec::new();
        for &host in shadow_hosts {
            let Some(node) = self.node(host) else {
                continue;
            };
            let owner_document = node.owner_document();
            let touched = self.mark_shadow_tree_scope_for_host(
                host,
                owner_document,
                node.flags().connected(),
                &mut stylesheet_owners,
            );
            if touched
                && was_connected
                && let Some(document) = owner_document
            {
                membership_documents.insert(document);
            }
        }
        if !membership_documents.is_empty() {
            self.record_connected_shadow_roots_mutation();
            self.record_document_tree_scope_mutations(membership_documents);
        }
        stylesheet_owners
            .into_iter()
            .map(|owner| {
                DomStylesheetOwnerChange::tree_connection_changed(
                    owner,
                    self.is_connected(owner),
                    self.dom.stylesheet_candidate_tree_scope_for_node(owner),
                )
            })
            .collect()
    }

    fn mark_shadow_tree_scope_for_host(
        &mut self,
        host: DomHandle,
        owner_document: Option<DomHandle>,
        connected: bool,
        stylesheet_owners: &mut Vec<DomHandle>,
    ) -> bool {
        let Some(shadow_root) = self.shadow_root_handle(host) else {
            return false;
        };
        for owner in self
            .dom
            .mark_subtree_tree_scope_collecting_stylesheet_owners(
                shadow_root,
                owner_document,
                connected,
                false,
            )
        {
            if !stylesheet_owners.contains(&owner) {
                stylesheet_owners.push(owner);
            }
        }
        let _ = self.mark_shadow_tree_scope_for_subtree(
            shadow_root,
            owner_document,
            connected,
            stylesheet_owners,
        );
        true
    }

    fn mark_shadow_tree_scope_for_subtree(
        &mut self,
        root: DomHandle,
        owner_document: Option<DomHandle>,
        connected: bool,
        stylesheet_owners: &mut Vec<DomHandle>,
    ) -> bool {
        let mut touched_shadow_root = false;
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            stack.extend(self.child_handles_reversed(handle));
            if let Some(shadow_root) = self.shadow_root_handle(handle) {
                for owner in self
                    .dom
                    .mark_subtree_tree_scope_collecting_stylesheet_owners(
                        shadow_root,
                        owner_document,
                        connected,
                        false,
                    )
                {
                    if !stylesheet_owners.contains(&owner) {
                        stylesheet_owners.push(owner);
                    }
                }
                touched_shadow_root = true;
                stack.push(shadow_root);
            }
        }
        touched_shadow_root
    }

    pub fn shadow_root_mode(&self, root: DomHandle) -> Option<String> {
        let host = self.shadow_hosts_by_root.borrow().get(&root).copied()?;
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.init.mode().to_owned())
    }

    pub fn shadow_root_delegates_focus(&self, root: DomHandle) -> Option<bool> {
        let host = self.shadow_hosts_by_root.borrow().get(&root).copied()?;
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.init.delegates_focus())
    }

    pub fn shadow_root_slot_assignment(&self, root: DomHandle) -> Option<String> {
        let host = self.shadow_hosts_by_root.borrow().get(&root).copied()?;
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.init.slot_assignment().to_owned())
    }

    pub fn shadow_root_clonable(&self, root: DomHandle) -> Option<bool> {
        let host = self.shadow_hosts_by_root.borrow().get(&root).copied()?;
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.init.clonable())
    }

    pub fn shadow_root_serializable(&self, root: DomHandle) -> Option<bool> {
        let host = self.shadow_hosts_by_root.borrow().get(&root).copied()?;
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.init.serializable())
    }

    pub fn shadow_root_reference_target(&self, root: DomHandle) -> Option<Option<String>> {
        let host = self.shadow_hosts_by_root.borrow().get(&root).copied()?;
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.init.reference_target().map(str::to_owned))
    }

    pub fn shadow_root_adopted_style_sheets(&self, root: DomHandle) -> Option<Option<String>> {
        let host = self.shadow_hosts_by_root.borrow().get(&root).copied()?;
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.init.adopted_style_sheets().map(str::to_owned))
    }

    pub fn shadow_root_uses_null_custom_element_registry(&self, root: DomHandle) -> Option<bool> {
        let host = self.shadow_hosts_by_root.borrow().get(&root).copied()?;
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.init.null_custom_element_registry())
    }

    pub fn shadow_root_is_declarative(&self, root: DomHandle) -> Option<bool> {
        let host = self.shadow_hosts_by_root.borrow().get(&root).copied()?;
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.declarative)
    }

    pub fn shadow_root_available_to_element_internals(&self, root: DomHandle) -> Option<bool> {
        let host = self.shadow_hosts_by_root.borrow().get(&root).copied()?;
        self.shadow_roots_by_host
            .borrow()
            .get(&host)
            .map(|state| state.available_to_element_internals)
    }

    pub fn set_shadow_root_available_to_element_internals(
        &mut self,
        root: DomHandle,
        available: bool,
    ) -> bool {
        let Some(host) = self.shadow_hosts_by_root.borrow().get(&root).copied() else {
            return false;
        };
        let changed = {
            let mut roots = self.shadow_roots_by_host.borrow_mut();
            let Some(state) = roots.get_mut(&host) else {
                return false;
            };
            if state.available_to_element_internals == available {
                return false;
            }
            state.available_to_element_internals = available;
            true
        };
        if changed {
            self.record_shadow_root_binding_mutation();
        }
        changed
    }

    pub fn reference_target_forwarded_handle(&self, host: DomHandle) -> Option<Option<DomHandle>> {
        let root = self.shadow_root_handle(host)?;
        let reference_target = self.shadow_root_reference_target(root)?;
        let Some(reference_target) = reference_target else {
            return Some(Some(host));
        };
        if reference_target.is_empty() {
            return Some(None);
        }
        Some(self.element_handle_by_id_in_subtree(root, &reference_target))
    }

    pub fn resolve_reference_target_chain(&self, handle: DomHandle) -> Option<DomHandle> {
        let mut current = Some(handle);
        for _ in 0..32 {
            let candidate = current?;
            match self.reference_target_forwarded_handle(candidate) {
                Some(Some(forwarded)) if forwarded != candidate => current = Some(forwarded),
                Some(Some(forwarded)) => return Some(forwarded),
                Some(None) => return None,
                None => return Some(candidate),
            }
        }
        None
    }

    pub fn set_shadow_root_reference_target(
        &mut self,
        root: DomHandle,
        reference_target: Option<String>,
    ) -> bool {
        let Some(host) = self.shadow_hosts_by_root.borrow().get(&root).copied() else {
            return false;
        };
        let changed = {
            let mut roots = self.shadow_roots_by_host.borrow_mut();
            let Some(state) = roots.get_mut(&host) else {
                return false;
            };
            if state.init.reference_target() == reference_target.as_deref() {
                return false;
            }
            state.init.set_reference_target(reference_target);
            true
        };
        if changed {
            // Reference-target forwarding changes live `labels` and form-control
            // resolution even though it is neither a tree nor an attribute
            // mutation. Keep it in the same query-version invalidation domain.
            self.record_mutation(MutationScope::QueryState);
        }
        changed
    }

    pub fn shadow_root_host(&self, root: DomHandle) -> Option<DomHandle> {
        self.shadow_hosts_by_root.borrow().get(&root).copied()
    }

    pub fn is_shadow_root(&self, handle: DomHandle) -> bool {
        self.shadow_hosts_by_root.borrow().contains_key(&handle)
    }

    pub fn snapshot_shadow_root_bindings(&self) -> Vec<ShadowRootBindingSnapshot> {
        self.shadow_roots_by_host
            .borrow()
            .iter()
            .map(|(&host, state)| ShadowRootBindingSnapshot {
                host,
                root: state.handle,
                init: state.init.clone(),
                declarative: state.declarative,
                available_to_element_internals: state.available_to_element_internals,
            })
            .collect()
    }

    pub fn snapshot_connected_shadow_root_bindings(&self) -> Vec<ConnectedShadowRootSnapshot> {
        let shadow_root_binding_version = self.shadow_root_binding_version.get();
        let connected_shadow_roots_version = self.connected_shadow_roots_version.get();
        if let Some(cache) = self.current_connected_shadow_roots_cache(
            shadow_root_binding_version,
            connected_shadow_roots_version,
        ) {
            return cache.bindings.clone();
        }
        let bindings = self
            .shadow_roots_by_host
            .borrow()
            .iter()
            .filter_map(|(&host, state)| {
                self.is_connected(host)
                    .then_some(ConnectedShadowRootSnapshot {
                        host,
                        root: state.handle,
                    })
            })
            .collect::<Vec<_>>();
        let bindings_by_host = bindings
            .iter()
            .map(|binding| (binding.host, binding.clone()))
            .collect();
        *self.connected_shadow_roots_cache.borrow_mut() = Some(CachedConnectedShadowRoots {
            shadow_root_binding_version,
            connected_shadow_roots_version,
            bindings: bindings.clone(),
            bindings_by_host,
        });
        bindings
    }

    pub fn snapshot_connected_shadow_root_bindings_related_to_light_tree_handle(
        &self,
        handle: DomHandle,
    ) -> Vec<ConnectedShadowRootSnapshot> {
        if handle == self.document_handle() {
            return self.snapshot_connected_shadow_root_bindings();
        }
        if self.node(handle).is_none() || self.containing_shadow_root(handle).is_some() {
            return Vec::new();
        }
        // A related binding can only come from a registered shadow root host.
        // Keep the common no-shadow-DOM case O(1): walking `handle`'s light
        // subtree here otherwise turns an unrelated style mutation at body or
        // document scope into a full DOM scan.
        if self.shadow_roots_by_host.borrow().is_empty() {
            return Vec::new();
        }

        let mut bindings = Vec::new();
        let mut seen_hosts = HashSet::new();
        let mut current = Some(handle);
        while let Some(candidate) = current {
            self.push_connected_shadow_root_binding_for_host(
                candidate,
                &mut bindings,
                &mut seen_hosts,
            );
            current = self.parent_node(candidate);
        }

        let mut stack = vec![handle];
        while let Some(candidate) = stack.pop() {
            self.push_connected_shadow_root_binding_for_host(
                candidate,
                &mut bindings,
                &mut seen_hosts,
            );
            let mut child = self.first_child(candidate);
            while let Some(current) = child {
                stack.push(current);
                child = self.next_sibling(current);
            }
        }

        bindings
    }

    fn push_connected_shadow_root_binding_for_host(
        &self,
        host: DomHandle,
        bindings: &mut Vec<ConnectedShadowRootSnapshot>,
        seen_hosts: &mut HashSet<DomHandle>,
    ) {
        if !seen_hosts.insert(host) {
            return;
        }
        if let Some(binding) = self.connected_shadow_root_binding_for_host(host) {
            bindings.push(binding);
        }
    }

    fn connected_shadow_root_binding_for_host(
        &self,
        host: DomHandle,
    ) -> Option<ConnectedShadowRootSnapshot> {
        let shadow_root_binding_version = self.shadow_root_binding_version.get();
        let connected_shadow_roots_version = self.connected_shadow_roots_version.get();
        if let Some(cache) = self.current_connected_shadow_roots_cache(
            shadow_root_binding_version,
            connected_shadow_roots_version,
        ) {
            return cache.bindings_by_host.get(&host).cloned();
        }
        self.is_connected(host)
            .then(|| self.shadow_root_handle(host))
            .flatten()
            .map(|root| ConnectedShadowRootSnapshot { host, root })
    }

    fn current_connected_shadow_roots_cache(
        &self,
        shadow_root_binding_version: u64,
        connected_shadow_roots_version: u64,
    ) -> Option<std::cell::Ref<'_, CachedConnectedShadowRoots>> {
        std::cell::Ref::filter_map(self.connected_shadow_roots_cache.borrow(), |cache| {
            cache.as_ref().filter(|cache| {
                cache.shadow_root_binding_version == shadow_root_binding_version
                    && cache.connected_shadow_roots_version == connected_shadow_roots_version
            })
        })
        .ok()
    }

    pub fn snapshot_connected_shadow_roots(&self) -> Vec<DomHandle> {
        let bindings = self.snapshot_connected_shadow_root_bindings();
        bindings.into_iter().map(|binding| binding.root).collect()
    }

    #[cfg(test)]
    pub fn snapshot_connected_shadow_root_bindings_for_test(
        &self,
    ) -> Vec<ConnectedShadowRootSnapshot> {
        let mut bindings = self.snapshot_connected_shadow_root_bindings();
        bindings.sort_by_key(|binding| binding.root.index());
        bindings
    }

    #[cfg(test)]
    pub fn snapshot_connected_shadow_roots_for_test(&self) -> Vec<DomHandle> {
        let mut roots = self.snapshot_connected_shadow_roots();
        roots.sort_by_key(|root| root.index());
        roots
    }

    #[cfg(test)]
    pub fn snapshot_connected_shadow_root_bindings_related_to_light_tree_handle_for_test(
        &self,
        handle: DomHandle,
    ) -> Vec<ConnectedShadowRootSnapshot> {
        let mut bindings =
            self.snapshot_connected_shadow_root_bindings_related_to_light_tree_handle(handle);
        bindings.sort_by_key(|binding| binding.root.index());
        bindings
    }

    #[cfg(test)]
    pub fn connected_shadow_roots_cache_versions_for_test(&self) -> Option<(u64, u64)> {
        self.connected_shadow_roots_cache
            .borrow()
            .as_ref()
            .map(|cache| {
                (
                    cache.shadow_root_binding_version,
                    cache.connected_shadow_roots_version,
                )
            })
    }

    #[cfg(test)]
    pub fn connected_shadow_roots_cache_binding_counts_for_test(&self) -> Option<(usize, usize)> {
        self.connected_shadow_roots_cache
            .borrow()
            .as_ref()
            .map(|cache| (cache.bindings.len(), cache.bindings_by_host.len()))
    }

    pub fn restore_shadow_root_bindings(&mut self, bindings: Vec<ShadowRootBindingSnapshot>) {
        let mut restored = false;
        let mut membership_documents = HashSet::new();
        for binding in bindings {
            let host = binding.host;
            let root = binding.root;
            let host_exists = self.node(host).and_then(Node::as_element).is_some();
            let root_exists = self.node(root).is_some_and(Node::is_document_fragment);
            if !host_exists || !root_exists {
                continue;
            }
            self.dom.register_stylesheet_candidate_tree_scope(root);
            self.shadow_roots_by_host.borrow_mut().insert(
                host,
                ShadowRootState {
                    handle: root,
                    init: binding.init,
                    declarative: binding.declarative,
                    available_to_element_internals: binding.available_to_element_internals,
                },
            );
            self.shadow_hosts_by_root.borrow_mut().insert(root, host);
            if self.is_connected(host)
                && let Some(document) = self.owner_document_handle(host)
            {
                membership_documents.insert(document);
            }
            restored = true;
        }
        if restored {
            self.record_shadow_root_binding_mutation();
            self.record_document_tree_scope_mutations(membership_documents);
        }
    }

    pub fn containing_shadow_root(&self, handle: DomHandle) -> Option<DomHandle> {
        let mut current = Some(handle);
        while let Some(candidate) = current {
            if self.is_shadow_root(candidate) {
                return Some(candidate);
            }
            current = self.node(candidate).and_then(Node::parent_node);
        }
        None
    }

    pub fn root_node_handle(&self, handle: DomHandle) -> Option<DomHandle> {
        self.node(handle)?;
        let mut current = handle;
        loop {
            if self.is_shadow_root(current) {
                return Some(current);
            }
            let Some(parent) = self.node(current).and_then(Node::parent_node) else {
                return Some(current);
            };
            current = parent;
        }
    }

    fn ensure_shadow_slot_name_index(&self, shadow_root: DomHandle) -> bool {
        if self
            .shadow_slot_name_indexes
            .borrow()
            .contains_key(&shadow_root)
        {
            return true;
        }
        if !self.is_shadow_root(shadow_root) {
            return false;
        }

        let slots_in_tree_order = self.collect_matching_elements(shadow_root, false, |candidate| {
            self.is_html_element_named(candidate, "slot")
        });
        let mut slots_by_name = HashMap::<String, Vec<DomHandle>>::new();
        for &slot in &slots_in_tree_order {
            slots_by_name
                .entry(self.slot_element_name(slot))
                .or_default()
                .push(slot);
        }
        self.shadow_slot_name_indexes.borrow_mut().insert(
            shadow_root,
            ShadowSlotNameIndex {
                slots_in_tree_order,
                slots_by_name,
            },
        );
        #[cfg(test)]
        self.shadow_slot_name_index_build_count.set(
            self.shadow_slot_name_index_build_count
                .get()
                .saturating_add(1),
        );
        true
    }

    fn shadow_slots_with_name(&self, shadow_root: DomHandle, slot_name: &str) -> Vec<DomHandle> {
        if !self.ensure_shadow_slot_name_index(shadow_root) {
            return Vec::new();
        }
        self.shadow_slot_name_indexes
            .borrow()
            .get(&shadow_root)
            .and_then(|index| index.slots_by_name.get(slot_name))
            .cloned()
            .unwrap_or_default()
    }

    fn first_shadow_slot_with_name(
        &self,
        shadow_root: DomHandle,
        slot_name: &str,
    ) -> Option<DomHandle> {
        if !self.ensure_shadow_slot_name_index(shadow_root) {
            return None;
        }
        self.shadow_slot_name_indexes
            .borrow()
            .get(&shadow_root)
            .and_then(|index| index.slots_by_name.get(slot_name))
            .and_then(|slots| slots.first())
            .copied()
    }

    fn shadow_slots_in_tree_order(&self, shadow_root: DomHandle) -> Vec<DomHandle> {
        if !self.ensure_shadow_slot_name_index(shadow_root) {
            return Vec::new();
        }
        self.shadow_slot_name_indexes
            .borrow()
            .get(&shadow_root)
            .map(|index| index.slots_in_tree_order.clone())
            .unwrap_or_default()
    }

    pub(super) fn invalidate_shadow_slot_name_index(&self, shadow_root: DomHandle) {
        self.shadow_slot_name_indexes
            .borrow_mut()
            .remove(&shadow_root);
    }

    pub(super) fn invalidate_shadow_slot_name_index_for_tree_parent(&self, parent: DomHandle) {
        let shadow_root = self
            .is_shadow_root(parent)
            .then_some(parent)
            .or_else(|| self.containing_shadow_root(parent));
        if let Some(shadow_root) = shadow_root {
            self.invalidate_shadow_slot_name_index(shadow_root);
        }
    }

    pub(super) fn invalidate_shadow_slot_name_index_for_attribute(
        &self,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
    ) {
        if namespace.is_some_and(|namespace| !namespace.is_empty())
            || !local_name.eq_ignore_ascii_case("name")
            || !self.is_html_element_named(handle, "slot")
        {
            return;
        }
        if let Some(shadow_root) = self.containing_shadow_root(handle) {
            self.invalidate_shadow_slot_name_index(shadow_root);
        }
    }

    #[cfg(test)]
    pub fn shadow_slot_name_index_build_count_for_test(&self) -> u64 {
        self.shadow_slot_name_index_build_count.get()
    }

    pub fn assigned_nodes_for_slot_with_options(
        &self,
        slot_handle: DomHandle,
        flatten: bool,
    ) -> Vec<DomHandle> {
        let mut visiting = Vec::new();
        self.assigned_nodes_for_slot_internal(slot_handle, flatten, &mut visiting)
    }

    fn assigned_nodes_for_slot_internal(
        &self,
        slot_handle: DomHandle,
        flatten: bool,
        visiting: &mut Vec<DomHandle>,
    ) -> Vec<DomHandle> {
        if !self.is_html_element_named(slot_handle, "slot") {
            return Vec::new();
        }
        let Some(shadow_root) = self.containing_shadow_root(slot_handle) else {
            return Vec::new();
        };
        let Some(host) = self.shadow_root_host(shadow_root) else {
            return Vec::new();
        };
        if self.shadow_root_slot_assignment(shadow_root).as_deref() == Some("manual") {
            let assigned = self.manual_assigned_nodes_for_slot(slot_handle, host);
            return self.flatten_slot_assignment(slot_handle, assigned, flatten, visiting);
        }
        let slot_name = self
            .node(slot_handle)
            .and_then(Node::as_element)
            .and_then(|element| element.attribute("name"))
            .unwrap_or_default()
            .to_owned();

        let is_first_matching_slot =
            self.first_shadow_slot_with_name(shadow_root, &slot_name) == Some(slot_handle);
        let assigned = if is_first_matching_slot {
            self.child_handles(host)
                .filter(|child| {
                    self.node(*child)
                        .is_some_and(|node| node.is_element() || node.is_text())
                        && self.slot_name_for_node(*child) == slot_name
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        self.flatten_slot_assignment(slot_handle, assigned, flatten, visiting)
    }

    fn flatten_slot_assignment(
        &self,
        slot_handle: DomHandle,
        assigned: Vec<DomHandle>,
        flatten: bool,
        visiting: &mut Vec<DomHandle>,
    ) -> Vec<DomHandle> {
        if !flatten {
            return assigned;
        }
        if visiting.contains(&slot_handle) {
            return assigned;
        }
        visiting.push(slot_handle);

        let nodes = if assigned.is_empty() {
            self.child_handles(slot_handle).collect()
        } else {
            assigned
        };

        let mut flattened = Vec::new();
        for node in nodes {
            if self.is_html_element_named(node, "slot")
                && self.containing_shadow_root(node).is_some()
            {
                flattened.extend(self.assigned_nodes_for_slot_internal(node, true, visiting));
            } else {
                flattened.push(node);
            }
        }

        visiting.pop();
        flattened
    }

    fn manual_assigned_nodes_for_slot(
        &self,
        slot_handle: DomHandle,
        host: DomHandle,
    ) -> Vec<DomHandle> {
        self.manual_slot_assignments
            .borrow()
            .get(&slot_handle)
            .into_iter()
            .flatten()
            .copied()
            .filter(|node| self.is_slotable_host_child(*node, host))
            .collect()
    }

    fn is_slotable_host_child(&self, handle: DomHandle, host: DomHandle) -> bool {
        self.node(handle).is_some_and(|node| {
            (node.is_element() || node.is_text()) && node.parent_node() == Some(host)
        })
    }

    pub fn assigned_slot_for_node(&self, handle: DomHandle) -> Option<DomHandle> {
        let parent = self.node(handle).and_then(Node::parent_node)?;
        let shadow_root = self.shadow_root_handle(parent)?;
        if self.shadow_root_slot_assignment(shadow_root).as_deref() == Some("manual") {
            return self
                .manual_slot_assignments
                .borrow()
                .iter()
                .find_map(|(&slot, assigned)| {
                    if assigned.contains(&handle)
                        && self.containing_shadow_root(slot) == Some(shadow_root)
                    {
                        Some(slot)
                    } else {
                        None
                    }
                });
        }
        let slot_name = self.slot_name_for_node(handle);
        self.first_shadow_slot_with_name(shadow_root, &slot_name)
    }

    pub fn assign_nodes_to_slot(
        &self,
        slot_handle: DomHandle,
        nodes: Vec<DomHandle>,
    ) -> Vec<DomSlotAssignmentChange> {
        if !self.is_html_element_named(slot_handle, "slot") {
            return Vec::new();
        }
        let deduped = nodes.into_iter().collect::<IndexSet<_>>();
        let mut impacted_slots = IndexSet::from([slot_handle]);
        {
            let assignments = self.manual_slot_assignments.borrow();
            for (&assigned_slot, assigned_nodes) in assignments.iter() {
                if assigned_nodes.iter().any(|node| deduped.contains(node)) {
                    impacted_slots.insert(assigned_slot);
                }
            }
        }
        let before = impacted_slots
            .iter()
            .map(|&slot| (slot, self.assigned_nodes_for_slot_with_options(slot, false)))
            .collect::<Vec<_>>();

        let mut assignments = self.manual_slot_assignments.borrow_mut();
        for assigned in assignments.values_mut() {
            assigned.retain(|node| !deduped.contains(node));
        }
        assignments.insert(slot_handle, deduped.into_iter().collect());
        drop(assignments);

        let changes = before
            .into_iter()
            .filter_map(|(slot, previous)| {
                let assigned = self.assigned_nodes_for_slot_with_options(slot, false);
                (previous != assigned)
                    .then(|| DomSlotAssignmentChange::new(slot, previous, assigned))
            })
            .collect::<Vec<_>>();
        let changed_slots = changes
            .iter()
            .map(DomSlotAssignmentChange::slot)
            .collect::<Vec<_>>();
        self.slots_in_tree_order(&changed_slots)
            .into_iter()
            .filter_map(|slot| changes.iter().find(|change| change.slot() == slot).cloned())
            .collect()
    }

    pub fn slots_in_tree_order(&self, slots: &[DomHandle]) -> Vec<DomHandle> {
        let mut roots = Vec::new();
        for &slot in slots {
            if let Some(root) = self
                .containing_shadow_root(slot)
                .or_else(|| self.root_node_handle(slot))
                && !roots.contains(&root)
            {
                roots.push(root);
            }
        }

        let mut ordered = Vec::new();
        for root in roots {
            let root_slots = if self.is_shadow_root(root) {
                self.shadow_slots_in_tree_order(root)
            } else {
                self.collect_matching_elements(root, false, |candidate| {
                    self.is_html_element_named(candidate, "slot")
                })
            };
            for slot in root_slots {
                if !slots.contains(&slot) {
                    continue;
                }
                if !ordered.contains(&slot) {
                    ordered.push(slot);
                }
            }
        }
        for &slot in slots {
            if !ordered.contains(&slot) {
                ordered.push(slot);
            }
        }
        ordered
    }

    fn matching_slots_for_host_child_with_name(
        &self,
        host: DomHandle,
        slot_name: &str,
    ) -> Vec<DomHandle> {
        let Some(shadow_root) = self.shadow_root_handle(host) else {
            return Vec::new();
        };
        self.shadow_slots_with_name(shadow_root, slot_name)
    }

    pub(super) fn subtree_removal_context(&self, root: DomHandle) -> SubtreeRemovalContext {
        let mut context = SubtreeRemovalContext::default();
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            if let Some(element) = self.node(handle).and_then(Node::as_element) {
                if element.popover_open() {
                    context.open_popovers.push(handle);
                }
                if element.is_html_element("slot")
                    && let Some(shadow_root) = self.containing_shadow_root(handle)
                {
                    context.shadow_slots.push((
                        shadow_root,
                        handle,
                        self.slot_element_name(handle),
                        self.assigned_nodes_for_slot_with_options(handle, false),
                    ));
                }
            }
            if self.shadow_root_handle(handle).is_some() {
                context.shadow_hosts.push(handle);
            }
            stack.extend(self.child_handles_reversed(handle));
        }
        context
    }

    fn shadow_tree_slots_in_subtrees(&self, roots: &[DomHandle]) -> Vec<ShadowTreeSlotSnapshot> {
        let mut stack = roots.iter().rev().copied().collect::<Vec<_>>();
        let mut slots = Vec::new();
        while let Some(handle) = stack.pop() {
            if self.is_html_element_named(handle, "slot")
                && let Some(shadow_root) = self.containing_shadow_root(handle)
            {
                slots.push((
                    shadow_root,
                    handle,
                    self.slot_element_name(handle),
                    self.assigned_nodes_for_slot_with_options(handle, false),
                ));
            }
            stack.extend(self.child_handles_reversed(handle));
        }
        slots
    }

    pub(super) fn slot_assignment_snapshots_for_host_child_names(
        &self,
        host: DomHandle,
        child: DomHandle,
        slot_names: &[&str],
    ) -> Vec<(DomHandle, Vec<DomHandle>)> {
        let Some(shadow_root) = self.shadow_root_handle(host) else {
            return Vec::new();
        };
        if self.shadow_root_slot_assignment(shadow_root).as_deref() == Some("manual") {
            return self
                .manual_slot_assignments
                .borrow()
                .iter()
                .filter(|&(&slot, assigned_nodes)| {
                    self.containing_shadow_root(slot) == Some(shadow_root)
                        && assigned_nodes.contains(&child)
                })
                .map(|(&slot, _assigned_nodes)| {
                    (slot, self.assigned_nodes_for_slot_with_options(slot, false))
                })
                .collect();
        }

        let mut slots = Vec::new();
        let mut seen_slots = HashSet::new();
        for &slot_name in slot_names {
            for slot in self.matching_slots_for_host_child_with_name(host, slot_name) {
                if seen_slots.insert(slot) {
                    slots.push(slot);
                }
            }
        }
        slots
            .into_iter()
            .map(|slot| (slot, self.assigned_nodes_for_slot_with_options(slot, false)))
            .collect()
    }

    pub(super) fn slot_assignment_snapshots_for_shadow_slot_names(
        &self,
        shadow_root: DomHandle,
        slot_names: &[&str],
    ) -> Vec<(DomHandle, Vec<DomHandle>)> {
        let mut slots = Vec::new();
        let mut seen_slots = HashSet::new();
        for &slot_name in slot_names {
            for slot in self.shadow_slots_with_name(shadow_root, slot_name) {
                if seen_slots.insert(slot) {
                    slots.push(slot);
                }
            }
        }
        slots
            .into_iter()
            .map(|slot| (slot, self.assigned_nodes_for_slot_with_options(slot, false)))
            .collect()
    }

    pub(super) fn slot_assignment_snapshots_for_inserted_shadow_tree_slots(
        &self,
        parent: DomHandle,
        inserted_roots: &[DomHandle],
    ) -> Vec<(DomHandle, Vec<DomHandle>)> {
        let shadow_root = if self.is_shadow_root(parent) {
            Some(parent)
        } else {
            self.containing_shadow_root(parent)
        };
        let Some(shadow_root) = shadow_root else {
            return Vec::new();
        };
        let mut slot_names = Vec::new();
        let mut seen_slot_names = HashSet::new();
        for &root in inserted_roots {
            for slot in self.collect_matching_elements(root, true, |candidate| {
                self.is_html_element_named(candidate, "slot")
            }) {
                let name = self.slot_element_name(slot);
                if !seen_slot_names.contains(name.as_str()) {
                    seen_slot_names.insert(name.clone());
                    slot_names.push(name);
                }
            }
        }
        let slot_names = slot_names.iter().map(String::as_str).collect::<Vec<_>>();
        self.slot_assignment_snapshots_for_shadow_slot_names(shadow_root, &slot_names)
    }

    pub(super) fn slot_assignment_snapshots_for_removed_shadow_tree_slots(
        &self,
        removed_slots: &[ShadowTreeSlotSnapshot],
    ) -> Vec<(DomHandle, Vec<DomHandle>)> {
        let mut snapshots = Vec::new();
        let mut seen = HashSet::new();
        for &(shadow_root, _slot, ref slot_name, _) in removed_slots {
            if !seen.insert((shadow_root, slot_name.as_str())) {
                continue;
            }
            snapshots.extend(
                self.slot_assignment_snapshots_for_shadow_slot_names(shadow_root, &[slot_name]),
            );
        }
        snapshots
    }

    pub(super) fn record_slot_assignment_changes_from_snapshots(
        &self,
        effects: &mut DomMutationEffects,
        snapshots: Vec<(DomHandle, Vec<DomHandle>)>,
    ) {
        for (slot, previous_assigned_nodes) in snapshots {
            let assigned_nodes = self.assigned_nodes_for_slot_with_options(slot, false);
            effects.mark_slot_assignment_change(slot, previous_assigned_nodes, assigned_nodes);
        }
    }

    pub(super) fn record_host_child_slot_changes_from_snapshots(
        &self,
        effects: &mut DomMutationEffects,
        mut snapshots: Vec<(DomHandle, Vec<DomHandle>)>,
    ) {
        // Preserve the existing slotchange order: slots whose concrete
        // assignment changed are recorded first, followed by every matching
        // slot in the original name/tree-order snapshot.
        for (slot, previous_assigned_nodes) in &mut snapshots {
            let assigned_nodes = self.assigned_nodes_for_slot_with_options(*slot, false);
            effects.mark_slot_assignment_change(
                *slot,
                std::mem::take(previous_assigned_nodes),
                assigned_nodes,
            );
        }
        for (slot, _) in snapshots {
            effects.mark_changed_slot(slot);
        }
    }

    pub(super) fn record_slot_changes_for_inserted_shadow_tree_slots_in_subtrees(
        &self,
        effects: &mut DomMutationEffects,
        inserted_roots: &[DomHandle],
    ) {
        if inserted_roots
            .iter()
            .all(|root| !self.is_shadow_root(*root) && self.containing_shadow_root(*root).is_none())
        {
            return;
        }
        for (shadow_root, slot, slot_name, assigned_nodes) in
            self.shadow_tree_slots_in_subtrees(inserted_roots)
        {
            effects.mark_slot_assignment_change(slot, Vec::new(), assigned_nodes);
            self.record_non_empty_slot_changes_for_shadow_tree_slot_name(
                effects,
                shadow_root,
                &slot_name,
            );
        }
    }

    pub(super) fn record_slot_changes_for_removed_shadow_tree_slots(
        &self,
        effects: &mut DomMutationEffects,
        removed_slots: &[ShadowTreeSlotSnapshot],
    ) {
        for &(shadow_root, slot, ref slot_name, ref previous_assigned_nodes) in removed_slots {
            effects.mark_slot_assignment_change(slot, previous_assigned_nodes.clone(), Vec::new());
            self.record_non_empty_slot_changes_for_shadow_tree_slot_name(
                effects,
                shadow_root,
                slot_name,
            );
        }
    }

    pub(super) fn shadow_tree_slot_name_change(
        &self,
        handle: DomHandle,
        attribute_name: &str,
    ) -> Option<(DomHandle, String, Vec<DomHandle>)> {
        if !attribute_name.eq_ignore_ascii_case("name")
            || !self.is_html_element_named(handle, "slot")
        {
            return None;
        }
        let shadow_root = self.containing_shadow_root(handle)?;
        Some((
            shadow_root,
            self.slot_element_name(handle),
            self.assigned_nodes_for_slot_with_options(handle, false),
        ))
    }

    pub(super) fn record_slot_changes_for_shadow_tree_slot_name_change(
        &self,
        effects: &mut DomMutationEffects,
        shadow_root: DomHandle,
        slot: DomHandle,
        prior_name: &str,
        prior_assigned_nodes: &[DomHandle],
    ) {
        let current_name = self.slot_element_name(slot);
        if current_name == prior_name {
            return;
        }
        effects.mark_slot_assignment_change(
            slot,
            prior_assigned_nodes.to_vec(),
            self.assigned_nodes_for_slot_with_options(slot, false),
        );
        self.record_non_empty_slot_changes_for_shadow_tree_slot_name(
            effects,
            shadow_root,
            prior_name,
        );
        self.record_non_empty_slot_changes_for_shadow_tree_slot_name(
            effects,
            shadow_root,
            &current_name,
        );
    }

    fn record_non_empty_slot_changes_for_shadow_tree_slot_name(
        &self,
        effects: &mut DomMutationEffects,
        shadow_root: DomHandle,
        slot_name: &str,
    ) {
        for slot in self.shadow_slots_with_name(shadow_root, slot_name) {
            if !self
                .assigned_nodes_for_slot_with_options(slot, false)
                .is_empty()
            {
                effects.mark_changed_slot(slot);
            }
        }
    }

    pub(super) fn record_slot_fallback_child_change(
        &self,
        effects: &mut DomMutationEffects,
        parent: DomHandle,
    ) {
        if self.is_html_element_named(parent, "slot")
            && self.containing_shadow_root(parent).is_some()
        {
            effects.mark_changed_slot(parent);
        }
    }

    pub(super) fn slot_name_for_node(&self, handle: DomHandle) -> String {
        self.node(handle)
            .and_then(Node::as_element)
            .and_then(|element| element.attribute("slot"))
            .unwrap_or_default()
            .to_owned()
    }

    fn slot_element_name(&self, handle: DomHandle) -> String {
        self.node(handle)
            .and_then(Node::as_element)
            .and_then(|element| element.attribute("name"))
            .unwrap_or_default()
            .to_owned()
    }

    pub fn create_processing_instruction(&mut self, target: &str, data: &str) -> DomHandle {
        self.dom.create_processing_instruction(target, data)
    }
}

fn can_host_shadow_root(element: &Element) -> bool {
    if element.namespace() != "http://www.w3.org/1999/xhtml" {
        return false;
    }
    if element.local_name().contains('-') {
        return true;
    }
    matches!(
        element.local_name(),
        "article"
            | "aside"
            | "blockquote"
            | "body"
            | "div"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "main"
            | "nav"
            | "p"
            | "section"
            | "span"
    )
}
