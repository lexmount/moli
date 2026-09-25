use std::{collections::HashMap, sync::Arc};

use super::*;

/// The canonical stylesheet candidate membership for every DOM TreeScope.
///
/// A Document is intrinsically a TreeScope. ShadowRoot handles are registered
/// explicitly when the binding is created because their backing node is also
/// represented as a DocumentFragment. Candidate membership is independent of
/// whether the scope is currently connected to the main document.
#[derive(Debug, Clone, Default)]
pub(crate) struct StylesheetCandidateRegistries {
    candidates_by_tree_scope: Arc<HashMap<DomHandle, Arc<Vec<DomHandle>>>>,
}

/// Cheap immutable candidate views grouped by their owning DOM TreeScope.
///
/// Candidate handles are in tree order within each scope. The map deliberately
/// exposes no ordering between TreeScopes because no cross-scope cascade order
/// exists.
#[derive(Debug, Clone, Default)]
pub struct StylesheetCandidateTreeScopeSnapshots {
    candidate_handles_by_tree_scope: HashMap<DomHandle, Arc<Vec<DomHandle>>>,
}

impl StylesheetCandidateTreeScopeSnapshots {
    pub fn get(&self, tree_scope: DomHandle) -> Option<&Arc<Vec<DomHandle>>> {
        self.candidate_handles_by_tree_scope.get(&tree_scope)
    }

    pub fn iter(&self) -> impl Iterator<Item = (DomHandle, &Arc<Vec<DomHandle>>)> {
        self.candidate_handles_by_tree_scope
            .iter()
            .map(|(tree_scope, candidates)| (*tree_scope, candidates))
    }

    pub fn len(&self) -> usize {
        self.candidate_handles_by_tree_scope.len()
    }

