use crate::native::host::StylesheetCandidateChanges;
use crate::native::{NativeDom, NativeNodeId, Node};

#[derive(Clone, Copy)]
enum StylesheetCandidateRegistryMutation {
    None,
    Register,
    Unregister,
}

#[derive(Default)]
pub(in crate::native::host) struct OwnerLifecycleChanges {
    stylesheet_candidates: StylesheetCandidateChanges,
    stylesheet_owners: Vec<NativeNodeId>,
    contains_base_state_owner: bool,
}

impl OwnerLifecycleChanges {
    fn merge(&mut self, other: Self) {
        self.stylesheet_candidates
            .merge(other.stylesheet_candidates);
        self.stylesheet_owners.extend(other.stylesheet_owners);
        self.contains_base_state_owner |= other.contains_base_state_owner;
    }
}

struct OwnerLifecycleTraversal {
    registry_mutation: StylesheetCandidateRegistryMutation,
    candidate_tree_scope: Option<NativeNodeId>,
    collect_stylesheet_owners: bool,
    detect_base_state_owner: bool,
    changes: OwnerLifecycleChanges,
}

impl OwnerLifecycleTraversal {
    fn new(
        registry_mutation: StylesheetCandidateRegistryMutation,
        candidate_tree_scope: Option<NativeNodeId>,
        collect_stylesheet_owners: bool,
        detect_base_state_owner: bool,
    ) -> Self {
        Self {
            registry_mutation,
            candidate_tree_scope,
            collect_stylesheet_owners,
            detect_base_state_owner,
            changes: OwnerLifecycleChanges::default(),
        }
    }
}

impl NativeDom {
    pub(crate) fn insert_before_with_stylesheet_candidate_changes(
        &mut self,
        parent: NativeNodeId,
        child: NativeNodeId,
        reference_child: Option<NativeNodeId>,
    ) -> Option<StylesheetCandidateChanges> {
        self.insert_before_with_stylesheet_candidate_changes_and_registration(
            parent,
            child,
            reference_child,
            true,
        )
        .map(|changes| changes.stylesheet_candidates)
    }

    fn insert_before_with_stylesheet_candidate_changes_and_registration(
        &mut self,
        parent: NativeNodeId,
        child: NativeNodeId,
        mut reference_child: Option<NativeNodeId>,
        commit_candidate_registration: bool,
    ) -> Option<OwnerLifecycleChanges> {
        if !self.can_have_children(parent)
            || self.is_host_including_inclusive_ancestor(child, parent)
        {
            return None;
        }
        let parent_type = self.node(parent)?.node_type();
        let child_type = self.node(child)?.node_type();
        if !Self::can_insert_child_type(parent_type, child_type) {
            return None;
        }
        if let Some(reference) = reference_child {
            if self.parent_node(reference) != Some(parent) {
                return None;
            }
            if reference == child {
                // DOM pre-insert changes a self-reference to the node's next
                // sibling before adopting (and therefore removing) the node.
                // This is observable through MutationObserver and DOMDebugger.
                reference_child = self.next_sibling(child);
            }
        }
        if self
            .node(child)
            .is_some_and(|node| node.data().is_document_fragment())
        {
            let fragment_children = self.child_ids(child).collect::<Vec<_>>();
            let all_children_are_insertable = fragment_children.iter().all(|fragment_child| {
                !self.is_host_including_inclusive_ancestor(*fragment_child, parent)
                    && self.node(*fragment_child).is_some_and(|node| {
                        !node.data().is_document_fragment()
                            && Self::can_insert_child_type(parent_type, node.node_type())
                    })
            });
            if !all_children_are_insertable {
                return None;
            }

            let mut changes = OwnerLifecycleChanges::default();
            for &fragment_child in &fragment_children {
                changes.merge(self.commit_validated_non_fragment_insertion(
                    parent,
                    fragment_child,
                    reference_child,
                    false,
                ));
            }
            if commit_candidate_registration
                && let Some(tree_scope) = self.stylesheet_candidate_tree_scope_for_node(parent)
            {
                self.register_stylesheet_candidates(
                    tree_scope,
                    changes.stylesheet_candidates.registered(),
                );
            }
            if commit_candidate_registration
                && changes.contains_base_state_owner
                && let Some(document) = self.document_tree_owner(parent)
            {
                self.process_base_element(document, false);
            }
            return Some(changes);
        }

        Some(self.commit_validated_non_fragment_insertion(
            parent,
            child,
            reference_child,
            commit_candidate_registration,
        ))
    }

