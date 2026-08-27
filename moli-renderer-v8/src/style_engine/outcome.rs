use indexmap::IndexSet;
use moli_selector::{
    MoliInvalidationResult, StyloDomStyleAdapter, StyloRetainedSourceStyleInvalidation,
    StyloSourceInvalidationFallbackReason,
};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    drain::{
        StyleInvalidationDrainBoundary, StyleInvalidationDrainSummary,
        StyleInvalidationDrainSummaryTraceSink,
    },
    invalidation::StyleInvalidationCleanupEffects,
    source_lifecycle::StyleSourceLifecycleReport,
    state::{RetainedStyleSystem, StyleDocumentGenerationSnapshot},
    target_queries::PendingStyleInvalidationTargetQueries,
    target_queries::StyleStructuralBoundaryCleanupRoots,
    target_result::{
        StyleInvalidationCleanupClassConstraint, StyleInvalidationDiagnosticTargetResult,
        StyleInvalidationDiagnosticTargetResultSummaryTraceCountsSink,
        StyleInvalidationFallbackTelemetry, StyleInvalidationRetainedCleanupPlanSink,
        StyleInvalidationRetainedCleanupRoots, StyleInvalidationRetainedCleanupRootsSink,
        StyleInvalidationRetainedCleanupSink, StyleInvalidationRetainedTargetResults,
        StyleInvalidationRetainedTargetResultsSink, StyleInvalidationRetainedTargetResultsTrace,
        retained_target_results_for_source_results,
    },
};

