use super::*;

struct TestStyloSourceResult {
    source_index: usize,
    kind: StyloSourceStyleInvalidationSourceResultKind,
    exact: bool,
    empty_result_is_exact: bool,
    matched_dependency_count: usize,
    fallback_reasons: Vec<StyloSourceInvalidationFallbackReason>,
    fallback_root_availability: Option<StyleSourceFallbackRootAvailability>,
    affected_roots: Vec<crate::document_runtime::DomHandle>,
}

impl TestStyloSourceResult {
    fn push_into(self, builder: &mut MoliInvalidationResultBuilder) {
        if self.exact {
            assert_eq!(
                self.kind,
                StyloSourceStyleInvalidationSourceResultKind::Exact
            );
            assert!(self.fallback_reasons.is_empty());
            builder.push_exact_source_result(
                self.source_index,
                self.affected_roots,
                self.empty_result_is_exact,
                self.matched_dependency_count,
            );
            return;
        }
        builder.push_fallback_source_result(
            self.source_index,
            self.kind,
            self.empty_result_is_exact,
            self.matched_dependency_count,
            self.fallback_reasons,
            self.fallback_root_availability,
            self.affected_roots,
        );
    }
}

macro_rules! stylo_source_result {
    (
        source_index: $source_index:expr,
        kind: $kind:expr,
        exact: $exact:expr,
        empty_result_is_exact: $empty_result_is_exact:expr,
        matched_dependency_count: $matched_dependency_count:expr,
        fallback_reasons: $fallback_reasons:expr,
        fallback_root_availability: $fallback_root_availability:expr,
        affected_roots: $affected_roots:expr $(,)?
    ) => {{
        TestStyloSourceResult {
            source_index: $source_index,
            kind: $kind,
            exact: $exact,
            empty_result_is_exact: $empty_result_is_exact,
            matched_dependency_count: $matched_dependency_count,
            fallback_reasons: $fallback_reasons,
            fallback_root_availability: $fallback_root_availability,
            affected_roots: $affected_roots,
        }
    }};
}

macro_rules! moli_invalidation_result {
    (source_results: $source_results:expr $(,)?) => {{
        let mut builder = MoliInvalidationResultBuilder::new();
        for source_result in $source_results {
            source_result.push_into(&mut builder);
        }
        builder.finish()
    }};
}

#[test]
fn document_stylesheet_dirty_mark_advances_only_the_source_generation() {
    let mut engine = MoliStyleEngine::new();
    let host = test_host();
    let document = host.document_handle();

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        0
    );
    engine.mark_document_stylesheet_set_dirty(document);

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.source_set_generation_for_document_for_test(document),
        1
    );
}
#[test]
fn pending_batch_needs_source_lifecycle_report_only_for_stylesheet_targets() {
    let host = test_host();
    let root = host.document_handle();
    let fallback_target =
        StyleInvalidationSourceTarget::fallback_root(&host, root).expect("document target");
    let fallback_query =
        PendingStyleInvalidationTargetQueries::fallback_roots_for_test(fallback_target, [root]);
    let fallback_batch = PendingStyleInvalidationBatch {
        work_items: vec![PendingStyleInvalidationWork::for_test(
            PendingStyleInvalidationWorkKind::Mutation,
            vec![fallback_query],
        )],
    };

    assert!(fallback_batch.stylesheet_source_ids().is_empty());

    let stylesheet_source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let stylesheet_query = PendingStyleInvalidationTargetQueries::retained_source(
        stylesheet_source_id.clone(),
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(root)]),
    );
    let stylesheet_batch = PendingStyleInvalidationBatch {
        work_items: vec![PendingStyleInvalidationWork::for_test(
            PendingStyleInvalidationWorkKind::Mutation,
            vec![stylesheet_query],
        )],
    };

    assert_eq!(
        stylesheet_batch.stylesheet_source_ids(),
        indexmap::IndexSet::from([stylesheet_source_id])
    );
}
#[test]
fn retained_target_result_preserves_fallback_reason() {
    let host = test_host();
    let root = host.document_handle();
    let target =
        StyleInvalidationSourceTarget::fallback_root(&host, root).expect("document target");
    let target_query =
        PendingStyleInvalidationTargetQueries::fallback_roots_for_test(target, [root]);
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Fallback,
                exact: false,
                empty_result_is_exact: false,
                matched_dependency_count: 1,
                fallback_reasons: vec![StyloSourceInvalidationFallbackReason::InexactEmptyResult],
                fallback_root_availability: Some(StyleSourceFallbackRootAvailability::Available {
                    root_count: 1,
                }),
                affected_roots: vec![root],
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(application.clear_all_reasons(), None);
    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::SourceFallbackSubtreeRoots
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::UnknownOrFallback
    );
    assert_eq!(application.diagnostic_target_results().len(), 1);
    let target_result = &application.diagnostic_target_results()[0];
    assert_eq!(target_result.target(), target_query.target());
    assert_eq!(
        target_result.kind(),
        StyleInvalidationDiagnosticTargetResultKind::Fallback
    );
    assert!(
        target_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::InexactEmptyResult)
    );
    assert_eq!(target_result.retained_query_count(), 0);
    assert_eq!(target_result.mutation_snapshot_count(), 0);
    assert_eq!(target_result.structural_boundary_cleanup_root_count(), 0);
    assert_eq!(target_result.affected_root_count(), 1);
    assert!(!target_result.exact());
    assert!(!target_result.empty_result_is_exact());
    assert_eq!(target_result.matched_dependency_count(), 1);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&root)
    ));
}

