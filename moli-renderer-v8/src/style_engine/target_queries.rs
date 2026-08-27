use std::collections::HashMap;

use crate::{document_runtime::DomHandle, dom::native::DomHost};
use indexmap::IndexSet;
use moli_selector::{
    MoliStyleMutationSnapshot as MoliMutationSnapshot, StyloPlannedFallbackRootInvalidationTarget,
    StyloPlannedFallbackRootInvalidationTargetPartsSink, StyloPlannedSourceDependencyInvalidation,
    StyloPlannedSourceDependencyInvalidationPartsSink,
    StyloPlannedSourceDependencyInvalidationTargetPartsSink, StyloRetainedSourceStyleInvalidation,
    StyloRetainedSourceStyleInvalidationKind,
    StyloRetainedStyleInvalidationQuery as RetainedStyleInvalidationQuery,
    StyloSourceInvalidationFallbackReason, stylo_merge_retained_source_invalidation_fallback_kind,
    stylo_merge_retained_source_invalidation_kind,
    stylo_retained_source_invalidation_kind_can_use_fallback_payload,
    stylo_retained_source_style_invalidation_from_parts,
};

use super::{
    StyleInvalidationSourceTarget, StyleScopeId, StyleSourceId,
    source_lifecycle::{StyleSourceLifecycleReport, StyleSourceLifecycleTargetAvailabilitySink},
    state::RetainedStyleSystem,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingStyleInvalidationTargetQueries {
    context: PendingStyleInvalidationTargetContext,
    payload: PendingStyleInvalidationTargetPayload,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingRetainedSourceStyleInvalidationInput<'a> {
    target: &'a StyleInvalidationSourceTarget,
    kind: StyloRetainedSourceStyleInvalidationKind,
    fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
    retained_queries: Option<&'a IndexSet<RetainedStyleInvalidationQuery>>,
    reasoned_fallback_roots: &'a IndexSet<DomHandle>,
    exact_safety_fallback_roots: &'a IndexSet<DomHandle>,
    fallback_reasons: &'a IndexSet<StyloSourceInvalidationFallbackReason>,
    mutation_snapshot: &'a MoliMutationSnapshot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PendingStyleInvalidationTargetTraceCounts {
    retained_query_count: usize,
    mutation_snapshot_count: usize,
    structural_boundary_cleanup_root_count: usize,
    fallback_root_bucket_count: usize,
}

pub(super) trait PendingStyleInvalidationTargetTraceCountsSink {
    fn record_pending_target_trace_counts(
        &mut self,
        retained_query_count: usize,
        mutation_snapshot_count: usize,
        structural_boundary_cleanup_root_count: usize,
        fallback_root_bucket_count: usize,
    );

    fn record_pending_target_source_scope_fallback_roots(&mut self, roots: Vec<DomHandle>);
}

pub(super) trait PendingStyleInvalidationTargetSourceIdSink {
    fn record_pending_target_stylesheet_source_id(&mut self, source_id: &StyleSourceId);
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingStyleInvalidationTargetResultInput<'a> {
    target: &'a StyleInvalidationSourceTarget,
    structural_boundary_cleanup_roots: &'a StyleStructuralBoundaryCleanupRoots,
    trace_counts: PendingStyleInvalidationTargetTraceCounts,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StyleStructuralBoundaryCleanupRoots {
    roots: IndexSet<DomHandle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingStyleInvalidationTargetPayload {
    retained_source_kind: StyloRetainedSourceStyleInvalidationKind,
    fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
    retained_queries: Option<IndexSet<RetainedStyleInvalidationQuery>>,
    fallback_root_buckets: PendingStyleInvalidationFallbackRootBuckets,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PendingStyleInvalidationFallbackRootBuckets {
    reasoned_fallback_roots: IndexSet<DomHandle>,
    exact_safety_fallback_roots: IndexSet<DomHandle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingStyleInvalidationTargetContext {
    target: StyleInvalidationSourceTarget,
    mutation_snapshot: MoliMutationSnapshot,
    structural_boundary_cleanup_roots: StyleStructuralBoundaryCleanupRoots,
    fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
}

struct PendingPlannedFallbackRootTargetQueryBuilder<'a> {
    host: &'a DomHost,
    target_queries: Vec<PendingStyleInvalidationTargetQueries>,
}

struct PendingPlannedSourceDependencyQueryBuilder<'a> {
    source_targets: &'a [StyleInvalidationSourceTarget],
    source_target: Option<StyleInvalidationSourceTarget>,
    structural_boundary_cleanup_roots: Option<Vec<DomHandle>>,
    target_query: Option<PendingStyleInvalidationTargetQueries>,
}

impl PendingStyleInvalidationTargetQueries {
    #[cfg(test)]
    pub(super) fn retained_source(
        source_id: StyleSourceId,
        queries: IndexSet<RetainedStyleInvalidationQuery>,
    ) -> Self {
        Self::retained_dependency_source(
            StyleInvalidationSourceTarget::stylesheet(source_id),
            queries,
        )
    }

    fn retained_dependency_source(
        target: StyleInvalidationSourceTarget,
        queries: IndexSet<RetainedStyleInvalidationQuery>,
    ) -> Self {
        debug_assert!(
            !queries.is_empty(),
            "retained-source target queries must not be empty"
        );
        Self::new(
            target,
            PendingStyleInvalidationTargetPayload::with_retained_queries(queries),
        )
    }

    pub(super) fn planned_source_dependency(
        source_targets: &[StyleInvalidationSourceTarget],
        planned_source: StyloPlannedSourceDependencyInvalidation,
    ) -> Self {
        PendingPlannedSourceDependencyQueryBuilder::build(source_targets, planned_source)
    }

    pub(super) fn planned_fallback_root_target(
        host: &DomHost,
        fallback_target: StyloPlannedFallbackRootInvalidationTarget,
    ) -> Vec<Self> {
        PendingPlannedFallbackRootTargetQueryBuilder::build(host, fallback_target)
    }

    fn new(
        target: StyleInvalidationSourceTarget,
        payload: PendingStyleInvalidationTargetPayload,
    ) -> Self {
        Self {
            context: PendingStyleInvalidationTargetContext::new(target),
            payload,
        }
    }

    #[cfg(test)]
    pub(super) fn source_fallback_for_test(
        source_id: StyleSourceId,
        roots: impl IntoIterator<Item = DomHandle>,
        reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
    ) -> Self {
        Self::source_fallback_with_kind(
            source_id,
            StyloRetainedSourceStyleInvalidationKind::FallbackOnly,
            roots,
        )
        .with_fallback_reasons(reasons)
    }

    #[cfg(test)]
    fn source_fallback_with_kind(
        source_id: StyleSourceId,
        fallback_kind: StyloRetainedSourceStyleInvalidationKind,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> Self {
        Self::dependency_source_fallback_with_kind(
            StyleInvalidationSourceTarget::stylesheet(source_id),
            fallback_kind,
            roots,
        )
    }

    fn dependency_source_fallback_with_kind(
        target: StyleInvalidationSourceTarget,
        fallback_kind: StyloRetainedSourceStyleInvalidationKind,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> Self {
        let target_query = Self::fallback_target_with_kind(target, fallback_kind, roots);
        debug_assert!(
            target_query.fallback_root_bucket_count() > 0,
            "source fallback roots must be explicit; use source_fallback_missing_roots for missing roots"
        );
        target_query
    }

    #[cfg(test)]
    fn source_fallback_missing_roots(source_id: StyleSourceId) -> Self {
        Self::dependency_source_fallback_missing_roots(StyleInvalidationSourceTarget::stylesheet(
            source_id,
        ))
    }

    fn dependency_source_fallback_missing_roots(target: StyleInvalidationSourceTarget) -> Self {
        Self::new(
            target,
            PendingStyleInvalidationTargetPayload::missing_fallback_roots(),
        )
    }

    #[cfg(test)]
    pub(super) fn source_fallback_missing_roots_for_test(
        source_id: StyleSourceId,
        reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
    ) -> Self {
        let mut target_query = Self::source_fallback_missing_roots(source_id);
        target_query
            .add_fallback_reason(StyloSourceInvalidationFallbackReason::MissingFallbackRoots);
        for reason in reasons {
            target_query.add_fallback_reason(reason);
        }
        target_query
    }

    #[cfg(test)]
    fn fallback_target(
        target: StyleInvalidationSourceTarget,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> Self {
        Self::fallback_target_with_kind(
            target,
            StyloRetainedSourceStyleInvalidationKind::FallbackOnly,
            roots,
        )
    }

    fn fallback_target_with_kind(
        target: StyleInvalidationSourceTarget,
        fallback_kind: StyloRetainedSourceStyleInvalidationKind,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> Self {
        debug_assert!(
            stylo_retained_source_invalidation_kind_can_use_fallback_payload(fallback_kind),
            "fallback target should not carry retained-query kind"
        );
        Self::new(
            target,
            PendingStyleInvalidationTargetPayload::with_fallback_kind(fallback_kind, roots),
        )
    }

    #[cfg(test)]
    pub(super) fn fallback_roots_for_test(
        target: StyleInvalidationSourceTarget,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> Self {
        debug_assert!(
            target.is_fallback_root(),
            "fallback-root target queries should use fallback-root targets"
        );
        Self::fallback_target(target, roots)
    }

    fn fallback_roots_with_kind(
        target: StyleInvalidationSourceTarget,
        fallback_kind: StyloRetainedSourceStyleInvalidationKind,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> Self {
        debug_assert!(
            target.is_fallback_root(),
            "fallback-root target queries should use fallback-root targets"
        );
        Self::fallback_target_with_kind(target, fallback_kind, roots)
    }

    fn add_fallback_reason(&mut self, reason: StyloSourceInvalidationFallbackReason) {
        self.context.add_fallback_reason(reason);
    }

    fn with_fallback_reasons(
        mut self,
        reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
    ) -> Self {
        for reason in reasons {
            self.add_fallback_reason(reason);
        }
        self
    }

    fn extend_reasoned_fallback_roots(&mut self, roots: impl IntoIterator<Item = DomHandle>) {
        self.payload.reasoned_fallback_roots_mut().extend(roots);
    }

    fn extend_exact_safety_fallback_roots(&mut self, roots: impl IntoIterator<Item = DomHandle>) {
        self.payload.exact_safety_fallback_roots_mut().extend(roots);
    }

    fn set_fallback_kind(
        &mut self,
        fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
    ) {
        self.payload.set_fallback_kind(fallback_kind);
    }

    #[cfg(test)]
    pub(super) fn add_fallback_reason_for_test(
        &mut self,
        reason: StyloSourceInvalidationFallbackReason,
    ) {
        self.add_fallback_reason(reason);
    }

    #[cfg(test)]
    pub(super) fn extend_reasoned_fallback_roots_for_test(
        &mut self,
        roots: impl IntoIterator<Item = DomHandle>,
    ) {
        self.extend_reasoned_fallback_roots(roots);
    }

    pub(super) fn set_mutation_snapshot(&mut self, snapshot: MoliMutationSnapshot) {
        self.context.set_mutation_snapshot(snapshot);
    }

    pub(super) fn extend_structural_boundary_cleanup_roots(
        &mut self,
        roots: impl IntoIterator<Item = DomHandle>,
    ) {
        self.context.extend_structural_boundary_cleanup_roots(roots);
    }

    #[cfg(not(test))]
    fn target(&self) -> &StyleInvalidationSourceTarget {
        &self.context.target
    }

    #[cfg(test)]
    pub(super) fn target(&self) -> &StyleInvalidationSourceTarget {
        &self.context.target
    }

    pub(super) fn record_stylesheet_source_id_into(
        &self,
        sink: &mut impl PendingStyleInvalidationTargetSourceIdSink,
    ) {
        if let Some(source_id) = self.context.target.stylesheet_source_id() {
            sink.record_pending_target_stylesheet_source_id(source_id);
        }
    }

    fn retained_source_invalidation_kind(&self) -> StyloRetainedSourceStyleInvalidationKind {
        self.payload.retained_source_kind()
    }

    pub(super) fn retained_source_invalidation_input(
        &self,
    ) -> PendingRetainedSourceStyleInvalidationInput<'_> {
        PendingRetainedSourceStyleInvalidationInput {
            target: self.target(),
            kind: self.retained_source_invalidation_kind(),
            fallback_kind: self.payload.fallback_kind(),
            retained_queries: self.retained_queries(),
            reasoned_fallback_roots: self.reasoned_fallback_root_set(),
            exact_safety_fallback_roots: self.exact_safety_fallback_root_set(),
            fallback_reasons: self.fallback_reasons(),
            mutation_snapshot: self.mutation_snapshot(),
        }
    }

    pub(super) fn target_result_input(&self) -> PendingStyleInvalidationTargetResultInput<'_> {
        PendingStyleInvalidationTargetResultInput {
            target: self.target(),
            structural_boundary_cleanup_roots: self.structural_boundary_cleanup_roots(),
            trace_counts: self.trace_counts(),
        }
    }

    fn trace_counts(&self) -> PendingStyleInvalidationTargetTraceCounts {
        PendingStyleInvalidationTargetTraceCounts {
            retained_query_count: self.retained_query_count(),
            mutation_snapshot_count: self.mutation_snapshot_count(),
            structural_boundary_cleanup_root_count: self.structural_boundary_cleanup_root_count(),
            fallback_root_bucket_count: self.fallback_root_bucket_count(),
        }
    }

    pub(super) fn record_trace_counts_into(
        &self,
        host: &DomHost,
        sink: &mut impl PendingStyleInvalidationTargetTraceCountsSink,
    ) {
        self.trace_counts().record_into(sink);
        sink.record_pending_target_source_scope_fallback_roots(
            self.context.target.source_scope_fallback_roots(host),
        );
    }

    #[cfg(test)]
    pub(super) fn kind(&self) -> StyloRetainedSourceStyleInvalidationKind {
        self.payload.retained_source_kind()
    }

    fn retained_queries(&self) -> Option<&IndexSet<RetainedStyleInvalidationQuery>> {
        self.payload.retained_queries()
    }

    #[cfg(not(test))]
    fn retained_query_count(&self) -> usize {
        self.retained_queries().map(IndexSet::len).unwrap_or(0)
    }

    #[cfg(test)]
    pub(super) fn retained_query_count(&self) -> usize {
        self.retained_queries().map(IndexSet::len).unwrap_or(0)
    }

    fn reasoned_fallback_root_set(&self) -> &IndexSet<DomHandle> {
        self.payload.reasoned_fallback_roots()
    }

    #[cfg(not(test))]
    fn fallback_root_bucket_count(&self) -> usize {
        self.payload.fallback_root_bucket_count()
    }

    #[cfg(test)]
    pub(super) fn fallback_root_bucket_count(&self) -> usize {
        self.payload.fallback_root_bucket_count()
    }

    fn exact_safety_fallback_root_set(&self) -> &IndexSet<DomHandle> {
        self.payload.exact_safety_fallback_roots()
    }

    fn mutation_snapshot(&self) -> &MoliMutationSnapshot {
        &self.context.mutation_snapshot
    }

    #[cfg(not(test))]
    fn mutation_snapshot_count(&self) -> usize {
        self.mutation_snapshot().len()
    }

    #[cfg(test)]
    pub(super) fn mutation_snapshot_count(&self) -> usize {
        self.mutation_snapshot().len()
    }

    fn structural_boundary_cleanup_roots(&self) -> &StyleStructuralBoundaryCleanupRoots {
        &self.context.structural_boundary_cleanup_roots
    }

    #[cfg(not(test))]
    fn structural_boundary_cleanup_root_count(&self) -> usize {
        self.structural_boundary_cleanup_roots().len()
    }

    #[cfg(test)]
    pub(super) fn structural_boundary_cleanup_root_count(&self) -> usize {
        self.structural_boundary_cleanup_roots().len()
    }

    #[cfg(test)]
    pub(super) fn structural_boundary_cleanup_roots_for_test(&self) -> &IndexSet<DomHandle> {
        &self.structural_boundary_cleanup_roots().roots
    }

    fn fallback_reasons(&self) -> &IndexSet<StyloSourceInvalidationFallbackReason> {
        &self.context.fallback_reasons
    }

    #[cfg(test)]
    pub(super) fn retained_queries_for_test(
        &self,
    ) -> Option<&IndexSet<RetainedStyleInvalidationQuery>> {
        self.retained_queries()
    }

    #[cfg(test)]
    pub(super) fn reasoned_fallback_root_set_for_test(&self) -> &IndexSet<DomHandle> {
        self.reasoned_fallback_root_set()
    }

    #[cfg(test)]
    pub(super) fn exact_safety_fallback_root_set_for_test(&self) -> &IndexSet<DomHandle> {
        self.exact_safety_fallback_root_set()
    }

    #[cfg(test)]
    pub(super) fn fallback_reasons_for_test(
        &self,
    ) -> &IndexSet<StyloSourceInvalidationFallbackReason> {
        self.fallback_reasons()
    }

    fn merge_from(&mut self, incoming: Self) {
        self.payload.merge_from(incoming.payload);
        self.context.merge_from(incoming.context);
    }
}

impl<'a> PendingPlannedFallbackRootTargetQueryBuilder<'a> {
    fn build(
        host: &'a DomHost,
        fallback_target: StyloPlannedFallbackRootInvalidationTarget,
    ) -> Vec<PendingStyleInvalidationTargetQueries> {
        let mut builder = Self {
            host,
            target_queries: Vec::new(),
        };
        fallback_target.drain_into(&mut builder);
        builder.target_queries
    }

    fn target_query_for_root(
        &self,
        root: DomHandle,
        fallback_kind: StyloRetainedSourceStyleInvalidationKind,
        fallback_reasons: &IndexSet<StyloSourceInvalidationFallbackReason>,
    ) -> Option<PendingStyleInvalidationTargetQueries> {
        self.host.node(root)?;
        let target = StyleInvalidationSourceTarget::fallback_root(self.host, root)?;
        Some(
            PendingStyleInvalidationTargetQueries::fallback_roots_with_kind(
                target,
                fallback_kind,
                [root],
            )
            .with_fallback_reasons(fallback_reasons.iter().copied()),
        )
    }
}

impl StyloPlannedFallbackRootInvalidationTargetPartsSink<DomHandle>
    for PendingPlannedFallbackRootTargetQueryBuilder<'_>
{
    fn set_planned_fallback_root_target_parts(
        &mut self,
        fallback_kind: StyloRetainedSourceStyleInvalidationKind,
        fallback_roots: Vec<DomHandle>,
        fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        let incoming = fallback_roots
            .into_iter()
            .filter_map(|root| self.target_query_for_root(root, fallback_kind, &fallback_reasons))
            .collect::<Vec<_>>();
        merge_pending_target_queries(&mut self.target_queries, incoming);
    }
}

impl<'a> PendingPlannedSourceDependencyQueryBuilder<'a> {
    fn build(
        source_targets: &'a [StyleInvalidationSourceTarget],
        planned_source: StyloPlannedSourceDependencyInvalidation,
    ) -> PendingStyleInvalidationTargetQueries {
        let mut builder = Self {
            source_targets,
            source_target: None,
            structural_boundary_cleanup_roots: None,
            target_query: None,
        };
        planned_source.drain_into(&mut builder);
        builder
            .target_query
            .expect("planned source dependency should produce target query")
    }

    fn source_target(&self) -> StyleInvalidationSourceTarget {
        self.source_target
            .as_ref()
            .expect("planned source dependency should set source target before target parts")
            .clone()
    }

    fn retained_queries(
        &self,
        exact_queries: Vec<RetainedStyleInvalidationQuery>,
        fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
        reasoned_fallback_roots: Vec<DomHandle>,
        exact_safety_fallback_roots: Vec<DomHandle>,
        fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    ) -> PendingStyleInvalidationTargetQueries {
        let mut target_query = PendingStyleInvalidationTargetQueries::retained_dependency_source(
            self.source_target(),
            exact_queries.into_iter().collect(),
        );
        target_query.set_fallback_kind(fallback_kind);
        target_query.extend_reasoned_fallback_roots(reasoned_fallback_roots);
        target_query.extend_exact_safety_fallback_roots(exact_safety_fallback_roots);
        target_query.with_fallback_reasons(fallback_reasons)
    }

    fn fallback_with_roots(
        &self,
        fallback_kind: StyloRetainedSourceStyleInvalidationKind,
        fallback_roots: Vec<DomHandle>,
        fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    ) -> PendingStyleInvalidationTargetQueries {
        let target_query =
            PendingStyleInvalidationTargetQueries::dependency_source_fallback_with_kind(
                self.source_target(),
                fallback_kind,
                fallback_roots,
            );
        target_query.with_fallback_reasons(fallback_reasons)
    }

    fn missing_fallback_roots(
        &self,
        fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    ) -> PendingStyleInvalidationTargetQueries {
        let target_query =
            PendingStyleInvalidationTargetQueries::dependency_source_fallback_missing_roots(
                self.source_target(),
            );
        target_query.with_fallback_reasons(fallback_reasons)
    }

    fn structural_boundary_cleanup_roots(&mut self) -> Vec<DomHandle> {
        self.structural_boundary_cleanup_roots
            .take()
            .unwrap_or_default()
    }

    fn set_target_query(&mut self, mut target_query: PendingStyleInvalidationTargetQueries) {
        target_query
            .extend_structural_boundary_cleanup_roots(self.structural_boundary_cleanup_roots());
        self.target_query = Some(target_query);
    }
}

impl StyloPlannedSourceDependencyInvalidationPartsSink<DomHandle>
    for PendingPlannedSourceDependencyQueryBuilder<'_>
{
    fn set_planned_source_dependency_source_index(&mut self, source_index: usize) {
        self.source_target = Some(
            self.source_targets
                .get(source_index)
                .expect("Stylo planned source index should reference a matching source")
                .clone(),
        );
    }

    fn set_planned_source_dependency_structural_boundary_cleanup_roots(
        &mut self,
        structural_boundary_cleanup_roots: Vec<DomHandle>,
    ) {
        self.structural_boundary_cleanup_roots = Some(structural_boundary_cleanup_roots);
    }
}

impl StyloPlannedSourceDependencyInvalidationTargetPartsSink<DomHandle>
    for PendingPlannedSourceDependencyQueryBuilder<'_>
{
    fn set_planned_retained_source_dependency_target_parts(
        &mut self,
        exact_queries: Vec<RetainedStyleInvalidationQuery>,
        fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
        reasoned_fallback_roots: Vec<DomHandle>,
        exact_safety_fallback_roots: Vec<DomHandle>,
        fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        self.set_target_query(self.retained_queries(
            exact_queries,
            fallback_kind,
            reasoned_fallback_roots,
            exact_safety_fallback_roots,
            fallback_reasons,
        ));
    }

    fn set_planned_fallback_source_dependency_target_parts(
        &mut self,
        fallback_kind: StyloRetainedSourceStyleInvalidationKind,
        fallback_roots: Vec<DomHandle>,
        fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        self.set_target_query(self.fallback_with_roots(
            fallback_kind,
            fallback_roots,
            fallback_reasons,
        ));
    }

    fn set_planned_missing_fallback_roots_source_dependency_target_parts(
        &mut self,
        fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        self.set_target_query(self.missing_fallback_roots(fallback_reasons));
    }
}

impl<'a> PendingRetainedSourceStyleInvalidationInput<'a> {
    pub(super) fn into_stylo_input(
        self,
        retained: Option<&'a RetainedStyleSystem>,
    ) -> StyloRetainedSourceStyleInvalidation<'a> {
        let cascade_data = retained.and_then(|retained| match self.target {
            StyleInvalidationSourceTarget::Stylesheet(source_id) => {
                retained.source_cascade_data.get(source_id)
            }
            StyleInvalidationSourceTarget::UserAgent { .. } => {
                Some(&retained.user_agent_cascade_data)
            }
            StyleInvalidationSourceTarget::FallbackRoot { .. } => None,
        });
        stylo_retained_source_style_invalidation_from_parts(
            self.kind,
            self.fallback_kind,
            cascade_data,
            self.shadow_root(),
            self.retained_queries,
            self.reasoned_fallback_roots,
            self.exact_safety_fallback_roots,
            self.fallback_reasons,
            self.mutation_snapshot,
        )
    }

    fn shadow_root(&self) -> Option<DomHandle> {
        match self.target.scope_id() {
            StyleScopeId::Document(_) => None,
            StyleScopeId::ShadowRoot(root) => Some(root),
        }
    }
}

impl<'a> PendingStyleInvalidationTargetResultInput<'a> {
    pub(super) fn clone_target_for_diagnostics(&self) -> StyleInvalidationSourceTarget {
        self.target.clone()
    }

    pub(super) fn record_lifecycle_target_availability_into(
        &self,
        lifecycle: &StyleSourceLifecycleReport,
        sink: &mut impl StyleSourceLifecycleTargetAvailabilitySink,
    ) {
        lifecycle.record_target_availability_for_source_target_into(self.target, sink);
    }

    pub(super) fn extend_structural_boundary_cleanup_roots_into(
        &self,
        target: &mut StyleStructuralBoundaryCleanupRoots,
    ) {
        target.extend(self.structural_boundary_cleanup_roots.iter().copied());
    }

    pub(super) fn source_scope_fallback_roots(&self, host: &DomHost) -> Vec<DomHandle> {
        self.target.source_scope_fallback_roots(host)
    }

    pub(super) fn record_trace_counts_into(
        &self,
        sink: &mut impl PendingStyleInvalidationTargetTraceCountsSink,
    ) {
        self.trace_counts.record_into(sink);
    }
}

impl PendingStyleInvalidationTargetTraceCounts {
    fn record_into(self, sink: &mut impl PendingStyleInvalidationTargetTraceCountsSink) {
        sink.record_pending_target_trace_counts(
            self.retained_query_count,
            self.mutation_snapshot_count,
            self.structural_boundary_cleanup_root_count,
            self.fallback_root_bucket_count,
        );
    }
}

impl PendingStyleInvalidationTargetPayload {
    fn with_retained_queries(queries: IndexSet<RetainedStyleInvalidationQuery>) -> Self {
        Self {
            retained_source_kind: StyloRetainedSourceStyleInvalidationKind::RetainedQueries,
            fallback_kind: None,
            retained_queries: Some(queries),
            fallback_root_buckets: PendingStyleInvalidationFallbackRootBuckets::default(),
        }
    }

    fn with_fallback_kind(
        retained_source_kind: StyloRetainedSourceStyleInvalidationKind,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> Self {
        debug_assert!(
            stylo_retained_source_invalidation_kind_can_use_fallback_payload(retained_source_kind),
            "fallback payload should not carry retained-query kind"
        );
        Self {
            retained_source_kind,
            fallback_kind: Some(retained_source_kind),
            retained_queries: None,
            fallback_root_buckets: PendingStyleInvalidationFallbackRootBuckets::with_reasoned(
                roots,
            ),
        }
    }

    fn missing_fallback_roots() -> Self {
        Self {
            retained_source_kind: StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots,
            fallback_kind: Some(StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots),
            retained_queries: None,
            fallback_root_buckets: PendingStyleInvalidationFallbackRootBuckets::default(),
        }
    }

    fn retained_source_kind(&self) -> StyloRetainedSourceStyleInvalidationKind {
        self.retained_source_kind
    }

    fn fallback_kind(&self) -> Option<StyloRetainedSourceStyleInvalidationKind> {
        self.fallback_kind
    }

    fn retained_queries(&self) -> Option<&IndexSet<RetainedStyleInvalidationQuery>> {
        self.retained_queries.as_ref()
    }

    fn reasoned_fallback_roots(&self) -> &IndexSet<DomHandle> {
        self.fallback_root_buckets.reasoned_roots()
    }

    fn reasoned_fallback_roots_mut(&mut self) -> &mut IndexSet<DomHandle> {
        self.fallback_root_buckets.reasoned_roots_mut()
    }

    fn exact_safety_fallback_roots(&self) -> &IndexSet<DomHandle> {
        self.fallback_root_buckets.exact_safety_roots()
    }

    fn exact_safety_fallback_roots_mut(&mut self) -> &mut IndexSet<DomHandle> {
        self.fallback_root_buckets.exact_safety_roots_mut()
    }

    fn fallback_root_bucket_count(&self) -> usize {
        self.fallback_root_buckets.total_count()
    }

    fn set_fallback_kind(
        &mut self,
        fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
    ) {
        self.fallback_kind = stylo_merge_retained_source_invalidation_fallback_kind(
            self.fallback_kind,
            fallback_kind,
        );
    }

    fn merge_from(&mut self, incoming: Self) {
        if let Some(incoming_queries) = incoming.retained_queries {
            self.retained_queries
                .get_or_insert_with(IndexSet::new)
                .extend(incoming_queries);
        }
        self.fallback_root_buckets
            .merge_from(incoming.fallback_root_buckets);
        self.set_fallback_kind(incoming.fallback_kind);
        self.retained_source_kind = stylo_merge_retained_source_invalidation_kind(
            self.retained_source_kind,
            incoming.retained_source_kind,
        );
    }
}

impl PendingStyleInvalidationFallbackRootBuckets {
    fn with_reasoned(roots: impl IntoIterator<Item = DomHandle>) -> Self {
        Self {
            reasoned_fallback_roots: roots.into_iter().collect(),
            exact_safety_fallback_roots: IndexSet::new(),
        }
    }

    fn reasoned_roots(&self) -> &IndexSet<DomHandle> {
        &self.reasoned_fallback_roots
    }

    fn reasoned_roots_mut(&mut self) -> &mut IndexSet<DomHandle> {
        &mut self.reasoned_fallback_roots
    }

    fn exact_safety_roots(&self) -> &IndexSet<DomHandle> {
        &self.exact_safety_fallback_roots
    }

    fn exact_safety_roots_mut(&mut self) -> &mut IndexSet<DomHandle> {
        &mut self.exact_safety_fallback_roots
    }

    fn total_count(&self) -> usize {
        self.reasoned_fallback_roots.len() + self.exact_safety_fallback_roots.len()
    }

    fn merge_from(&mut self, incoming: Self) {
        self.reasoned_fallback_roots
            .extend(incoming.reasoned_fallback_roots);
        self.exact_safety_fallback_roots
            .extend(incoming.exact_safety_fallback_roots);
    }
}

impl PendingStyleInvalidationTargetContext {
    fn new(target: StyleInvalidationSourceTarget) -> Self {
        Self {
            target,
            mutation_snapshot: MoliMutationSnapshot::default(),
            structural_boundary_cleanup_roots: StyleStructuralBoundaryCleanupRoots::default(),
            fallback_reasons: IndexSet::new(),
        }
    }

    fn add_fallback_reason(&mut self, reason: StyloSourceInvalidationFallbackReason) {
        self.fallback_reasons.insert(reason);
    }

    fn set_mutation_snapshot(&mut self, snapshot: MoliMutationSnapshot) {
        self.mutation_snapshot = snapshot;
    }

    fn extend_structural_boundary_cleanup_roots(
        &mut self,
        roots: impl IntoIterator<Item = DomHandle>,
    ) {
        self.structural_boundary_cleanup_roots.extend(roots);
    }

    fn merge_from(&mut self, incoming: Self) {
        debug_assert_eq!(
            self.target, incoming.target,
            "target-query contexts should only merge for the same target"
        );
        self.mutation_snapshot
            .merge_from(incoming.mutation_snapshot);
        self.structural_boundary_cleanup_roots
            .extend(incoming.structural_boundary_cleanup_roots.into_roots());
        self.fallback_reasons.extend(incoming.fallback_reasons);
    }
}

impl StyleStructuralBoundaryCleanupRoots {
    pub(super) fn from_roots(roots: impl IntoIterator<Item = DomHandle>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &DomHandle> {
        self.roots.iter()
    }

    pub(super) fn len(&self) -> usize {
        self.roots.len()
    }

    pub(super) fn into_roots(self) -> IndexSet<DomHandle> {
        self.roots
    }

    pub(super) fn extend(&mut self, roots: impl IntoIterator<Item = DomHandle>) {
        self.roots.extend(roots);
    }
}

pub(super) fn merge_pending_target_queries(
    existing: &mut Vec<PendingStyleInvalidationTargetQueries>,
    incoming: Vec<PendingStyleInvalidationTargetQueries>,
) {
    let mut indices = pending_target_query_indices(existing);
    for incoming_item in incoming {
        merge_pending_target_query(existing, &mut indices, incoming_item);
    }
}

fn pending_target_query_indices(
    target_queries: &[PendingStyleInvalidationTargetQueries],
) -> HashMap<StyleInvalidationSourceTarget, usize> {
    target_queries
        .iter()
        .enumerate()
        .map(|(index, target_query)| (target_query.target().clone(), index))
        .collect()
}

fn merge_pending_target_query(
    existing: &mut Vec<PendingStyleInvalidationTargetQueries>,
    indices: &mut HashMap<StyleInvalidationSourceTarget, usize>,
    incoming_item: PendingStyleInvalidationTargetQueries,
) {
    if let Some(&index) = indices.get(incoming_item.target()) {
        let existing_item = &mut existing[index];
        existing_item.merge_from(incoming_item);
    } else {
        indices.insert(incoming_item.target().clone(), existing.len());
        existing.push(incoming_item);
    }
}
