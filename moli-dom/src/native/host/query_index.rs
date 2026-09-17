use super::*;

const HTML_NAMESPACE_URI: &str = "http://www.w3.org/1999/xhtml";

#[derive(Debug, Clone, Default)]
pub(super) struct ElementQueryIndex {
    qualified_names: Option<QualifiedNameElementIndex>,
    namespace_local_names: Option<NamespaceLocalNameElementIndex>,
}

#[derive(Debug, Clone, Default)]
struct QualifiedNameElementIndex {
    qualified_name_handles: HashMap<String, IndexSet<DomHandle>>,
}

#[derive(Debug, Clone, Default)]
struct NamespaceLocalNameElementIndex {
    handles_by_namespace_and_local_name: HashMap<String, HashMap<String, IndexSet<DomHandle>>>,
}

impl ElementQueryIndex {
    pub(super) fn has_materialized_index(&self) -> bool {
        self.qualified_names.is_some() || self.namespace_local_names.is_some()
    }

    pub(super) fn qualified_names_are_materialized(&self) -> bool {
        self.qualified_names.is_some()
    }

    pub(super) fn namespace_local_names_are_materialized(&self) -> bool {
        self.namespace_local_names.is_some()
    }

    pub(super) fn materialize_qualified_names<'a>(
        &mut self,
        elements: impl IntoIterator<Item = (DomHandle, &'a Element)>,
    ) {
        if self.qualified_names.is_some() {
            return;
        }
        let mut index = QualifiedNameElementIndex::default();
        for (handle, element) in elements {
            Self::record_qualified_name_in(&mut index, handle, element);
        }
        self.qualified_names = Some(index);
    }

    pub(super) fn materialize_namespace_local_names<'a>(
        &mut self,
        elements: impl IntoIterator<Item = (DomHandle, &'a Element)>,
    ) {
        if self.namespace_local_names.is_some() {
            return;
        }
        let mut index = NamespaceLocalNameElementIndex::default();
        for (handle, element) in elements {
            Self::record_namespace_local_name_in(&mut index, handle, element);
        }
        self.namespace_local_names = Some(index);
    }

    pub(super) fn record_materialized(&mut self, handle: DomHandle, element: &Element) {
        self.record_qualified_name(handle, element);
        if let Some(index) = self.namespace_local_names.as_mut() {
            Self::record_namespace_local_name_in(index, handle, element);
        }
    }

    pub(super) fn record_qualified_name(&mut self, handle: DomHandle, element: &Element) {
        if let Some(index) = self.qualified_names.as_mut() {
            Self::record_qualified_name_in(index, handle, element);
        }
    }

    pub(super) fn rekey_qualified_name(
        &mut self,
        handle: DomHandle,
        previous_qualified_name: &str,
        current_qualified_name: &str,
    ) {
        if previous_qualified_name == current_qualified_name {
            return;
        }
        let Some(index) = self.qualified_names.as_mut() else {
            return;
        };
        let remove_previous_key = index
            .qualified_name_handles
            .get_mut(previous_qualified_name)
            .is_some_and(|handles| {
                handles.swap_remove(&handle);
                handles.is_empty()
            });
        if remove_previous_key {
            index.qualified_name_handles.remove(previous_qualified_name);
        }
        index
            .qualified_name_handles
            .entry(current_qualified_name.to_owned())
            .or_default()
            .insert(handle);
    }

    pub(super) fn qualified_name_candidates(
        &self,
        qualified_name: &str,
    ) -> Option<&IndexSet<DomHandle>> {
        self.qualified_names
            .as_ref()?
            .qualified_name_handles
            .get(qualified_name)
    }

    pub(super) fn namespace_local_name_candidates(
        &self,
        namespace: &str,
        local_name: &str,
    ) -> Option<&IndexSet<DomHandle>> {
        self.namespace_local_names
            .as_ref()?
            .handles_by_namespace_and_local_name
            .get(namespace)?
            .get(local_name)
    }

    fn record_qualified_name_in(
        index: &mut QualifiedNameElementIndex,
        handle: DomHandle,
        element: &Element,
    ) {
        index
            .qualified_name_handles
            .entry(element.qualified_name())
            .or_default()
            .insert(handle);
    }