#[test]
fn source_fallback_roots_remain_compact_until_cache_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let fallback_root = host.create_element("article");
    let child = host.create_element("p");
    let grandchild = host.create_element("span");
    let sibling = host.create_element("aside");
    let shadow_root = host
        .attach_shadow_root(fallback_root, "open")
        .expect("article should host a shadow root");
    let shadow_child = host.create_element("em");

    assert!(host.append_child(document, fallback_root));
    assert!(host.append_child(document, sibling));
    assert!(host.append_child(fallback_root, child));
    assert!(host.append_child(child, grandchild));
    assert!(host.append_child(shadow_root, shadow_child));

    let target =
        StyleInvalidationSourceTarget::fallback_root(&host, fallback_root).expect("element target");
    let target_query = PendingStyleInvalidationTargetQueries::fallback_roots_for_test(
        target,
        indexmap::IndexSet::from([fallback_root]),
    );
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Fallback,
                exact: false,
                empty_result_is_exact: false,
                matched_dependency_count: 1,
                fallback_reasons: vec![StyloSourceInvalidationFallbackReason::FullSelector],
                fallback_root_availability: Some(StyleSourceFallbackRootAvailability::Available {
                    root_count: 1,
                }),
                affected_roots: vec![fallback_root],
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::SourceFallbackSubtreeRoots
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::UnknownOrFallback
    );
    assert_eq!(application.affected_root_count(), 1);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&fallback_root)
                && !roots.contains(&child)
                && !roots.contains(&grandchild)
                && !roots.contains(&shadow_root)
                && !roots.contains(&shadow_child)
                && !roots.contains(&sibling)
    ));
}

#[test]
fn exact_affected_roots_remain_compact_until_cache_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let exact_root = host.create_element("section");
    let child = host.create_element("div");
    let grandchild = host.create_element("span");
    let sibling = host.create_element("aside");
    let shadow_root = host
        .attach_shadow_root(exact_root, "open")
        .expect("section should host a shadow root");
    let shadow_child = host.create_element("em");

    assert!(host.append_child(document, exact_root));
    assert!(host.append_child(document, sibling));
    assert!(host.append_child(exact_root, child));
    assert!(host.append_child(child, grandchild));
    assert!(host.append_child(shadow_root, shadow_child));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let target_query = PendingStyleInvalidationTargetQueries::retained_source(
        source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(exact_root)]),
    );
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                exact: true,
                empty_result_is_exact: true,
                matched_dependency_count: 1,
                fallback_reasons: Vec::new(),
                fallback_root_availability: None,
                affected_roots: vec![exact_root],
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::ExactAffectedSubtreeRoots
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::DescendantInherited
    );
    assert_eq!(application.affected_root_count(), 1);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::ExactAffectedSubtreeRoots(roots)
            if roots.contains(&exact_root)
                && !roots.contains(&child)
                && !roots.contains(&grandchild)
                && !roots.contains(&shadow_root)
                && !roots.contains(&shadow_child)
                && !roots.contains(&sibling)
    ));
}

#[test]
fn retained_target_result_applies_structural_boundary_cleanup_roots_on_match() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let mut target_query = PendingStyleInvalidationTargetQueries::retained_source(
        source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(root)]),
    );
    target_query.extend_structural_boundary_cleanup_roots([root]);
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                exact: true,
                empty_result_is_exact: true,
                matched_dependency_count: 1,
                fallback_reasons: Vec::new(),
                fallback_root_availability: None,
                affected_roots: Vec::new(),
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::StructuralBoundarySubtreeRoots
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::DescendantInherited
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::StructuralBoundarySubtreeRoots(roots)
            if roots.contains(&root)
    ));
    assert_eq!(
        application.diagnostic_target_results()[0].structural_boundary_cleanup_root_count(),
        1
    );
}

#[test]
fn structural_boundary_roots_remain_compact_until_cache_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let structural_root = host.create_element("nav");
    let child = host.create_element("a");
    let grandchild = host.create_element("span");
    let sibling = host.create_element("main");
    let shadow_root = host
        .attach_shadow_root(structural_root, "open")
        .expect("nav should host a shadow root");
    let shadow_child = host.create_element("strong");

    assert!(host.append_child(document, structural_root));
    assert!(host.append_child(document, sibling));
    assert!(host.append_child(structural_root, child));
    assert!(host.append_child(child, grandchild));
    assert!(host.append_child(shadow_root, shadow_child));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let mut target_query = PendingStyleInvalidationTargetQueries::retained_source(
        source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(structural_root)]),
    );
    target_query.extend_structural_boundary_cleanup_roots([structural_root]);
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                exact: true,
                empty_result_is_exact: true,
                matched_dependency_count: 1,
                fallback_reasons: Vec::new(),
                fallback_root_availability: None,
                affected_roots: Vec::new(),
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::StructuralBoundarySubtreeRoots
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::DescendantInherited
    );
    assert_eq!(application.affected_root_count(), 1);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::StructuralBoundarySubtreeRoots(roots)
            if roots.contains(&structural_root)
                && !roots.contains(&child)
                && !roots.contains(&grandchild)
                && !roots.contains(&shadow_root)
                && !roots.contains(&shadow_child)
                && !roots.contains(&sibling)
    ));
    assert_eq!(
        application.diagnostic_target_results()[0].structural_boundary_cleanup_root_count(),
        1
    );
}
#[test]
fn retained_target_result_applies_structural_boundary_cleanup_roots_without_match() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let mut target_query = PendingStyleInvalidationTargetQueries::retained_source(
        source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(root)]),
    );
    target_query.extend_structural_boundary_cleanup_roots([root]);
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                exact: true,
                empty_result_is_exact: true,
                matched_dependency_count: 0,
                fallback_reasons: Vec::new(),
                fallback_root_availability: None,
                affected_roots: Vec::new(),
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::StructuralBoundarySubtreeRoots
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::StructuralBoundarySubtreeRoots(roots)
            if roots.contains(&root)
    ));
    assert_eq!(
        application.diagnostic_target_results()[0].structural_boundary_cleanup_root_count(),
        1
    );
}

#[test]
fn structural_boundary_cleanup_does_not_require_scoped_fallback_trace() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let mut target_query = PendingStyleInvalidationTargetQueries::retained_source(
        source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(root)]),
    );
    target_query.extend_structural_boundary_cleanup_roots([root]);
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                exact: true,
                empty_result_is_exact: true,
                matched_dependency_count: 1,
                fallback_reasons: Vec::new(),
                fallback_root_availability: None,
                affected_roots: Vec::new(),
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let finalized_result = outcome.finalize(&host);

    assert_eq!(
        finalized_result.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::StructuralBoundarySubtreeRoots
    );
    assert!(!finalized_result.requires_scoped_fallback_trace_for_test());
}

