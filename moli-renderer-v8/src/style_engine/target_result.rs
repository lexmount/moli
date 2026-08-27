use indexmap::{IndexMap, IndexSet};
use moli_selector::{
    MoliInvalidationResult, MoliInvalidationSourceResultsSink, StyloSourceAffectedRootsCleanup,
    StyloSourceAffectedRootsCleanupSink,
    StyloSourceFallbackRootAvailability as StyleSourceFallbackRootAvailability,
    StyloSourceFallbackRootAvailabilitySummary, StyloSourceFallbackRootAvailabilitySummarySink,
    StyloSourceInvalidationFallbackReason, StyloSourceStyleInvalidationSourceResult,
    StyloSourceStyleInvalidationSourceResultKind,
    StyloSourceStyleInvalidationSourceResultKindSummary,
    StyloSourceStyleInvalidationSourceResultKindSummarySink,
    StyloSourceStyleInvalidationSourceResultParts,
    StyloSourceStyleInvalidationSourceResultPartsSink,
    StyloSourceStyleInvalidationSourceResultSink,
    StyloSourceStyleInvalidationTargetResultCleanupFacts,
    StyloSourceStyleInvalidationTargetResultCleanupFactsPartsSink,
    StyloSourceStyleInvalidationTargetResultCleanupFactsSink,
    StyloSourceStyleInvalidationTargetResultDiagnosticFacts,
    StyloSourceStyleInvalidationTargetResultDiagnosticFactsPartsSink,
    StyloSourceStyleInvalidationTargetResultDiagnosticFactsSink,
    StyloSourceStyleInvalidationTargetResultRecord,
};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    StyleInvalidationSourceTarget,
    source_lifecycle::{
        StyleSourceLifecycleReport, StyleSourceLifecycleTargetAvailability,
        StyleSourceLifecycleTargetAvailabilitySink,
        StyleSourceLifecycleTargetAvailabilitySummarySink,
    },
    target_queries::{
        PendingStyleInvalidationTargetQueries, PendingStyleInvalidationTargetResultInput,
        PendingStyleInvalidationTargetTraceCountsSink, StyleStructuralBoundaryCleanupRoots,
    },
};

#[cfg(test)]
use super::source_lifecycle::{StyleSourceDocumentKind, StyleSourceLifecycleAvailability};

pub(super) type StyleInvalidationDiagnosticTargetResultKind =
    StyloSourceStyleInvalidationSourceResultKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StyleDiagnosticSourceTargetAvailability {
    lifecycle: Option<StyleSourceLifecycleTargetAvailability>,
    fallback_roots: Option<StyleSourceFallbackRootAvailability>,
}

struct StyleInvalidationDiagnosticTargetResultClassification {
    kind: StyleInvalidationDiagnosticTargetResultKind,
    diagnostic_source_availability: StyleDiagnosticSourceTargetAvailability,
}

struct StyleInvalidationDiagnosticTargetResultBuilder<'target, 'lifecycle> {
    target_input: PendingStyleInvalidationTargetResultInput<'target>,
    diagnostics: StyleInvalidationDiagnosticTargetResultContext<'lifecycle>,
    target_result: Option<StyleInvalidationDiagnosticTargetResult>,
}

struct StyleInvalidationDiagnosticTargetResultContext<'lifecycle> {
    diagnostic_source_lifecycle: Option<&'lifecycle StyleSourceLifecycleReport>,
}

struct StyleInvalidationRetainedTargetResultsDiagnostics<'lifecycle> {
    diagnostic_source_lifecycle: Option<&'lifecycle StyleSourceLifecycleReport>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StyleInvalidationDiagnosticTargetResultTraceCounts {
    recorded_retained_query_count: usize,
    recorded_mutation_snapshot_count: usize,
    recorded_structural_boundary_cleanup_root_count: usize,
}

struct StyleInvalidationRetainedTargetResultsBuilder<'host, 'target_queries, 'lifecycle> {
    host: &'host DomHost,
    diagnostics: Option<StyleInvalidationRetainedTargetResultsDiagnostics<'lifecycle>>,
    target_queries: &'target_queries [PendingStyleInvalidationTargetQueries],
    cleanup_plan: StyleInvalidationRetainedCleanupPlanBuilder,
    trace: StyleInvalidationRetainedTargetResultsTrace,
}

struct StyleInvalidationTargetCleanupFactsBuilder<'host, 'target_input, 'target_queries, 'cleanup> {
    host: &'host DomHost,
    target_input: &'target_input PendingStyleInvalidationTargetResultInput<'target_queries>,
    cleanup_plan: &'cleanup mut StyleInvalidationRetainedCleanupPlanBuilder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StyleInvalidationDiagnosticTargetResult {
    target: Option<StyleInvalidationSourceTarget>,
    kind: StyleInvalidationDiagnosticTargetResultKind,
    exact: bool,
    empty_result_is_exact: bool,
    matched_dependency_count: usize,
    fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    diagnostic_source_availability: Option<StyleDiagnosticSourceTargetAvailability>,
    retained_query_count: usize,
    mutation_snapshot_count: usize,
    structural_boundary_cleanup_root_count: usize,
    affected_root_count: usize,
}

