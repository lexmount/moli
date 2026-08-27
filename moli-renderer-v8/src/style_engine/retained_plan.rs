use indexmap::IndexSet;
use moli_selector::{
    MoliStyleMutationSnapshot as MoliMutationSnapshot,
    StyloRetainedStyleChildListInvalidationQueries,
    StyloRetainedStyleChildListInvalidationQueriesSink,
    StyloRetainedStyleInvalidationQuery as RetainedStyleInvalidationQuery,
    StyloSourceDependencyBoundaryRoots, StyloSourceDependencyRequestRequirement,
    StyloStyleSourceScope as StyleSourceScope,
};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    StyleViewport,
    cause::PendingCauseFallback,
    request::{RetainedSourceDependencyRequestContext, RetainedSourceDependencyRequestPlan},
    source_document::DocumentStyleSourceStores,
    target_plan::{
        SourceDependencyTargetPlanResolution, pending_target_queries_for_source_dependency_requests,
    },
    target_queries::{PendingStyleInvalidationTargetQueries, StyleStructuralBoundaryCleanupRoots},
};

pub(super) struct RetainedBaseQueryPlan {
    request_plan: RetainedSourceDependencyRequestPlan,
    structural_boundary_cleanup_roots: StyleStructuralBoundaryCleanupRoots,
    empty_target_fallback_roots: IndexSet<DomHandle>,
    relative_previous_sibling_cleanup_roots: IndexSet<DomHandle>,
}

impl RetainedBaseQueryPlan {
    fn new(
        request_plan: RetainedSourceDependencyRequestPlan,
        structural_boundary_cleanup_roots: StyleStructuralBoundaryCleanupRoots,
    ) -> Self {
        Self::new_with_empty_target_fallback_roots(
            request_plan,
            structural_boundary_cleanup_roots,
            IndexSet::new(),
        )
    }

    fn new_with_empty_target_fallback_roots(
        request_plan: RetainedSourceDependencyRequestPlan,
        structural_boundary_cleanup_roots: StyleStructuralBoundaryCleanupRoots,
        empty_target_fallback_roots: IndexSet<DomHandle>,
    ) -> Self {
        Self::new_with_child_list_structural_roots(
            request_plan,
            structural_boundary_cleanup_roots,
            empty_target_fallback_roots,
            IndexSet::new(),
        )
    }

    fn new_with_child_list_structural_roots(
        request_plan: RetainedSourceDependencyRequestPlan,
        structural_boundary_cleanup_roots: StyleStructuralBoundaryCleanupRoots,
        empty_target_fallback_roots: IndexSet<DomHandle>,
        relative_previous_sibling_cleanup_roots: IndexSet<DomHandle>,
    ) -> Self {
        Self {
            request_plan,
            structural_boundary_cleanup_roots,
            empty_target_fallback_roots,
            relative_previous_sibling_cleanup_roots,
        }
    }

    pub(super) fn exact(queries: IndexSet<RetainedStyleInvalidationQuery>) -> Self {
        Self::new(
            RetainedSourceDependencyRequestPlan::exact(queries),
            StyleStructuralBoundaryCleanupRoots::default(),
        )
    }

    pub(super) fn exact_with_empty_target_fallback_roots(
        queries: IndexSet<RetainedStyleInvalidationQuery>,
        empty_target_fallback_roots: IndexSet<DomHandle>,
    ) -> Self {
        Self::new_with_empty_target_fallback_roots(
            RetainedSourceDependencyRequestPlan::exact(queries),
            StyleStructuralBoundaryCleanupRoots::default(),
            empty_target_fallback_roots,
        )
    }

    pub(super) fn child_list_structural_boundary_cleanup_roots(
        child_list: StyloRetainedStyleChildListInvalidationQueries,
    ) -> Self {
        let mut plan = Self::new_with_child_list_structural_roots(
            RetainedSourceDependencyRequestPlan::default(),
            StyleStructuralBoundaryCleanupRoots::default(),
            IndexSet::new(),
            IndexSet::new(),
        );
        child_list.drain_into(&mut plan);
        plan
    }

    pub(super) fn structural_boundary_cleanup_roots_for_all_queries(
        queries: IndexSet<RetainedStyleInvalidationQuery>,
        base_roots: IndexSet<DomHandle>,
    ) -> Self {
        Self::new(
            RetainedSourceDependencyRequestPlan::all_queries_require_child_list_structural_dependency(
                queries,
            ),
            StyleStructuralBoundaryCleanupRoots::from_roots(base_roots),
        )
    }

    pub(super) fn merge_from(&mut self, incoming: Self) {
        self.request_plan.extend(incoming.request_plan);
        self.structural_boundary_cleanup_roots
            .extend(incoming.structural_boundary_cleanup_roots.into_roots());
        self.empty_target_fallback_roots
            .extend(incoming.empty_target_fallback_roots);
        self.relative_previous_sibling_cleanup_roots
            .extend(incoming.relative_previous_sibling_cleanup_roots);
    }