    pub fn is_empty(&self) -> bool {
        self.candidate_handles_by_tree_scope.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StylesheetCandidateChanges {
    registered: Vec<StylesheetCandidateTransition>,
    unregistered: Vec<StylesheetCandidateTransition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StylesheetCandidateTransition {
    owner: DomHandle,
    tree_scope: DomHandle,
}

impl StylesheetCandidateChanges {
    pub(super) fn registered(&self) -> &[StylesheetCandidateTransition] {
        &self.registered
    }

    pub(super) fn unregistered(&self) -> &[StylesheetCandidateTransition] {
        &self.unregistered
    }

    pub(super) fn into_owner_changes(self) -> impl Iterator<Item = DomStylesheetOwnerChange> {
        self.unregistered
            .into_iter()
            .map(|change| DomStylesheetOwnerChange::unregistered(change.owner, change.tree_scope))
            .chain(self.registered.into_iter().map(|change| {
                DomStylesheetOwnerChange::registered(change.owner, change.tree_scope)
            }))
    }

    pub(super) fn merge(&mut self, other: Self) {
        self.registered.extend(other.registered);
        self.unregistered.extend(other.unregistered);
    }

    pub(super) fn record_registered(&mut self, owner: DomHandle, tree_scope: DomHandle) {
        self.registered
            .push(StylesheetCandidateTransition { owner, tree_scope });
    }

    pub(super) fn record_unregistered(&mut self, owner: DomHandle, tree_scope: DomHandle) {
        self.unregistered
            .push(StylesheetCandidateTransition { owner, tree_scope });
    }
}

impl StylesheetCandidateTransition {
    pub(super) fn owner(self) -> DomHandle {
        self.owner
    }

    pub(super) fn tree_scope(self) -> DomHandle {
        self.tree_scope
    }
}

impl NativeDom {
    pub fn stylesheet_candidate_handles_for_tree_scope(
        &self,
        tree_scope: NativeNodeId,
    ) -> Arc<Vec<NativeNodeId>> {
        if !self.is_stylesheet_candidate_tree_scope(tree_scope) {
            return Arc::new(Vec::new());
        }
        self.stylesheet_candidate_registries
            .candidates_by_tree_scope
            .get(&tree_scope)
            .cloned()
            .unwrap_or_default()
    }

    pub fn stylesheet_candidate_handles_before_in_tree_scope(
        &self,
        tree_scope: NativeNodeId,
        stop_at: Option<NativeNodeId>,
    ) -> Vec<NativeNodeId> {
        let candidates = self.stylesheet_candidate_handles_for_tree_scope(tree_scope);
        let Some(stop_at) = stop_at.filter(|stop_at| {
            self.stylesheet_candidate_tree_scope_for_node(*stop_at) == Some(tree_scope)
        }) else {
            return candidates.as_ref().clone();
        };
        candidates
            .iter()
            .copied()
            .take_while(|candidate| self.handle_precedes(*candidate, stop_at))
            .collect()
    }

    pub(crate) fn register_stylesheet_candidate_tree_scope(&mut self, tree_scope: NativeNodeId) {
        if !self
            .node(tree_scope)
            .is_some_and(|node| node.is_document_fragment())
        {
            return;
        }
        if self
            .stylesheet_candidate_registries
            .candidates_by_tree_scope
            .contains_key(&tree_scope)
        {
            return;
        }
        Arc::make_mut(
            &mut self
                .stylesheet_candidate_registries
                .candidates_by_tree_scope,
        )
        .entry(tree_scope)
        .or_insert_with(|| Arc::new(Vec::new()));
        let Some(node) = self.node(tree_scope) else {
            return;
        };
        let owner_document = node.owner_document();
        let connected = node.flags().connected();
        let _ = self.retarget_subtree_and_register_stylesheet_candidates(
            tree_scope,
            owner_document,
            connected,
            false,
            true,
            false,
        );
    }

    pub(super) fn register_stylesheet_candidates(
        &mut self,
        tree_scope: NativeNodeId,
        new_candidates: &[StylesheetCandidateTransition],
    ) {
        let Some(first_candidate) = new_candidates.first().copied() else {
            return;
        };
        debug_assert!(
            new_candidates
                .iter()
                .all(|candidate| candidate.tree_scope() == tree_scope)
        );
        let insertion_index = self
            .stylesheet_candidate_registries
            .candidates_by_tree_scope
            .get(&tree_scope)
            .map(|existing_candidates| {
                existing_candidates.partition_point(|existing| {
                    self.handle_precedes(*existing, first_candidate.owner())
                })
            })
            .unwrap_or(0);
        let candidates = Arc::make_mut(
            Arc::make_mut(
                &mut self
                    .stylesheet_candidate_registries
                    .candidates_by_tree_scope,
            )
            .entry(tree_scope)
            .or_insert_with(|| Arc::new(Vec::new())),
        );
        candidates.splice(
            insertion_index..insertion_index,
            new_candidates.iter().map(|candidate| candidate.owner()),
        );
    }

    pub(super) fn unregister_stylesheet_candidates(
        &mut self,
        tree_scope: NativeNodeId,
        removed_candidates: &[StylesheetCandidateTransition],
    ) {
        let Some(first_candidate) = removed_candidates.first().copied() else {
            return;
        };
        debug_assert!(
            removed_candidates
                .iter()
                .all(|candidate| candidate.tree_scope() == tree_scope)
        );
        let Some(candidates) = Arc::make_mut(
            &mut self
                .stylesheet_candidate_registries
                .candidates_by_tree_scope,
        )
        .get_mut(&tree_scope) else {
            return;
        };
        let candidates = Arc::make_mut(candidates);
        let first_owner = first_candidate.owner();
        let last_owner = removed_candidates
            .last()
            .map(|candidate| candidate.owner())
            .unwrap_or(first_owner);
        let batch_len = removed_candidates.len();
        let registry_len = candidates.len();
        let start = candidates
            .iter()
            .position(|candidate| *candidate == first_owner)
            .unwrap_or_else(|| {
                panic!(
                    "stylesheet candidate batch is missing from its old tree scope: \
                     tree_scope={tree_scope:?}, first_owner={first_owner:?}, \
                     last_owner={last_owner:?}, batch_len={batch_len}, \
                     registry_len={registry_len}"
                )
            });
        let end = start + batch_len;
        assert!(
            candidates.get(start..end).is_some_and(|existing| {
                existing
                    .iter()
                    .copied()
                    .eq(removed_candidates.iter().map(|candidate| candidate.owner()))
            }),
            "stylesheet candidate batch is not contiguous in tree order: \
             tree_scope={tree_scope:?}, first_owner={first_owner:?}, \
             last_owner={last_owner:?}, batch_len={batch_len}, start={start}, \
             end={end}, registry_len={registry_len}"
        );
        candidates.drain(start..end);
    }

    pub(super) fn stylesheet_candidate_tree_scope_for_node(
        &self,
        handle: NativeNodeId,
    ) -> Option<NativeNodeId> {
        let mut root = handle;
        while let Some(parent) = self.parent_node(root) {
            root = parent;
        }
        self.is_stylesheet_candidate_tree_scope(root)
            .then_some(root)
    }

    fn is_stylesheet_candidate_tree_scope(&self, handle: NativeNodeId) -> bool {
        self.node(handle).is_some_and(Node::is_document)
            || self
                .stylesheet_candidate_registries
                .candidates_by_tree_scope
                .contains_key(&handle)
    }

    pub(super) fn is_stylesheet_candidate(&self, handle: NativeNodeId) -> bool {
        self.node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.is_html_element("link") || element.is_inline_style_element()
            })
    }

    fn handle_precedes(&self, left: NativeNodeId, right: NativeNodeId) -> bool {
        self.node(left).is_some_and(|node| {
            node.compare_document_position(self, right) & Node::DOCUMENT_POSITION_FOLLOWING != 0
        })
    }
}

impl DomHost {
    pub fn is_inline_style_sheet_owner(&self, handle: DomHandle) -> bool {
        self.node(handle)
            .and_then(Node::as_element)
            .is_some_and(Element::is_inline_style_element)
    }

