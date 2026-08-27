use super::cause::{
    PendingCauseFallback, PendingStyleInvalidationCause, PendingStyleInvalidationWorkKind,
};
use super::drain::StyleInvalidationDrainSummary;
use super::invalidation::{StyleInvalidationCleanupEffects, handle_is_in_style_subtrees};
use super::outcome::{
    StyleInvalidationCleanupClass, StyleInvalidationCleanupTarget,
    StyleInvalidationCleanupTargetKind, StyleInvalidationOutcome,
    retained_source_invalidation_outcome,
};
use super::pending_invalidation::{PendingStyleInvalidationBatch, PendingStyleInvalidationWork};
use super::scope::{mutation_effects_have_source_scope, source_scope_for_mutations};
use super::source_dirty::StyleSourceDirtyReason;
use super::source_lifecycle::{
    StyleSourceDocumentKind, StyleSourceLifecycleAvailability, StyleSourceLifecycleOwner,
    StyleSourceLifecycleOwnerDetailTrace, StyleSourceLifecycleOwnerDetailTraceSink,
    StyleSourceLifecycleRecord, StyleSourceLifecycleReport, StyleSourceLifecycleUnavailableReason,
    StyleSourceLifecycleWithoutSourceReason,
};
use super::target_result::StyleInvalidationDiagnosticTargetResultKind;
use super::*;
use std::collections::HashSet;

use crate::dom::native::{NativeDom, NativeNodeId};
use moli_selector::{
    MoliInvalidationResultBuilder, MoliInvalidationSourceResultsSink,
    MoliStyleMutationSnapshot as MoliMutationSnapshot, StyloPlannedFallbackRootInvalidationTarget,
    StyloPlannedFallbackRootInvalidationTargetPartsSink, StyloPlannedSourceDependencyInvalidation,
    StyloPlannedSourceDependencyInvalidationPartsSink,
    StyloPlannedSourceDependencyInvalidationTargetPartsSink, StyloRetainedSourceStyleInvalidation,
    StyloRetainedSourceStyleInvalidationKind, StyloRetainedSourceStyleInvalidationSink,
    StyloSourceAffectedRootsCleanup, StyloSourceAffectedRootsCleanupSink,
    StyloSourceDependencyBoundaryRoots, StyloSourceDependencyInvalidationBatchPlan,
    StyloSourceDependencyInvalidationBatchPlanSink, StyloSourceDependencyInvalidationBatchSource,
    StyloSourceDependencyInvalidationRequest, StyloSourceDependencyRequestRequirement,
    StyloSourceDependencySummary,
    StyloSourceFallbackRootAvailability as StyleSourceFallbackRootAvailability,
    StyloSourceInvalidationFallbackReason, StyloSourceStyleInvalidationSourceResult,
    StyloSourceStyleInvalidationSourceResultKind, StyloSourceStyleInvalidationSourceResultParts,
    StyloSourceStyleInvalidationSourceResultPartsSink,
    StyloSourceStyleInvalidationSourceResultSink,
    StyloSourceStyleInvalidationTargetResultCleanupFacts,
    StyloSourceStyleInvalidationTargetResultCleanupFactsPartsSink,
    StyloSourceStyleInvalidationTargetResultCleanupFactsSink,
    StyloSourceStyleInvalidationTargetResultRecord, StyloStyleInvalidationQuery,
    stylo_attribute_change_can_skip_fallback_without_dependency,
    stylo_removed_element_dependency_snapshots as removed_element_dependency_snapshots,
    stylo_retained_source_style_invalidation_from_parts,
    stylo_source_dependency_invalidation_batch_plan, stylo_source_scope_fallback_plan,
};
use selectors::{Element as SelectorsElement, matching::ElementSelectorFlags};
use style::LocalName;
use style::dom::TElement;
use style::{servo_arc::Arc as ServoArc, stylist::CascadeData};

fn test_host() -> DomHost {
    DomHost::from_dom(NativeDom::new_html(
        url::Url::parse("https://example.test/").expect("valid test url"),
    ))
}

#[derive(Default)]
struct FallbackRootTargetRootsForTest {
    roots: Vec<DomHandle>,
}

#[derive(Default)]
struct SourceResultsForTest {
    roots: indexmap::IndexSet<DomHandle>,
    has_fallback_reasons: bool,
}