#[test]
fn exact_and_structural_mixed_cleanup_does_not_require_scoped_fallback_trace() {
    let mut host = test_host();
    let document = host.document_handle();
    let exact_root = host.create_element("section");
    let structural_root = host.create_element("nav");
    assert!(host.append_child(document, exact_root));
    assert!(host.append_child(document, structural_root));

    let exact_source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let structural_source_id = StyleSourceId::document_adopted_style_sheet(document, 1);
    let mut structural_query = PendingStyleInvalidationTargetQueries::retained_source(
        structural_source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(structural_root)]),
    );
    structural_query.extend_structural_boundary_cleanup_roots([structural_root]);
    let target_queries = vec![
        PendingStyleInvalidationTargetQueries::retained_source(
            exact_source_id,
            indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(exact_root)]),
        ),
        structural_query,
    ];
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [
                stylo_source_result! {
                    source_index: 0,
                    kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                    exact: true,
                    empty_result_is_exact: true,
                    matched_dependency_count: 1,
                    fallback_reasons: Vec::new(),
                    fallback_root_availability: None,
                    affected_roots: vec![exact_root],
                },
                stylo_source_result! {
                    source_index: 1,
                    kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                    exact: true,
                    empty_result_is_exact: true,
                    matched_dependency_count: 1,
                    fallback_reasons: Vec::new(),
                    fallback_root_availability: None,
                    affected_roots: Vec::new(),
                },
            ],
        },
        &target_queries,
        StyleInvalidationCleanupEffects::default(),
    );

    let finalized_result = outcome.finalize(&host);

    assert_eq!(
        finalized_result.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::MixedSubtreeRoots
    );
    assert_eq!(
        finalized_result.cleanup_class(),
        StyleInvalidationCleanupClass::DescendantInherited
    );
    assert!(!finalized_result.requires_scoped_fallback_trace_for_test());
}

#[test]
fn finalized_cleanup_preserves_mixed_subtree_sources() {
    let mut host = test_host();
    let document = host.document_handle();
    let exact_root = host.create_element("section");
    let exact_child = host.create_element("span");
    let fallback_root = host.create_element("article");
    let fallback_child = host.create_element("p");
    let structural_root = host.create_element("nav");
    let structural_child = host.create_element("a");
    let sibling = host.create_element("aside");
    let exact_shadow_root = host
        .attach_shadow_root(exact_root, "open")
        .expect("section should host a shadow root");
    let exact_shadow_child = host.create_element("em");
    let fallback_shadow_root = host
        .attach_shadow_root(fallback_root, "open")
        .expect("article should host a shadow root");
    let fallback_shadow_child = host.create_element("strong");
    let structural_shadow_root = host
        .attach_shadow_root(structural_root, "open")
        .expect("nav should host a shadow root");
    let structural_shadow_child = host.create_element("small");
    assert!(host.append_child(document, exact_root));
    assert!(host.append_child(document, fallback_root));
    assert!(host.append_child(document, structural_root));
    assert!(host.append_child(document, sibling));
    assert!(host.append_child(exact_root, exact_child));
    assert!(host.append_child(fallback_root, fallback_child));
    assert!(host.append_child(structural_root, structural_child));
    assert!(host.append_child(exact_shadow_root, exact_shadow_child));
    assert!(host.append_child(fallback_shadow_root, fallback_shadow_child));
    assert!(host.append_child(structural_shadow_root, structural_shadow_child));

    let exact_source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let fallback_source_id = StyleSourceId::document_adopted_style_sheet(document, 1);
    let structural_source_id = StyleSourceId::document_adopted_style_sheet(document, 2);
    let mut structural_query = PendingStyleInvalidationTargetQueries::retained_source(
        structural_source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(structural_root)]),
    );
    structural_query.extend_structural_boundary_cleanup_roots([structural_root]);
    let target_queries = vec![
        PendingStyleInvalidationTargetQueries::retained_source(
            exact_source_id,
            indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(exact_root)]),
        ),
        PendingStyleInvalidationTargetQueries::retained_source(
            fallback_source_id,
            indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(fallback_root)]),
        ),
        structural_query,
    ];
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [
                stylo_source_result! {
                    source_index: 0,
                    kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                    exact: true,
                    empty_result_is_exact: true,
                    matched_dependency_count: 1,
                    fallback_reasons: Vec::new(),
                    fallback_root_availability: None,
                    affected_roots: vec![exact_root],
                },
                stylo_source_result! {
                    source_index: 1,
                    kind: StyloSourceStyleInvalidationSourceResultKind::Fallback,
                    exact: false,
                    empty_result_is_exact: false,
                    matched_dependency_count: 1,
                    fallback_reasons: vec![
                        StyloSourceInvalidationFallbackReason::UnsupportedDependency,
                    ],
                    fallback_root_availability: Some(
                        StyleSourceFallbackRootAvailability::Available { root_count: 1 },
                    ),
                    affected_roots: vec![fallback_root],
                },
                stylo_source_result! {
                    source_index: 2,
                    kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                    exact: true,
                    empty_result_is_exact: true,
                    matched_dependency_count: 0,
                    fallback_reasons: Vec::new(),
                    fallback_root_availability: None,
                    affected_roots: Vec::new(),
                },
            ],
        },
        &target_queries,
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::MixedSubtreeRoots
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::UnknownOrFallback
    );
    assert_eq!(application.affected_root_count(), 3);
    let telemetry = application.fallback_telemetry();
    assert_eq!(telemetry.exact_target_result_count(), 2);
    assert_eq!(telemetry.inexact_target_result_count(), 1);
    assert_eq!(telemetry.fallback_target_result_count(), 1);
    assert_eq!(telemetry.source_result_affected_root_count(), 2);
    assert_eq!(
        telemetry.fallback_reason_target_count(
            StyloSourceInvalidationFallbackReason::UnsupportedDependency,
        ),
        1
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::MixedSubtreeRoots(groups)
            if groups.exact_affected_roots().contains(&exact_root)
                && !groups.exact_affected_roots().contains(&exact_child)
                && !groups.exact_affected_roots().contains(&exact_shadow_root)
                && !groups.exact_affected_roots().contains(&exact_shadow_child)
                && groups.source_fallback_roots().contains(&fallback_root)
                && !groups.source_fallback_roots().contains(&fallback_child)
                && !groups.source_fallback_roots().contains(&fallback_shadow_root)
                && !groups.source_fallback_roots().contains(&fallback_shadow_child)
                && groups.structural_boundary_roots().contains(&structural_root)
                && !groups.structural_boundary_roots().contains(&structural_child)
                && !groups.structural_boundary_roots().contains(&structural_shadow_root)
                && !groups.structural_boundary_roots().contains(&structural_shadow_child)
                && !groups.all_roots().contains(&sibling)
    ));
}