    pub fn is_style_element_parsing_children(&self, handle: DomHandle) -> bool {
        self.node(handle)
            .and_then(Node::as_element)
            .is_some_and(Element::style_children_parsing)
    }

    pub fn stylesheet_candidate_handles_for_tree_scope(
        &self,
        tree_scope: DomHandle,
    ) -> Arc<Vec<DomHandle>> {
        self.dom
            .stylesheet_candidate_handles_for_tree_scope(tree_scope)
    }

    pub fn stylesheet_candidate_handles_before_in_tree_scope(
        &self,
        tree_scope: DomHandle,
        stop_at: Option<DomHandle>,
    ) -> Vec<DomHandle> {
        self.dom
            .stylesheet_candidate_handles_before_in_tree_scope(tree_scope, stop_at)
    }

    pub fn stylesheet_candidate_tree_scope_snapshots_for_document(
        &self,
        document: DomHandle,
    ) -> StylesheetCandidateTreeScopeSnapshots {
        if !self.node(document).is_some_and(Node::is_document) {
            return StylesheetCandidateTreeScopeSnapshots::default();
        }
        let mut candidate_handles_by_tree_scope = HashMap::new();
        candidate_handles_by_tree_scope.insert(
            document,
            self.stylesheet_candidate_handles_for_tree_scope(document),
        );
        let shadow_roots = self
            .shadow_roots_by_host
            .borrow()
            .values()
            .map(|state| state.handle)
            .collect::<Vec<_>>();
        for shadow_root in shadow_roots {
            if self.shadow_tree_scope_belongs_to_document(shadow_root, document) {
                candidate_handles_by_tree_scope.insert(
                    shadow_root,
                    self.stylesheet_candidate_handles_for_tree_scope(shadow_root),
                );
            }
        }
        StylesheetCandidateTreeScopeSnapshots {
            candidate_handles_by_tree_scope,
        }
    }