#[cfg(test)]
use super::target_result::StyleInvalidationDiagnosticTargetResultSummary;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct StyleInvalidationOutcome {
    cleanup: StyleInvalidationOutcomeCleanupPlan,
    cleanup_effects: StyleInvalidationCleanupEffects,
    trace: StyleInvalidationCleanupTrace,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct StyleInvalidationOutcomeCleanupPlan {
    target: StyleInvalidationCleanup,
    class_constraint: StyleInvalidationCleanupClassConstraint,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum StyleInvalidationCleanup {
    #[default]
    None,
    Roots(StyleInvalidationCleanupRoots),
    ClearAll(IndexSet<StyloSourceInvalidationFallbackReason>),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct StyleInvalidationCleanupRoots {
    exact_affected_roots: IndexSet<DomHandle>,
    source_fallback_roots: IndexSet<DomHandle>,
    structural_boundary_roots: IndexSet<DomHandle>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StyleInvalidationCleanupRootGroups {
    exact_affected_roots: IndexSet<DomHandle>,
    source_fallback_roots: IndexSet<DomHandle>,
    structural_boundary_roots: IndexSet<DomHandle>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StyleInvalidationDrainDiagnosticTargetResultCounts {
    diagnostic_target_result_count: usize,
    missing_fallback_roots_target_count: usize,
    retained_source_unavailable_target_count: usize,
    context_fallback_target_count: usize,
    source_scope_fallback_target_count: usize,
    source_lifecycle_available_without_source_target_count: usize,
    source_lifecycle_unavailable_target_count: usize,
    child_document_source_target_count: usize,
    detached_document_source_target_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct StyleInvalidationDrainTraceFields {
    document: Option<DomHandle>,
    boundary: Option<StyleInvalidationDrainBoundary>,
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

#[derive(Debug, PartialEq, Eq)]
pub(super) enum StyleInvalidationCleanupTarget {
    Noop,
    ExactAffectedSubtreeRoots(IndexSet<DomHandle>),
    SourceFallbackSubtreeRoots(IndexSet<DomHandle>),
    StructuralBoundarySubtreeRoots(IndexSet<DomHandle>),
    MixedSubtreeRoots(StyleInvalidationCleanupRootGroups),
    ClearAll(IndexSet<StyloSourceInvalidationFallbackReason>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StyleInvalidationCleanupTargetKind {
    Noop,
    ExactAffectedSubtreeRoots,
    SourceFallbackSubtreeRoots,
    StructuralBoundarySubtreeRoots,
    MixedSubtreeRoots,
    ClearAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StyleInvalidationCleanupClass {
    Noop,
    DescendantInherited,
    UnknownOrFallback,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct FinalizedStyleInvalidationResult {
    cleanup: StyleInvalidationFinalCleanup,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct StyleInvalidationFinalCleanup {
    target: StyleInvalidationCleanupTarget,
    class: StyleInvalidationCleanupClass,
    effects: StyleInvalidationCleanupEffects,
    trace: StyleInvalidationCleanupTrace,
}

pub(super) struct StyleInvalidationCleanupApplication {
    target: StyleInvalidationCleanupApplicationTarget,
    context: StyleInvalidationCleanupApplicationContext,
}

pub(super) struct StyleInvalidationCleanupApplicationSubtrees {
    affected_roots: IndexSet<DomHandle>,
    shadow_cascade_roots: IndexSet<DomHandle>,
}

pub(super) struct StyleInvalidationCleanupApplicationContext {
    target_kind: StyleInvalidationCleanupTargetKind,
    class: StyleInvalidationCleanupClass,
    clear_all_reasons: Option<IndexSet<StyloSourceInvalidationFallbackReason>>,
    clear_shadow_cascade_data_for_cleanup_target: bool,
    trace_scoped_fallback: bool,
    trace: StyleInvalidationCleanupTrace,
}

enum StyleInvalidationCleanupApplicationTarget {
    Noop,
    ClearAll,
    SubtreeRoots(StyleInvalidationCleanupApplicationSubtrees),
}

pub(super) trait StyleInvalidationCleanupApplicationSink {
    fn apply_noop_cleanup_application(&self) -> bool;

    fn apply_clear_all_cleanup_application(
        &self,
        context: &StyleInvalidationCleanupApplicationContext,
    ) -> bool;

    fn apply_subtree_roots_cleanup_application(
        &self,
        host: &DomHost,
        subtrees: StyleInvalidationCleanupApplicationSubtrees,
        context: &StyleInvalidationCleanupApplicationContext,
    ) -> bool;
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct StyleInvalidationCleanupTrace {
    retained_target_results: StyleInvalidationRetainedTargetResultsTrace,
}

impl StyleInvalidationOutcomeCleanupPlan {
    fn from_retained_cleanup_roots(roots: StyleInvalidationRetainedCleanupRoots) -> Self {
        let mut cleanup_roots = StyleInvalidationCleanupRoots::default();
        roots.drain_into(&mut cleanup_roots);
        Self::from_cleanup_roots(cleanup_roots)
    }

    fn from_cleanup_roots(roots: StyleInvalidationCleanupRoots) -> Self {
        let target = if roots.is_empty() {
            StyleInvalidationCleanup::None
        } else {
            StyleInvalidationCleanup::Roots(roots)
        };
        Self {
            target,
            class_constraint: StyleInvalidationCleanupClassConstraint::default(),
        }
    }

    fn retained_clear_all(
        reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
    ) -> Self {
        Self {
            target: StyleInvalidationCleanup::retained_clear_all(reasons),
            ..Self::default()
        }
    }

    fn extend(&mut self, other: Self) {
        self.target.extend(other.target);
        self.class_constraint.merge(other.class_constraint);
    }

    fn set_class_constraint(
        &mut self,
        cleanup_class_constraint: StyleInvalidationCleanupClassConstraint,
    ) {
        self.class_constraint = cleanup_class_constraint;
    }

    fn into_final_cleanup(
        self,
        host: &DomHost,
        effects: StyleInvalidationCleanupEffects,
        trace: StyleInvalidationCleanupTrace,
    ) -> StyleInvalidationFinalCleanup {
        let target = self.target.into_cleanup_target(host);
        StyleInvalidationFinalCleanup::new(target, effects, trace, self.class_constraint)
    }
}

impl StyleInvalidationOutcome {
    #[cfg(test)]
    pub(super) fn retained_clear_all_for_test(
        reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
    ) -> Self {
        Self {
            cleanup: StyleInvalidationOutcomeCleanupPlan::retained_clear_all(reasons),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn from_retained_source_result(
        host: &DomHost,
        result: MoliInvalidationResult,
        target_queries: &[PendingStyleInvalidationTargetQueries],
        cleanup_effects: StyleInvalidationCleanupEffects,
    ) -> Self {
        Self::from_retained_source_result_with_lifecycle(
            host,
            result,
            None,
            target_queries,
            cleanup_effects,
        )
    }

    fn from_retained_source_result_with_lifecycle(
        host: &DomHost,
        result: MoliInvalidationResult,
        diagnostic_source_lifecycle: Option<&StyleSourceLifecycleReport>,
        target_queries: &[PendingStyleInvalidationTargetQueries],
        cleanup_effects: StyleInvalidationCleanupEffects,
    ) -> Self {
        let retained_results = retained_target_results_for_source_results(
            host,
            result,
            diagnostic_source_lifecycle,
            target_queries,
            retain_target_result_diagnostics_in_cleanup_trace(),
        );
        let mut outcome = Self {
            cleanup_effects,
            ..Self::default()
        };
        retained_results.drain_into(&mut outcome);
        outcome
    }

    pub(super) fn extend(&mut self, other: Self) {
        self.cleanup.extend(other.cleanup);
        self.cleanup_effects.extend(other.cleanup_effects);
        self.trace.extend(other.trace);
    }

    pub(super) fn finalize(self, host: &DomHost) -> FinalizedStyleInvalidationResult {
        let cleanup = self
            .cleanup
            .into_final_cleanup(host, self.cleanup_effects, self.trace);
        FinalizedStyleInvalidationResult { cleanup }
    }
}

impl StyleInvalidationRetainedTargetResultsSink for StyleInvalidationOutcome {
    fn set_retained_target_results(
        &mut self,
        retained_results: StyleInvalidationRetainedTargetResults,
    ) {
        let (cleanup, trace) = retained_results.into_cleanup_and_trace();
        cleanup.drain_into(&mut self.cleanup);
        self.trace.extend_retained_target_results_trace(trace);
    }
}

impl StyleInvalidationRetainedCleanupPlanSink for StyleInvalidationOutcomeCleanupPlan {
    fn set_retained_cleanup_class_constraint(
        &mut self,
        cleanup_class_constraint: StyleInvalidationCleanupClassConstraint,
    ) {
        self.set_class_constraint(cleanup_class_constraint);
    }
}

impl StyleInvalidationRetainedCleanupSink for StyleInvalidationOutcomeCleanupPlan {
    fn set_retained_cleanup_roots(&mut self, roots: StyleInvalidationRetainedCleanupRoots) {
        *self = Self::from_retained_cleanup_roots(roots);
    }

    fn set_retained_clear_all_reasons(
        &mut self,
        reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        *self = Self::retained_clear_all(reasons);
    }
}

impl FinalizedStyleInvalidationResult {
    pub(super) fn into_cleanup_application(self) -> StyleInvalidationCleanupApplication {
        self.cleanup.into_application()
    }

    #[cfg(test)]
    pub(super) fn cleanup_target(&self) -> &StyleInvalidationCleanupTarget {
        &self.cleanup.target
    }

    pub(super) fn trace_drain_attempt(&self, summary: &StyleInvalidationDrainSummary) {
        self.cleanup.trace_drain_attempt(summary);
    }

    #[cfg(test)]
    pub(super) fn clears_shadow_cascade_data_for_cleanup_target(&self) -> bool {
        self.cleanup
            .effects
            .clears_shadow_cascade_data_for_cleanup_target()
    }

    #[cfg(test)]
    pub(super) fn cleanup_class(&self) -> StyleInvalidationCleanupClass {
        self.cleanup.class
    }

    #[cfg(test)]
    pub(super) fn cleanup_target_kind(&self) -> StyleInvalidationCleanupTargetKind {
        self.cleanup.target.kind()
    }

    #[cfg(test)]
    pub(super) fn requires_scoped_fallback_trace_for_test(&self) -> bool {
        self.cleanup
            .target
            .requires_scoped_fallback_trace(self.cleanup.class)
    }

    #[cfg(test)]
    pub(super) fn clear_all_reasons(
        &self,
    ) -> Option<&IndexSet<StyloSourceInvalidationFallbackReason>> {
        self.cleanup.target.clear_all_reasons()
    }

    #[cfg(test)]
    pub(super) fn affected_root_count(&self) -> usize {
        self.cleanup.target.affected_root_count()
    }

    #[cfg(test)]
    pub(super) fn diagnostic_target_results(&self) -> &[StyleInvalidationDiagnosticTargetResult] {
        self.cleanup.trace.diagnostic_target_results()
    }

    #[cfg(test)]
    pub(super) fn diagnostic_target_result_summary(
        &self,
    ) -> &StyleInvalidationDiagnosticTargetResultSummary {
        self.cleanup.trace.diagnostic_target_result_summary()
    }

    #[cfg(test)]
    pub(super) fn fallback_telemetry(&self) -> StyleInvalidationFallbackTelemetry {
        self.cleanup.trace.fallback_telemetry()
    }
}

impl StyleInvalidationFinalCleanup {
    fn new(
        target: StyleInvalidationCleanupTarget,
        effects: StyleInvalidationCleanupEffects,
        trace: StyleInvalidationCleanupTrace,
        cleanup_class_constraint: StyleInvalidationCleanupClassConstraint,
    ) -> Self {
        let class = cleanup_class_constraint.cleanup_class_for_target(&target);
        Self {
            target,
            class,
            effects,
            trace,
        }
    }

    pub(super) fn into_application(self) -> StyleInvalidationCleanupApplication {
        let target_kind = self.target.kind();
        let trace_scoped_fallback = moli_trace::style_invalidation_trace_enabled()
            && self.target.requires_scoped_fallback_trace(self.class);
        let (target, clear_all_reasons) = self.target.into_application_parts();
        StyleInvalidationCleanupApplication {
            target,
            context: StyleInvalidationCleanupApplicationContext {
                target_kind,
                class: self.class,
                clear_all_reasons,
                clear_shadow_cascade_data_for_cleanup_target: self
                    .effects
                    .clears_shadow_cascade_data_for_cleanup_target(),
                trace_scoped_fallback,
                trace: self.trace,
            },
        }
    }
}

impl StyleInvalidationCleanupApplication {
    pub(super) fn apply_to(
        self,
        host: &DomHost,
        target: &impl StyleInvalidationCleanupApplicationSink,
    ) -> bool {
        match self.target {
            StyleInvalidationCleanupApplicationTarget::Noop => {
                target.apply_noop_cleanup_application()
            }
            StyleInvalidationCleanupApplicationTarget::ClearAll => {
                target.apply_clear_all_cleanup_application(&self.context)
            }
            StyleInvalidationCleanupApplicationTarget::SubtreeRoots(subtrees) => {
                target.apply_subtree_roots_cleanup_application(host, subtrees, &self.context)
            }
        }
    }
}

impl StyleInvalidationCleanupApplicationSubtrees {
    fn new(affected_roots: IndexSet<DomHandle>, shadow_cascade_roots: IndexSet<DomHandle>) -> Self {
        Self {
            affected_roots,
            shadow_cascade_roots,
        }
    }

    fn from_affected_roots(affected_roots: IndexSet<DomHandle>) -> Self {
        Self {
            affected_roots,
            shadow_cascade_roots: IndexSet::new(),
        }
    }

    fn from_source_fallback_roots(roots: IndexSet<DomHandle>) -> Self {
        Self {
            affected_roots: roots.clone(),
            shadow_cascade_roots: roots,
        }
    }

    pub(super) fn into_root_sets(self) -> (IndexSet<DomHandle>, IndexSet<DomHandle>) {
        (self.affected_roots, self.shadow_cascade_roots)
    }
}

impl StyleInvalidationCleanupApplicationContext {
    pub(super) fn clear_all_reasons(
        &self,
    ) -> Option<&IndexSet<StyloSourceInvalidationFallbackReason>> {
        self.clear_all_reasons.as_ref()
    }

    pub(super) fn clears_shadow_cascade_data_for_cleanup_target(&self) -> bool {
        self.clear_shadow_cascade_data_for_cleanup_target
    }

    pub(super) fn trace_clear_all_fallback(
        &self,
        document: DomHandle,
        generations_before: StyleDocumentGenerationSnapshot,
        generations_after: StyleDocumentGenerationSnapshot,
    ) {
        if moli_trace::style_invalidation_trace_enabled() {
            let reasons = self
                .clear_all_reasons
                .as_ref()
                .expect("clear-all cleanup application should carry fallback reasons");
            tracing::info!(
                document = ?document,
                ?reasons,
                cleanup_target_kind = ?self.target_kind,
                cleanup_class = ?self.class,
                trace = ?self.trace,
                clear_shadow_cascade_data_for_cleanup_target =
                    self.clear_shadow_cascade_data_for_cleanup_target,
                source_set_generation_before = generations_before.source_set_generation,
                source_set_generation_after = generations_after.source_set_generation,
                computed_cache_generation_before = generations_before.computed_cache_generation,
                computed_cache_generation_after = generations_after.computed_cache_generation,
                retained_style_system_generation_before =
                    generations_before.retained_style_system_generation,
                retained_style_system_generation_after =
                    generations_after.retained_style_system_generation,
                target_context_epoch_before = generations_before.target_context_epoch,
                target_context_epoch_after = generations_after.target_context_epoch,
                "style invalidation fallback cleared all style caches"
            );
        }
    }

    pub(super) fn trace_scoped_fallback(
        &self,
        document: DomHandle,
        affected_roots: &IndexSet<DomHandle>,
        invalidated_cache_handles: &IndexSet<DomHandle>,
        shadow_cascade_roots: &IndexSet<DomHandle>,
        generations_before: StyleDocumentGenerationSnapshot,
        generations_after: StyleDocumentGenerationSnapshot,
    ) {
        if !self.trace_scoped_fallback {
            return;
        }
        tracing::info!(
            document = ?document,
            cleanup_target_kind = ?self.target_kind,
            cleanup_class = ?self.class,
            trace = ?self.trace,
            clear_shadow_cascade_data_for_cleanup_target =
                self.clear_shadow_cascade_data_for_cleanup_target,
            affected_root_count = affected_roots.len(),
            affected_roots = ?affected_roots,
            invalidated_cache_handle_count = invalidated_cache_handles.len(),
            invalidated_cache_handles = ?invalidated_cache_handles,
            source_fallback_shadow_cascade_root_count = shadow_cascade_roots.len(),
            source_fallback_shadow_cascade_roots = ?shadow_cascade_roots,
            source_set_generation_before = generations_before.source_set_generation,
            source_set_generation_after = generations_after.source_set_generation,
            computed_cache_generation_before = generations_before.computed_cache_generation,
            computed_cache_generation_after = generations_after.computed_cache_generation,
            retained_style_system_generation_before =
                generations_before.retained_style_system_generation,
            retained_style_system_generation_after =
                generations_after.retained_style_system_generation,
            target_context_epoch_before = generations_before.target_context_epoch,
            target_context_epoch_after = generations_after.target_context_epoch,
            "style invalidation fallback cleared scoped style caches"
        );
    }
}

impl StyleInvalidationFinalCleanup {
    fn trace_drain_attempt(&self, summary: &StyleInvalidationDrainSummary) {
        let mut drain_fields = StyleInvalidationDrainTraceFields::default();
        summary.record_trace_fields_into(&mut drain_fields);
        let fallback_telemetry = self.trace.fallback_telemetry();
        let mut diagnostic_target_result_counts =
            StyleInvalidationDrainDiagnosticTargetResultCounts::default();
        self.trace
            .record_diagnostic_target_result_counts_into(&mut diagnostic_target_result_counts);
        tracing::info!(
            document = ?drain_fields
                .document
                .expect("drain summary trace fields should include document"),
            boundary = ?drain_fields
                .boundary
                .expect("drain summary trace fields should include boundary"),
            work_item_count = drain_fields.work_item_count,
            mutation_work_item_count = drain_fields.mutation_work_item_count,
            state_work_item_count = drain_fields.state_work_item_count,
            custom_state_work_item_count = drain_fields.custom_state_work_item_count,
            focus_work_item_count = drain_fields.focus_work_item_count,
            target_work_item_count = drain_fields.target_work_item_count,
            target_query_count = drain_fields.target_query_count,
            retained_query_count = drain_fields.retained_query_count,
            mutation_snapshot_count = drain_fields.mutation_snapshot_count,
            structural_boundary_cleanup_root_count =
                drain_fields.structural_boundary_cleanup_root_count,
            fallback_root_bucket_count = drain_fields.fallback_root_bucket_count,
            source_scope_fallback_root_count = drain_fields.source_scope_fallback_roots.len(),
            source_scope_fallback_roots = ?drain_fields.source_scope_fallback_roots,
            clear_shadow_cascade_data_for_cleanup_target =
                self.effects.clears_shadow_cascade_data_for_cleanup_target(),
            exact_target_result_count = fallback_telemetry.exact_target_result_count(),
            inexact_target_result_count = fallback_telemetry.inexact_target_result_count(),
            fallback_target_result_count =
                fallback_telemetry.fallback_target_result_count(),
            fallback_reason_hit_count = fallback_telemetry.fallback_reason_hit_count(),
            fallback_reason_target_counts = ?fallback_telemetry.fallback_reason_target_counts(),
            source_result_affected_root_count =
                fallback_telemetry.source_result_affected_root_count(),
            retained_source_unavailable_target_count =
                diagnostic_target_result_counts.retained_source_unavailable_target_count,
            missing_fallback_roots_target_count =
                diagnostic_target_result_counts.missing_fallback_roots_target_count,
            context_fallback_target_count = diagnostic_target_result_counts.context_fallback_target_count,
            source_scope_fallback_target_count =
                diagnostic_target_result_counts.source_scope_fallback_target_count,
            source_lifecycle_available_without_source_target_count =
                diagnostic_target_result_counts.source_lifecycle_available_without_source_target_count,
            source_lifecycle_unavailable_target_count =
                diagnostic_target_result_counts.source_lifecycle_unavailable_target_count,
            child_document_source_target_count =
                diagnostic_target_result_counts.child_document_source_target_count,
            detached_document_source_target_count =
                diagnostic_target_result_counts.detached_document_source_target_count,
            clear_all_reasons = ?self.target.clear_all_reasons(),
            cleanup_target_kind = ?self.target.kind(),
            cleanup_class = ?self.class,
            diagnostic_target_result_count = diagnostic_target_result_counts.diagnostic_target_result_count,
            diagnostic_target_results = ?self.trace.diagnostic_target_results(),
            affected_root_count = self.target.affected_root_count(),
            "style invalidation drain attempt summary"
        );
    }
}

impl StyleInvalidationDrainSummaryTraceSink for StyleInvalidationDrainTraceFields {
    fn record_drain_document(&mut self, document: DomHandle) {
        self.document = Some(document);
    }

    fn record_drain_boundary(&mut self, boundary: StyleInvalidationDrainBoundary) {
        self.boundary = Some(boundary);
    }

    fn record_drain_work_item_count(&mut self, count: usize) {
        self.work_item_count += count;
    }

    fn record_drain_mutation_work_item_count(&mut self, count: usize) {
        self.mutation_work_item_count += count;
    }

    fn record_drain_state_work_item_count(&mut self, count: usize) {
        self.state_work_item_count += count;
    }

    fn record_drain_custom_state_work_item_count(&mut self, count: usize) {
        self.custom_state_work_item_count += count;
    }

    fn record_drain_focus_work_item_count(&mut self, count: usize) {
        self.focus_work_item_count += count;
    }

    fn record_drain_target_work_item_count(&mut self, count: usize) {
        self.target_work_item_count += count;
    }

    fn record_drain_target_query_count(&mut self, count: usize) {
        self.target_query_count += count;
    }

    fn record_drain_retained_query_count(&mut self, count: usize) {
        self.retained_query_count += count;
    }

    fn record_drain_mutation_snapshot_count(&mut self, count: usize) {
        self.mutation_snapshot_count += count;
    }

    fn record_drain_structural_boundary_cleanup_root_count(&mut self, count: usize) {
        self.structural_boundary_cleanup_root_count += count;
    }

    fn record_drain_fallback_root_bucket_count(&mut self, count: usize) {
        self.fallback_root_bucket_count += count;
    }

    fn record_drain_source_scope_fallback_roots(&mut self, roots: &IndexSet<DomHandle>) {
        self.source_scope_fallback_roots
            .extend(roots.iter().copied());
    }
}

impl StyleInvalidationCleanupTrace {
    fn extend(&mut self, other: Self) {
        self.retained_target_results
            .extend(other.retained_target_results);
    }

    fn extend_retained_target_results_trace(
        &mut self,
        trace: StyleInvalidationRetainedTargetResultsTrace,
    ) {
        self.retained_target_results.extend(trace);
    }

    pub(super) fn diagnostic_target_results(&self) -> &[StyleInvalidationDiagnosticTargetResult] {
        self.retained_target_results.diagnostic_target_results()
    }

    fn fallback_telemetry(&self) -> StyleInvalidationFallbackTelemetry {
        self.retained_target_results.fallback_telemetry()
    }

    fn record_diagnostic_target_result_counts_into(
        &self,
        sink: &mut impl StyleInvalidationDiagnosticTargetResultSummaryTraceCountsSink,
    ) {
        self.retained_target_results
            .record_diagnostic_target_result_counts_into(sink);
    }

    #[cfg(test)]
    pub(super) fn diagnostic_target_result_summary(
        &self,
    ) -> &StyleInvalidationDiagnosticTargetResultSummary {
        self.retained_target_results
            .diagnostic_target_result_summary()
    }
}

impl StyleInvalidationDiagnosticTargetResultSummaryTraceCountsSink
    for StyleInvalidationDrainDiagnosticTargetResultCounts
{
    fn record_diagnostic_target_result_count(&mut self, count: usize) {
        self.diagnostic_target_result_count += count;
    }

    fn record_missing_fallback_roots_target_count(&mut self, count: usize) {
        self.missing_fallback_roots_target_count += count;
    }

    fn record_retained_source_unavailable_target_count(&mut self, count: usize) {
        self.retained_source_unavailable_target_count += count;
    }

    fn record_context_fallback_target_count(&mut self, count: usize) {
        self.context_fallback_target_count += count;
    }

    fn record_source_scope_fallback_target_count(&mut self, count: usize) {
        self.source_scope_fallback_target_count += count;
    }

    fn record_source_lifecycle_available_without_source_target_count(&mut self, count: usize) {
        self.source_lifecycle_available_without_source_target_count += count;
    }

    fn record_source_lifecycle_unavailable_target_count(&mut self, count: usize) {
        self.source_lifecycle_unavailable_target_count += count;
    }

    fn record_child_document_source_target_count(&mut self, count: usize) {
        self.child_document_source_target_count += count;
    }

    fn record_detached_document_source_target_count(&mut self, count: usize) {
        self.detached_document_source_target_count += count;
    }
}

impl StyleInvalidationCleanupTarget {
    #[cfg(not(test))]
    fn kind(&self) -> StyleInvalidationCleanupTargetKind {
        self.kind_inner()
    }

    #[cfg(test)]
    pub(super) fn kind(&self) -> StyleInvalidationCleanupTargetKind {
        self.kind_inner()
    }

    fn kind_inner(&self) -> StyleInvalidationCleanupTargetKind {
        match self {
            Self::Noop => StyleInvalidationCleanupTargetKind::Noop,
            Self::ExactAffectedSubtreeRoots(_) => {
                StyleInvalidationCleanupTargetKind::ExactAffectedSubtreeRoots
            }
            Self::SourceFallbackSubtreeRoots(_) => {
                StyleInvalidationCleanupTargetKind::SourceFallbackSubtreeRoots
            }
            Self::StructuralBoundarySubtreeRoots(_) => {
                StyleInvalidationCleanupTargetKind::StructuralBoundarySubtreeRoots
            }
            Self::MixedSubtreeRoots(_) => StyleInvalidationCleanupTargetKind::MixedSubtreeRoots,
            Self::ClearAll(_) => StyleInvalidationCleanupTargetKind::ClearAll,
        }
    }

    #[cfg(not(test))]
    fn affected_root_count(&self) -> usize {
        self.affected_root_count_inner()
    }

    #[cfg(test)]
    pub(super) fn affected_root_count(&self) -> usize {
        self.affected_root_count_inner()
    }

    fn affected_root_count_inner(&self) -> usize {
        match self {
            Self::Noop | Self::ClearAll(_) => 0,
            Self::ExactAffectedSubtreeRoots(roots)
            | Self::SourceFallbackSubtreeRoots(roots)
            | Self::StructuralBoundarySubtreeRoots(roots) => roots.len(),
            Self::MixedSubtreeRoots(groups) => groups.all_roots().len(),
        }
    }

    fn clear_all_reasons(&self) -> Option<&IndexSet<StyloSourceInvalidationFallbackReason>> {
        match self {
            Self::Noop
            | Self::ExactAffectedSubtreeRoots(_)
            | Self::SourceFallbackSubtreeRoots(_)
            | Self::StructuralBoundarySubtreeRoots(_)
            | Self::MixedSubtreeRoots(_) => None,
            Self::ClearAll(reasons) => Some(reasons),
        }
    }

    #[cfg(not(test))]
    fn requires_scoped_fallback_trace(&self, class: StyleInvalidationCleanupClass) -> bool {
        self.requires_scoped_fallback_trace_inner(class)
    }

    #[cfg(test)]
    pub(super) fn requires_scoped_fallback_trace(
        &self,
        class: StyleInvalidationCleanupClass,
    ) -> bool {
        self.requires_scoped_fallback_trace_inner(class)
    }

    fn requires_scoped_fallback_trace_inner(&self, class: StyleInvalidationCleanupClass) -> bool {
        !matches!(self, Self::Noop | Self::ClearAll(_))
            && matches!(class, StyleInvalidationCleanupClass::UnknownOrFallback)
    }

    fn into_application_parts(
        self,
    ) -> (
        StyleInvalidationCleanupApplicationTarget,
        Option<IndexSet<StyloSourceInvalidationFallbackReason>>,
    ) {
        match self {
            Self::Noop => (StyleInvalidationCleanupApplicationTarget::Noop, None),
            Self::ClearAll(reasons) => (
                StyleInvalidationCleanupApplicationTarget::ClearAll,
                Some(reasons),
            ),
            Self::ExactAffectedSubtreeRoots(roots)
            | Self::StructuralBoundarySubtreeRoots(roots) => (
                StyleInvalidationCleanupApplicationTarget::SubtreeRoots(
                    StyleInvalidationCleanupApplicationSubtrees::from_affected_roots(roots),
                ),
                None,
            ),
            Self::SourceFallbackSubtreeRoots(roots) => (
                StyleInvalidationCleanupApplicationTarget::SubtreeRoots(
                    StyleInvalidationCleanupApplicationSubtrees::from_source_fallback_roots(roots),
                ),
                None,
            ),
            Self::MixedSubtreeRoots(groups) => (
                StyleInvalidationCleanupApplicationTarget::SubtreeRoots(
                    groups.into_application_subtrees(),
                ),
                None,
            ),
        }
    }
}

impl StyleInvalidationCleanup {
    fn retained_clear_all(
        reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
    ) -> Self {
        Self::ClearAll(reasons.into_iter().collect())
    }

    fn extend(&mut self, other: Self) {
        match other {
            Self::None => {}
            Self::ClearAll(incoming_reasons) => match self {
                Self::None | Self::Roots(_) => {
                    *self = Self::ClearAll(incoming_reasons);
                }
                Self::ClearAll(existing_reasons) => {
                    existing_reasons.extend(incoming_reasons);
                }
            },
            Self::Roots(incoming) => match self {
                Self::None => {
                    *self = Self::Roots(incoming);
                }
                Self::Roots(existing) => {
                    existing.extend(incoming);
                }
                Self::ClearAll(_) => {}
            },
        }
    }

    fn into_cleanup_target(self, host: &DomHost) -> StyleInvalidationCleanupTarget {
        match self {
            Self::None => StyleInvalidationCleanupTarget::Noop,
            Self::Roots(roots) => roots.into_cleanup_target(host),
            Self::ClearAll(reasons) => StyleInvalidationCleanupTarget::ClearAll(reasons),
        }
    }
}

impl StyleInvalidationCleanupRoots {
    fn is_empty(&self) -> bool {
        self.exact_affected_roots.is_empty()
            && self.source_fallback_roots.is_empty()
            && self.structural_boundary_roots.is_empty()
    }

    fn extend(&mut self, other: Self) {
        self.exact_affected_roots.extend(other.exact_affected_roots);
        self.source_fallback_roots
            .extend(other.source_fallback_roots);
        self.structural_boundary_roots
            .extend(other.structural_boundary_roots);
    }

    fn into_cleanup_target(self, host: &DomHost) -> StyleInvalidationCleanupTarget {
        let existing_roots = |roots: IndexSet<DomHandle>| {
            roots
                .into_iter()
                .filter(|root| host.node(*root).is_some())
                .collect()
        };
        let groups = StyleInvalidationCleanupRootGroups::new(
            existing_roots(self.exact_affected_roots),
            existing_roots(self.source_fallback_roots),
            existing_roots(self.structural_boundary_roots),
        );
        groups.into_cleanup_target()
    }
}

impl StyleInvalidationRetainedCleanupRootsSink for StyleInvalidationCleanupRoots {
    fn extend_retained_exact_affected_roots(&mut self, roots: IndexSet<DomHandle>) {
        self.exact_affected_roots.extend(roots);
    }

    fn extend_retained_source_fallback_roots(&mut self, roots: IndexSet<DomHandle>) {
        self.source_fallback_roots.extend(roots);
    }

    fn extend_retained_structural_boundary_cleanup_roots(
        &mut self,
        roots: StyleStructuralBoundaryCleanupRoots,
    ) {
        self.structural_boundary_roots.extend(roots.into_roots());
    }
}

impl StyleInvalidationCleanupRootGroups {
    fn new(
        exact_affected_roots: IndexSet<DomHandle>,
        source_fallback_roots: IndexSet<DomHandle>,
        structural_boundary_roots: IndexSet<DomHandle>,
    ) -> Self {
        Self {
            exact_affected_roots,
            source_fallback_roots,
            structural_boundary_roots,
        }
    }

    #[cfg(test)]
    pub(super) fn exact_affected_roots(&self) -> &IndexSet<DomHandle> {
        &self.exact_affected_roots
    }

    #[cfg(test)]
    pub(super) fn source_fallback_roots(&self) -> &IndexSet<DomHandle> {
        &self.source_fallback_roots
    }

    #[cfg(test)]
    pub(super) fn structural_boundary_roots(&self) -> &IndexSet<DomHandle> {
        &self.structural_boundary_roots
    }

    fn non_empty_group_count(&self) -> usize {
        [
            !self.exact_affected_roots.is_empty(),
            !self.source_fallback_roots.is_empty(),
            !self.structural_boundary_roots.is_empty(),
        ]
        .into_iter()
        .filter(|non_empty| *non_empty)
        .count()
    }

    fn into_cleanup_target(self) -> StyleInvalidationCleanupTarget {
        match self.non_empty_group_count() {
            0 => StyleInvalidationCleanupTarget::Noop,
            1 if !self.exact_affected_roots.is_empty() => {
                StyleInvalidationCleanupTarget::ExactAffectedSubtreeRoots(self.exact_affected_roots)
            }
            1 if !self.source_fallback_roots.is_empty() => {
                StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(
                    self.source_fallback_roots,
                )
            }
            1 if !self.structural_boundary_roots.is_empty() => {
                StyleInvalidationCleanupTarget::StructuralBoundarySubtreeRoots(
                    self.structural_boundary_roots,
                )
            }
            _ => StyleInvalidationCleanupTarget::MixedSubtreeRoots(self),
        }
    }

    pub(super) fn all_roots(&self) -> IndexSet<DomHandle> {
        let mut roots = self.exact_affected_roots.clone();
        roots.extend(self.source_fallback_roots.iter().copied());
        roots.extend(self.structural_boundary_roots.iter().copied());
        roots
    }

    pub(super) fn into_all_roots(self) -> IndexSet<DomHandle> {
        let mut roots = self.exact_affected_roots;
        roots.extend(self.source_fallback_roots);
        roots.extend(self.structural_boundary_roots);
        roots
    }

    fn into_application_subtrees(self) -> StyleInvalidationCleanupApplicationSubtrees {
        let shadow_cascade_roots = self.source_fallback_roots.clone();
        StyleInvalidationCleanupApplicationSubtrees::new(
            self.into_all_roots(),
            shadow_cascade_roots,
        )
    }

    fn has_any_root(&self) -> bool {
        !self.exact_affected_roots.is_empty()
            || !self.source_fallback_roots.is_empty()
            || !self.structural_boundary_roots.is_empty()
    }

    fn has_source_fallback_roots(&self) -> bool {
        !self.source_fallback_roots.is_empty()
    }
}

impl StyleInvalidationCleanupClassConstraint {
    fn merge(&mut self, other: Self) {
        if let Self::UnknownOrFallback = other {
            *self = Self::UnknownOrFallback;
        }
    }

    fn cleanup_class_for_target(
        self,
        target: &StyleInvalidationCleanupTarget,
    ) -> StyleInvalidationCleanupClass {
        match self {
            Self::None => target.cleanup_class_without_constraint(),
            Self::UnknownOrFallback => StyleInvalidationCleanupClass::UnknownOrFallback,
        }
    }
}

impl StyleInvalidationCleanupTarget {
    fn cleanup_class_without_constraint(&self) -> StyleInvalidationCleanupClass {
        match self {
            Self::Noop => StyleInvalidationCleanupClass::Noop,
            Self::ExactAffectedSubtreeRoots(roots)
            | Self::StructuralBoundarySubtreeRoots(roots)
                if !roots.is_empty() =>
            {
                StyleInvalidationCleanupClass::DescendantInherited
            }
            Self::MixedSubtreeRoots(groups) if groups.has_source_fallback_roots() => {
                StyleInvalidationCleanupClass::UnknownOrFallback
            }
            Self::MixedSubtreeRoots(groups) if groups.has_any_root() => {
                StyleInvalidationCleanupClass::DescendantInherited
            }
            Self::SourceFallbackSubtreeRoots(_)
            | Self::ClearAll(_)
            | Self::ExactAffectedSubtreeRoots(_)
            | Self::StructuralBoundarySubtreeRoots(_)
            | Self::MixedSubtreeRoots(_) => StyleInvalidationCleanupClass::UnknownOrFallback,
        }
    }
}

pub(super) fn retained_source_invalidation_outcome(
    dom_adapter: &StyloDomStyleAdapter,
    host: &DomHost,
    diagnostic_source_lifecycle: Option<&StyleSourceLifecycleReport>,
    retained: Option<&RetainedStyleSystem>,
    target_queries: &[PendingStyleInvalidationTargetQueries],
    cleanup_effects: StyleInvalidationCleanupEffects,
) -> StyleInvalidationOutcome {
    if target_queries.is_empty() {
        return StyleInvalidationOutcome::default();
    }
    dom_adapter.with_bound_host(host, |adapter| {
        let source_inputs = retained_source_invalidation_inputs(retained, target_queries);
        let result = adapter.collect_retained_source_style_invalidation_result(
            host,
            retained.map(|retained| &retained.stylist),
            retained
                .map(|retained| retained.shadow_cascade_data.as_slice())
                .unwrap_or_default(),
            source_inputs,
        );
        StyleInvalidationOutcome::from_retained_source_result_with_lifecycle(
            host,
            result,
            diagnostic_source_lifecycle,
            target_queries,
            cleanup_effects,
        )
    })
}

#[cfg(test)]
fn retain_target_result_diagnostics_in_cleanup_trace() -> bool {
    true
}

#[cfg(not(test))]
fn retain_target_result_diagnostics_in_cleanup_trace() -> bool {
    moli_trace::style_invalidation_trace_enabled()
}

fn retained_source_invalidation_inputs<'a>(
    retained: Option<&'a RetainedStyleSystem>,
    target_queries: &'a [PendingStyleInvalidationTargetQueries],
) -> impl Iterator<Item = StyloRetainedSourceStyleInvalidation<'a>> + 'a {
    target_queries.iter().map(move |source| {
        let input = source.retained_source_invalidation_input();
        input.into_stylo_input(retained)
    })
}
