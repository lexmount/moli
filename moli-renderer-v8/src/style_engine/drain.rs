use indexmap::IndexSet;
use moli_selector::StyloDomStyleAdapter;

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    cause::PendingStyleInvalidationWorkKind,
    cleanup::StyleInvalidationCleanup,
    invalidation::StyleInvalidationCleanupEffects,
    outcome::{StyleInvalidationOutcome, retained_source_invalidation_outcome},
    pending_invalidation::{PendingStyleInvalidationBatch, PendingStyleInvalidationWork},
    source_document::DocumentStyleSourceStores,
    source_lifecycle::{StyleSourceDocumentContext, StyleSourceLifecycleReport},
    state::StyleDocumentState,
    target_queries::PendingStyleInvalidationTargetTraceCountsSink,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::style_engine) enum StyleInvalidationDrainBoundary {
    ComputedStyleRead,
    RuntimeEvaluate,
    RuntimePendingWorkFlush,
    SelectedPageTask,
    NonScriptPageTask,
    #[cfg(test)]
    TestExplicit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StyleInvalidationTurnExitBoundary {
    RuntimeEvaluate,
    RuntimePendingWorkFlush,
    SelectedPageTask,
    NonScriptPageTask,
}

impl From<StyleInvalidationTurnExitBoundary> for StyleInvalidationDrainBoundary {
    fn from(boundary: StyleInvalidationTurnExitBoundary) -> Self {
        match boundary {
            StyleInvalidationTurnExitBoundary::RuntimeEvaluate => Self::RuntimeEvaluate,
            StyleInvalidationTurnExitBoundary::RuntimePendingWorkFlush => {
                Self::RuntimePendingWorkFlush
            }
            StyleInvalidationTurnExitBoundary::SelectedPageTask => Self::SelectedPageTask,
            StyleInvalidationTurnExitBoundary::NonScriptPageTask => Self::NonScriptPageTask,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct StyleInvalidationDrainSummary {
    document: DomHandle,
    boundary: StyleInvalidationDrainBoundary,
    work_item_count: usize,
    mutation_work_item_count: usize,
    state_work_item_count: usize,
    custom_state_work_item_count: usize,
    focus_work_item_count: usize,
    target_work_item_count: usize,
    target_query_count: usize,
    retained_query_count: usize,
    mutation_snapshot_count: usize,
    structural_boundary_cleanup_root_count: usize,
    fallback_root_bucket_count: usize,
    source_scope_fallback_roots: IndexSet<DomHandle>,
}

pub(super) trait StyleInvalidationDrainSummaryTraceSink {
    fn record_drain_document(&mut self, document: DomHandle);

    fn record_drain_boundary(&mut self, boundary: StyleInvalidationDrainBoundary);

    fn record_drain_work_item_count(&mut self, count: usize);

    fn record_drain_mutation_work_item_count(&mut self, count: usize);

    fn record_drain_state_work_item_count(&mut self, count: usize);

    fn record_drain_custom_state_work_item_count(&mut self, count: usize);

    fn record_drain_focus_work_item_count(&mut self, count: usize);

    fn record_drain_target_work_item_count(&mut self, count: usize);

    fn record_drain_target_query_count(&mut self, count: usize);

    fn record_drain_retained_query_count(&mut self, count: usize);

    fn record_drain_mutation_snapshot_count(&mut self, count: usize);

    fn record_drain_structural_boundary_cleanup_root_count(&mut self, count: usize);

    fn record_drain_fallback_root_bucket_count(&mut self, count: usize);

    fn record_drain_source_scope_fallback_roots(&mut self, roots: &IndexSet<DomHandle>);
}

impl StyleInvalidationDrainSummary {
    pub(super) fn from_work_items(
        host: &DomHost,
        document: DomHandle,
        boundary: StyleInvalidationDrainBoundary,
        work_items: &[PendingStyleInvalidationWork],
    ) -> Self {
        let mut summary = StyleInvalidationDrainSummary {
            document,
            boundary,
            work_item_count: work_items.len(),
            mutation_work_item_count: 0,
            state_work_item_count: 0,
            custom_state_work_item_count: 0,
            focus_work_item_count: 0,
            target_work_item_count: 0,
            target_query_count: 0,
            retained_query_count: 0,
            mutation_snapshot_count: 0,
            structural_boundary_cleanup_root_count: 0,
            fallback_root_bucket_count: 0,
            source_scope_fallback_roots: IndexSet::new(),
        };
        for work_item in work_items {
            match work_item.kind {
                PendingStyleInvalidationWorkKind::Mutation => {
                    summary.mutation_work_item_count += 1;
                }
                PendingStyleInvalidationWorkKind::StateChange => {
                    summary.state_work_item_count += 1;
                }
                PendingStyleInvalidationWorkKind::CustomStateChange => {
                    summary.custom_state_work_item_count += 1;
                }
                PendingStyleInvalidationWorkKind::FocusChange => {
                    summary.focus_work_item_count += 1;
                }
                PendingStyleInvalidationWorkKind::TargetChange => {
                    summary.target_work_item_count += 1;
                }
            }
            summary.target_query_count += work_item.target_queries.len();
            for target in &work_item.target_queries {
                target.record_trace_counts_into(host, &mut summary);
            }
        }
        summary
    }

    pub(super) fn record_trace_fields_into(
        &self,
        sink: &mut impl StyleInvalidationDrainSummaryTraceSink,
    ) {
        sink.record_drain_document(self.document);
        sink.record_drain_boundary(self.boundary);
        sink.record_drain_work_item_count(self.work_item_count);
        sink.record_drain_mutation_work_item_count(self.mutation_work_item_count);
        sink.record_drain_state_work_item_count(self.state_work_item_count);
        sink.record_drain_custom_state_work_item_count(self.custom_state_work_item_count);
        sink.record_drain_focus_work_item_count(self.focus_work_item_count);
        sink.record_drain_target_work_item_count(self.target_work_item_count);
        sink.record_drain_target_query_count(self.target_query_count);
        sink.record_drain_retained_query_count(self.retained_query_count);
        sink.record_drain_mutation_snapshot_count(self.mutation_snapshot_count);
        sink.record_drain_structural_boundary_cleanup_root_count(
            self.structural_boundary_cleanup_root_count,
        );
        sink.record_drain_fallback_root_bucket_count(self.fallback_root_bucket_count);
        sink.record_drain_source_scope_fallback_roots(&self.source_scope_fallback_roots);
    }

    #[cfg(test)]
    pub(super) fn document_for_test(&self) -> DomHandle {
        self.document
    }

    #[cfg(test)]
    pub(super) fn boundary_for_test(&self) -> StyleInvalidationDrainBoundary {
        self.boundary
    }

    #[cfg(test)]
    pub(super) fn counts_for_test(&self) -> [(&'static str, usize); 12] {
        [
            ("work_item_count", self.work_item_count),
            ("mutation_work_item_count", self.mutation_work_item_count),
            ("state_work_item_count", self.state_work_item_count),
            (
                "custom_state_work_item_count",
                self.custom_state_work_item_count,
            ),
            ("focus_work_item_count", self.focus_work_item_count),
            ("target_work_item_count", self.target_work_item_count),
            ("target_query_count", self.target_query_count),
            ("retained_query_count", self.retained_query_count),
            ("mutation_snapshot_count", self.mutation_snapshot_count),
            (
                "structural_boundary_cleanup_root_count",
                self.structural_boundary_cleanup_root_count,
            ),
            (
                "fallback_root_bucket_count",
                self.fallback_root_bucket_count,
            ),
            (
                "source_scope_fallback_root_count",
                self.source_scope_fallback_roots.len(),
            ),
        ]
    }

    #[cfg(test)]
    pub(super) fn source_scope_fallback_roots_for_test(&self) -> Vec<DomHandle> {
        self.source_scope_fallback_roots.iter().copied().collect()
    }
}

impl PendingStyleInvalidationTargetTraceCountsSink for StyleInvalidationDrainSummary {
    fn record_pending_target_trace_counts(
        &mut self,
        retained_query_count: usize,
        mutation_snapshot_count: usize,
        structural_boundary_cleanup_root_count: usize,
        fallback_root_bucket_count: usize,
    ) {
        self.retained_query_count += retained_query_count;
        self.mutation_snapshot_count += mutation_snapshot_count;
        self.structural_boundary_cleanup_root_count += structural_boundary_cleanup_root_count;
        self.fallback_root_bucket_count += fallback_root_bucket_count;
    }

    fn record_pending_target_source_scope_fallback_roots(&mut self, roots: Vec<DomHandle>) {
        self.source_scope_fallback_roots.extend(roots);
    }
}

pub(super) fn drain_style_invalidations(
    dom_adapter: &StyloDomStyleAdapter,
    document_state: &StyleDocumentState,
    invalidation_cleanup: StyleInvalidationCleanup<'_>,
    host: &DomHost,
    source_stores: &DocumentStyleSourceStores<'_>,
    document_context: StyleSourceDocumentContext<'_>,
    document: DomHandle,
    pending: PendingStyleInvalidationBatch,
    boundary: StyleInvalidationDrainBoundary,
) {
    if !pending.has_work() {
        return;
    }
    let trace_summary = moli_trace::style_invalidation_trace_enabled().then(|| {
        StyleInvalidationDrainSummary::from_work_items(
            host,
            document,
            boundary,
            &pending.work_items,
        )
    });
    let mut outcome = StyleInvalidationOutcome::default();
    let diagnostic_source_lifecycle = diagnostic_source_lifecycle_report_for_invalidation_drain(
        source_stores,
        host,
        document_context,
        &pending,
    );
    for work_item in pending.work_items {
        let PendingStyleInvalidationWork { target_queries, .. } = work_item;
        let cleanup_effects =
            StyleInvalidationCleanupEffects::clear_shadow_cascade_data_for_cleanup_target();
        let item_outcome = document_state
            .try_with_retained_style_system(|retained| {
                retained_source_invalidation_outcome(
                    dom_adapter,
                    host,
                    diagnostic_source_lifecycle.as_ref(),
                    Some(retained),
                    &target_queries,
                    cleanup_effects,
                )
            })
            .unwrap_or_else(|| {
                retained_source_invalidation_outcome(
                    dom_adapter,
                    host,
                    diagnostic_source_lifecycle.as_ref(),
                    None,
                    &target_queries,
                    cleanup_effects,
                )
            });
        outcome.extend(item_outcome);
    }
    let finalized_result = outcome.finalize(host);
    if let Some(summary) = trace_summary {
        finalized_result.trace_drain_attempt(&summary);
    }
    invalidation_cleanup.apply_finalized_result(host, finalized_result);
}

fn diagnostic_source_lifecycle_report_for_invalidation_drain(
    source_stores: &DocumentStyleSourceStores<'_>,
    host: &DomHost,
    document_context: StyleSourceDocumentContext<'_>,
    pending: &PendingStyleInvalidationBatch,
) -> Option<StyleSourceLifecycleReport> {
    if !retain_diagnostic_source_lifecycle_report_in_invalidation_drain() {
        return None;
    }
    let lifecycle_source_ids = pending.stylesheet_source_ids();
    (!lifecycle_source_ids.is_empty()).then(|| {
        source_stores.source_lifecycle_report_for_source_ids(
            host,
            document_context,
            lifecycle_source_ids,
        )
    })
}

#[cfg(test)]
fn retain_diagnostic_source_lifecycle_report_in_invalidation_drain() -> bool {
    true
}

#[cfg(not(test))]
fn retain_diagnostic_source_lifecycle_report_in_invalidation_drain() -> bool {
    moli_trace::style_invalidation_trace_enabled()
}