    fn shadow_tree_scope_belongs_to_document(
        &self,
        shadow_root: DomHandle,
        document: DomHandle,
    ) -> bool {
        let mut scope = shadow_root;
        loop {
            let Some(host) = self.shadow_root_host(scope) else {
                return false;
            };
            let Some(host_scope) = self.root_node_handle(host) else {
                return false;
            };
            if host_scope == document {
                return true;
            }
            if !self.is_shadow_root(host_scope) {
                return false;
            }
            scope = host_scope;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            Url::parse("https://example.test/").expect("test URL parses"),
        ))
    }

    #[test]
    fn registry_is_tree_ordered_and_isolated_by_tree_scope() {
        let mut host = test_host();
        let document = host.document_handle();
        let container = host.create_element("div");
        let link = host.create_element("link");
        let style = host.create_element("style");
        let host_element = host.create_element("section");
        let shadow_root = host
            .attach_shadow_root(host_element, "open")
            .expect("section accepts a shadow root");
        let shadow_style = host.create_element("style");

        assert!(host.append_child(document, container));
        assert!(host.append_child(container, link));
        assert!(host.append_child(container, style));
        assert!(host.append_child(document, host_element));
        assert!(host.append_child(shadow_root, shadow_style));

        assert_eq!(
            host.stylesheet_candidate_handles_for_tree_scope(document),
            vec![link, style].into()
        );
        assert_eq!(
            host.stylesheet_candidate_handles_for_tree_scope(shadow_root),
            vec![shadow_style].into()
        );
        let snapshots = host.stylesheet_candidate_tree_scope_snapshots_for_document(document);
        assert_eq!(snapshots.len(), 2);
        let document_snapshot = snapshots
            .get(document)
            .expect("document candidate snapshot");
        let shadow_snapshot = snapshots
            .get(shadow_root)
            .expect("shadow candidate snapshot");
        assert_eq!(document_snapshot.as_ref(), &[link, style]);
        assert_eq!(shadow_snapshot.as_ref(), &[shadow_style]);
        assert!(Arc::ptr_eq(
            document_snapshot,
            &host.stylesheet_candidate_handles_for_tree_scope(document),
        ));
        assert!(Arc::ptr_eq(
            shadow_snapshot,
            &host.stylesheet_candidate_handles_for_tree_scope(shadow_root),
        ));

        assert!(host.insert_before(container, style, Some(link)));
        assert_eq!(
            host.stylesheet_candidate_handles_for_tree_scope(document),
            vec![style, link].into()
        );
        assert!(host.remove_child(container, style));
        assert_eq!(
            host.stylesheet_candidate_handles_for_tree_scope(document),
            vec![link].into()
        );

        let wrapper = host.create_element("div");
        let nested_style = host.create_element("style");
        assert!(host.append_child(wrapper, nested_style));
        let inserted = host.append_child_effects(document, wrapper);
        assert!(inserted.stylesheet_owners().changes().iter().any(|change| {
            change.owner() == nested_style
                && matches!(change.kind(), DomStylesheetOwnerChangeKind::Registered)
                && change.tree_scopes().old().is_none()
                && change.tree_scopes().current_scope().is_none()
                && change.tree_scopes().new_scope() == Some(document)
        }));
        assert_eq!(
            host.stylesheet_candidate_handles_for_tree_scope(document),
            vec![link, nested_style].into()
        );

        let moved = host.append_child_effects(shadow_root, nested_style);
        let moved_kinds = moved
            .stylesheet_owners()
            .changes()
            .iter()
            .filter(|change| change.owner() == nested_style)
            .map(DomStylesheetOwnerChange::kind)
            .collect::<Vec<_>>();
        assert_eq!(
            moved_kinds,
            vec![
                &DomStylesheetOwnerChangeKind::Unregistered,
                &DomStylesheetOwnerChangeKind::Registered,
            ]
        );
        assert_eq!(
            moved
                .stylesheet_owners()
                .changes()
                .iter()
                .filter(|change| change.owner() == nested_style)
                .map(DomStylesheetOwnerChange::tree_scopes)
                .collect::<Vec<_>>(),
            vec![
                DomStylesheetOwnerTreeScopes::from_parts(Some(document), None, None),
                DomStylesheetOwnerTreeScopes::from_parts(None, None, Some(shadow_root)),
            ]
        );
        assert_eq!(
            host.stylesheet_candidate_handles_for_tree_scope(shadow_root),
            vec![shadow_style, nested_style].into()
        );
    }