#[test]
fn mixed_cleanup_clears_shadow_cascade_data_only_for_source_fallback_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let exact_root = host.create_element("section");
    let fallback_root = host.create_element("article");
    let structural_root = host.create_element("nav");
    let exact_shadow_root = host
        .attach_shadow_root(exact_root, "open")
        .expect("section should host a shadow root");
    let fallback_shadow_root = host
        .attach_shadow_root(fallback_root, "open")
        .expect("article should host a shadow root");
    let structural_shadow_root = host
        .attach_shadow_root(structural_root, "open")
        .expect("nav should host a shadow root");
    assert!(host.append_child(document, exact_root));
    assert!(host.append_child(document, fallback_root));
    assert!(host.append_child(document, structural_root));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.shadow_stylesheet_sources.push((
        exact_shadow_root,
        vec![StyloStylesheetSource::new(
            ":host { color: rgb(1, 2, 3); }".into(),
            document_url.clone(),
        )],
    ));
    inputs.shadow_stylesheet_sources.push((
        fallback_shadow_root,
        vec![StyloStylesheetSource::new(
            ":host { color: rgb(4, 5, 6); }".into(),
            document_url.clone(),
        )],
    ));
    inputs.shadow_stylesheet_sources.push((
        structural_shadow_root,
        vec![StyloStylesheetSource::new(
            ":host { color: rgb(7, 8, 9); }".into(),
            document_url.clone(),
        )],
    ));
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);

    let (exact_cascade_data, fallback_cascade_data, structural_cascade_data) = engine
        .with_retained_style_system_for_document_for_test(document, |retained| {
            let cascade_data_for_root = |root| {
                retained
                    .shadow_cascade_data
                    .iter()
                    .find(|(shadow_root, _)| *shadow_root == root)
                    .expect("retained system should track shadow root")
                    .1
                    .clone()
            };
            (
                cascade_data_for_root(exact_shadow_root),
                cascade_data_for_root(fallback_shadow_root),
                cascade_data_for_root(structural_shadow_root),
            )
        });

    let exact_source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let fallback_source_id = StyleSourceId::document_adopted_style_sheet(document, 1);
    let structural_source_id = StyleSourceId::document_adopted_style_sheet(document, 2);
    let mut structural_query = PendingStyleInvalidationTargetQueries::retained_source(
        structural_source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(structural_root)]),
    );
    structural_query.extend_structural_boundary_cleanup_roots([structural_root]);
    let target_queries = vec![
        PendingStyleInvalidationTargetQueries::retained_source(
            exact_source_id,
            indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(exact_root)]),
        ),
        PendingStyleInvalidationTargetQueries::retained_source(
            fallback_source_id,
            indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(fallback_root)]),
        ),
        structural_query,
    ];
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [
                stylo_source_result! {
                    source_index: 0,
                    kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                    exact: true,
                    empty_result_is_exact: true,
                    matched_dependency_count: 1,
                    fallback_reasons: Vec::new(),
                    fallback_root_availability: None,
                    affected_roots: vec![exact_root],
                },
                stylo_source_result! {
                    source_index: 1,
                    kind: StyloSourceStyleInvalidationSourceResultKind::Fallback,
                    exact: false,
                    empty_result_is_exact: false,
                    matched_dependency_count: 1,
                    fallback_reasons: vec![
                        StyloSourceInvalidationFallbackReason::UnsupportedDependency,
                    ],
                    fallback_root_availability: Some(
                        StyleSourceFallbackRootAvailability::Available { root_count: 1 },
                    ),
                    affected_roots: vec![fallback_root],
                },
                stylo_source_result! {
                    source_index: 2,
                    kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                    exact: true,
                    empty_result_is_exact: true,
                    matched_dependency_count: 0,
                    fallback_reasons: Vec::new(),
                    fallback_root_availability: None,
                    affected_roots: Vec::new(),
                },
            ],
        },
        &target_queries,
        StyleInvalidationCleanupEffects::clear_shadow_cascade_data_for_cleanup_target(),
    );
    let finalized_result = outcome.finalize(&host);
    assert_eq!(
        finalized_result.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::MixedSubtreeRoots
    );

    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            exact_shadow_root,
            exact_cascade_data,
        );
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            fallback_shadow_root,
            fallback_cascade_data,
        );
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            structural_shadow_root,
            structural_cascade_data,
        );

    let world = engine.world_for_document(document);
    assert!(
        engine
            .cache_cleanup_for_world(&world)
            .apply_finalized_result(&host, finalized_result)
    );

    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(exact_shadow_root)
    );
    assert!(
        !engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(fallback_shadow_root)
    );
    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(structural_shadow_root)
    );
}

#[test]
fn style_invalidation_outcome_maps_source_result_to_target_result() {
    let host = test_host();
    let root = host.document_handle();
    let target =
        StyleInvalidationSourceTarget::fallback_root(&host, root).expect("document target");
    let target_query =
        PendingStyleInvalidationTargetQueries::fallback_roots_for_test(target, [root]);
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Fallback,
                exact: false,
                empty_result_is_exact: false,
                matched_dependency_count: 1,
                fallback_reasons: vec![StyloSourceInvalidationFallbackReason::InexactEmptyResult],
                fallback_root_availability: Some(StyleSourceFallbackRootAvailability::Available {
                    root_count: 1,
                }),
                affected_roots: vec![root],
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert_eq!(
        application.diagnostic_target_results()[0].target(),
        target_query.target()
    );
    assert!(
        application.diagnostic_target_results()[0]
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::InexactEmptyResult)
    );
}