    /// Commit one insertion after the complete operation has passed preflight.
    ///
    /// Keeping this path infallible prevents a DocumentFragment batch from
    /// returning after only a prefix of its children has moved.
    fn commit_validated_non_fragment_insertion(
        &mut self,
        parent: NativeNodeId,
        child: NativeNodeId,
        reference_child: Option<NativeNodeId>,
        commit_candidate_registration: bool,
    ) -> OwnerLifecycleChanges {
        debug_assert!(self.can_have_children(parent));
        debug_assert!(self.node(child).is_some_and(|node| {
            !node.data().is_document_fragment()
                && self.node(parent).is_some_and(|parent| {
                    Self::can_insert_child_type(parent.node_type(), node.node_type())
                })
        }));
        debug_assert!(
            reference_child.is_none()
                || reference_child.is_some_and(|reference_child| {
                    self.parent_node(reference_child) == Some(parent) && reference_child != child
                })
        );

        let mut changes = self.detach_from_parent_with_owner_lifecycle_changes(child);

        let previous = match reference_child {
            Some(reference_child) => self.previous_sibling(reference_child),
            None => self.last_child(parent),
        };
        {
            let child_node = self.node_mut(child).expect("child must exist");
            child_node.adopt_into(parent, previous, reference_child);
        }

        match previous {
            Some(previous) => {
                self.node_mut(previous)
                    .expect("previous sibling must exist")
                    .set_next_sibling(Some(child));
            }
            None => {
                self.node_mut(parent)
                    .expect("parent must exist")
                    .set_first_child(Some(child));
            }
        }

        match reference_child {
            Some(reference_child) => {
                self.node_mut(reference_child)
                    .expect("reference child must exist")
                    .set_prev_sibling(Some(child));
            }
            None => {
                self.node_mut(parent)
                    .expect("parent must exist")
                    .set_last_child(Some(child));
            }
        }

        if reference_child.is_some()
            && self
                .node(parent)
                .is_some_and(|node| node.last_child().is_none())
        {
            self.node_mut(parent)
                .expect("parent must exist")
                .set_last_child(Some(child));
        }

        let owner_document = if self.node(parent).and_then(Node::as_document).is_some() {
            Some(parent)
        } else {
            self.node(parent).and_then(Node::owner_document)
        };
        let parent_connected = self
            .node(parent)
            .is_some_and(|node| node.flags().connected());
        let inserted_changes = self.retarget_subtree_and_register_stylesheet_candidates(
            child,
            owner_document,
            parent_connected,
            parent_connected,
            commit_candidate_registration,
            true,
        );
        let inserted_base_state_owner = inserted_changes.contains_base_state_owner;
        changes.merge(inserted_changes);
        if commit_candidate_registration
            && inserted_base_state_owner
            && let Some(document) = self.document_tree_owner(child)
        {
            self.process_base_element(document, false);
        }
        changes
    }

    pub(crate) fn remove_child_with_stylesheet_candidate_changes(
        &mut self,
        parent: NativeNodeId,
        child: NativeNodeId,
    ) -> Option<StylesheetCandidateChanges> {
        if self.parent_node(child) != Some(parent) {
            return None;
        }
        Some(self.detach_from_parent_with_stylesheet_candidate_changes(child))
    }

    pub(crate) fn detach_from_parent_with_stylesheet_candidate_changes(
        &mut self,
        child: NativeNodeId,
    ) -> StylesheetCandidateChanges {
        self.detach_from_parent_with_owner_lifecycle_changes(child)
            .stylesheet_candidates
    }