    #[test]
    fn detached_document_and_silent_construction_keep_canonical_membership() {
        let mut host = test_host();
        let detached_document = host.create_detached_html_document();
        let style = host.create_element("style");
        let ordinary_fragment = host.create_document_fragment();
        let fragment_style = host.create_element("style");

        assert!(host.append_child_without_mutation_effects(detached_document, style));
        assert!(host.append_child_without_mutation_effects(ordinary_fragment, fragment_style));
        assert_eq!(
            host.stylesheet_candidate_handles_for_tree_scope(detached_document),
            vec![style].into()
        );
        assert!(
            host.stylesheet_candidate_handles_for_tree_scope(ordinary_fragment)
                .is_empty()
        );
    }

    #[test]
    fn registry_reads_and_snapshots_share_each_scope_until_that_scope_mutates() {
        let mut host = test_host();
        let document = host.document_handle();
        let style = host.create_element("style");
        let shadow_host = host.create_element("section");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("section accepts a shadow root");
        let shadow_style = host.create_element("style");
        assert!(host.append_child(document, style));
        assert!(host.append_child(shadow_root, shadow_style));

        let snapshot = host.dom().clone();
        let live_document_read = host.stylesheet_candidate_handles_for_tree_scope(document);
        let second_document_read = host.stylesheet_candidate_handles_for_tree_scope(document);
        let snapshot_document_read = snapshot.stylesheet_candidate_handles_for_tree_scope(document);
        let snapshot_shadow_read =
            snapshot.stylesheet_candidate_handles_for_tree_scope(shadow_root);
        assert!(Arc::ptr_eq(&live_document_read, &second_document_read));
        assert!(Arc::ptr_eq(&live_document_read, &snapshot_document_read));
        assert!(Arc::ptr_eq(
            &host
                .dom()
                .stylesheet_candidate_registries
                .candidates_by_tree_scope,
            &snapshot
                .stylesheet_candidate_registries
                .candidates_by_tree_scope,
        ));

        assert!(host.remove_child(document, style));
        assert!(!Arc::ptr_eq(
            &host
                .dom()
                .stylesheet_candidate_registries
                .candidates_by_tree_scope,
            &snapshot
                .stylesheet_candidate_registries
                .candidates_by_tree_scope,
        ));
        let live_document_after = host.stylesheet_candidate_handles_for_tree_scope(document);
        let live_shadow_after = host.stylesheet_candidate_handles_for_tree_scope(shadow_root);
        assert!(!Arc::ptr_eq(&live_document_after, &snapshot_document_read));
        assert!(Arc::ptr_eq(&live_shadow_after, &snapshot_shadow_read));
        assert_eq!(
            snapshot.stylesheet_candidate_handles_for_tree_scope(document),
            vec![style].into()
        );
    }

    #[test]
    fn clonable_shadow_root_construction_registers_existing_candidates() {
        let mut host = test_host();
        let element = host.create_element("section");
        let mut init = ShadowRootInit::new("open");
        init.set_clonable(true);
        let shadow_root = host
            .attach_shadow_root_with_init(element, init)
            .expect("section accepts a clonable shadow root");
        let style = host.create_element("style");
        assert!(host.append_child(shadow_root, style));

        let clone = host
            .clone_node(element, true)
            .expect("host with clonable shadow root clones");
        let cloned_shadow_root = host
            .shadow_root_handle(clone)
            .expect("clone retains shadow root binding");
        let cloned_candidates =
            host.stylesheet_candidate_handles_for_tree_scope(cloned_shadow_root);
        assert_eq!(cloned_candidates.len(), 1);
        assert!(host.is_html_element_named(cloned_candidates[0], "style"));
        assert_ne!(cloned_candidates[0], style);
    }