#[test]
fn style_invalidation_default_outcome_finalizes_to_noop() {
    let host = test_host();
    let application = StyleInvalidationOutcome::default().finalize(&host);

    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::Noop
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::Noop
    );
    assert_eq!(application.affected_root_count(), 0);
    assert!(application.diagnostic_target_results().is_empty());
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::Noop
    ));
}

#[test]
fn exact_empty_retained_result_finalizes_to_noop_without_cleanup_roots() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let target_query = PendingStyleInvalidationTargetQueries::retained_source(
        source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(root)]),
    );
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Exact,
                exact: true,
                empty_result_is_exact: true,
                matched_dependency_count: 0,
                fallback_reasons: Vec::new(),
                fallback_root_availability: None,
                affected_roots: Vec::new(),
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::Noop
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::Noop
    );
    assert_eq!(application.affected_root_count(), 0);
    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::Noop
    ));
}

#[test]
fn fallback_root_clear_all_reasons_scope_to_fallback_root_target() {
    let host = test_host();
    let root = host.document_handle();
    let target =
        StyleInvalidationSourceTarget::fallback_root(&host, root).expect("document target");
    let target_query = PendingStyleInvalidationTargetQueries::fallback_roots_for_test(target, []);
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Fallback,
                exact: false,
                empty_result_is_exact: false,
                matched_dependency_count: 0,
                fallback_reasons: vec![StyloSourceInvalidationFallbackReason::MissingFallbackRoots],
                fallback_root_availability: Some(StyleSourceFallbackRootAvailability::Missing),
                affected_roots: Vec::new(),
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(application.clear_all_reasons(), None);
    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::SourceFallbackSubtreeRoots
    );
    assert!(application.affected_root_count() >= 1);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&root)
    ));
}
#[test]
fn style_invalidation_outcome_scopes_missing_roots_to_source_fallback_even_with_other_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let detached_document = host.create_detached_html_document();
    let document_target =
        StyleInvalidationSourceTarget::fallback_root(&host, document).expect("document target");
    let missing_source_id = StyleSourceId::document_adopted_style_sheet(detached_document, 0);
    let target_queries = vec![
        PendingStyleInvalidationTargetQueries::fallback_roots_for_test(document_target, [document]),
        PendingStyleInvalidationTargetQueries::source_fallback_missing_roots_for_test(
            missing_source_id,
            Some(StyloSourceInvalidationFallbackReason::MissingFallbackRoots),
        ),
    ];
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [
                stylo_source_result! {
                    source_index: 0,
                    kind: StyloSourceStyleInvalidationSourceResultKind::FallbackOnly,
                    exact: false,
                    empty_result_is_exact: false,
                    matched_dependency_count: 0,
                    fallback_reasons: vec![
                        StyloSourceInvalidationFallbackReason::UnsupportedDependency,
                    ],
                    fallback_root_availability: Some(
                        StyleSourceFallbackRootAvailability::Available { root_count: 1 },
                    ),
                    affected_roots: vec![document],
                },
                stylo_source_result! {
                    source_index: 1,
                    kind: StyloSourceStyleInvalidationSourceResultKind::MissingFallbackRoots,
                    exact: false,
                    empty_result_is_exact: false,
                    matched_dependency_count: 0,
                    fallback_reasons: vec![
                        StyloSourceInvalidationFallbackReason::MissingFallbackRoots,
                    ],
                    fallback_root_availability: Some(StyleSourceFallbackRootAvailability::Missing),
                    affected_roots: Vec::new(),
                },
            ],
        },
        &target_queries,
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(application.clear_all_reasons(), None);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&document) && roots.contains(&detached_document)
    ));
    assert!(application.affected_root_count() >= 2);
    assert_eq!(application.diagnostic_target_results().len(), 2);
    assert!(
        application
            .diagnostic_target_results()
            .iter()
            .any(|result| {
                result
                    .fallback_reasons()
                    .contains(&StyloSourceInvalidationFallbackReason::UnsupportedDependency)
                    && result.kind() == StyleInvalidationDiagnosticTargetResultKind::FallbackOnly
            })
    );
    assert!(
        application
            .diagnostic_target_results()
            .iter()
            .any(|result| {
                result
                    .fallback_reasons()
                    .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
                    && result.kind()
                        == StyleInvalidationDiagnosticTargetResultKind::MissingFallbackRoots
            })
    );
}
#[test]
fn style_invalidation_outcome_scopes_rootless_source_fallback_even_with_other_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let detached_document = host.create_detached_html_document();
    let document_target =
        StyleInvalidationSourceTarget::fallback_root(&host, document).expect("document target");
    let missing_source_id = StyleSourceId::document_adopted_style_sheet(detached_document, 0);
    let target_queries = vec![
        PendingStyleInvalidationTargetQueries::fallback_roots_for_test(document_target, [document]),
        PendingStyleInvalidationTargetQueries::retained_source(
            missing_source_id.clone(),
            indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(
                detached_document,
            )]),
        ),
    ];
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [
                stylo_source_result! {
                    source_index: 0,
                    kind: StyloSourceStyleInvalidationSourceResultKind::Fallback,
                    exact: false,
                    empty_result_is_exact: false,
                    matched_dependency_count: 0,
                    fallback_reasons: vec![
                        StyloSourceInvalidationFallbackReason::UnsupportedDependency,
                    ],
                    fallback_root_availability: Some(
                        StyleSourceFallbackRootAvailability::Available { root_count: 1 },
                    ),
                    affected_roots: vec![document],
                },
                stylo_source_result! {
                    source_index: 1,
                    kind: StyloSourceStyleInvalidationSourceResultKind::MissingRetainedStyleSystem,
                    exact: false,
                    empty_result_is_exact: false,
                    matched_dependency_count: 0,
                    fallback_reasons: vec![
                        StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem,
                    ],
                    fallback_root_availability: None,
                    affected_roots: Vec::new(),
                },
            ],
        },
        &target_queries,
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);

    assert_eq!(application.clear_all_reasons(), None);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&document) && roots.contains(&detached_document)
    ));
    assert!(application.affected_root_count() >= 2);
    assert_eq!(application.diagnostic_target_results().len(), 2);
    let missing_target = StyleInvalidationSourceTarget::stylesheet(missing_source_id);
    let missing_result = application
        .diagnostic_target_results()
        .iter()
        .find(|result| result.target() == &missing_target)
        .expect("missing retained source target result");
    assert_eq!(
        missing_result.kind(),
        StyleInvalidationDiagnosticTargetResultKind::MissingRetainedStyleSystem
    );
    assert_eq!(missing_result.affected_root_count(), 0);
    assert!(
        missing_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem)
    );
}
#[test]
fn fallback_root_inexact_empty_result_scopes_to_fallback_root_target() {
    let host = test_host();
    let root = host.document_handle();
    let target =
        StyleInvalidationSourceTarget::fallback_root(&host, root).expect("document target");
    let target_query = PendingStyleInvalidationTargetQueries::fallback_roots_for_test(target, []);
    let outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Fallback,
                exact: false,
                empty_result_is_exact: false,
                matched_dependency_count: 0,
                fallback_reasons: vec![StyloSourceInvalidationFallbackReason::InexactEmptyResult],
                fallback_root_availability: None,
                affected_roots: Vec::new(),
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );

    let application = outcome.finalize(&host);
    assert_eq!(application.clear_all_reasons(), None);
    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::SourceFallbackSubtreeRoots
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&root)
    ));
    assert_eq!(application.diagnostic_target_results().len(), 1);
    let target_result = &application.diagnostic_target_results()[0];
    assert_eq!(
        target_result.kind(),
        StyleInvalidationDiagnosticTargetResultKind::Fallback
    );
    assert!(
        target_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::InexactEmptyResult)
    );
}
#[test]
fn fallback_root_clear_all_reasons_scope_to_fallback_root_and_preserve_diagnostics() {
    let host = test_host();
    let root = host.document_handle();
    let target =
        StyleInvalidationSourceTarget::fallback_root(&host, root).expect("document target");
    let target_query = PendingStyleInvalidationTargetQueries::fallback_roots_for_test(target, []);
    let mut outcome = StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Fallback,
                exact: false,
                empty_result_is_exact: false,
                matched_dependency_count: 0,
                fallback_reasons: vec![StyloSourceInvalidationFallbackReason::MissingFallbackRoots],
                fallback_root_availability: Some(StyleSourceFallbackRootAvailability::Missing),
                affected_roots: Vec::new(),
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    );
    outcome.extend(StyleInvalidationOutcome::from_retained_source_result(
        &host,
        moli_invalidation_result! {
            source_results: [stylo_source_result! {
                source_index: 0,
                kind: StyloSourceStyleInvalidationSourceResultKind::Fallback,
                exact: false,
                empty_result_is_exact: false,
                matched_dependency_count: 0,
                fallback_reasons: vec![
                    StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem,
                ],
                fallback_root_availability: None,
                affected_roots: Vec::new(),
            }],
        },
        std::slice::from_ref(&target_query),
        StyleInvalidationCleanupEffects::default(),
    ));

    let application = outcome.finalize(&host);

    assert_eq!(application.clear_all_reasons(), None);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&root)
    ));
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .retained_source_unavailable_target_count(),
        0
    );
    assert!(
        application
            .diagnostic_target_results()
            .iter()
            .any(|result| {
                result.target().is_fallback_root()
                    && result.kind() == StyleInvalidationDiagnosticTargetResultKind::Fallback
                    && result
                        .fallback_reasons()
                        .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
            })
    );
    assert!(
        application
            .diagnostic_target_results()
            .iter()
            .any(|result| {
                result.target().is_fallback_root()
                    && result.kind() == StyleInvalidationDiagnosticTargetResultKind::Fallback
                    && result.fallback_reasons().contains(
                        &StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem,
                    )
            })
    );
}
#[test]
fn pending_source_fallback_reason_reaches_outcome_for_fallback_only_source() {
    let host = test_host();
    let engine = MoliStyleEngine::new();
    let root = host.document_handle();
    let target =
        StyleInvalidationSourceTarget::fallback_root(&host, root).expect("document target");
    let mut target_query =
        PendingStyleInvalidationTargetQueries::fallback_roots_for_test(target, [root]);
    target_query
        .add_fallback_reason_for_test(StyloSourceInvalidationFallbackReason::UnsupportedDependency);
    let outcome = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        root,
        StyleSourceDocumentContext::for_root_document(root),
        None,
        std::slice::from_ref(&target_query),
        false,
    );

    let application = outcome.finalize(&host);

    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert_eq!(
        application.diagnostic_target_results()[0].kind(),
        StyleInvalidationDiagnosticTargetResultKind::Fallback
    );
    assert!(
        application.diagnostic_target_results()[0]
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::UnsupportedDependency)
    );
}
#[test]
fn fallback_only_source_emits_each_target_result_fallback_reason() {
    let host = test_host();
    let engine = MoliStyleEngine::new();
    let root = host.document_handle();
    let target =
        StyleInvalidationSourceTarget::fallback_root(&host, root).expect("document target");
    let mut target_query =
        PendingStyleInvalidationTargetQueries::fallback_roots_for_test(target, [root]);
    target_query
        .add_fallback_reason_for_test(StyloSourceInvalidationFallbackReason::UnsupportedDependency);
    target_query.add_fallback_reason_for_test(
        StyloSourceInvalidationFallbackReason::UnsupportedStateDependency,
    );

    let outcome = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        root,
        StyleSourceDocumentContext::for_root_document(root),
        None,
        std::slice::from_ref(&target_query),
        false,
    );
    let application = outcome.finalize(&host);

    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert!(
        application.diagnostic_target_results()[0]
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::UnsupportedDependency)
    );
    assert!(
        application.diagnostic_target_results()[0]
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::UnsupportedStateDependency)
    );
    let telemetry = application.fallback_telemetry();
    assert_eq!(telemetry.exact_target_result_count(), 0);
    assert_eq!(telemetry.inexact_target_result_count(), 1);
    assert_eq!(telemetry.fallback_target_result_count(), 1);
    assert_eq!(telemetry.source_result_affected_root_count(), 1);
    assert_eq!(
        telemetry.fallback_reason_target_count(
            StyloSourceInvalidationFallbackReason::UnsupportedDependency,
        ),
        1
    );
    assert_eq!(
        telemetry.fallback_reason_target_count(
            StyloSourceInvalidationFallbackReason::UnsupportedStateDependency,
        ),
        1
    );
    assert_eq!(
        telemetry.fallback_reason_target_count(StyloSourceInvalidationFallbackReason::FullSelector),
        0
    );
}
#[test]
fn structural_boundary_fallback_target_queries_dedupe_duplicate_targets() {
    let host = test_host();
    let root = host.document_handle();

    let target_queries = PendingCauseFallback::default()
        .target_queries_for_structural_boundary_roots(&host, [root, root]);

    assert_eq!(target_queries.len(), 1);
    assert!(target_queries[0].target().is_fallback_root());
    assert_eq!(target_queries[0].retained_query_count(), 0);
    assert_eq!(
        target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .len(),
        1
    );
    assert!(
        target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&root)
    );
}
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "retained-source target queries must not be empty")]
fn retained_target_query_rejects_empty_query_set() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);

    let _ = PendingStyleInvalidationTargetQueries::retained_source(
        source_id,
        indexmap::IndexSet::new(),
    );
}
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "fallback-root target queries should use fallback-root targets")]
fn fallback_target_query_rejects_real_stylesheet_source() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);

    let _ = PendingStyleInvalidationTargetQueries::fallback_roots_for_test(
        StyleInvalidationSourceTarget::stylesheet(source_id),
        [root],
    );
}
#[test]
fn real_stylesheet_source_fallback_uses_empty_query_payload() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);

    let target_query = PendingStyleInvalidationTargetQueries::source_fallback_for_test(
        source_id.clone(),
        [root],
        None,
    );

    assert_eq!(
        target_query.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert_eq!(target_query.retained_query_count(), 0);
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&root)
    );

    let outcome = retained_source_invalidation_outcome_for_document_for_test(
        &MoliStyleEngine::new(),
        &host,
        root,
        StyleSourceDocumentContext::for_root_document(root),
        None,
        std::slice::from_ref(&target_query),
        false,
    );
    let application = outcome.finalize(&host);
    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert_eq!(
        application.diagnostic_target_results()[0]
            .diagnostic_source_target_availability()
            .and_then(|availability| availability.fallback_roots()),
        Some(StyleSourceFallbackRootAvailability::Available { root_count: 1 })
    );
}
#[test]
fn missing_source_roots_fallback_reaches_outcome_as_source_fallback() {
    let host = test_host();
    let engine = MoliStyleEngine::new();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);

    let target_query =
        PendingStyleInvalidationTargetQueries::source_fallback_missing_roots_for_test(
            source_id.clone(),
            Some(StyloSourceInvalidationFallbackReason::MissingFallbackRoots),
        );

    assert_eq!(
        target_query.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert_eq!(target_query.retained_query_count(), 0);
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .is_empty()
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
    );

    let outcome = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        root,
        StyleSourceDocumentContext::for_root_document(root),
        None,
        std::slice::from_ref(&target_query),
        false,
    );
    let application = outcome.finalize(&host);

    assert_eq!(application.clear_all_reasons(), None);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&root)
    ));
    assert_eq!(application.diagnostic_target_results().len(), 1);
    let target_result = &application.diagnostic_target_results()[0];
    assert_eq!(target_result.target(), target_query.target());
    assert_eq!(
        target_result.kind(),
        StyleInvalidationDiagnosticTargetResultKind::MissingFallbackRoots
    );
    assert_eq!(
        target_result
            .diagnostic_source_target_availability()
            .and_then(|availability| availability.fallback_roots()),
        Some(StyleSourceFallbackRootAvailability::Missing)
    );
    assert_eq!(target_result.affected_root_count(), 0);
}