    fn detach_from_parent_with_owner_lifecycle_changes(
        &mut self,
        child: NativeNodeId,
    ) -> OwnerLifecycleChanges {
        let Some(parent) = self.parent_node(child) else {
            return OwnerLifecycleChanges::default();
        };
        let old_tree_scope = self.stylesheet_candidate_tree_scope_for_node(child);
        let owner_document = self.node(child).and_then(Node::owner_document);
        let base_document = self.document_tree_owner(child);
        let previous = self.previous_sibling(child);
        let next = self.next_sibling(child);

        if let Some(previous) = previous {
            self.node_mut(previous)
                .expect("previous sibling must exist")
                .set_next_sibling(next);
        } else {
            self.node_mut(parent)
                .expect("parent must exist")
                .set_first_child(next);
        }

        if let Some(next) = next {
            self.node_mut(next)
                .expect("next sibling must exist")
                .set_prev_sibling(previous);
        } else {
            self.node_mut(parent)
                .expect("parent must exist")
                .set_last_child(previous);
        }

        self.node_mut(child)
            .expect("child must exist")
            .clear_tree_links();
        let changes = self.retarget_subtree_and_unregister_stylesheet_candidates(
            child,
            owner_document,
            false,
            false,
            old_tree_scope,
            base_document.is_some(),
        );
        if changes.contains_base_state_owner
            && let Some(document) = base_document
        {
            self.process_base_element(document, false);
        }
        changes
    }

    pub(crate) fn mark_subtree_tree_scope_collecting_stylesheet_owners(
        &mut self,
        node_id: NativeNodeId,
        owner_document: Option<NativeNodeId>,
        connected: bool,
        in_document_tree: bool,
    ) -> Vec<NativeNodeId> {
        let mut traversal = OwnerLifecycleTraversal::new(
            StylesheetCandidateRegistryMutation::None,
            None,
            true,
            false,
        );
        self.retarget_owner_lifecycle_subtree(
            node_id,
            owner_document,
            connected,
            in_document_tree,
            &mut traversal,
        );
        traversal.changes.stylesheet_owners
    }

    pub(crate) fn mark_subtree_tree_scope_preserving_stylesheet_candidates(
        &mut self,
        node_id: NativeNodeId,
        owner_document: Option<NativeNodeId>,
        connected: bool,
        in_document_tree: bool,
    ) {
        let mut traversal = OwnerLifecycleTraversal::new(
            StylesheetCandidateRegistryMutation::None,
            None,
            false,
            false,
        );
        self.retarget_owner_lifecycle_subtree(
            node_id,
            owner_document,
            connected,
            in_document_tree,
            &mut traversal,
        );
    }

    pub(in crate::native::host) fn retarget_subtree_and_register_stylesheet_candidates(
        &mut self,
        root: NativeNodeId,
        owner_document: Option<NativeNodeId>,
        connected: bool,
        in_document_tree: bool,
        commit_registration: bool,
        detect_base_state_owner: bool,
    ) -> OwnerLifecycleChanges {
        let tree_scope = self.stylesheet_candidate_tree_scope_for_node(root);
        let mutation = tree_scope
            .map(|_| StylesheetCandidateRegistryMutation::Register)
            .unwrap_or(StylesheetCandidateRegistryMutation::None);
        let mut traversal =
            OwnerLifecycleTraversal::new(mutation, tree_scope, false, detect_base_state_owner);
        self.retarget_owner_lifecycle_subtree(
            root,
            owner_document,
            connected,
            in_document_tree,
            &mut traversal,
        );
        if commit_registration && let Some(tree_scope) = tree_scope {
            self.register_stylesheet_candidates(
                tree_scope,
                traversal.changes.stylesheet_candidates.registered(),
            );
        }
        traversal.changes
    }

    fn retarget_subtree_and_unregister_stylesheet_candidates(
        &mut self,
        root: NativeNodeId,
        owner_document: Option<NativeNodeId>,
        connected: bool,
        in_document_tree: bool,
        old_tree_scope: Option<NativeNodeId>,
        detect_base_state_owner: bool,
    ) -> OwnerLifecycleChanges {
        let mutation = old_tree_scope
            .map(|_| StylesheetCandidateRegistryMutation::Unregister)
            .unwrap_or(StylesheetCandidateRegistryMutation::None);
        let mut traversal =
            OwnerLifecycleTraversal::new(mutation, old_tree_scope, false, detect_base_state_owner);
        self.retarget_owner_lifecycle_subtree(
            root,
            owner_document,
            connected,
            in_document_tree,
            &mut traversal,
        );
        if let Some(tree_scope) = old_tree_scope {
            self.unregister_stylesheet_candidates(
                tree_scope,
                traversal.changes.stylesheet_candidates.unregistered(),
            );
        }
        traversal.changes
    }

