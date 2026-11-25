use std::collections::HashSet;

use indexmap::IndexSet;
use moli_selector::StyloDomStyleAdapter;
use style::invalidation::element::restyle_hints::RestyleHint;

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    cache::PseudoStyleCache,
    invalidation::handle_is_in_style_subtrees,
    outcome::{
        FinalizedStyleInvalidationResult, StyleInvalidationCleanupApplicationContext,
        StyleInvalidationCleanupApplicationSink, StyleInvalidationCleanupApplicationSubtrees,
    },
    state::StyleDocumentState,
    style_paths::StyleInvalidationScope,
};

pub(super) struct StyleInvalidationCleanup<'a> {
    document: DomHandle,
    dom_adapter: &'a StyloDomStyleAdapter,
    pseudo_style_cache: &'a PseudoStyleCache,
    document_state: &'a StyleDocumentState,
}

impl<'a> StyleInvalidationCleanup<'a> {
    pub(super) fn new(
        document: DomHandle,
        dom_adapter: &'a StyloDomStyleAdapter,
        pseudo_style_cache: &'a PseudoStyleCache,
        document_state: &'a StyleDocumentState,
    ) -> Self {
        Self {
            document,
            dom_adapter,
            pseudo_style_cache,
            document_state,
        }
    }

    pub(super) fn clear_for_retained_style_system_rebuild(&self) {
        self.document_state
            .clear_invalidation_clear_all_fallback_reasons();
        self.clear_shadow_cascade_data_for_document_world();
        self.dom_adapter
            .clear_element_data_for_document(self.document);
        self.pseudo_style_cache.clear();
        self.document_state.clear_selector_caches();
        self.document_state.bump_computed_cache_generation();
        self.document_state.style_paths.clear();
    }

    /// The adapter already set the native hints. Publish their affected scopes
    /// to the demand-driven scheduler and keep the original cascade modes.
    pub(super) fn retain_stylesheet_invalidation_targets(
        &self,
        host: &DomHost,
        targets: Vec<(DomHandle, RestyleHint)>,
    ) {
        if targets.is_empty() {
            return;
        }
        self.document_state.record_style_targets(
            host,
            self.document,
            targets.iter().map(|&(handle, hint)| {
                let scope = if hint.contains(RestyleHint::RESTYLE_DESCENDANTS) {
                    StyleInvalidationScope::Subtree
                } else {
                    StyleInvalidationScope::Element
                };
                (handle, scope)
            }),
        );
        self.invalidate_target_caches(targets.into_iter().map(|(handle, _)| handle));
    }