#[test]
fn shadow_root_missing_source_roots_fallback_uses_shadow_scope_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    assert!(host.append_child(document, shadow_host));
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let engine = MoliStyleEngine::new();
    let source_id = StyleSourceId::shadow_root_adopted_style_sheet(shadow_root, 0);
    let target_query =
        PendingStyleInvalidationTargetQueries::source_fallback_missing_roots_for_test(
            source_id,
            Some(StyloSourceInvalidationFallbackReason::MissingFallbackRoots),
        );

    let outcome = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
        None,
        std::slice::from_ref(&target_query),
        false,
    );
    let application = outcome.finalize(&host);

    assert_eq!(application.clear_all_reasons(), None);
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&shadow_root) && roots.contains(&shadow_host)
    ));
    assert_eq!(
        application.diagnostic_target_results()[0].kind(),
        StyleInvalidationDiagnosticTargetResultKind::MissingFallbackRoots
    );
}

#[test]
fn target_query_drain_clear_all_when_retained_fallback_has_no_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [active, detached] {
        assert!(
            engine
                .computed_style_property_value(
                    &host,
                    &document_url,
                    handle,
                    "display",
                    None,
                    &inputs,
                    None,
                )
                .is_some()
        );
    }
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
    ensure_adapter_element_data(&engine, &host, active);
    ensure_adapter_element_data(&engine, &host, detached);
    assert!(engine.dom_adapter.has_element_data(active));
    assert!(engine.dom_adapter.has_element_data(detached));
    let target_epoch = engine.target_context_epoch_for_document_for_test(document);

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let mut queries = indexmap::IndexSet::new();
    queries.insert(RetainedStyleInvalidationQuery::attribute(
        active,
        "data-state".to_owned(),
    ));
    engine.clear_retained_style_system_for_document_for_test(document);
    engine.queue_style_invalidation_targets_for_document_for_test(
        document,
        PendingStyleInvalidationCause::Mutation(Vec::new()),
        vec![PendingStyleInvalidationTargetQueries::retained_source(
            source_id.clone(),
            queries,
        )],
    );

    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
    assert!(
        element_style_is_dirty_for_test(&engine, &host, active),
        "scoped fallback should retain the last computed values and mark them dirty"
    );
    assert!(engine.dom_adapter.has_element_data(detached));
    assert!(engine.target_context_epoch_for_document_for_test(document) > target_epoch);
}