    #[test]
    fn before_query_uses_arbitrary_parser_boundary_without_rescanning() {
        let mut host = test_host();
        let document = host.document_handle();
        let first = host.create_element("style");
        let script = host.create_element("script");
        let second = host.create_element("link");
        assert!(host.append_child(document, first));
        assert!(host.append_child(document, script));
        assert!(host.append_child(document, second));

        assert_eq!(
            host.stylesheet_candidate_handles_before_in_tree_scope(document, Some(script)),
            vec![first]
        );
    }

    #[test]
    fn document_fragment_candidates_register_as_one_tree_ordered_transition() {
        let mut host = test_host();
        let document = host.document_handle();
        let before = host.create_element("style");
        let after = host.create_element("link");
        let fragment = host.create_document_fragment();
        let first = host.create_element("style");
        let ordinary = host.create_element("div");
        let second = host.create_element("link");

        assert!(host.append_child(document, before));
        assert!(host.append_child(document, after));
        assert!(host.append_child(fragment, first));
        assert!(host.append_child(fragment, ordinary));
        assert!(host.append_child(fragment, second));

        let effects = host.insert_before_effects(document, fragment, Some(after));

        assert_eq!(
            effects
                .stylesheet_owners()
                .changes()
                .iter()
                .map(|change| (change.owner(), change.kind()))
                .collect::<Vec<_>>(),
            vec![
                (first, &DomStylesheetOwnerChangeKind::Registered),
                (second, &DomStylesheetOwnerChangeKind::Registered),
            ]
        );
        assert_eq!(
            effects
                .stylesheet_owners()
                .changes()
                .iter()
                .map(DomStylesheetOwnerChange::tree_scopes)
                .collect::<Vec<_>>(),
            vec![
                DomStylesheetOwnerTreeScopes::from_parts(None, None, Some(document)),
                DomStylesheetOwnerTreeScopes::from_parts(None, None, Some(document)),
            ]
        );
        assert_eq!(
            host.stylesheet_candidate_handles_for_tree_scope(document),
            vec![before, first, second, after].into()
        );
        assert!(host.child_handles(fragment).next().is_none());
    }

    #[test]
    fn adoption_reports_owner_changes_during_the_existing_tree_scope_walk() {
        let mut host = test_host();
        let new_document = host.create_detached_html_document();
        let wrapper = host.create_element("div");
        let style = host.create_element("style");
        let link = host.create_element("link");
        assert!(host.append_child(wrapper, style));
        assert!(host.append_child(wrapper, link));

        let (_, changes) = host
            .adopt_node_with_stylesheet_owner_changes(new_document, wrapper)
            .expect("detached wrapper can be adopted");
        assert_eq!(
            changes
                .iter()
                .map(|change| (change.owner(), change.kind()))
                .collect::<Vec<_>>(),
            vec![
                (style, &DomStylesheetOwnerChangeKind::OwnerDocumentChanged),
                (link, &DomStylesheetOwnerChangeKind::OwnerDocumentChanged),
            ]
        );
    }

    #[test]
    fn shadow_owner_connection_changes_are_directed_without_registry_rebuild() {
        let mut host = test_host();
        let document = host.document_handle();
        let host_element = host.create_element("section");
        let shadow_root = host
            .attach_shadow_root(host_element, "open")
            .expect("section accepts a shadow root");
        let style = host.create_element("style");

        let registered = host.append_child_effects(shadow_root, style);
        assert!(
            registered
                .stylesheet_owners()
                .changes()
                .iter()
                .any(|change| {
                    change.owner() == style
                        && matches!(change.kind(), DomStylesheetOwnerChangeKind::Registered)
                })
        );

        let connected = host.append_child_effects(document, host_element);
        assert!(
            connected
                .stylesheet_owners()
                .changes()
                .iter()
                .any(|change| {
                    change.owner() == style
                        && matches!(
                            change.kind(),
                            DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected: true }
                        )
                })
        );
        assert_eq!(
            host.stylesheet_candidate_handles_for_tree_scope(shadow_root),
            vec![style].into()
        );