    pub(super) fn invalidate_detached_subtrees(
        &self,
        host: &DomHost,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> bool {
        self.document_state
            .clear_invalidation_clear_all_fallback_reasons();
        let roots = self.existing_subtree_roots(host, roots);
        let selector_flag_handles = self.handles_in_style_subtrees(
            host,
            &roots,
            self.dom_adapter
                .element_selector_flag_handles_for_document(self.document),
        );
        let eagerly_invalidated_roots = self.record_style_paths(host, &roots);
        let cleared = eagerly_invalidated_roots.is_some();
        if eagerly_invalidated_roots.is_some() {
            for &handle in &selector_flag_handles {
                self.dom_adapter.clear_element_selector_flags(handle);
            }
            self.clear_shadow_cascade_data_in_subtrees(host, roots.iter().copied());
        }
        cleared
    }

    pub(super) fn invalidate_inline_style(
        &self,
        host: &DomHost,
        root: DomHandle,
        can_replace_rules: bool,
    ) -> bool {
        self.document_state
            .clear_invalidation_clear_all_fallback_reasons();
        let roots = self.existing_subtree_roots(host, [root]);
        if roots.is_empty() {
            return false;
        }
        if can_replace_rules {
            self.record_style_paths_targets(
                host,
                &roots,
                |_| StyleInvalidationScope::Element,
                RestyleHint::RESTYLE_STYLE_ATTRIBUTE,
            );
        } else {
            self.record_style_paths(host, &roots);
        }
        self.dom_adapter.clear_inline_style_attribute(root);
        true
    }

    pub(super) fn invalidate_subtrees(
        &self,
        host: &DomHost,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> bool {
        self.document_state
            .clear_invalidation_clear_all_fallback_reasons();
        let roots = self.existing_subtree_roots(host, roots);
        self.record_style_paths(host, &roots).is_some()
    }

    pub(super) fn invalidate_subtrees_and_shadow_cascade_data(
        &self,
        host: &DomHost,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> bool {
        self.document_state
            .clear_invalidation_clear_all_fallback_reasons();
        let roots = self.existing_subtree_roots(host, roots);
        let cleared = self.record_style_paths(host, &roots).is_some();
        if cleared {
            self.clear_shadow_cascade_data_in_subtrees(host, roots.iter().copied());
        }
        cleared
    }

    pub(super) fn apply_finalized_result(
        &self,
        host: &DomHost,
        finalized_result: FinalizedStyleInvalidationResult,
    ) -> bool {
        let cleanup_application = finalized_result.into_cleanup_application();
        cleanup_application.apply_to(host, self)
    }

    fn apply_subtree_cleanup_target(
        &self,
        host: &DomHost,
        subtrees: StyleInvalidationCleanupApplicationSubtrees,
        context: &StyleInvalidationCleanupApplicationContext,
    ) -> bool {
        let generations_before = self.document_state.generation_snapshot();
        self.document_state
            .clear_invalidation_clear_all_fallback_reasons();
        let StyleInvalidationCleanupApplicationSubtrees {
            exact_nodes: mut affected_roots,
            subtree_roots,
            shadow_cascade_roots,
        } = subtrees;
        affected_roots.extend(subtree_roots.iter().copied());
        let complete_selector_targets = context.has_complete_selector_targets();
        let eagerly_invalidated_roots = self
            .record_style_paths_targets(
                host,
                &affected_roots,
                |handle| {
                    if complete_selector_targets && !subtree_roots.contains(&handle) {
                        StyleInvalidationScope::Element
                    } else {
                        StyleInvalidationScope::Subtree
                    }
                },
                RestyleHint::RESTYLE_SELF,
            )
            .expect("subtree cleanup application should carry an existing root");
        let cleared_shadow_cascade_roots =
            if context.clears_shadow_cascade_data_for_cleanup_target() {
                self.clear_shadow_cascade_data_in_subtrees(host, shadow_cascade_roots)
            } else {
                IndexSet::new()
            };
        let generations_after = self.document_state.generation_snapshot();
        context.trace_scoped_fallback(
            self.document,
            &affected_roots,
            &eagerly_invalidated_roots,
            &cleared_shadow_cascade_roots,
            generations_before,
            generations_after,
        );
        true
    }

    fn clear_all_for_invalidation_fallback(
        &self,
        context: &StyleInvalidationCleanupApplicationContext,
    ) {
        self.document_state.invalidate_screen_style_comparisons();
        let generations_before = self.document_state.generation_snapshot();
        self.document_state
            .record_invalidation_clear_all_fallback_reasons(
                context
                    .clear_all_reasons()
                    .expect("clear-all fallback cleanup should carry reasons")
                    .iter()
                    .copied(),
            );
        self.dom_adapter
            .clear_element_data_for_document(self.document);
        if context.clears_shadow_cascade_data_for_cleanup_target() {
            self.clear_shadow_cascade_data_for_document_world();
        }
        self.pseudo_style_cache.clear();
        self.document_state.clear_selector_caches();
        self.document_state.style_paths.clear();
        self.document_state.bump_computed_context_epoch();
        let generations_after = self.document_state.generation_snapshot();
        context.trace_clear_all_fallback(self.document, generations_before, generations_after);
    }

    pub(in crate::style_engine) fn clear_shadow_cascade_data_for_document_world(&self) {
        self.dom_adapter
            .clear_shadow_cascade_data_for_document(self.document);
    }

    fn clear_shadow_cascade_data_in_subtrees(
        &self,
        host: &DomHost,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> IndexSet<DomHandle> {
        let root_set = roots.into_iter().collect::<HashSet<_>>();
        if root_set.is_empty() {
            return IndexSet::new();
        }
        let shadow_roots = self
            .dom_adapter
            .shadow_cascade_roots_for_document(self.document)
            .into_iter()
            .filter(|root| handle_is_in_style_subtrees(host, *root, &root_set))
            .collect::<IndexSet<_>>();
        if !shadow_roots.is_empty() {
            self.dom_adapter
                .clear_shadow_cascade_data_for_roots(shadow_roots.iter().copied());
        }
        shadow_roots
    }

    fn existing_subtree_roots(
        &self,
        host: &DomHost,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> IndexSet<DomHandle> {
        roots
            .into_iter()
            .filter(|root| host.node(*root).is_some())
            .collect()
    }

    fn handles_in_style_subtrees(
        &self,
        host: &DomHost,
        roots: &IndexSet<DomHandle>,
        candidates: impl IntoIterator<Item = DomHandle>,
    ) -> IndexSet<DomHandle> {
        if roots.is_empty() {
            return IndexSet::new();
        }
        let root_set = roots.iter().copied().collect::<HashSet<_>>();
        candidates
            .into_iter()
            .filter(|handle| handle_is_in_style_subtrees(host, *handle, &root_set))
            .collect()
    }

    fn record_style_paths(
        &self,
        host: &DomHost,
        roots: &IndexSet<DomHandle>,
    ) -> Option<IndexSet<DomHandle>> {
        self.record_style_paths_targets(
            host,
            roots,
            |_| StyleInvalidationScope::Subtree,
            RestyleHint::RESTYLE_SELF,
        )
    }

    fn record_style_paths_targets(
        &self,
        host: &DomHost,
        roots: &IndexSet<DomHandle>,
        scope: impl Fn(DomHandle) -> StyleInvalidationScope,
        hint: RestyleHint,
    ) -> Option<IndexSet<DomHandle>> {
        if roots.is_empty() {
            return None;
        }
        self.document_state.record_style_targets(
            host,
            self.document,
            roots.iter().copied().map(|root| (root, scope(root))),
        );

        // Dirty the roots themselves immediately. Descendants are handled
        // lazily from the retained root generation, avoiding O(P * depth)
        // expansion over every published style in the document. Keep doing
        // the exact-root retirement even when a deferred mutation is drained
        // after that root was adopted into another Document: the old world
        // may still own its pseudo sidecar, while the adapter moves its
        // ElementData to the current owner bucket on this targeted access.
        let immediately_invalidated = roots
            .iter()
            .copied()
            .filter(|handle| self.dom_adapter.mark_element_style_dirty(*handle, hint))
            .collect::<IndexSet<_>>();
        self.invalidate_target_caches(roots.iter().copied());
        Some(immediately_invalidated)
    }

    fn invalidate_target_caches(&self, handles: impl IntoIterator<Item = DomHandle>) {
        self.pseudo_style_cache.invalidate_handles(handles);
        self.document_state.clear_selector_caches();
        self.document_state.bump_computed_context_epoch();
    }
}

impl StyleInvalidationCleanupApplicationSink for StyleInvalidationCleanup<'_> {
    fn apply_noop_cleanup_application(&self) -> bool {
        self.document_state
            .clear_invalidation_clear_all_fallback_reasons();
        false
    }

    fn apply_clear_all_cleanup_application(
        &self,
        context: &StyleInvalidationCleanupApplicationContext,
    ) -> bool {
        self.clear_all_for_invalidation_fallback(context);
        true
    }

    fn apply_subtree_roots_cleanup_application(
        &self,
        host: &DomHost,
        subtrees: StyleInvalidationCleanupApplicationSubtrees,
        context: &StyleInvalidationCleanupApplicationContext,
    ) -> bool {
        self.apply_subtree_cleanup_target(host, subtrees, context)
    }
}