#[test]
fn clear_all_fallback_application_records_reasons_in_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let detached_document = host.create_detached_html_document();
    let engine = MoliStyleEngine::new();

    let outcome = StyleInvalidationOutcome::retained_clear_all_for_test([
        StyloSourceInvalidationFallbackReason::FullSelector,
    ]);
    let world = engine.world_for_document(document);
    assert!(
        engine
            .cache_cleanup_for_world(&world)
            .apply_finalized_result(&host, outcome.finalize(&host))
    );

    assert_eq!(
        engine.invalidation_clear_all_fallback_reasons_for_document_for_test(document),
        vec![StyloSourceInvalidationFallbackReason::FullSelector]
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::InvalidationClearAllFallback]
    );
    assert!(
        engine
            .invalidation_clear_all_fallback_reasons_for_document_for_test(detached_document)
            .is_empty()
    );
    assert!(
        engine
            .source_dirty_scope_reasons_for_document_for_test(detached_document)
            .is_empty()
    );

    assert!(
        !engine
            .cache_cleanup_for_world(&world)
            .apply_finalized_result(&host, StyleInvalidationOutcome::default().finalize(&host))
    );
    assert!(
        engine
            .invalidation_clear_all_fallback_reasons_for_document_for_test(document)
            .is_empty()
    );
}