        let disconnected = host.remove_child_effects(document, host_element);
        assert!(
            disconnected
                .stylesheet_owners()
                .changes()
                .iter()
                .any(|change| {
                    change.owner() == style
                        && matches!(
                            change.kind(),
                            DomStylesheetOwnerChangeKind::TreeConnectionChanged {
                                connected: false
                            }
                        )
                })
        );
        assert_eq!(
            host.stylesheet_candidate_handles_for_tree_scope(shadow_root),
            vec![style].into()
        );
    }

    #[test]
    fn batched_registry_transitions_preserve_the_final_owner_lifecycle() {
        let mut host = test_host();
        let document = host.document_handle();
        let style = host.create_element("style");
        let mut effects = host.append_child_effects(document, style);
        effects.merge(host.remove_child_effects(document, style));
        effects.merge(host.append_child_effects(document, style));
        effects.merge(host.remove_child_effects(document, style));

        assert_eq!(
            effects
                .stylesheet_owners()
                .changes()
                .iter()
                .filter(|change| change.owner() == style)
                .map(DomStylesheetOwnerChange::kind)
                .collect::<Vec<_>>(),
            vec![
                &DomStylesheetOwnerChangeKind::Registered,
                &DomStylesheetOwnerChangeKind::Unregistered,
                &DomStylesheetOwnerChangeKind::Registered,
                &DomStylesheetOwnerChangeKind::Unregistered,
            ]
        );
        assert_eq!(
            effects
                .stylesheet_owners()
                .changes()
                .iter()
                .filter(|change| change.owner() == style)
                .map(DomStylesheetOwnerChange::tree_scopes)
                .collect::<Vec<_>>(),
            vec![
                DomStylesheetOwnerTreeScopes::from_parts(None, None, Some(document)),
                DomStylesheetOwnerTreeScopes::from_parts(Some(document), None, None),
                DomStylesheetOwnerTreeScopes::from_parts(None, None, Some(document)),
                DomStylesheetOwnerTreeScopes::from_parts(Some(document), None, None),
            ]
        );
    }

    #[test]
    fn owner_mutations_report_their_current_tree_scope_without_subtree_rescans() {
        let mut host = test_host();
        let document = host.document_handle();
        let detached_document = host.create_detached_html_document();
        let style = host.create_element("style");
        let text = host.create_text_node("body { color: red; }");

        assert!(host.append_child(document, style));

        let attribute = host.set_attribute_effects(style, "media", "screen");
        assert_eq!(
            attribute
                .stylesheet_owners()
                .changes()
                .iter()
                .map(DomStylesheetOwnerChange::tree_scopes)
                .collect::<Vec<_>>(),
            vec![DomStylesheetOwnerTreeScopes::from_parts(
                None,
                Some(document),
                None,
            )]
        );

        let contents = host.append_child_effects(style, text);
        assert_eq!(
            contents
                .stylesheet_owners()
                .changes()
                .iter()
                .map(DomStylesheetOwnerChange::tree_scopes)
                .collect::<Vec<_>>(),
            vec![DomStylesheetOwnerTreeScopes::from_parts(
                None,
                Some(document),
                None,
            )]
        );

        let moved = host.append_child_effects(detached_document, style);
        assert_eq!(
            moved
                .stylesheet_owners()
                .changes()
                .iter()
                .filter(|change| change.owner() == style)
                .map(DomStylesheetOwnerChange::tree_scopes)
                .collect::<Vec<_>>(),
            vec![
                DomStylesheetOwnerTreeScopes::from_parts(Some(document), None, None),
                DomStylesheetOwnerTreeScopes::from_parts(None, None, Some(detached_document)),
            ]
        );

        let ordinary = host.create_element("div");
        let ordinary_effects = host.append_child_effects(detached_document, ordinary);
        assert!(ordinary_effects.stylesheet_owners().changes().is_empty());
    }
}