    pub(super) fn target_queries(
        &self,
        host: &DomHost,
        source_stores: &DocumentStyleSourceStores<'_>,
        emulated_media: &crate::protocol_types::EmulatedMediaOverrides,
        viewport: StyleViewport,
        document: DomHandle,
        cause_fallback: &PendingCauseFallback,
        request_context: &RetainedSourceDependencyRequestContext<'_>,
        source_scope: &StyleSourceScope,
        mutation_snapshot: &MoliMutationSnapshot,
    ) -> Vec<PendingStyleInvalidationTargetQueries> {
        if self.request_plan.is_empty() {
            return self.fallback_target_queries_for_empty_queries(host, cause_fallback);
        }
        if self.can_skip_for_missing_dependency_match(cause_fallback)
            && !source_stores.has_dependency_match_for_request_plan(host, &self.request_plan)
        {
            return Vec::new();
        }

        let requests = self
            .request_plan
            .source_dependency_requests(request_context);
        debug_assert_eq!(source_stores.document(), document);
        let matching_sources =
            source_stores.matching_dependency_sources(host, source_scope, emulated_media, viewport);
        if matching_sources.is_empty() && source_stores.has_document_retained_sources(host) {
            return cause_fallback.target_queries_for_source_scope(host, source_scope);
        }
        let mut empty_target_fallback_roots = self
            .structural_boundary_cleanup_roots
            .iter()
            .copied()
            .collect::<IndexSet<_>>();
        empty_target_fallback_roots.extend(self.empty_target_fallback_roots.iter().copied());
        let empty_target_fallback_roots =
            empty_target_fallback_roots.into_iter().collect::<Vec<_>>();
        let relative_previous_sibling_cleanup_roots = self
            .relative_previous_sibling_cleanup_roots
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let target_plan = pending_target_queries_for_source_dependency_requests(
            host,
            &matching_sources,
            &requests,
            cause_fallback,
            StyloSourceDependencyBoundaryRoots::new(
                &empty_target_fallback_roots,
                &relative_previous_sibling_cleanup_roots,
            ),
        );
        let resolution = target_plan.into_target_queries(host, cause_fallback);
        let mut target_queries = match resolution {
            SourceDependencyTargetPlanResolution::Finalized(target_queries) => {
                return target_queries;
            }
            SourceDependencyTargetPlanResolution::PendingBaseRoots(target_queries) => {
                target_queries
            }
        };
        if target_queries.is_empty() && !cause_fallback.is_empty() {
            return cause_fallback.target_queries_for_source_scope(host, source_scope);
        }
        self.apply_mutation_snapshot_and_base_roots(
            host,
            mutation_snapshot,
            cause_fallback,
            &mut target_queries,
        );
        let zero_matched_dependency_result_is_exact =
            request_context.zero_matched_dependency_result_is_exact();
        for target_query in &mut target_queries {
            target_query.set_zero_matched_dependency_result_is_exact(
                zero_matched_dependency_result_is_exact,
            );
        }
        target_queries
    }

    fn apply_mutation_snapshot_and_base_roots(
        &self,
        host: &DomHost,
        mutation_snapshot: &MoliMutationSnapshot,
        cause_fallback: &PendingCauseFallback,
        target_queries: &mut Vec<PendingStyleInvalidationTargetQueries>,
    ) {
        if !target_queries.is_empty() {
            cause_fallback
                .merge_cause_roots_into_source_dependency_target_queries(host, target_queries);
        }
        for target_query in target_queries.iter_mut() {
            target_query.set_mutation_snapshot(mutation_snapshot.clone());
        }
    }

    fn fallback_target_queries_for_empty_queries(
        &self,
        host: &DomHost,
        cause_fallback: &PendingCauseFallback,
    ) -> Vec<PendingStyleInvalidationTargetQueries> {
        cause_fallback.target_queries_for_structural_boundary_roots(
            host,
            self.structural_boundary_cleanup_roots.iter().copied(),
        )
    }

    fn can_skip_for_missing_dependency_match(&self, cause_fallback: &PendingCauseFallback) -> bool {
        cause_fallback.is_empty()
            && self.structural_boundary_cleanup_roots.len() == 0
            && self.empty_target_fallback_roots.is_empty()
            && self.relative_previous_sibling_cleanup_roots.is_empty()
    }
}

impl StyloRetainedStyleChildListInvalidationQueriesSink<DomHandle> for RetainedBaseQueryPlan {
    fn record_child_list_retained_query(
        &mut self,
        query: RetainedStyleInvalidationQuery,
        requirement: StyloSourceDependencyRequestRequirement,
    ) {
        self.request_plan
            .record_query_requirement(query, requirement);
    }

    fn extend_child_list_base_roots(&mut self, roots: Vec<DomHandle>) {
        self.structural_boundary_cleanup_roots.extend(roots);
    }

    fn extend_child_list_empty_target_fallback_roots(&mut self, roots: Vec<DomHandle>) {
        self.empty_target_fallback_roots.extend(roots);
    }

    fn extend_child_list_relative_previous_sibling_cleanup_roots(&mut self, roots: Vec<DomHandle>) {
        self.relative_previous_sibling_cleanup_roots.extend(roots);
    }
}