    fn record_namespace_local_name_in(
        index: &mut NamespaceLocalNameElementIndex,
        handle: DomHandle,
        element: &Element,
    ) {
        index
            .handles_by_namespace_and_local_name
            .entry(element.namespace().to_owned())
            .or_default()
            .entry(element.local_name().to_owned())
            .or_default()
            .insert(handle);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryRootInclusion {
    Inclusive,
    DescendantsOnly,
}

impl QueryRootInclusion {
    fn from_include_root(include_root: bool) -> Self {
        if include_root {
            Self::Inclusive
        } else {
            Self::DescendantsOnly
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementQueryScope {
    DocumentTree {
        document: DomHandle,
    },
    LightSubtree {
        root: DomHandle,
        root_inclusion: QueryRootInclusion,
    },
}

impl ElementQueryScope {
    fn for_root(host: &DomHost, root: DomHandle, include_root: bool) -> Option<Self> {
        let node = host.node(root)?;
        if node.is_document() {
            Some(Self::DocumentTree { document: root })
        } else {
            Some(Self::LightSubtree {
                root,
                root_inclusion: QueryRootInclusion::from_include_root(include_root),
            })
        }
    }

    fn contains(self, host: &DomHost, handle: DomHandle) -> bool {
        match self {
            Self::DocumentTree { document } => host.node(handle).is_some_and(|node| {
                node.owner_document() == Some(document)
                    && if document == host.document_handle() {
                        node.flags().in_document_tree()
                    } else {
                        host.root_node_handle(handle) == Some(document)
                    }
            }),
            Self::LightSubtree {
                root,
                root_inclusion,
            } => {
                if handle == root {
                    return root_inclusion == QueryRootInclusion::Inclusive;
                }
                let mut current = host.parent_node(handle);
                let mut seen = HashSet::new();
                while let Some(candidate) = current.filter(|candidate| seen.insert(*candidate)) {
                    if candidate == root {
                        return true;
                    }
                    current = host.parent_node(candidate);
                }
                false
            }
        }
    }

    fn traversal_root(self) -> (DomHandle, QueryRootInclusion) {
        match self {
            Self::DocumentTree { document } => (document, QueryRootInclusion::DescendantsOnly),
            Self::LightSubtree {
                root,
                root_inclusion,
            } => (root, root_inclusion),
        }
    }
}

impl DomHost {
    pub fn script_handles_in_light_subtree(&self, root: DomHandle) -> Vec<DomHandle> {
        if self.node(root).is_none() {
            return Vec::new();
        }
        let mut handles = Vec::new();
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            if self.node(handle).is_some_and(Node::is_script_element) {
                handles.push(handle);
            }
            let mut children = self.child_handles(handle).collect::<Vec<_>>();
            children.reverse();
            stack.extend(children);
        }
        handles
    }

    pub fn html_elements_by_local_name_in_document_tree_order(
        &self,
        document: DomHandle,
        local_name: &str,
    ) -> Vec<DomHandle> {
        if !self.node(document).is_some_and(Node::is_document) {
            return Vec::new();
        }
        self.elements_by_tag_name_ns(document, Some(HTML_NAMESPACE_URI), local_name, true)
    }

    /// Indexed candidates in a subtree, including descendants of shadow roots.
    /// The result has no tree-order guarantee. Empty tag indexes do not traverse
    /// the subtree, including when the root contains many unrelated elements.
    pub fn html_elements_by_local_name_in_shadow_including_subtree(
        &self,
        root: DomHandle,
        local_name: &str,
    ) -> Vec<DomHandle> {
        self.ensure_namespace_local_name_element_index();
        let index = self.element_query_index.borrow();
        let Some(candidates) =
            index.namespace_local_name_candidates(HTML_NAMESPACE_URI, local_name)
        else {
            return Vec::new();
        };
        candidates
            .iter()
            .copied()
            .filter(|handle| self.is_host_including_inclusive_ancestor(root, *handle))
            .collect()
    }

    fn ensure_qualified_name_element_index(&self) {
        if self
            .element_query_index
            .borrow()
            .qualified_names_are_materialized()
        {
            return;
        }
        self.element_query_index
            .borrow_mut()
            .materialize_qualified_names(
                self.dom
                    .nodes()
                    .filter_map(|node| node.as_element().map(|element| (node.id(), element))),
            );
    }

    fn ensure_namespace_local_name_element_index(&self) {
        if self
            .element_query_index
            .borrow()
            .namespace_local_names_are_materialized()
        {
            return;
        }
        self.element_query_index
            .borrow_mut()
            .materialize_namespace_local_names(
                self.dom
                    .nodes()
                    .filter_map(|node| node.as_element().map(|element| (node.id(), element))),
            );
    }

    fn element_query_matches_in_tree_order(
        &self,
        scope: ElementQueryScope,
        candidates: impl IntoIterator<Item = DomHandle>,
        candidate_count: usize,
        matches: impl Fn(&Element) -> bool,
    ) -> Vec<DomHandle> {
        if candidate_count == 0 {
            return Vec::new();
        }
        // A small subtree must not pay for unrelated retained nodes in the
        // global index. Bound the traversal by the candidate count so sparse
        // queries in large trees can still use the index.
        if let Some(handles) =
            self.element_query_matches_within_budget(scope, candidate_count, &matches)
        {
            return handles;
        }
        let mut handles = candidates
            .into_iter()
            .filter(|handle| {
                scope.contains(self, *handle)
                    && self
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(&matches)
            })
            .collect::<Vec<_>>();
        if handles.len() <= 1 {
            return handles;
        }
        if handles.len().saturating_mul(handles.len()) >= self.dom.len() {
            let candidates = handles.into_iter().collect::<HashSet<_>>();
            return self.element_query_candidates_in_scope_tree_order(scope, &candidates);
        }
        handles
            .sort_unstable_by(|left, right| self.compare_handles_in_document_order(*left, *right));
        handles
    }

    fn element_query_matches_within_budget(
        &self,
        scope: ElementQueryScope,
        candidate_count: usize,
        matches: &impl Fn(&Element) -> bool,
    ) -> Option<Vec<DomHandle>> {
        let (root, inclusion) = scope.traversal_root();
        let mut nodes = self
            .dom
            .light_subtree_handles(root, inclusion == QueryRootInclusion::Inclusive);
        let mut handles = Vec::new();
        for _ in 0..candidate_count {
            let Some(handle) = nodes.next() else {
                return Some(handles);
            };
            if self
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(matches)
            {
                handles.push(handle);
            }
        }
        nodes.next().is_none().then_some(handles)
    }

    fn element_query_candidates_in_scope_tree_order(
        &self,
        scope: ElementQueryScope,
        candidates: &HashSet<DomHandle>,
    ) -> Vec<DomHandle> {
        let (root, root_inclusion) = scope.traversal_root();
        let mut stack = Vec::new();
        if root_inclusion == QueryRootInclusion::Inclusive {
            stack.push(root);
        } else {
            stack.extend(self.child_handles_reversed(root));
        }
        let mut handles = Vec::with_capacity(candidates.len());
        while let Some(handle) = stack.pop() {
            if candidates.contains(&handle) {
                handles.push(handle);
                if handles.len() == candidates.len() {
                    break;
                }
            }
            stack.extend(self.child_handles_reversed(handle));
        }
        handles
    }

    fn indexed_elements_by_qualified_name_in_tree_order(
        &self,
        root: DomHandle,
        tag_name: &str,
        include_root: bool,
        is_html_document: bool,
    ) -> Option<Vec<DomHandle>> {
        if tag_name == "*" {
            return None;
        }
        let scope = ElementQueryScope::for_root(self, root, include_root)?;
        self.ensure_qualified_name_element_index();
        let lowercase_tag_name = is_html_document.then(|| tag_name.to_ascii_lowercase());
        let index = self.element_query_index.borrow();
        let exact_candidates = index.qualified_name_candidates(tag_name);
        let html_case_folded_candidates = lowercase_tag_name
            .as_deref()
            .filter(|lowercase_tag_name| *lowercase_tag_name != tag_name)
            .and_then(|lowercase_tag_name| index.qualified_name_candidates(lowercase_tag_name));
        let candidate_count = exact_candidates.map_or(0, IndexSet::len)
            + html_case_folded_candidates.map_or(0, IndexSet::len);
        Some(
            self.element_query_matches_in_tree_order(
                scope,
                exact_candidates
                    .into_iter()
                    .chain(html_case_folded_candidates)
                    .flat_map(|candidates| candidates.iter().copied()),
                candidate_count,
                |element| element.matches_tag_name_in_html_document(tag_name, is_html_document),
            ),
        )
    }

    pub fn elements_by_tag_name(
        &self,
        root: DomHandle,
        tag_name: &str,
        include_root: bool,
    ) -> Vec<DomHandle> {
        self.dom.elements_by_tag_name(root, tag_name, include_root)
    }

    pub fn elements_by_tag_name_in_html_document(
        &self,
        root: DomHandle,
        tag_name: &str,
        include_root: bool,
        is_html_document: bool,
    ) -> Vec<DomHandle> {
        self.dom.elements_by_tag_name_in_html_document(
            root,
            tag_name,
            include_root,
            is_html_document,
        )
    }

    pub fn cached_elements_by_tag_name_in_html_document(
        &self,
        root: DomHandle,
        tag_name: &str,
        include_root: bool,
        is_html_document: bool,
    ) -> Vec<DomHandle> {
        // This is intentionally narrower than elements_by_tag_name: bridge live
        // collections can ask the same HTML-aware tag query many times per turn,
        // so cache by query_version and leave one-off selector/query paths alone.
        if self.node(root).is_none() {
            return Vec::new();
        }
        let query = format!("{is_html_document}\0{tag_name}");
        let key = LiveCollectionCacheKey {
            root,
            kind: LiveCollectionCacheKind::TagName,
            query: Some(query),
            include_root,
        };
        let version = self.query_version.get();
        if let Some(cached) = self
            .live_collection_cache
            .borrow()
            .get(&key)
            .filter(|cached| cached.version == version)
        {
            return cached.handles.clone();
        }

        let handles = self
            .indexed_elements_by_qualified_name_in_tree_order(
                root,
                tag_name,
                include_root,
                is_html_document,
            )
            .unwrap_or_else(|| {
                self.dom.elements_by_tag_name_in_html_document(
                    root,
                    tag_name,
                    include_root,
                    is_html_document,
                )
            });
        self.live_collection_cache.borrow_mut().insert(
            key,
            CachedLiveCollection {
                version,
                handles: handles.clone(),
            },
        );
        handles
    }

    pub fn node_document_is_html_document(&self, root: DomHandle) -> Option<bool> {
        self.dom.node_document_is_html_document(root)
    }

    pub fn elements_by_tag_name_ns(
        &self,
        root: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
        include_root: bool,
    ) -> Vec<DomHandle> {
        if namespace == Some("*") || local_name == "*" {
            return self
                .dom
                .elements_by_tag_name_ns(root, namespace, local_name, include_root);
        }
        let Some(scope) = ElementQueryScope::for_root(self, root, include_root) else {
            return Vec::new();
        };
        self.ensure_namespace_local_name_element_index();
        let namespace = namespace.unwrap_or_default();
        let index = self.element_query_index.borrow();
        let candidates = index.namespace_local_name_candidates(namespace, local_name);
        self.element_query_matches_in_tree_order(
            scope,
            candidates
                .into_iter()
                .flat_map(|candidates| candidates.iter().copied()),
            candidates.map_or(0, IndexSet::len),
            |element| element.matches_tag_name_ns(Some(namespace), local_name),
        )
    }

    pub fn elements_by_class_name(
        &self,
        root: DomHandle,
        class_name: &str,
        include_root: bool,
    ) -> Vec<DomHandle> {
        self.dom
            .elements_by_class_name(root, class_name, include_root)
    }

    pub fn elements_by_name(
        &self,
        root: DomHandle,
        name: &str,
        include_root: bool,
    ) -> Vec<DomHandle> {
        self.dom.elements_by_name(root, name, include_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ))
    }

    #[test]
    fn global_named_item_candidate_check_tracks_live_attributes_in_detached_trees() {
        let mut host = test_host();
        assert!(!host.has_element_with_named_item_key("late"));
        assert!(!host.has_element_with_named_item_key(""));
        let input = host.create_element("input");
        assert!(host.set_attribute(input, "name", "late"));
        assert!(host.has_element_with_named_item_key("late"));
        assert!(host.set_attribute(input, "name", "renamed"));
        assert!(!host.has_element_with_named_item_key("late"));
        assert!(host.has_element_with_named_item_key("renamed"));
        assert!(host.set_attribute(input, "id", "renamed"));
        assert!(host.remove_attribute(input, "name"));
        assert!(host.has_element_with_named_item_key("renamed"));
        let root = host.create_element("div");
        assert!(host.append_child(root, input));
        assert!(host.has_element_with_named_item_key("renamed"));
        assert!(host.remove_attribute(input, "id"));
        assert!(!host.has_element_with_named_item_key("renamed"));
        assert!(host.set_attribute(input, "name", ""));
        assert!(!host.has_element_with_named_item_key(""));
    }

    #[test]
    fn named_candidate_visits_are_unscoped_deduplicated_and_short_circuitable() {
        use std::ops::ControlFlow;
        let mut host = test_host();
        assert!(!host.has_element_with_named_item_key("shared"));
        let detached = host.create_element("input");
        let shadow_host = host.create_element("div");
        let shadow = host.attach_shadow_root(shadow_host, "open").unwrap();
        let shadow_input = host.create_element("input");
        assert!(host.append_child(shadow, shadow_input));
        let other_document = host.create_detached_html_document();
        let other_input = host.create_element("input");
        assert!(host.append_child(other_document, other_input));
        for handle in [detached, shadow_input, other_input] {
            assert!(host.set_attribute(handle, "id", "shared"));
            assert!(host.set_attribute(handle, "name", "shared"));
        }
        let mut found = Vec::new();
        let result = host.visit_element_candidates_by_id_or_name("shared", |handle| {
            found.push(handle);
            ControlFlow::<()>::Continue(())
        });
        assert!(result.is_continue());
        assert_eq!(found.len(), 3);
        for handle in [detached, shadow_input, other_input] {
            assert!(found.contains(&handle));
        }
        let mut visits = 0;
        let first = host.visit_element_candidates_by_id_or_name("shared", |handle| {
            visits += 1;
            ControlFlow::Break(handle)
        });
        assert!(first.is_break());
        assert_eq!(visits, 1);
        assert!(host.set_attribute(detached, "id", "renamed"));
        assert!(host.remove_attribute(detached, "name"));
        let _ = host.visit_element_candidates_by_id_or_name("shared", |handle| {
            assert_ne!(handle, detached);
            ControlFlow::<()>::Continue(())
        });
        assert_eq!(
            host.visit_element_candidates_by_id_or_name("renamed", ControlFlow::Break),
            ControlFlow::Break(detached)
        );
        assert!(
            host.element_handles_by_id_or_name_matching("shared", |_| true)
                .is_empty()
        );
    }

    #[test]
    fn named_candidates_track_detached_clones_and_imports_after_materialization() {
        use std::ops::ControlFlow;
        let mut host = test_host();
        let source = host.create_element("input");
        assert!(host.set_attribute(source, "id", "cloned"));
        assert!(host.set_attribute(source, "name", "cloned"));
        assert!(host.has_element_with_named_item_key("cloned"));
        let clone = host.clone_node(source, false).unwrap();
        let document = host.create_detached_html_document();
        let imported = host.import_node(document, source, false).unwrap();
        let foreign = host.snapshot_document();
        let foreign_import = host
            .import_foreign_node(document, &foreign, source, false)
            .unwrap();
        let mut candidates = Vec::new();
        let _ = host.visit_element_candidates_by_id_or_name("cloned", |handle| {
            candidates.push(handle);
            ControlFlow::<()>::Continue(())
        });
        assert_eq!(candidates, vec![source, clone, imported, foreign_import]);
    }

    #[test]
    fn class_name_collections_follow_document_mode_and_adoption() {
        use selectors::matching::QuirksMode;

        let mut host = test_host();
        let document = host.document_handle();
        let other = host.create_detached_html_document();
        let root = host.create_element("section");
        let lower = host.create_element("span");
        let upper = host.create_element("span");
        assert!(host.append_child(document, root));
        assert!(host.append_child(root, lower));
        assert!(host.append_child(root, upper));
        host.set_attribute(root, "class", "a");
        host.set_attribute(lower, "class", "a");
        host.set_attribute(upper, "class", "A");

        for (mode, expected) in [
            (QuirksMode::Quirks, vec![lower, upper]),
            (QuirksMode::LimitedQuirks, vec![upper]),
            (QuirksMode::NoQuirks, vec![upper]),
            (QuirksMode::Quirks, vec![lower, upper]),
        ] {
            assert!(host.set_document_quirks_mode_for_handle(document, mode));
            for _ in 0..2 {
                assert_eq!(
                    host.resolve_live_collection(root, "className", Some("A"), false),
                    Some(expected.clone())
                );
            }
        }
        assert_eq!(
            host.elements_by_class_name(document, "A", false),
            vec![root, lower, upper]
        );

        assert!(host.remove_child(document, root));
        assert_eq!(
            host.resolve_live_collection(root, "className", Some("A"), false),
            Some(vec![lower, upper])
        );
        assert_eq!(host.adopt_node(other, root), Some(root));
        assert_eq!(host.owner_document_handle(lower), Some(other));
        assert_eq!(
            host.resolve_live_collection(root, "className", Some("A"), false),
            Some(vec![upper])
        );
        assert!(host.elements_by_class_name(document, "A", false).is_empty());
        assert!(host.elements_by_class_name(other, "A", false).is_empty());
        assert!(host.append_child(other, root));
        assert_eq!(host.elements_by_class_name(other, "A", false), vec![upper]);
        assert!(host.remove_child(other, root));
        assert_eq!(
            host.resolve_live_collection(root, "className", Some("A"), false),
            Some(vec![upper])
        );
        assert_eq!(host.adopt_node(document, root), Some(root));
        assert_eq!(
            host.resolve_live_collection(root, "className", Some("A"), false),
            Some(vec![lower, upper])
        );
    }

    #[test]
    fn cached_tag_name_collection_reuses_query_until_mutation() {
        let mut host = test_host();
        let document = host.document_handle();
        let head = host.create_element("head");
        let first = host.create_element("meta");
        let second = host.create_element("meta");

        assert!(host.append_child(document, head));
        assert!(host.append_child(head, first));
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "meta", true, true),
            vec![first]
        );
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "meta", true, true),
            vec![first]
        );