#[derive(Default)]
struct PlannedSourceDependencyInputForTest {
    kind: Option<StyloRetainedSourceStyleInvalidationKind>,
    fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
    queries: Option<indexmap::IndexSet<RetainedStyleInvalidationQuery>>,
    reasoned_fallback_roots: indexmap::IndexSet<DomHandle>,
    exact_safety_fallback_roots: indexmap::IndexSet<DomHandle>,
    fallback_reasons: indexmap::IndexSet<StyloSourceInvalidationFallbackReason>,
}

#[derive(Default)]
struct RetainedSourceInputForTest {
    fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
    retained_fallback_kind: Option<Option<StyloRetainedSourceStyleInvalidationKind>>,
    zero_matched_dependency_result_is_exact: Option<bool>,
}

impl StyloPlannedFallbackRootInvalidationTargetPartsSink<DomHandle>
    for FallbackRootTargetRootsForTest
{
    fn set_planned_fallback_root_target_parts(
        &mut self,
        _fallback_kind: StyloRetainedSourceStyleInvalidationKind,
        fallback_roots: Vec<DomHandle>,
        _fallback_reasons: indexmap::IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        self.roots = fallback_roots;
    }
}

impl MoliInvalidationSourceResultsSink<DomHandle> for SourceResultsForTest {
    fn record_moli_invalidation_source_result_count(&mut self, _count: usize) {}

    fn record_moli_invalidation_source_result(
        &mut self,
        result: StyloSourceStyleInvalidationSourceResult,
    ) {
        result.drain_into(self);
    }
}

impl StyloSourceStyleInvalidationSourceResultSink<DomHandle> for SourceResultsForTest {
    fn retain_source_style_invalidation_target_result_diagnostics(&self) -> bool {
        false
    }

    fn record_source_style_invalidation_source_result(
        &mut self,
        parts: StyloSourceStyleInvalidationSourceResultParts<DomHandle>,
    ) {
        parts.drain_into(self);
    }
}

impl StyloSourceStyleInvalidationSourceResultPartsSink<DomHandle> for SourceResultsForTest {
    fn record_source_style_invalidation_source_result_parts(
        &mut self,
        _source_index: usize,
        affected_roots: StyloSourceAffectedRootsCleanup,
        target_result_record: StyloSourceStyleInvalidationTargetResultRecord,
    ) {
        affected_roots.drain_into(self);
        let diagnostic_facts = target_result_record.drain_cleanup_into(self);
        debug_assert!(
            diagnostic_facts.is_none(),
            "test source-result sink does not retain diagnostic target-result facts"
        );
    }
}

impl StyloSourceStyleInvalidationTargetResultCleanupFactsSink for SourceResultsForTest {
    fn set_source_style_invalidation_target_result_cleanup_facts(
        &mut self,
        facts: StyloSourceStyleInvalidationTargetResultCleanupFacts,
    ) {
        facts.drain_parts_into(self);
    }
}

impl StyloSourceStyleInvalidationTargetResultCleanupFactsPartsSink for SourceResultsForTest {
    fn set_source_style_invalidation_target_result_cleanup_fact_parts(
        &mut self,
        fallback_context_reasons: Vec<StyloSourceInvalidationFallbackReason>,
        _clear_all_cleanup_reasons: Vec<StyloSourceInvalidationFallbackReason>,
        _include_fallback_context_for_clear_all: bool,
        _requires_fallback_handling: bool,
    ) {
        self.has_fallback_reasons |= !fallback_context_reasons.is_empty();
    }
}

impl StyloSourceAffectedRootsCleanupSink<DomHandle> for SourceResultsForTest {
    fn extend_exact_affected_roots(&mut self, roots: &[DomHandle]) {
        self.roots.extend(roots.iter().copied());
    }

    fn extend_source_fallback_roots(&mut self, roots: &[DomHandle]) {
        self.roots.extend(roots.iter().copied());
    }
}

impl StyloPlannedSourceDependencyInvalidationPartsSink<DomHandle>
    for PlannedSourceDependencyInputForTest
{
    fn set_planned_source_dependency_source_index(&mut self, _source_index: usize) {}

    fn set_planned_source_dependency_structural_boundary_cleanup_roots(
        &mut self,
        _structural_boundary_cleanup_roots: Vec<DomHandle>,
    ) {
    }
}