/// Aggregate fallback telemetry for one invalidation drain.
///
/// This is derived from retained diagnostic target results only when style
/// invalidation tracing is enabled (and in tests). Normal rendering does not
/// retain the target results or maintain counters alongside mutations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StyleInvalidationFallbackTelemetry {
    exact_target_result_count: usize,
    inexact_target_result_count: usize,
    fallback_target_result_count: usize,
    source_result_affected_root_count: usize,
    fallback_reason_target_counts: IndexMap<StyloSourceInvalidationFallbackReason, usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StyleInvalidationDiagnosticTargetResultSummary {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StyleInvalidationDiagnosticTargetResultSummaryTraceCounts {
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

pub(super) trait StyleInvalidationDiagnosticTargetResultSummaryTraceCountsSink {
    fn record_diagnostic_target_result_count(&mut self, count: usize);

    fn record_missing_fallback_roots_target_count(&mut self, count: usize);

    fn record_retained_source_unavailable_target_count(&mut self, count: usize);

    fn record_context_fallback_target_count(&mut self, count: usize);

    fn record_source_scope_fallback_target_count(&mut self, count: usize);

    fn record_source_lifecycle_available_without_source_target_count(&mut self, count: usize);

    fn record_source_lifecycle_unavailable_target_count(&mut self, count: usize);

    fn record_child_document_source_target_count(&mut self, count: usize);

    fn record_detached_document_source_target_count(&mut self, count: usize);
}

pub(super) struct StyleInvalidationRetainedTargetResults {
    cleanup: StyleInvalidationRetainedCleanupPlan,
    trace: StyleInvalidationRetainedTargetResultsTrace,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct StyleInvalidationRetainedTargetResultsTrace {
    diagnostic_target_results: Vec<StyleInvalidationDiagnosticTargetResult>,
    summary: StyleInvalidationDiagnosticTargetResultSummary,
}

pub(super) trait StyleInvalidationRetainedTargetResultsSink {
    fn set_retained_target_results(
        &mut self,
        retained_results: StyleInvalidationRetainedTargetResults,
    );
}

pub(super) struct StyleInvalidationRetainedCleanupPlan {
    cleanup: StyleInvalidationRetainedCleanup,
    cleanup_class_constraint: StyleInvalidationCleanupClassConstraint,
}

#[derive(Default)]
struct StyleInvalidationRetainedCleanupPlanBuilder {
    roots: StyleInvalidationRetainedCleanupRoots,
    decision: StyleInvalidationRetainedCleanupDecision,
    class_constraint: StyleInvalidationCleanupClassConstraint,
}

pub(super) trait StyleInvalidationRetainedCleanupPlanSink:
    StyleInvalidationRetainedCleanupSink
{
    fn set_retained_cleanup_class_constraint(
        &mut self,
        cleanup_class_constraint: StyleInvalidationCleanupClassConstraint,
    );
}

pub(super) enum StyleInvalidationRetainedCleanup {
    Roots(StyleInvalidationRetainedCleanupRoots),
    ClearAll(IndexSet<StyloSourceInvalidationFallbackReason>),
}

pub(super) trait StyleInvalidationRetainedCleanupSink {
    fn set_retained_cleanup_roots(&mut self, roots: StyleInvalidationRetainedCleanupRoots);

    fn set_retained_clear_all_reasons(
        &mut self,
        reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    );
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum StyleInvalidationCleanupClassConstraint {
    #[default]
    None,
    UnknownOrFallback,
}

#[derive(Default)]
pub(super) struct StyleInvalidationRetainedCleanupRoots {
    exact_affected_roots: IndexSet<DomHandle>,
    source_fallback_roots: IndexSet<DomHandle>,
    structural_boundary_cleanup_roots: StyleStructuralBoundaryCleanupRoots,
}

pub(super) trait StyleInvalidationRetainedCleanupRootsSink {
    fn extend_retained_exact_affected_roots(&mut self, roots: IndexSet<DomHandle>);

    fn extend_retained_source_fallback_roots(&mut self, roots: IndexSet<DomHandle>);

    fn extend_retained_structural_boundary_cleanup_roots(
        &mut self,
        roots: StyleStructuralBoundaryCleanupRoots,
    );
}

#[derive(Default)]
struct StyleInvalidationRetainedCleanupDecision {
    clear_all_cleanup_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    fallback_context_reasons: Vec<StyloSourceInvalidationFallbackReason>,
    include_fallback_context_for_clear_all: bool,
}

impl StyleInvalidationDiagnosticTargetResult {
    #[cfg(test)]
    pub(super) fn target(&self) -> &StyleInvalidationSourceTarget {
        self.target
            .as_ref()
            .expect("retained diagnostic target result should carry target identity")
    }

    #[cfg(test)]
    pub(super) fn kind(&self) -> StyleInvalidationDiagnosticTargetResultKind {
        self.kind
    }

    #[cfg(test)]
    pub(super) fn exact(&self) -> bool {
        self.exact
    }

    #[cfg(test)]
    pub(super) fn empty_result_is_exact(&self) -> bool {
        self.empty_result_is_exact
    }

    #[cfg(test)]
    pub(super) fn matched_dependency_count(&self) -> usize {
        self.matched_dependency_count
    }

    #[cfg(test)]
    pub(super) fn fallback_reasons(&self) -> &IndexSet<StyloSourceInvalidationFallbackReason> {
        &self.fallback_reasons
    }

    #[cfg(test)]
    pub(super) fn diagnostic_source_target_availability(
        &self,
    ) -> Option<&StyleDiagnosticSourceTargetAvailability> {
        self.diagnostic_source_availability.as_ref()
    }

    #[cfg(test)]
    pub(super) fn retained_query_count(&self) -> usize {
        self.retained_query_count
    }

    #[cfg(test)]
    pub(super) fn mutation_snapshot_count(&self) -> usize {
        self.mutation_snapshot_count
    }

    #[cfg(test)]
    pub(super) fn structural_boundary_cleanup_root_count(&self) -> usize {
        self.structural_boundary_cleanup_root_count
    }

    #[cfg(test)]
    pub(super) fn affected_root_count(&self) -> usize {
        self.affected_root_count
    }

    fn record_summary_into(&self, summary: &mut StyleInvalidationDiagnosticTargetResultSummary) {
        summary.diagnostic_target_result_count += 1;
        self.kind.record_summary_into(summary);
        if let Some(diagnostic_source_availability) = &self.diagnostic_source_availability {
            diagnostic_source_availability.record_summary_into(summary);
        }
    }
}

impl StyleInvalidationFallbackTelemetry {
    fn from_target_results(results: &[StyleInvalidationDiagnosticTargetResult]) -> Self {
        let mut telemetry = Self::default();
        for result in results {
            if result.exact {
                telemetry.exact_target_result_count += 1;
            } else {
                telemetry.inexact_target_result_count += 1;
            }
            if !result.fallback_reasons.is_empty() {
                telemetry.fallback_target_result_count += 1;
            }
            telemetry.source_result_affected_root_count += result.affected_root_count;
            // A source result stores reasons in an IndexSet, so each reason is
            // counted at most once per source target. Selector duplication
            // therefore cannot inflate the fallback hit rate.
            for reason in &result.fallback_reasons {
                *telemetry
                    .fallback_reason_target_counts
                    .entry(*reason)
                    .or_default() += 1;
            }
        }
        telemetry
    }

    pub(super) fn exact_target_result_count(&self) -> usize {
        self.exact_target_result_count
    }

    pub(super) fn inexact_target_result_count(&self) -> usize {
        self.inexact_target_result_count
    }

    pub(super) fn fallback_target_result_count(&self) -> usize {
        self.fallback_target_result_count
    }

    pub(super) fn source_result_affected_root_count(&self) -> usize {
        self.source_result_affected_root_count
    }

    pub(super) fn fallback_reason_hit_count(&self) -> usize {
        self.fallback_reason_target_counts.values().sum()
    }

    pub(super) fn fallback_reason_target_counts(
        &self,
    ) -> &IndexMap<StyloSourceInvalidationFallbackReason, usize> {
        &self.fallback_reason_target_counts
    }

    #[cfg(test)]
    pub(super) fn fallback_reason_target_count(
        &self,
        reason: StyloSourceInvalidationFallbackReason,
    ) -> usize {
        self.fallback_reason_target_counts
            .get(&reason)
            .copied()
            .unwrap_or_default()
    }
}

impl StyleDiagnosticSourceTargetAvailability {
    fn with_fallback_roots(fallback_roots: Option<StyleSourceFallbackRootAvailability>) -> Self {
        Self {
            lifecycle: None,
            fallback_roots,
        }
    }

    fn is_empty(&self) -> bool {
        self.lifecycle.is_none() && self.fallback_roots.is_none()
    }

    fn record_summary_into(&self, summary: &mut StyleInvalidationDiagnosticTargetResultSummary) {
        if let Some(fallback_roots) = &self.fallback_roots {
            fallback_roots.record_summary_into(summary);
        }
        if let Some(lifecycle) = &self.lifecycle {
            lifecycle.record_summary_into(summary);
        }
    }

    #[cfg(test)]
    pub(super) fn document_kind(&self) -> Option<StyleSourceDocumentKind> {
        self.lifecycle
            .as_ref()
            .and_then(StyleSourceLifecycleTargetAvailability::document_kind)
    }

    #[cfg(test)]
    pub(super) fn lifecycle(&self) -> Option<&StyleSourceLifecycleAvailability> {
        self.lifecycle
            .as_ref()
            .map(StyleSourceLifecycleTargetAvailability::lifecycle)
    }

    #[cfg(test)]
    pub(super) fn fallback_roots(&self) -> Option<StyleSourceFallbackRootAvailability> {
        self.fallback_roots
    }
}

impl StyleInvalidationRetainedTargetResults {
    pub(super) fn drain_into(self, target: &mut impl StyleInvalidationRetainedTargetResultsSink) {
        target.set_retained_target_results(self);
    }

    pub(super) fn into_cleanup_and_trace(
        self,
    ) -> (
        StyleInvalidationRetainedCleanupPlan,
        StyleInvalidationRetainedTargetResultsTrace,
    ) {
        (self.cleanup, self.trace)
    }
}

impl StyleInvalidationRetainedTargetResultsTrace {
    fn reserve_diagnostic_target_results(&mut self, source_result_count: usize) {
        self.diagnostic_target_results.reserve(source_result_count);
    }

    fn record_diagnostic_target_result(
        &mut self,
        target_result: StyleInvalidationDiagnosticTargetResult,
    ) {
        target_result.record_summary_into(&mut self.summary);
        self.diagnostic_target_results.push(target_result);
    }

    pub(super) fn extend(&mut self, other: Self) {
        self.summary.extend(other.summary);
        self.diagnostic_target_results
            .extend(other.diagnostic_target_results);
    }

    pub(super) fn diagnostic_target_results(&self) -> &[StyleInvalidationDiagnosticTargetResult] {
        &self.diagnostic_target_results
    }

    pub(super) fn fallback_telemetry(&self) -> StyleInvalidationFallbackTelemetry {
        StyleInvalidationFallbackTelemetry::from_target_results(&self.diagnostic_target_results)
    }

    pub(super) fn record_diagnostic_target_result_counts_into(
        &self,
        sink: &mut impl StyleInvalidationDiagnosticTargetResultSummaryTraceCountsSink,
    ) {
        self.summary.record_trace_counts_into(sink);
    }

    #[cfg(test)]
    pub(super) fn diagnostic_target_result_summary(
        &self,
    ) -> &StyleInvalidationDiagnosticTargetResultSummary {
        &self.summary
    }
}

impl StyleInvalidationRetainedCleanup {
    pub(super) fn drain_into(self, target: &mut impl StyleInvalidationRetainedCleanupSink) {
        match self {
            Self::Roots(roots) => target.set_retained_cleanup_roots(roots),
            Self::ClearAll(reasons) => {
                target.set_retained_clear_all_reasons(reasons);
            }
        }
    }
}

impl StyleInvalidationRetainedCleanupPlan {
    fn new(
        cleanup: StyleInvalidationRetainedCleanup,
        cleanup_class_constraint: StyleInvalidationCleanupClassConstraint,
    ) -> Self {
        Self {
            cleanup,
            cleanup_class_constraint,
        }
    }

    pub(super) fn drain_into(self, target: &mut impl StyleInvalidationRetainedCleanupPlanSink) {
        self.cleanup.drain_into(target);
        target.set_retained_cleanup_class_constraint(self.cleanup_class_constraint);
    }
}

impl StyleInvalidationRetainedCleanupPlanBuilder {
    fn extend_target_structural_boundary_cleanup_roots(
        &mut self,
        target_input: &PendingStyleInvalidationTargetResultInput<'_>,
    ) {
        self.roots
            .extend_target_structural_boundary_cleanup_roots(target_input);
    }

    fn extend_affected_roots(&mut self, affected_roots: StyloSourceAffectedRootsCleanup) {
        affected_roots.drain_into(&mut self.roots);
    }

    fn require_unknown_or_fallback_cleanup_class(&mut self) {
        self.class_constraint = StyleInvalidationCleanupClassConstraint::UnknownOrFallback;
    }

    fn record_fallback_context_cleanup_reasons(
        &mut self,
        include_for_clear_all: bool,
        reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
    ) {
        if include_for_clear_all {
            self.decision.include_fallback_context_for_clear_all = true;
        }
        self.decision.fallback_context_reasons.extend(reasons);
    }

    fn extend_clear_all_cleanup_reasons(
        &mut self,
        reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
    ) {
        self.decision.clear_all_cleanup_reasons.extend(reasons);
    }

    fn extend_source_fallback_roots(&mut self, roots: impl IntoIterator<Item = DomHandle>) {
        self.roots.extend_source_fallback_roots(roots);
    }

    fn into_cleanup_plan(self) -> StyleInvalidationRetainedCleanupPlan {
        StyleInvalidationRetainedCleanupPlan::new(
            self.decision.into_cleanup(self.roots),
            self.class_constraint,
        )
    }
}

impl StyleInvalidationRetainedCleanupRoots {
    fn extend_target_structural_boundary_cleanup_roots(
        &mut self,
        target_input: &PendingStyleInvalidationTargetResultInput<'_>,
    ) {
        target_input.extend_structural_boundary_cleanup_roots_into(
            &mut self.structural_boundary_cleanup_roots,
        );
    }

    fn extend_source_fallback_roots(&mut self, roots: impl IntoIterator<Item = DomHandle>) {
        self.source_fallback_roots.extend(roots);
    }

    pub(super) fn drain_into(self, target: &mut impl StyleInvalidationRetainedCleanupRootsSink) {
        target.extend_retained_exact_affected_roots(self.exact_affected_roots);
        target.extend_retained_source_fallback_roots(self.source_fallback_roots);
        target.extend_retained_structural_boundary_cleanup_roots(
            self.structural_boundary_cleanup_roots,
        );
    }
}

impl StyloSourceAffectedRootsCleanupSink<DomHandle> for StyleInvalidationRetainedCleanupRoots {
    fn extend_exact_affected_roots(&mut self, roots: &[DomHandle]) {
        self.exact_affected_roots.extend(roots.iter().copied());
    }

    fn extend_source_fallback_roots(&mut self, roots: &[DomHandle]) {
        self.source_fallback_roots.extend(roots.iter().copied());
    }
}

impl StyleInvalidationDiagnosticTargetResultClassification {
    fn for_retained_source_result(
        target_input: &PendingStyleInvalidationTargetResultInput<'_>,
        source_result_kind: StyloSourceStyleInvalidationSourceResultKind,
        fallback_root_availability: Option<StyleSourceFallbackRootAvailability>,
        diagnostic_source_lifecycle: Option<&StyleSourceLifecycleReport>,
    ) -> Self {
        let mut diagnostic_source_availability =
            StyleDiagnosticSourceTargetAvailability::with_fallback_roots(
                fallback_root_availability,
            );
        if let Some(diagnostic_source_lifecycle) = diagnostic_source_lifecycle {
            target_input.record_lifecycle_target_availability_into(
                diagnostic_source_lifecycle,
                &mut diagnostic_source_availability,
            );
        }

        Self {
            kind: source_result_kind,
            diagnostic_source_availability,
        }
    }
}

impl StyleSourceLifecycleTargetAvailabilitySink for StyleDiagnosticSourceTargetAvailability {
    fn record_source_lifecycle_target_availability(
        &mut self,
        availability: StyleSourceLifecycleTargetAvailability,
    ) {
        self.lifecycle = Some(availability);
    }
}

impl<'lifecycle> StyleInvalidationDiagnosticTargetResultContext<'lifecycle> {
    fn new(diagnostic_source_lifecycle: Option<&'lifecycle StyleSourceLifecycleReport>) -> Self {
        Self {
            diagnostic_source_lifecycle,
        }
    }

    fn diagnostic_source_lifecycle(&self) -> Option<&'lifecycle StyleSourceLifecycleReport> {
        self.diagnostic_source_lifecycle
    }
}

impl<'lifecycle> StyleInvalidationRetainedTargetResultsDiagnostics<'lifecycle> {
    fn new(diagnostic_source_lifecycle: Option<&'lifecycle StyleSourceLifecycleReport>) -> Self {
        Self {
            diagnostic_source_lifecycle,
        }
    }

    fn target_result_diagnostics(
        &self,
    ) -> StyleInvalidationDiagnosticTargetResultContext<'lifecycle> {
        StyleInvalidationDiagnosticTargetResultContext::new(self.diagnostic_source_lifecycle)
    }
}

impl<'target, 'lifecycle> StyleInvalidationDiagnosticTargetResultBuilder<'target, 'lifecycle> {
    fn new(
        target_input: PendingStyleInvalidationTargetResultInput<'target>,
        diagnostics: StyleInvalidationDiagnosticTargetResultContext<'lifecycle>,
    ) -> Self {
        Self {
            target_input,
            diagnostics,
            target_result: None,
        }
    }

    fn into_target_result(self) -> StyleInvalidationDiagnosticTargetResult {
        self.target_result
            .expect("diagnostic target-result facts should produce diagnostic target result")
    }
}

impl StyloSourceStyleInvalidationTargetResultDiagnosticFactsSink
    for StyleInvalidationDiagnosticTargetResultBuilder<'_, '_>
{
    fn set_source_style_invalidation_target_result_diagnostic_facts(
        &mut self,
        facts: StyloSourceStyleInvalidationTargetResultDiagnosticFacts,
    ) {
        facts.drain_parts_into(self);
    }
}

impl StyloSourceStyleInvalidationTargetResultDiagnosticFactsPartsSink
    for StyleInvalidationDiagnosticTargetResultBuilder<'_, '_>
{
    fn set_source_style_invalidation_target_result_diagnostic_fact_parts(
        &mut self,
        kind: StyleInvalidationDiagnosticTargetResultKind,
        exact: bool,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
        fallback_reasons: Vec<StyloSourceInvalidationFallbackReason>,
        fallback_root_availability: Option<StyleSourceFallbackRootAvailability>,
        affected_root_count: usize,
    ) {
        let diagnostic_classification =
            StyleInvalidationDiagnosticTargetResultClassification::for_retained_source_result(
                &self.target_input,
                kind,
                fallback_root_availability,
                self.diagnostics.diagnostic_source_lifecycle(),
            );
        let mut trace_counts = StyleInvalidationDiagnosticTargetResultTraceCounts::default();
        self.target_input
            .record_trace_counts_into(&mut trace_counts);
        let diagnostic_source_availability = if diagnostic_classification
            .diagnostic_source_availability
            .is_empty()
        {
            None
        } else {
            Some(diagnostic_classification.diagnostic_source_availability)
        };
        assert!(
            self.target_result.is_none(),
            "diagnostic target-result facts should only be recorded once"
        );
        self.target_result = Some(StyleInvalidationDiagnosticTargetResult {
            target: Some(self.target_input.clone_target_for_diagnostics()),
            kind: diagnostic_classification.kind,
            exact,
            empty_result_is_exact,
            matched_dependency_count,
            diagnostic_source_availability,
            fallback_reasons: fallback_reasons.into_iter().collect(),
            retained_query_count: trace_counts.recorded_retained_query_count,
            mutation_snapshot_count: trace_counts.recorded_mutation_snapshot_count,
            structural_boundary_cleanup_root_count: trace_counts
                .recorded_structural_boundary_cleanup_root_count,
            affected_root_count,
        });
    }
}

impl StyloSourceStyleInvalidationTargetResultCleanupFactsSink
    for StyleInvalidationTargetCleanupFactsBuilder<'_, '_, '_, '_>
{
    fn set_source_style_invalidation_target_result_cleanup_facts(
        &mut self,
        facts: StyloSourceStyleInvalidationTargetResultCleanupFacts,
    ) {
        facts.drain_parts_into(self);
    }
}

impl StyloSourceStyleInvalidationTargetResultCleanupFactsPartsSink
    for StyleInvalidationTargetCleanupFactsBuilder<'_, '_, '_, '_>
{
    fn set_source_style_invalidation_target_result_cleanup_fact_parts(
        &mut self,
        fallback_context_reasons: Vec<StyloSourceInvalidationFallbackReason>,
        clear_all_cleanup_reasons: Vec<StyloSourceInvalidationFallbackReason>,
        include_fallback_context_for_clear_all: bool,
        requires_fallback_handling: bool,
    ) {
        if requires_fallback_handling {
            self.cleanup_plan
                .require_unknown_or_fallback_cleanup_class();
        }
        self.cleanup_plan.record_fallback_context_cleanup_reasons(
            include_fallback_context_for_clear_all,
            fallback_context_reasons,
        );
        self.record_clear_all_cleanup_reasons(clear_all_cleanup_reasons);
    }
}

impl<'host, 'target_input, 'target_queries, 'cleanup>
    StyleInvalidationTargetCleanupFactsBuilder<'host, 'target_input, 'target_queries, 'cleanup>
{
    fn new(
        host: &'host DomHost,
        target_input: &'target_input PendingStyleInvalidationTargetResultInput<'target_queries>,
        cleanup_plan: &'cleanup mut StyleInvalidationRetainedCleanupPlanBuilder,
    ) -> Self {
        Self {
            host,
            target_input,
            cleanup_plan,
        }
    }

    fn record_clear_all_cleanup_reasons(
        &mut self,
        reasons: Vec<StyloSourceInvalidationFallbackReason>,
    ) {
        if reasons.is_empty() {
            return;
        }
        let source_scope_fallback_roots = self.target_input.source_scope_fallback_roots(self.host);
        if source_scope_fallback_roots.is_empty() {
            self.cleanup_plan.extend_clear_all_cleanup_reasons(reasons);
            return;
        }
        self.cleanup_plan
            .extend_source_fallback_roots(source_scope_fallback_roots);
    }
}

impl PendingStyleInvalidationTargetTraceCountsSink
    for StyleInvalidationDiagnosticTargetResultTraceCounts
{
    fn record_pending_target_trace_counts(
        &mut self,
        retained_query_count: usize,
        mutation_snapshot_count: usize,
        structural_boundary_cleanup_root_count: usize,
        _fallback_root_bucket_count: usize,
    ) {
        self.recorded_retained_query_count += retained_query_count;
        self.recorded_mutation_snapshot_count += mutation_snapshot_count;
        self.recorded_structural_boundary_cleanup_root_count +=
            structural_boundary_cleanup_root_count;
    }

    fn record_pending_target_source_scope_fallback_roots(&mut self, _roots: Vec<DomHandle>) {}
}

impl StyleInvalidationDiagnosticTargetResultSummary {
    fn trace_counts(&self) -> StyleInvalidationDiagnosticTargetResultSummaryTraceCounts {
        StyleInvalidationDiagnosticTargetResultSummaryTraceCounts {
            diagnostic_target_result_count: self.diagnostic_target_result_count,
            missing_fallback_roots_target_count: self.missing_fallback_roots_target_count,
            retained_source_unavailable_target_count: self.retained_source_unavailable_target_count,
            context_fallback_target_count: self.context_fallback_target_count,
            source_scope_fallback_target_count: self.source_scope_fallback_target_count,
            source_lifecycle_available_without_source_target_count: self
                .source_lifecycle_available_without_source_target_count,
            source_lifecycle_unavailable_target_count: self
                .source_lifecycle_unavailable_target_count,
            child_document_source_target_count: self.child_document_source_target_count,
            detached_document_source_target_count: self.detached_document_source_target_count,
        }
    }

    pub(super) fn record_trace_counts_into(
        &self,
        sink: &mut impl StyleInvalidationDiagnosticTargetResultSummaryTraceCountsSink,
    ) {
        self.trace_counts().record_into(sink);
    }

    #[cfg(test)]
    pub(super) fn missing_fallback_roots_target_count(&self) -> usize {
        self.missing_fallback_roots_target_count
    }

    #[cfg(test)]
    pub(super) fn retained_source_unavailable_target_count(&self) -> usize {
        self.retained_source_unavailable_target_count
    }

    #[cfg(test)]
    pub(super) fn context_fallback_target_count(&self) -> usize {
        self.context_fallback_target_count
    }

    #[cfg(test)]
    pub(super) fn source_scope_fallback_target_count(&self) -> usize {
        self.source_scope_fallback_target_count
    }

    #[cfg(test)]
    pub(super) fn source_lifecycle_available_without_source_target_count(&self) -> usize {
        self.source_lifecycle_available_without_source_target_count
    }

    #[cfg(test)]
    pub(super) fn source_lifecycle_unavailable_target_count(&self) -> usize {
        self.source_lifecycle_unavailable_target_count
    }

    #[cfg(test)]
    pub(super) fn child_document_source_target_count(&self) -> usize {
        self.child_document_source_target_count
    }

    #[cfg(test)]
    pub(super) fn detached_document_source_target_count(&self) -> usize {
        self.detached_document_source_target_count
    }

    pub(super) fn extend(&mut self, other: Self) {
        self.diagnostic_target_result_count += other.diagnostic_target_result_count;
        self.missing_fallback_roots_target_count += other.missing_fallback_roots_target_count;
        self.retained_source_unavailable_target_count +=
            other.retained_source_unavailable_target_count;
        self.context_fallback_target_count += other.context_fallback_target_count;
        self.source_scope_fallback_target_count += other.source_scope_fallback_target_count;
        self.source_lifecycle_available_without_source_target_count +=
            other.source_lifecycle_available_without_source_target_count;
        self.source_lifecycle_unavailable_target_count +=
            other.source_lifecycle_unavailable_target_count;
        self.child_document_source_target_count += other.child_document_source_target_count;
        self.detached_document_source_target_count += other.detached_document_source_target_count;
    }
}

impl StyleSourceLifecycleTargetAvailabilitySummarySink
    for StyleInvalidationDiagnosticTargetResultSummary
{
    fn record_source_lifecycle_available_without_source_target(&mut self) {
        self.source_lifecycle_available_without_source_target_count += 1;
    }

    fn record_source_lifecycle_unavailable_target(&mut self) {
        self.source_lifecycle_unavailable_target_count += 1;
    }

    fn record_child_document_source_target(&mut self) {
        self.child_document_source_target_count += 1;
    }

    fn record_detached_document_source_target(&mut self) {
        self.detached_document_source_target_count += 1;
    }
}

impl StyloSourceFallbackRootAvailabilitySummarySink
    for StyleInvalidationDiagnosticTargetResultSummary
{
    fn record_missing_fallback_roots_target(&mut self) {
        self.missing_fallback_roots_target_count += 1;
    }
}

impl StyloSourceStyleInvalidationSourceResultKindSummarySink
    for StyleInvalidationDiagnosticTargetResultSummary
{
    fn record_retained_source_unavailable_target(&mut self) {
        self.retained_source_unavailable_target_count += 1;
    }

    fn record_source_scope_fallback_target(&mut self) {
        self.source_scope_fallback_target_count += 1;
    }

    fn record_context_fallback_target(&mut self) {
        self.context_fallback_target_count += 1;
    }
}

impl StyleInvalidationDiagnosticTargetResultSummaryTraceCounts {
    fn record_into(
        self,
        sink: &mut impl StyleInvalidationDiagnosticTargetResultSummaryTraceCountsSink,
    ) {
        sink.record_diagnostic_target_result_count(self.diagnostic_target_result_count);
        sink.record_missing_fallback_roots_target_count(self.missing_fallback_roots_target_count);
        sink.record_retained_source_unavailable_target_count(
            self.retained_source_unavailable_target_count,
        );
        sink.record_context_fallback_target_count(self.context_fallback_target_count);
        sink.record_source_scope_fallback_target_count(self.source_scope_fallback_target_count);
        sink.record_source_lifecycle_available_without_source_target_count(
            self.source_lifecycle_available_without_source_target_count,
        );
        sink.record_source_lifecycle_unavailable_target_count(
            self.source_lifecycle_unavailable_target_count,
        );
        sink.record_child_document_source_target_count(self.child_document_source_target_count);
        sink.record_detached_document_source_target_count(
            self.detached_document_source_target_count,
        );
    }
}

impl StyleInvalidationRetainedCleanupDecision {
    fn into_cleanup(
        self,
        roots: StyleInvalidationRetainedCleanupRoots,
    ) -> StyleInvalidationRetainedCleanup {
        if !self.clear_all_cleanup_reasons.is_empty() {
            let mut clear_all_cleanup_reasons = self.clear_all_cleanup_reasons;
            if self.include_fallback_context_for_clear_all {
                clear_all_cleanup_reasons.extend(self.fallback_context_reasons);
            }
            return StyleInvalidationRetainedCleanup::ClearAll(clear_all_cleanup_reasons);
        }
        StyleInvalidationRetainedCleanup::Roots(roots)
    }
}

pub(super) fn retained_target_results_for_source_results(
    host: &DomHost,
    source_results: MoliInvalidationResult,
    diagnostic_source_lifecycle: Option<&StyleSourceLifecycleReport>,
    target_queries: &[PendingStyleInvalidationTargetQueries],
    retain_diagnostic_target_results: bool,
) -> StyleInvalidationRetainedTargetResults {
    let mut builder = StyleInvalidationRetainedTargetResultsBuilder::new(
        host,
        diagnostic_source_lifecycle,
        target_queries,
        retain_diagnostic_target_results,
    );
    source_results.drain_source_results_into(&mut builder);
    builder.into_target_results()
}

impl<'host, 'target_queries, 'lifecycle>
    StyleInvalidationRetainedTargetResultsBuilder<'host, 'target_queries, 'lifecycle>
{
    fn new(
        host: &'host DomHost,
        diagnostic_source_lifecycle: Option<&'lifecycle StyleSourceLifecycleReport>,
        target_queries: &'target_queries [PendingStyleInvalidationTargetQueries],
        retain_diagnostic_target_results: bool,
    ) -> Self {
        let diagnostics = retain_diagnostic_target_results.then(|| {
            StyleInvalidationRetainedTargetResultsDiagnostics::new(diagnostic_source_lifecycle)
        });
        Self {
            host,
            diagnostics,
            target_queries,
            cleanup_plan: StyleInvalidationRetainedCleanupPlanBuilder::default(),
            trace: StyleInvalidationRetainedTargetResultsTrace::default(),
        }
    }

    fn reserve_diagnostic_target_results(&mut self, source_result_count: usize) {
        debug_assert_eq!(
            source_result_count,
            self.target_queries.len(),
            "retained source result should include one result per target query"
        );
        if self.diagnostics.is_some() {
            self.trace
                .reserve_diagnostic_target_results(source_result_count);
        }
    }

    fn record_source_result(&mut self, result: StyloSourceStyleInvalidationSourceResult) {
        result.drain_into(self);
    }

    fn record_source_result_index(
        &mut self,
        source_index: usize,
    ) -> PendingStyleInvalidationTargetResultInput<'target_queries> {
        let target_query = self
            .target_queries
            .get(source_index)
            .expect("retained source result must reference its target query");
        let target_input = target_query.target_result_input();
        self.cleanup_plan
            .extend_target_structural_boundary_cleanup_roots(&target_input);
        target_input
    }

    fn record_source_result_cleanup_and_diagnostics(
        &mut self,
        target_input: PendingStyleInvalidationTargetResultInput<'target_queries>,
        affected_roots: StyloSourceAffectedRootsCleanup,
        target_result_record: StyloSourceStyleInvalidationTargetResultRecord,
    ) {
        self.cleanup_plan.extend_affected_roots(affected_roots);
        let diagnostic_facts = {
            let mut cleanup_builder = StyleInvalidationTargetCleanupFactsBuilder::new(
                self.host,
                &target_input,
                &mut self.cleanup_plan,
            );
            target_result_record.drain_cleanup_into(&mut cleanup_builder)
        };
        if let Some(diagnostic_facts) = diagnostic_facts {
            let Some(retained_diagnostics) = self.diagnostics.as_ref() else {
                debug_assert!(
                    false,
                    "diagnostic target-result facts should only be retained when requested"
                );
                return;
            };
            let diagnostics = retained_diagnostics.target_result_diagnostics();
            let mut target_result_builder =
                StyleInvalidationDiagnosticTargetResultBuilder::new(target_input, diagnostics);
            diagnostic_facts.drain_into(&mut target_result_builder);
            self.trace
                .record_diagnostic_target_result(target_result_builder.into_target_result());
        }
    }

    fn into_target_results(self) -> StyleInvalidationRetainedTargetResults {
        StyleInvalidationRetainedTargetResults {
            cleanup: self.cleanup_plan.into_cleanup_plan(),
            trace: self.trace,
        }
    }
}

impl StyloSourceStyleInvalidationSourceResultSink<DomHandle>
    for StyleInvalidationRetainedTargetResultsBuilder<'_, '_, '_>
{
    fn retain_source_style_invalidation_target_result_diagnostics(&self) -> bool {
        self.diagnostics.is_some()
    }

    fn record_source_style_invalidation_source_result(
        &mut self,
        parts: StyloSourceStyleInvalidationSourceResultParts<DomHandle>,
    ) {
        parts.drain_into(self);
    }
}

impl StyloSourceStyleInvalidationSourceResultPartsSink<DomHandle>
    for StyleInvalidationRetainedTargetResultsBuilder<'_, '_, '_>
{
    fn record_source_style_invalidation_source_result_parts(
        &mut self,
        source_index: usize,
        affected_roots: StyloSourceAffectedRootsCleanup,
        target_result_record: StyloSourceStyleInvalidationTargetResultRecord,
    ) {
        let target_input = self.record_source_result_index(source_index);
        self.record_source_result_cleanup_and_diagnostics(
            target_input,
            affected_roots,
            target_result_record,
        );
    }
}

impl MoliInvalidationSourceResultsSink<DomHandle>
    for StyleInvalidationRetainedTargetResultsBuilder<'_, '_, '_>
{
    fn record_moli_invalidation_source_result_count(&mut self, count: usize) {
        self.reserve_diagnostic_target_results(count);
    }

    fn record_moli_invalidation_source_result(
        &mut self,
        result: StyloSourceStyleInvalidationSourceResult,
    ) {
        self.record_source_result(result);
    }
}