        assert!(host.append_child(head, second));
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "meta", true, true),
            vec![first, second]
        );
    }

    #[test]
    fn cached_tag_name_collection_scopes_candidates_to_detached_document_root() {
        let mut host = test_host();
        let document = host.document_handle();
        let main_body = host.create_element("body");
        assert!(host.append_child(document, main_body));
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "body", false, true),
            vec![main_body]
        );

        let detached_document = host.create_detached_html_document();
        let detached_html = host.create_parser_element_without_attributes_for_document(
            detached_document,
            "html".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        let detached_body = host.create_parser_element_without_attributes_for_document(
            detached_document,
            "body".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(host.append_child(detached_document, detached_html));
        assert!(host.append_child(detached_html, detached_body));

        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(
                detached_document,
                "body",
                false,
                true,
            ),
            vec![detached_body]
        );
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(
                detached_document,
                "BODY",
                false,
                true,
            ),
            vec![detached_body]
        );
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "body", false, true),
            vec![main_body]
        );
    }

    #[test]
    fn qualified_name_index_preserves_html_case_fold_and_foreign_exact_match() {
        let mut host = test_host();
        let document = host.document_handle();
        let html_div = host.create_element("div");
        let foreign_lower = host
            .create_element_ns(Some("urn:test"), "div")
            .expect("foreign lowercase div");
        let foreign_upper = host
            .create_element_ns(Some("urn:test"), "DIV")
            .expect("foreign uppercase div");

        assert!(host.append_child(document, html_div));
        assert!(host.append_child(document, foreign_lower));
        assert!(host.append_child(document, foreign_upper));
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "DIV", true, true),
            vec![html_div, foreign_upper]
        );
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "div", true, true),
            vec![html_div, foreign_lower]
        );

        assert!(host.insert_before(document, foreign_upper, Some(html_div)));
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "DIV", true, true),
            vec![foreign_upper, html_div]
        );
    }

    #[test]
    fn elements_by_name_matches_only_html_namespace_elements() {
        let mut host = test_host();
        let document = host.document_handle();
        let root = host.create_element("div");
        let html = host.create_element("p");
        let svg = host
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
            .expect("SVG element");
        let mathml = host
            .create_element_ns(Some("http://www.w3.org/1998/Math/MathML"), "math")
            .expect("MathML element");

        for element in [html, svg, mathml] {
            assert!(host.set_attribute(element, "name", "target"));
            assert!(host.append_child(root, element));
        }
        assert!(host.append_child(document, root));

        assert_eq!(host.elements_by_name(document, "target", true), vec![html]);
        assert!(host.elements_by_name(document, "", true).is_empty());
    }

    #[test]
    fn namespace_local_name_query_encodes_light_tree_scope_and_tree_order() {
        let mut host = test_host();
        let document = host.document_handle();
        let root = host.create_element("section");
        let first_light_style = host.create_element("style");
        let second_light_style = host.create_element("style");
        let sibling_style = host.create_element("style");
        let shadow_root = host
            .attach_shadow_root(root, "open")
            .expect("section should host a shadow root");
        let shadow_style = host.create_element("style");

        assert!(host.append_child(document, root));
        assert!(host.append_child(root, first_light_style));
        assert!(host.append_child(root, second_light_style));
        assert!(host.append_child(document, sibling_style));
        assert!(host.append_child(shadow_root, shadow_style));

        assert_eq!(
            host.elements_by_tag_name_ns(document, Some(HTML_NAMESPACE_URI), "style", true,),
            vec![first_light_style, second_light_style, sibling_style]
        );
        assert_eq!(
            host.elements_by_tag_name_ns(root, Some(HTML_NAMESPACE_URI), "style", false),
            vec![first_light_style, second_light_style]
        );
        assert_eq!(
            host.elements_by_tag_name_ns(shadow_root, Some(HTML_NAMESPACE_URI), "style", false,),
            vec![shadow_style]
        );

        assert!(host.insert_before(root, second_light_style, Some(first_light_style)));
        assert_eq!(
            host.elements_by_tag_name_ns(root, Some(HTML_NAMESPACE_URI), "style", false),
            vec![second_light_style, first_light_style]
        );

        let detached_document = host.create_detached_html_document();
        let detached_style = host.create_element("style");
        assert!(host.append_child_without_mutation_effects(detached_document, detached_style));
        assert_eq!(
            host.html_elements_by_local_name_in_document_tree_order(detached_document, "style"),
            vec![detached_style]
        );
        assert_eq!(
            host.elements_by_tag_name_ns(detached_style, Some(HTML_NAMESPACE_URI), "style", true,),
            vec![detached_style]
        );
        assert!(
            host.elements_by_tag_name_ns(detached_style, Some(HTML_NAMESPACE_URI), "style", false,)
                .is_empty()
        );
    }

    #[test]
    fn shadow_including_tag_query_tracks_new_detached_and_moved_candidates() {
        let mut host = test_host();
        let document = host.document_handle();
        let root = host.create_element("section");
        assert!(host.append_child(document, root));
        assert!(
            host.html_elements_by_local_name_in_shadow_including_subtree(root, "form")
                .is_empty()
        );
        let light_form = host.create_element("form");
        let shadow_root = host.attach_shadow_root(root, "closed").unwrap();
        let shadow_form = host.create_element("form");
        let detached_form = host.create_element("form");
        let foreign_form = host
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "form")
            .unwrap();
        assert!(host.append_child(root, light_form));
        assert!(host.append_child(root, foreign_form));
        assert!(host.append_child(shadow_root, shadow_form));
        let forms = host.html_elements_by_local_name_in_shadow_including_subtree(document, "form");
        assert_eq!(forms.len(), 2);
        assert!(forms.contains(&light_form) && forms.contains(&shadow_form));
        assert_eq!(
            host.html_elements_by_local_name_in_shadow_including_subtree(detached_form, "form"),
            vec![detached_form]
        );
        assert!(host.remove_child(document, root));
        assert!(
            host.html_elements_by_local_name_in_shadow_including_subtree(document, "form")
                .is_empty()
        );
        assert_eq!(
            host.html_elements_by_local_name_in_shadow_including_subtree(root, "form")
                .len(),
            2
        );
        assert!(host.append_child(document, shadow_form));
        assert_eq!(
            host.html_elements_by_local_name_in_shadow_including_subtree(document, "form"),
            vec![shadow_form]
        );
    }

    #[test]
    fn tag_queries_keep_subtree_boundaries_with_unrelated_index_candidates() {
        fn assert_query(
            host: &DomHost,
            root: DomHandle,
            name: &str,
            include_root: bool,
            expected: &[DomHandle],
        ) {
            assert_eq!(
                host.elements_by_tag_name_ns(root, Some(HTML_NAMESPACE_URI), name, include_root),
                expected
            );
            assert_eq!(
                host.cached_elements_by_tag_name_in_html_document(root, name, include_root, true),
                expected
            );
        }

        let mut host = test_host();
        let document = host.document_handle();
        let parent = host.create_element("section");
        assert!(host.append_child(document, parent));
        let root = host.create_element("meta");
        let text = host.create_text_node("between elements");
        let nested = host.create_element("meta");
        let outside = host.create_element("meta");
        assert!(host.append_child(parent, root));
        assert!(host.append_child(parent, outside));
        assert!(host.append_child(root, text));
        assert!(host.append_child(root, nested));
        // Materialize both indexes before retained, unrelated nodes are added.
        assert_query(&host, root, "meta", true, &[root, nested]);
        let unrelated = host.create_detached_html_document();
        for _ in 0..64 {
            let meta = host.create_element("meta");
            assert!(host.append_child(unrelated, meta));
        }
        assert_query(&host, root, "meta", true, &[root, nested]);
        assert_query(&host, root, "meta", false, &[nested]);
        assert_query(&host, nested, "meta", true, &[nested]);
        assert_query(&host, nested, "meta", false, &[]);
        assert!(host.insert_before(root, nested, Some(text)));
        assert_query(&host, parent, "meta", false, &[root, nested, outside]);

        let shadow = host.attach_shadow_root(parent, "open").unwrap();
        let shadow_meta = host.create_element("meta");
        assert!(host.append_child(shadow, shadow_meta));
        assert_query(&host, parent, "meta", false, &[root, nested, outside]);
        assert_query(&host, shadow, "meta", false, &[shadow_meta]);
        let fragment = host.create_document_fragment();
        assert!(host.append_child(fragment, root));
        assert_query(&host, fragment, "meta", true, &[root, nested]);
        assert_query(&host, document, "meta", true, &[outside]);

        // Sparse matches must retain the same scope and order in a wider tree.
        let wide = host.create_element("div");
        assert!(host.append_child(parent, wide));
        for _ in 0..64 {
            let leaf = host.create_element("span");
            assert!(host.append_child(wide, leaf));
        }
        let last = host.create_element("needle");
        let first = host.create_element("needle");
        let excluded = host.create_element("needle");
        assert!(host.append_child(wide, first));
        assert!(host.append_child(wide, last));
        assert!(host.append_child(parent, excluded));
        assert_query(&host, wide, "needle", false, &[first, last]);
        assert!(host.insert_before(wide, last, Some(first)));
        assert_query(&host, wide, "needle", false, &[last, first]);
    }

    #[test]
    fn silent_staging_splice_keeps_versions_stable_and_query_index_complete() {
        let mut host = test_host();
        let detached_document = host.create_detached_html_document();
        let detached_style = host.create_element("style");

        assert!(
            host.html_elements_by_local_name_in_document_tree_order(detached_document, "style")
                .is_empty()
        );
        let dom_version = host.dom_version();
        let query_version = host.query_version();

        assert!(host.append_child_without_mutation_effects(detached_document, detached_style));
        assert_eq!(host.dom_version(), dom_version);
        assert_eq!(host.query_version(), query_version);
        assert_eq!(
            host.html_elements_by_local_name_in_document_tree_order(detached_document, "style"),
            vec![detached_style]
        );

        assert!(host.remove_child_without_mutation_effects(detached_document, detached_style));
        assert_eq!(host.dom_version(), dom_version);
        assert_eq!(host.query_version(), query_version);
        assert!(
            host.html_elements_by_local_name_in_document_tree_order(detached_document, "style")
                .is_empty()
        );
    }

    #[test]
    fn shared_script_query_honors_requested_light_tree_root_and_namespace() {
        let mut host = test_host();
        let document = host.document_handle();
        let root = host.create_element("section");
        let html_script = host.create_element("script");
        let svg_script = host
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "script")
            .expect("SVG script");
        let shadow_root = host
            .attach_shadow_root(root, "open")
            .expect("section should host a shadow root");
        let shadow_script = host.create_element("script");

        assert!(host.append_child(document, root));
        assert!(host.append_child(root, html_script));
        assert!(host.append_child(root, svg_script));
        assert!(host.append_child(shadow_root, shadow_script));
        assert_eq!(
            host.script_handles_in_light_subtree(document),
            vec![html_script, svg_script]
        );

        let detached_document = host.create_detached_html_document();
        let detached_script = host.create_element("script");
        assert!(host.append_child_without_mutation_effects(detached_document, detached_script));
        assert_eq!(
            host.script_handles_in_light_subtree(detached_document),
            vec![detached_script]
        );
    }

    #[test]
    fn element_query_index_tracks_connectivity_and_current_document_order() {
        let mut host = test_host();
        let document = host.document_handle();
        let head = host.create_element("head");
        let first = host.create_element("meta");
        let unrelated = host.create_element("div");
        let foreign_meta = host
            .create_element_ns(Some("urn:test"), "meta")
            .expect("foreign meta");

        assert!(host.append_child(document, head));
        assert!(host.append_child(head, first));
        assert!(host.append_child(head, foreign_meta));
        assert_eq!(
            host.html_elements_by_local_name_in_document_tree_order(document, "meta"),
            vec![first]
        );
        assert!(host.element_query_index.borrow().has_materialized_index());

        assert!(host.append_child(head, unrelated));
        assert!(host.element_query_index.borrow().has_materialized_index());
        let second = host.create_element("meta");
        assert!(host.append_child(head, second));
        assert_eq!(
            host.html_elements_by_local_name_in_document_tree_order(document, "meta"),
            vec![first, second]
        );

        assert!(host.insert_before(head, second, Some(first)));
        assert_eq!(
            host.html_elements_by_local_name_in_document_tree_order(document, "meta"),
            vec![second, first]
        );
        assert!(host.remove_child(head, second));
        assert_eq!(
            host.html_elements_by_local_name_in_document_tree_order(document, "meta"),
            vec![first]
        );
    }

    #[test]
    fn prefix_mutation_rekeys_only_qualified_name_identity() {
        let mut host = test_host();
        let document = host.document_handle();
        let element = host
            .create_element_ns(Some(HTML_NAMESPACE_URI), "old:meta")
            .expect("prefixed HTML element");
        assert!(host.append_child(document, element));

        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "OLD:META", true, true),
            vec![element]
        );
        assert_eq!(
            host.html_elements_by_local_name_in_document_tree_order(document, "meta"),
            vec![element]
        );

        assert!(host.set_element_prefix(element, Some("new".to_owned())));
        assert!(
            host.cached_elements_by_tag_name_in_html_document(document, "old:meta", true, true)
                .is_empty()
        );
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "NEW:META", true, true),
            vec![element]
        );
        assert_eq!(
            host.elements_by_tag_name_ns(document, Some(HTML_NAMESPACE_URI), "meta", true,),
            vec![element]
        );
    }

    #[test]
    fn element_query_index_tracks_creation_and_parser_paths_after_materialization() {
        let mut host = test_host();
        let document = host.document_handle();

        assert!(
            host.elements_by_tag_name_ns(document, Some("urn:query"), "late", true)
                .is_empty()
        );
        let created = host
            .create_element_ns(Some("urn:query"), "created:late")
            .expect("created element");
        assert_eq!(
            host.elements_by_tag_name_ns(created, Some("urn:query"), "late", true),
            vec![created]
        );
        assert!(
            host.elements_by_tag_name_ns(created, Some("urn:query"), "late", false)
                .is_empty()
        );

        let parser_created = host.create_parser_element_without_attributes_for_document(
            document,
            "late".to_owned(),
            "urn:query".to_owned(),
            Some("parser".to_owned()),
        );
        assert!(host.append_child(document, parser_created));
        assert_eq!(
            host.elements_by_tag_name_ns(document, Some("urn:query"), "late", true),
            vec![parser_created]
        );
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(document, "parser:late", true, true,),
            vec![parser_created]
        );
    }

    #[test]
    fn element_query_index_tracks_clone_and_import_paths_after_materialization() {
        let mut host = test_host();
        let source = host
            .create_element_ns(Some("urn:clone"), "source:item")
            .expect("source element");
        let source_child = host
            .create_element_ns(Some("urn:clone"), "source:leaf")
            .expect("source child");
        assert!(host.append_child(source, source_child));

        assert_eq!(
            host.elements_by_tag_name_ns(source, Some("urn:clone"), "leaf", false),
            vec![source_child]
        );
        let cloned = host.clone_node(source, true).expect("deep clone");
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(cloned, "source:item", true, false,),
            vec![cloned]
        );
        assert_eq!(
            host.elements_by_tag_name_ns(cloned, Some("urn:clone"), "leaf", false)
                .len(),
            1
        );

        let target_document = host.create_detached_xml_document();
        let imported = host
            .import_node(target_document, source, true)
            .expect("deep import");
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(
                imported,
                "source:item",
                true,
                false,
            ),
            vec![imported]
        );
        assert_eq!(
            host.elements_by_tag_name_ns(imported, Some("urn:clone"), "leaf", false)
                .len(),
            1
        );
    }

    #[test]
    fn document_query_case_and_scope_follow_cross_document_adoption() {
        let mut host = test_host();
        let html_document = host.document_handle();
        let xml_document = host.create_detached_xml_document();
        let element = host.create_element("widget");
        assert!(host.append_child(html_document, element));

        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(html_document, "WIDGET", true, true,),
            vec![element]
        );
        assert!(host.remove_child(html_document, element));
        let version_before_adoption = host.query_version();
        assert_eq!(host.adopt_node(xml_document, element), Some(element));
        assert!(host.query_version() > version_before_adoption);
        assert!(host.append_child(xml_document, element));

        assert!(
            host.cached_elements_by_tag_name_in_html_document(html_document, "widget", true, true,)
                .is_empty()
        );
        assert!(
            host.cached_elements_by_tag_name_in_html_document(xml_document, "WIDGET", true, false,)
                .is_empty()
        );
        assert_eq!(
            host.cached_elements_by_tag_name_in_html_document(xml_document, "widget", true, false,),
            vec![element]
        );
        assert_eq!(
            host.elements_by_tag_name_ns(xml_document, Some(HTML_NAMESPACE_URI), "widget", true,),
            vec![element]
        );
    }

    #[test]
    fn element_handle_by_name_uses_index_until_query_mutation() {
        let mut host = test_host();
        let document = host.document_handle();
        let first = host.create_element("div");
        let second = host.create_element("span");

        assert!(host.set_attribute(first, "name", "target"));
        assert!(host.append_child(document, first));
        assert_eq!(host.element_handle_by_name("target"), Some(first));
        assert_eq!(host.element_handle_by_name("target"), Some(first));
        assert!(host.name_index.borrow().is_some());

        assert!(host.set_attribute(second, "name", "target"));
        assert!(host.name_index.borrow().is_some());
        assert!(host.append_child(document, second));
        assert!(host.name_index.borrow().is_some());
        assert_eq!(host.element_handle_by_name("target"), Some(first));

        assert!(host.remove_attribute(first, "name"));
        assert!(host.name_index.borrow().is_some());
        assert_eq!(host.element_handle_by_name("target"), Some(second));

        let late = host.create_element("div");
        assert!(host.set_attribute(late, "name", "late"));
        assert_eq!(host.element_handle_by_name("late"), None);
        assert!(host.append_child(document, late));
        assert_eq!(host.element_handle_by_name("late"), Some(late));
    }

    #[test]
    fn named_candidate_index_shares_values_between_directions() {
        let mut host = test_host();
        let first = host.create_element("div");
        let second = host.create_element("span");
        for element in [first, second] {
            assert!(host.set_attribute(element, "id", "shared-identifier"));
            assert!(host.append_child(host.document_handle(), element));
        }

        assert_eq!(host.element_handle_by_id("shared-identifier"), Some(first));

        let index = host.id_index.borrow();
        let index = index.as_ref().expect("materialized ID index");
        let forward = index
            .handles_by_value
            .keys()
            .find(|value| value.as_ref() == "shared-identifier")
            .expect("forward index key");
        for element in [first, second] {
            let reverse = index
                .value_by_handle
                .get(&element)
                .expect("reverse index value");
            assert!(forward.ptr_eq(reverse));
        }
        assert_eq!(
            std::mem::size_of::<ThinArcStr>(),
            std::mem::size_of::<usize>()
        );
    }

    #[test]
    fn named_candidate_index_expands_and_compacts_duplicate_values() {
        let mut host = test_host();
        let first = host.create_element("div");
        let second = host.create_element("div");
        assert!(host.set_attribute(first, "id", "duplicate"));
        assert!(host.append_child(host.document_handle(), first));
        assert_eq!(host.element_handle_by_id("duplicate"), Some(first));
        assert!(matches!(
            host.id_index
                .borrow()
                .as_ref()
                .and_then(|index| index.handles_by_value.get("duplicate")),
            Some(NamedElementHandles::One(handle)) if *handle == first
        ));

        assert!(host.set_attribute(second, "id", "duplicate"));
        assert!(host.append_child(host.document_handle(), second));
        assert!(matches!(
            host.id_index
                .borrow()
                .as_ref()
                .and_then(|index| index.handles_by_value.get("duplicate")),
            Some(NamedElementHandles::Many(handles)) if handles.len() == 2
        ));

        assert!(host.remove_attribute(second, "id"));
        assert!(matches!(
            host.id_index
                .borrow()
                .as_ref()
                .and_then(|index| index.handles_by_value.get("duplicate")),
            Some(NamedElementHandles::One(handle)) if *handle == first
        ));
    }

    #[test]
    fn document_root_named_subtree_lookups_use_indexes_without_widening_nested_scope() {
        let mut host = test_host();
        let document = host.document_handle();
        let outside = host.create_element("div");
        let nested_root = host.create_element("section");
        let nested = host.create_element("span");

        assert!(host.set_attribute(outside, "id", "duplicate"));
        assert!(host.set_attribute(outside, "name", "outside-name"));
        assert!(host.set_attribute(nested, "id", "duplicate"));
        assert!(host.set_attribute(nested, "name", "nested-name"));
        assert!(host.append_child(document, outside));
        assert!(host.append_child(document, nested_root));
        assert!(host.append_child(nested_root, nested));
        assert!(host.id_index.borrow().is_none());
        assert!(host.name_index.borrow().is_none());

        assert_eq!(
            host.element_handle_by_id_in_subtree(document, "duplicate"),
            Some(outside)
        );
        assert_eq!(
            host.element_handle_by_name_in_subtree(document, "nested-name"),
            Some(nested)
        );
        assert!(host.id_index.borrow().is_some());
        assert!(host.name_index.borrow().is_some());

        assert_eq!(
            host.element_handle_by_id_in_subtree(nested_root, "duplicate"),
            Some(nested)
        );
        assert_eq!(
            host.element_handle_by_name_in_subtree(nested_root, "outside-name"),
            None
        );
    }

    #[test]
    fn named_candidate_index_uses_current_document_order_after_move() {
        let mut host = test_host();
        let document = host.document_handle();
        let first = host.create_element("div");
        let second = host.create_element("div");

        assert!(host.set_attribute(first, "id", "target"));
        assert!(host.set_attribute(second, "id", "target"));
        assert!(host.append_child(document, first));
        assert!(host.append_child(document, second));
        assert_eq!(host.element_handle_by_id("target"), Some(first));

        assert!(host.insert_before(document, second, Some(first)));
        assert!(host.id_index.borrow().is_some());
        assert_eq!(host.element_handle_by_id("target"), Some(second));
    }

    #[test]
    fn combined_named_candidates_are_deduplicated_filtered_and_document_ordered() {
        let mut host = test_host();
        let document = host.document_handle();
        let id_only = host.create_element("div");
        let id_and_name = host.create_element("span");
        let accepted_name = host.create_element("form");

        assert!(host.set_attribute(id_only, "id", "target"));
        assert!(host.set_attribute(id_and_name, "id", "target"));
        assert!(host.set_attribute(id_and_name, "name", "target"));
        assert!(host.set_attribute(accepted_name, "name", "target"));
        assert!(host.append_child(document, id_only));
        assert!(host.append_child(document, id_and_name));
        assert!(host.append_child(document, accepted_name));

        assert_eq!(
            host.element_handles_by_id_or_name_matching("target", |handle| {
                handle == accepted_name
            }),
            vec![id_only, id_and_name, accepted_name]
        );

        assert!(host.insert_before(document, accepted_name, Some(id_only)));
        assert_eq!(
            host.element_handles_by_id_or_name_matching("target", |handle| {
                handle == accepted_name
            }),
            vec![accepted_name, id_only, id_and_name]
        );

        assert!(host.remove_attribute(id_and_name, "id"));
        assert_eq!(
            host.element_handles_by_id_or_name_matching("target", |handle| {
                handle == accepted_name
            }),
            vec![accepted_name, id_only]
        );

        let nested_root = host.create_element("section");
        let nested_id = host.create_element("div");
        let rejected_nested_name = host.create_element("span");
        let accepted_nested_name = host.create_element("form");
        assert!(host.set_attribute(nested_id, "id", "target"));
        assert!(host.set_attribute(rejected_nested_name, "name", "target"));
        assert!(host.set_attribute(accepted_nested_name, "name", "target"));
        assert!(host.append_child(document, nested_root));
        assert!(host.append_child(nested_root, nested_id));
        assert!(host.append_child(nested_root, rejected_nested_name));
        assert!(host.append_child(nested_root, accepted_nested_name));

        assert_eq!(
            host.element_handles_by_id_or_name_matching_in_subtree(
                nested_root,
                "target",
                |handle| handle == accepted_nested_name,
            ),
            vec![nested_id, accepted_nested_name]
        );
    }

    #[test]
    fn named_candidate_index_tracks_deep_subtree_connectivity_without_rebuild() {
        let mut host = test_host();
        let document = host.document_handle();
        let root = host.create_element("div");
        let mut parent = root;
        for _ in 0..4096 {
            let child = host.create_element("div");
            assert!(host.append_child(parent, child));
            parent = child;
        }

        assert!(host.set_attribute(parent, "name", "target"));
        assert_eq!(host.element_handle_by_name("target"), None);
        assert!(host.name_index.borrow().is_some());

        assert!(host.append_child(document, root));
        assert!(host.name_index.borrow().is_some());
        assert_eq!(host.element_handle_by_name("target"), Some(parent));

        assert!(host.remove_child(document, root));
        assert!(host.name_index.borrow().is_some());
        assert_eq!(host.element_handle_by_name("target"), None);
    }
}