impl StyloPlannedSourceDependencyInvalidationTargetPartsSink<DomHandle>
    for PlannedSourceDependencyInputForTest
{
    fn set_planned_retained_source_dependency_target_parts(
        &mut self,
        exact_queries: Vec<RetainedStyleInvalidationQuery>,
        fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
        reasoned_fallback_roots: Vec<DomHandle>,
        exact_safety_fallback_roots: Vec<DomHandle>,
        fallback_reasons: indexmap::IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        self.kind = Some(StyloRetainedSourceStyleInvalidationKind::RetainedQueries);
        self.fallback_kind = fallback_kind;
        self.queries = Some(exact_queries.into_iter().collect());
        self.reasoned_fallback_roots = reasoned_fallback_roots.into_iter().collect();
        self.exact_safety_fallback_roots = exact_safety_fallback_roots.into_iter().collect();
        self.fallback_reasons = fallback_reasons;
    }

    fn set_planned_fallback_source_dependency_target_parts(
        &mut self,
        fallback_kind: StyloRetainedSourceStyleInvalidationKind,
        fallback_roots: Vec<DomHandle>,
        fallback_reasons: indexmap::IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        self.kind = Some(fallback_kind);
        self.reasoned_fallback_roots = fallback_roots.into_iter().collect();
        self.fallback_reasons = fallback_reasons;
    }

    fn set_planned_missing_fallback_roots_source_dependency_target_parts(
        &mut self,
        fallback_reasons: indexmap::IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        self.kind = Some(StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots);
        self.fallback_reasons = fallback_reasons;
    }
}

impl<'a> StyloRetainedSourceStyleInvalidationSink<'a, DomHandle, MoliMutationSnapshot>
    for RetainedSourceInputForTest
{
    fn run_retained_source_style_invalidation_queries(
        &mut self,
        fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
        _cascade_data: Option<&'a ServoArc<CascadeData>>,
        _shadow_root: Option<DomHandle>,
        _queries: &'a indexmap::IndexSet<RetainedStyleInvalidationQuery>,
        zero_matched_dependency_result_is_exact: bool,
        _reasoned_fallback_roots: &'a indexmap::IndexSet<DomHandle>,
        _exact_safety_fallback_roots: &'a indexmap::IndexSet<DomHandle>,
        _fallback_reasons: &'a indexmap::IndexSet<StyloSourceInvalidationFallbackReason>,
        _mutation_snapshot: &'a MoliMutationSnapshot,
    ) {
        self.retained_fallback_kind = Some(fallback_kind);
        self.zero_matched_dependency_result_is_exact =
            Some(zero_matched_dependency_result_is_exact);
    }

    fn run_fallback_source_style_invalidation(
        &mut self,
        kind: StyloRetainedSourceStyleInvalidationKind,
        _fallback_roots: &'a indexmap::IndexSet<DomHandle>,
        _fallback_reasons: &'a indexmap::IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        self.fallback_kind = Some(kind);
    }
}

fn fallback_root_target_roots_for_test(
    target: StyloPlannedFallbackRootInvalidationTarget,
) -> Vec<DomHandle> {
    let mut sink = FallbackRootTargetRootsForTest::default();
    target.drain_into(&mut sink);
    sink.roots
}

fn planned_source_dependency_input_for_test(
    planned: StyloPlannedSourceDependencyInvalidation,
) -> PlannedSourceDependencyInputForTest {
    let mut sink = PlannedSourceDependencyInputForTest::default();
    planned.drain_into(&mut sink);
    sink
}

fn retained_source_input_for_test(
    input: StyloRetainedSourceStyleInvalidation<'_>,
) -> RetainedSourceInputForTest {
    let mut sink = RetainedSourceInputForTest::default();
    input.drain_into(&mut sink);
    sink
}

fn source_scope_fallback_roots_for_test(
    host: &DomHost,
    source_scope: &StyleSourceScope,
) -> Vec<DomHandle> {
    let target = stylo_source_scope_fallback_plan(
        host,
        source_scope,
        [StyloSourceInvalidationFallbackReason::SourceScopeFallback],
    );
    fallback_root_target_roots_for_test(target)
}