#[test]
fn subtree_fallback_cleanup_clears_only_affected_shadow_cascade_data() {
    let mut host = test_host();
    let document = host.document_handle();
    let first_host = host.create_element("section");
    let second_host = host.create_element("article");
    assert!(host.append_child(document, first_host));
    assert!(host.append_child(document, second_host));
    let first_shadow_root = host
        .attach_shadow_root(first_host, "open")
        .expect("first host should accept a shadow root");
    let second_shadow_root = host
        .attach_shadow_root(second_host, "open")
        .expect("second host should accept a shadow root");

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        ":host { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    let second_source = StyloStylesheetSource::new(
        ":host { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((first_shadow_root, vec![first_source]));
    inputs
        .shadow_stylesheet_sources
        .push((second_shadow_root, vec![second_source]));
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);

    let (first_cascade_data, second_cascade_data) = engine
        .with_retained_style_system_for_document_for_test(document, |retained| {
            let first = retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == first_shadow_root)
                .expect("retained system should track the first shadow root")
                .1
                .clone();
            let second = retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == second_shadow_root)
                .expect("retained system should track the second shadow root")
                .1
                .clone();
            (first, second)
        });
    let source_id = StyleSourceId::shadow_root_adopted_style_sheet(first_shadow_root, 0);
    let target_query = PendingStyleInvalidationTargetQueries::source_fallback_for_test(
        source_id,
        [first_shadow_root, first_host],
        [StyloSourceInvalidationFallbackReason::SourceScopeFallback],
    );
    let outcome = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
        None,
        std::slice::from_ref(&target_query),
        true,
    );
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            first_shadow_root,
            first_cascade_data,
        );
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            second_shadow_root,
            second_cascade_data,
        );
    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(first_shadow_root)
    );
    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(second_shadow_root)
    );
    let finalized_result = outcome.finalize(&host);
    assert_eq!(
        finalized_result.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::SourceFallbackSubtreeRoots
    );
    assert!(matches!(
        finalized_result.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&first_shadow_root)
                && !roots.contains(&second_shadow_root)
    ));
    let world = engine.world_for_document(document);
    assert!(
        engine
            .cache_cleanup_for_world(&world)
            .apply_finalized_result(&host, finalized_result)
    );

    assert!(
        !engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(first_shadow_root)
    );
    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(second_shadow_root)
    );
}

#[test]
fn detached_document_rootless_source_fallback_clears_only_source_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [active, detached] {
        assert!(
            engine
                .computed_style_property_value(
                    &host,
                    &document_url,
                    handle,
                    "display",
                    None,
                    &inputs,
                    None,
                )
                .is_some()
        );
    }
    ensure_adapter_element_data(&engine, &host, active);
    ensure_adapter_element_data(&engine, &host, detached);
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );

    let source_id = StyleSourceId::document_adopted_style_sheet(detached_document, 0);
    engine.queue_style_invalidation_targets_for_document_for_test(
        detached_document,
        PendingStyleInvalidationCause::Mutation(Vec::new()),
        vec![
            PendingStyleInvalidationTargetQueries::source_fallback_missing_roots_for_test(
                source_id,
                Some(StyloSourceInvalidationFallbackReason::MissingFallbackRoots),
            ),
        ],
    );

    engine.drain_pending_style_invalidations_for_document_for_test(&host, detached_document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        0
    );
    assert!(engine.dom_adapter.has_element_data(active));
    assert!(
        element_style_is_dirty_for_test(&engine, &host, detached),
        "scoped fallback should retain the last computed values and mark them dirty"
    );
}

fn ensure_adapter_element_data(
    engine: &MoliStyleEngine,
    host: &crate::dom::native::DomHost,
    handle: crate::document_runtime::DomHandle,
) {
    engine.dom_adapter.with_bound_host(host, |adapter| {
        let element = adapter.element(host, handle).expect("element");
        unsafe {
            let _ = element.ensure_data();
        }
    });
}

fn element_style_is_dirty_for_test(
    engine: &MoliStyleEngine,
    host: &crate::dom::native::DomHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    engine.dom_adapter.with_bound_host(host, |adapter| {
        let element = adapter.element(host, handle).expect("element");
        element
            .borrow_data()
            .is_some_and(|data| !data.hint.is_empty())
    })
}