    fn retarget_owner_lifecycle_subtree(
        &mut self,
        root: NativeNodeId,
        owner_document: Option<NativeNodeId>,
        connected: bool,
        in_document_tree: bool,
        traversal: &mut OwnerLifecycleTraversal,
    ) {
        let mut stack = vec![root];
        let mut template_contents_to_adopt = Vec::new();
        while let Some(handle) = stack.pop() {
            let previous_owner_document = self.node(handle).and_then(Node::owner_document);
            let template_contents = (previous_owner_document != owner_document)
                .then(|| {
                    self.node(handle)
                        .and_then(Node::as_element)
                        .filter(|element| element.is_html_element("template"))
                        .and_then(|element| element.template_contents())
                })
                .flatten();
            let maintains_candidates = traversal.collect_stylesheet_owners
                || !matches!(
                    traversal.registry_mutation,
                    StylesheetCandidateRegistryMutation::None
                );
            let is_candidate = maintains_candidates && self.is_stylesheet_candidate(handle);
            if is_candidate {
                if traversal.collect_stylesheet_owners {
                    traversal.changes.stylesheet_owners.push(handle);
                }
                match traversal.registry_mutation {
                    StylesheetCandidateRegistryMutation::None => {}
                    StylesheetCandidateRegistryMutation::Register => {
                        if let Some(tree_scope) = traversal.candidate_tree_scope {
                            traversal
                                .changes
                                .stylesheet_candidates
                                .record_registered(handle, tree_scope);
                        }
                    }
                    StylesheetCandidateRegistryMutation::Unregister => {
                        if let Some(tree_scope) = traversal.candidate_tree_scope {
                            traversal
                                .changes
                                .stylesheet_candidates
                                .record_unregistered(handle, tree_scope);
                        }
                    }
                }
            }
            if traversal.detect_base_state_owner && self.is_base_state_owner(handle) {
                traversal.changes.contains_base_state_owner = true;
            }
            let node = self.node_mut(handle).expect("node must exist");
            node.set_tree_scope(owner_document, connected, in_document_tree);
            stack.extend(self.child_ids_reversed(handle));
            if let (Some(template_contents), Some(document)) = (template_contents, owner_document) {
                template_contents_to_adopt.push((template_contents, document));
            }
        }
        self.retarget_adopted_template_contents(template_contents_to_adopt, traversal);
    }

    fn retarget_adopted_template_contents(
        &mut self,
        mut pending: Vec<(NativeNodeId, NativeNodeId)>,
        traversal: &mut OwnerLifecycleTraversal,
    ) {
        while let Some((contents, template_document)) = pending.pop() {
            let contents_document =
                self.appropriate_template_contents_owner_document(template_document);
            let mut stack = vec![contents];
            while let Some(handle) = stack.pop() {
                let previous_owner_document = self.node(handle).and_then(Node::owner_document);
                let nested_template_contents = (previous_owner_document != Some(contents_document))
                    .then(|| {
                        self.node(handle)
                            .and_then(Node::as_element)
                            .filter(|element| element.is_html_element("template"))
                            .and_then(|element| element.template_contents())
                    })
                    .flatten();
                if traversal.collect_stylesheet_owners && self.is_stylesheet_candidate(handle) {
                    traversal.changes.stylesheet_owners.push(handle);
                }
                self.node_mut(handle)
                    .expect("template content node must exist")
                    .set_tree_scope(Some(contents_document), false, false);
                stack.extend(self.child_ids_reversed(handle));
                if let Some(nested_template_contents) = nested_template_contents {
                    pending.push((nested_template_contents, contents_document));
                }
            }
        }
    }
}