fn shadow_root_source_scope_fallback_roots_for_test(
    host: &DomHost,
    root: DomHandle,
    source_scope: &StyleSourceScope,
) -> Vec<DomHandle> {
    stylo_stylesheet_source_scope_fallback_roots(
        host,
        StyloStylesheetSourceScopeFallbackInput::ShadowRootAdopted { root },
        source_scope,
    )
}

fn cleanup_effects_for_test(
    clear_shadow_cascade_data_for_cleanup_target: bool,
) -> StyleInvalidationCleanupEffects {
    if clear_shadow_cascade_data_for_cleanup_target {
        StyleInvalidationCleanupEffects::clear_shadow_cascade_data_for_cleanup_target()
    } else {
        StyleInvalidationCleanupEffects::default()
    }
}
fn lifecycle_availability_for(
    records: &[StyleSourceLifecycleRecord],
    owner: StyleSourceLifecycleOwner,
) -> Option<&StyleSourceLifecycleAvailability> {
    records
        .iter()
        .find(|record| record.owner() == owner)
        .map(StyleSourceLifecycleRecord::availability)
}
fn lifecycle_document_kind_for(
    records: &[StyleSourceLifecycleRecord],
    owner: StyleSourceLifecycleOwner,
) -> Option<StyleSourceDocumentKind> {
    records
        .iter()
        .find(|record| record.owner() == owner)
        .and_then(StyleSourceLifecycleRecord::document_kind)
}
fn lifecycle_owner_detail_for(
    details: &[StyleSourceLifecycleOwnerDetailTrace],
    owner: StyleSourceLifecycleOwner,
) -> Option<&StyleSourceLifecycleOwnerDetailTrace> {
    details.iter().find(|detail| detail.owner() == owner)
}
fn source_lifecycle_report_for_document_for_test(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document: DomHandle,
    document_context: StyleSourceDocumentContext<'_>,
) -> StyleSourceLifecycleReport {
    let world = engine.world_for_document(document);
    let source_stores = world.borrow_source_stores();
    source_stores.source_lifecycle_report(host, document_context)
}
fn retained_source_invalidation_outcome_for_document_for_test(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document: DomHandle,
    document_context: StyleSourceDocumentContext<'_>,
    retained: Option<&super::state::RetainedStyleSystem>,
    target_queries: &[PendingStyleInvalidationTargetQueries],
    clear_shadow_cascade_data_for_cleanup_target: bool,
) -> StyleInvalidationOutcome {
    let source_lifecycle =
        source_lifecycle_report_for_document_for_test(engine, host, document, document_context);
    retained_source_invalidation_outcome(
        &engine.dom_adapter,
        host,
        Some(&source_lifecycle),
        retained,
        target_queries,
        cleanup_effects_for_test(clear_shadow_cascade_data_for_cleanup_target),
    )
}
fn retained_style_invalidation_query_for_test(
    root: DomHandle,
    query: StyloStyleInvalidationQuery<'_>,
) -> RetainedStyleInvalidationQuery {
    match query {
        StyloStyleInvalidationQuery::Universal => RetainedStyleInvalidationQuery::universal(root),
        StyloStyleInvalidationQuery::Type(local_name) => {
            RetainedStyleInvalidationQuery::element_type(root, local_name.to_owned())
        }
        StyloStyleInvalidationQuery::Attribute(name) => {
            RetainedStyleInvalidationQuery::attribute(root, name.to_owned())
        }
        StyloStyleInvalidationQuery::Class(token) => {
            RetainedStyleInvalidationQuery::class(root, token.to_owned())
        }
        StyloStyleInvalidationQuery::Id(value) => {
            RetainedStyleInvalidationQuery::id(root, value.to_owned())
        }
        StyloStyleInvalidationQuery::State(state) => {
            RetainedStyleInvalidationQuery::state(root, state)
        }
        StyloStyleInvalidationQuery::CustomState(name) => {
            RetainedStyleInvalidationQuery::custom_state(root, name.to_owned())
        }
    }
}

#[derive(Default)]
struct SourceDependencyBatchPlanForTest {
    work_sources: Vec<StyloPlannedSourceDependencyInvalidation>,
    requires_source_fallback: bool,
}

impl StyloSourceDependencyInvalidationBatchPlanSink<DomHandle>
    for SourceDependencyBatchPlanForTest
{
    fn set_source_dependency_batch_work(
        &mut self,
        sources: Vec<StyloPlannedSourceDependencyInvalidation>,
        _boundary_fallback: Option<StyloPlannedFallbackRootInvalidationTarget>,
    ) {
        self.work_sources = sources;
    }

    fn set_source_dependency_batch_requires_source_fallback(
        &mut self,
        _source: StyloPlannedSourceDependencyInvalidation,
    ) {
        self.requires_source_fallback = true;
    }
}

fn source_dependency_batch_plan_for_test(
    plan: StyloSourceDependencyInvalidationBatchPlan,
) -> SourceDependencyBatchPlanForTest {
    let mut sink = SourceDependencyBatchPlanForTest::default();
    plan.drain_into(&mut sink);
    sink
}

fn collect_source_invalidation_roots_for_test(
    engine: &MoliStyleEngine,
    host: &DomHost,
    root: DomHandle,
    query: StyloStyleInvalidationQuery<'_>,
) -> (Vec<DomHandle>, bool) {
    let mut roots = Vec::new();
    let mut requires_fallback = false;
    engine.with_retained_style_system_for_document_for_test(host.document_handle(), |retained| {
        engine.dom_adapter.with_bound_host(host, |adapter| {
            let retained_query = retained_style_invalidation_query_for_test(root, query);
            for cascade_data in retained.source_cascade_data.values() {
                let summary = StyloSourceDependencySummary::from_cascade_data(cascade_data);
                let requests = [StyloSourceDependencyInvalidationRequest::new(
                    &retained_query,
                    None,
                    StyloSourceDependencyRequestRequirement::exact(),
                )];
                let sources = [StyloSourceDependencyInvalidationBatchSource::new(
                    &summary,
                    &[],
                    &[],
                )];
                let plan = stylo_source_dependency_invalidation_batch_plan(
                    host,
                    &sources,
                    &requests,
                    StyloSourceDependencyBoundaryRoots::default(),
                );
                let plan = source_dependency_batch_plan_for_test(plan);
                if plan.requires_source_fallback {
                    requires_fallback = true;
                    continue;
                }
                for planned in plan.work_sources {
                    let planned = planned_source_dependency_input_for_test(planned);
                    let mutation_snapshot = Default::default();
                    let input = match planned
                        .kind
                        .expect("planned source test input should carry kind")
                    {
                        StyloRetainedSourceStyleInvalidationKind::RetainedQueries => {
                            stylo_retained_source_style_invalidation_from_parts(
                                StyloRetainedSourceStyleInvalidationKind::RetainedQueries,
                                planned.fallback_kind,
                                Some(cascade_data),
                                None,
                                Some(
                                    planned
                                        .queries
                                        .as_ref()
                                        .expect("retained source test input should carry queries"),
                                ),
                                false,
                                &planned.reasoned_fallback_roots,
                                &planned.exact_safety_fallback_roots,
                                &planned.fallback_reasons,
                                &mutation_snapshot,
                            )
                        }
                        StyloRetainedSourceStyleInvalidationKind::FallbackOnly
                        | StyloRetainedSourceStyleInvalidationKind::ContextFallback
                        | StyloRetainedSourceStyleInvalidationKind::SourceScopeFallback
                        | StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots => {
                            stylo_retained_source_style_invalidation_from_parts(
                                planned
                                    .kind
                                    .expect("planned source test input should carry kind"),
                                None,
                                None,
                                None,
                                None,
                                false,
                                &planned.reasoned_fallback_roots,
                                &planned.exact_safety_fallback_roots,
                                &planned.fallback_reasons,
                                &mutation_snapshot,
                            )
                        }
                    };
                    let result = adapter.collect_retained_source_style_invalidation_result(
                        host,
                        Some(&retained.stylist),
                        retained.shadow_cascade_data.as_slice(),
                        std::iter::once(input),
                    );
                    let mut source_results = SourceResultsForTest::default();
                    result.drain_source_results_into(&mut source_results);
                    if source_results.has_fallback_reasons {
                        requires_fallback = true;
                    }
                    roots.extend(source_results.roots);
                }
            }
        });
    });
    (roots, requires_fallback)
}

mod char_child;
mod dependency;
mod invalidator;
mod lifecycle;
mod outcome;
mod registered_properties;
mod slotted_focus;
mod subtree_cache;
mod subtree_context;
