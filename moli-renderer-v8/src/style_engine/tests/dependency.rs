use super::*;

fn matching_source(
    id: StyleSourceId,
    dependency_summary: &StyloSourceDependencySummary,
    fallback_roots: Vec<DomHandle>,
) -> super::source_record::MatchingStyleDependencySource {
    super::source_record::MatchingStyleDependencySource::new_for_test(
        id,
        dependency_summary,
        fallback_roots,
    )
}

fn source_dependency_request<'a>(
    query: &'a RetainedStyleInvalidationQuery,
    context: Option<moli_selector::StyloDependencyInvalidationFallbackContext>,
    requirement: StyloSourceDependencyRequestRequirement,
) -> StyloSourceDependencyInvalidationRequest<'a> {
    StyloSourceDependencyInvalidationRequest::new(query, context, requirement)
}

fn exact_source_dependency_request<'a>(
    query: &'a RetainedStyleInvalidationQuery,
    context: Option<moli_selector::StyloDependencyInvalidationFallbackContext>,
) -> StyloSourceDependencyInvalidationRequest<'a> {
    source_dependency_request(
        query,
        context,
        StyloSourceDependencyRequestRequirement::exact(),
    )
}

fn source_scope_fallback_target_queries_for_test(
    host: &DomHost,
    source_scope: &StyleSourceScope,
    reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
) -> Vec<PendingStyleInvalidationTargetQueries> {
    PendingStyleInvalidationTargetQueries::planned_fallback_root_target(
        host,
        stylo_source_scope_fallback_plan(host, source_scope, reasons),
    )
}

#[test]
fn pending_target_query_merge_preserves_all_fallback_reasons() {
    let host = test_host();
    let root = host.document_handle();
    let target =
        StyleInvalidationSourceTarget::fallback_root(&host, root).expect("document target");
    let mut target_query =
        PendingStyleInvalidationTargetQueries::fallback_roots_for_test(target.clone(), [root]);
    target_query
        .add_fallback_reason_for_test(StyloSourceInvalidationFallbackReason::UnsupportedDependency);
    let mut target_queries = vec![target_query];

    super::target_queries::merge_pending_target_queries(&mut target_queries, {
        let mut incoming =
            PendingStyleInvalidationTargetQueries::fallback_roots_for_test(target, [root]);
        incoming.add_fallback_reason_for_test(
            StyloSourceInvalidationFallbackReason::UnsupportedStateDependency,
        );
        vec![incoming]
    });

    assert_eq!(target_queries.len(), 1);
    assert!(
        target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::UnsupportedDependency)
    );
    assert!(
        target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::UnsupportedStateDependency)
    );
}
#[test]
fn missing_source_roots_fallback_preserves_dependency_reason_and_missing_roots() {
    let host = test_host();
    let engine = MoliStyleEngine::new();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);

    let target_query =
        PendingStyleInvalidationTargetQueries::source_fallback_missing_roots_for_test(
            source_id.clone(),
            Some(StyloSourceInvalidationFallbackReason::FullSelector),
        );

    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::FullSelector)
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
    assert_eq!(
        target_result.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
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
    assert!(
        target_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
    );
    assert!(
        target_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::FullSelector)
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .missing_fallback_roots_target_count(),
        1
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::UnknownOrFallback
    );
}
#[test]
fn missing_retained_style_system_reaches_cleanup_application_table() {
    let host = test_host();
    let mut engine = MoliStyleEngine::new();
    let root = host.document_handle();
    engine.set_document_adopted_style_sheet_sources(root, Vec::new());
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let mut target_query = PendingStyleInvalidationTargetQueries::retained_source(
        source_id.clone(),
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(root)]),
    );
    target_query.extend_reasoned_fallback_roots_for_test([root]);

    let outcome = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        root,
        StyleSourceDocumentContext::for_root_document(root),
        None,
        std::slice::from_ref(&target_query),
        true,
    );
    let application = outcome.finalize(&host);

    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .retained_source_unavailable_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_lifecycle_available_without_source_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_lifecycle_unavailable_target_count(),
        0
    );
    assert!(application.clears_shadow_cascade_data_for_cleanup_target());
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::UnknownOrFallback
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .retained_source_unavailable_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_lifecycle_available_without_source_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_lifecycle_unavailable_target_count(),
        0
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::UnknownOrFallback
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&root)
    ));
    assert_eq!(application.diagnostic_target_results().len(), 1);
    let target_result = &application.diagnostic_target_results()[0];
    assert_eq!(
        target_result.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert_eq!(
        target_result.kind(),
        StyleInvalidationDiagnosticTargetResultKind::MissingRetainedStyleSystem
    );
    assert!(!target_result.exact());
    assert!(!target_result.empty_result_is_exact());
    assert_eq!(target_result.matched_dependency_count(), 0);
    assert_eq!(target_result.retained_query_count(), 1);
    assert_eq!(
        target_result
            .diagnostic_source_target_availability()
            .and_then(|availability| availability.fallback_roots()),
        Some(StyleSourceFallbackRootAvailability::Available { root_count: 1 })
    );
    assert_eq!(target_result.affected_root_count(), 1);
    assert!(
        target_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem)
    );
    assert_eq!(
        target_result
            .diagnostic_source_target_availability()
            .and_then(|availability| availability.lifecycle().cloned()),
        Some(StyleSourceLifecycleAvailability::AvailableWithoutSource {
            reason: StyleSourceLifecycleWithoutSourceReason::EmptyDocumentAdoptedStyleSheets,
        })
    );
    let availability = target_result
        .diagnostic_source_target_availability()
        .expect("target result should include source availability");
    assert_eq!(
        availability.document_kind(),
        Some(StyleSourceDocumentKind::Root)
    );
    assert_eq!(
        availability.lifecycle(),
        Some(&StyleSourceLifecycleAvailability::AvailableWithoutSource {
            reason: StyleSourceLifecycleWithoutSourceReason::EmptyDocumentAdoptedStyleSheets,
        })
    );
    assert_eq!(
        availability.fallback_roots(),
        Some(StyleSourceFallbackRootAvailability::Available { root_count: 1 })
    );
}
#[test]
fn missing_retained_cascade_data_reaches_cleanup_application_table() {
    let host = test_host();
    let mut engine = MoliStyleEngine::new();
    let root = host.document_handle();
    engine.set_document_adopted_style_sheet_sources(root, Vec::new());
    let inputs = FullStyleWorldSnapshot::default();
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, root, key, &inputs);

    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let mut target_query = PendingStyleInvalidationTargetQueries::retained_source(
        source_id.clone(),
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(root)]),
    );
    target_query.extend_reasoned_fallback_roots_for_test([root]);

    let outcome = engine.with_retained_style_system_for_document_for_test(root, |retained| {
        retained_source_invalidation_outcome_for_document_for_test(
            &engine,
            &host,
            root,
            StyleSourceDocumentContext::for_root_document(root),
            Some(retained),
            std::slice::from_ref(&target_query),
            false,
        )
    });
    let application = outcome.finalize(&host);

    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .retained_source_unavailable_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_lifecycle_available_without_source_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_lifecycle_unavailable_target_count(),
        0
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::UnknownOrFallback
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .retained_source_unavailable_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_lifecycle_available_without_source_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_lifecycle_unavailable_target_count(),
        0
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&root)
    ));
    assert_eq!(application.diagnostic_target_results().len(), 1);
    let target_result = &application.diagnostic_target_results()[0];
    assert_eq!(
        target_result.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert_eq!(
        target_result.kind(),
        StyleInvalidationDiagnosticTargetResultKind::MissingRetainedCascadeData
    );
    assert!(!target_result.exact());
    assert_eq!(target_result.retained_query_count(), 1);
    assert_eq!(
        target_result
            .diagnostic_source_target_availability()
            .and_then(|availability| availability.fallback_roots()),
        Some(StyleSourceFallbackRootAvailability::Available { root_count: 1 })
    );
    assert_eq!(target_result.affected_root_count(), 1);
    assert!(
        target_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::MissingRetainedCascadeData)
    );
    assert_eq!(
        target_result
            .diagnostic_source_target_availability()
            .and_then(|availability| availability.lifecycle().cloned()),
        Some(StyleSourceLifecycleAvailability::AvailableWithoutSource {
            reason: StyleSourceLifecycleWithoutSourceReason::EmptyDocumentAdoptedStyleSheets,
        })
    );
}
#[test]
fn style_invalidation_required_source_fallback_missing_roots_is_explicit() {
    let host = test_host();
    let document = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text("section:empty { color: red; }", &base_url);
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        Vec::new(),
    )];
    let query = RetainedStyleInvalidationQuery::element_type(document, "section".to_owned());
    let requests = [source_dependency_request(
        &query,
        None,
        StyloSourceDependencyRequestRequirement::child_list_structural(),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[], &[]),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots
    );
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
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::InexactEmptyResult)
    );
}
#[test]
fn source_scope_fallback_targets_are_explicitly_labeled() {
    let host = test_host();
    let document = host.document_handle();
    let source_scope = StyleSourceScope::for_document(document);

    let target_queries = source_scope_fallback_target_queries_for_test(
        &host,
        &source_scope,
        [StyloSourceInvalidationFallbackReason::SourceScopeFallback],
    );

    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert!(target_query.target().is_fallback_root());
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::SourceScopeFallback
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&document)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::SourceScopeFallback)
    );
}
#[test]
fn pending_cause_state_source_scope_fallback_preserves_computed_reason() {
    let host = test_host();
    let document = host.document_handle();
    let source_scope = StyleSourceScope::for_document(document);

    let target_queries = PendingCauseFallback::from_cause(
        &host,
        &PendingStyleInvalidationCause::StateChange {
            element: document,
            state: StyloElementState::HOVER,
            old_state: None,
        },
    )
    .target_queries_for_source_scope(&host, &source_scope);

    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::SourceScopeFallback
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&document)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::SourceScopeFallback)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::UnsupportedStateDependency)
    );
}

#[test]
fn media_state_change_without_snapshot_uses_retained_query() {
    let mut host = test_host();
    let document = host.document_handle();
    let video = host.create_element("video");
    let target = host.create_element("section");
    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.append_child(document, video));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "video:playing + #target { color: rgb(1, 2, 3); }".into(),
        document_url,
    )
    .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
        document, 0,
    )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source]);

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let source_scope = super::scope::source_scope_for_element_state_change(
        &host,
        video,
        StyloElementState::PAUSED,
    )
    .expect("media state source scope");
    let target_queries = {
        let world = engine.world_for_document(document);
        let linked_sources = world.linked_stylesheet_sources.borrow();
        let document_adopted_sources = world.adopted_style_sheet_sources.borrow();
        super::planner::target_queries_for_pending_cause_with_document_adopted_sources(
            &host,
            &linked_sources,
            &document_adopted_sources,
            &engine.dom_adapter,
            &media,
            StyleViewport::default(),
            host.document_handle(),
            &PendingStyleInvalidationCause::StateChange {
                element: video,
                state: StyloElementState::PAUSED,
                old_state: None,
            },
            &source_scope,
        )
    };

    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries
    );
    assert!(
        !target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::UnsupportedStateDependency)
    );
}

#[test]
fn source_scope_fallback_has_explicit_target_result_kind() {
    let host = test_host();
    let document = host.document_handle();
    let engine = MoliStyleEngine::new();
    let source_scope = StyleSourceScope::for_document(document);
    let target_queries = source_scope_fallback_target_queries_for_test(
        &host,
        &source_scope,
        [StyloSourceInvalidationFallbackReason::UnsupportedStateDependency],
    );

    let outcome = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
        None,
        &target_queries,
        false,
    );
    let application = outcome.finalize(&host);

    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_scope_fallback_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .retained_source_unavailable_target_count(),
        0
    );
    let target_result = &application.diagnostic_target_results()[0];
    assert!(target_result.target().is_fallback_root());
    assert_eq!(
        target_result.kind(),
        StyleInvalidationDiagnosticTargetResultKind::SourceScopeFallback
    );
    assert!(
        target_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::SourceScopeFallback)
    );
    assert!(
        target_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::UnsupportedStateDependency)
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::UnknownOrFallback
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_scope_fallback_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .retained_source_unavailable_target_count(),
        0
    );
}
#[test]
fn pending_cause_default_roots_do_not_get_source_scope_reason() {
    let host = test_host();
    let document = host.document_handle();
    let source_scope = StyleSourceScope::for_document(document);

    let target_queries = PendingCauseFallback::from_cause(
        &host,
        &PendingStyleInvalidationCause::Mutation(vec![StyleMutationEffect::ConnectedSubtree {
            root: document,
        }]),
    )
    .target_queries_for_source_scope(&host, &source_scope);

    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&document)
    );
    assert!(
        !target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::SourceScopeFallback)
    );
    assert!(target_query.fallback_reasons_for_test().is_empty());
}
#[test]
fn source_dependency_plan_translation_preserves_exact_queries_fallback_roots_and_reason() {
    let mut host = test_host();
    let root = host.document_handle();
    let parent = host.create_element("section");
    let exact_root = host.create_element("div");
    let nth_root = host.create_element("div");
    let later = host.create_element("div");
    assert!(host.append_child(root, parent));
    assert!(host.append_child(parent, exact_root));
    assert!(host.append_child(parent, nth_root));
    assert!(host.append_child(parent, later));
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        ".exact { color: green; }
         section > div:nth-child(odd of :not(.c)) { color: red; }",
        &base_url,
    );
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        Vec::new(),
    )];
    let exact_query = RetainedStyleInvalidationQuery::class(exact_root, "exact".to_owned());
    let nth_query = RetainedStyleInvalidationQuery::class(nth_root, "c".to_owned());
    let requests = [
        exact_source_dependency_request(&exact_query, None),
        exact_source_dependency_request(
            &nth_query,
            Some(
                moli_selector::StyloDependencyInvalidationFallbackContext::from_mutation_relation(
                    Some(parent),
                    Some(exact_root),
                    Some(later),
                ),
            ),
        ),
    ];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries
    );
    assert!(
        target_query
            .retained_queries_for_test()
            .is_some_and(|queries| queries.contains(&exact_query))
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&nth_root)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );
}
#[test]
fn source_dependency_plan_translation_preserves_required_source_fallback_reason() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "section > div:nth-child(odd of :not(.c)) { color: red; }",
        &base_url,
    );
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        Vec::new(),
    )];
    let query = RetainedStyleInvalidationQuery::class(root, "c".to_owned());
    let requests = [exact_source_dependency_request(&query, None)];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .is_empty()
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
    );
}

#[test]
fn source_dependency_plan_translation_preserves_required_source_fallback_reason_set() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "section > div:nth-child(odd of :not(.c)) { color: red; }
         section:has(:is(.item + .item + .item > .child)) { background-color: blue; }",
        &base_url,
    );
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        Vec::new(),
    )];
    let nth_query = RetainedStyleInvalidationQuery::class(root, "c".to_owned());
    let nested_query = RetainedStyleInvalidationQuery::class(root, "item".to_owned());
    let requests = [
        exact_source_dependency_request(&nth_query, None),
        exact_source_dependency_request(&nested_query, None),
    ];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::NestedRelativeSelectorDependency)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
    );
}

#[test]
fn source_dependency_required_source_fallback_keeps_source_target_when_roots_exist() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "section > div:nth-child(odd of :not(.c)) { color: red; }",
        &base_url,
    );
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![root],
    )];
    let query = RetainedStyleInvalidationQuery::class(root, "c".to_owned());
    let requests = [exact_source_dependency_request(&query, None)];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id.clone())
    );
    assert!(!target_query.target().is_fallback_root());
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::FallbackOnly
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&root)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );

    let engine = MoliStyleEngine::new();
    let outcome = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        root,
        StyleSourceDocumentContext::for_root_document(root),
        None,
        target_queries,
        false,
    );
    let application = outcome.finalize(&host);
    let diagnostic_target = &application.diagnostic_target_results()[0];
    assert_eq!(
        diagnostic_target.kind(),
        StyleInvalidationDiagnosticTargetResultKind::Fallback
    );
    assert_eq!(
        diagnostic_target.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert!(
        diagnostic_target
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );
}

#[test]
fn source_dependency_required_source_fallback_uses_selector_plan_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let plan_root = host.create_element("section");
    assert!(host.append_child(document, plan_root));
    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "section > div:nth-child(odd of :not(.c)) { color: red; }",
        &base_url,
    );
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![plan_root],
    )];
    let query = RetainedStyleInvalidationQuery::class(plan_root, "c".to_owned());
    let requests = [exact_source_dependency_request(&query, None)];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&plan_root)
    );
    assert!(
        !target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&document)
    );
}

#[test]
fn source_dependency_plan_translation_preserves_missing_roots_fallback_kind() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "section > div:nth-child(odd of :not(.c)) { color: red; }",
        &base_url,
    );
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        Vec::new(),
    )];
    let query = RetainedStyleInvalidationQuery::class(root, "c".to_owned());
    let requests = [exact_source_dependency_request(&query, None)];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .is_empty()
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
    );
}

#[test]
fn source_dependency_plan_translation_preserves_structural_cleanup_roots() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata =
        stylo_source_metadata_for_css_text("#subject:has(+ #old-next) { color: red; }", &base_url);
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![root],
    )];
    let query = RetainedStyleInvalidationQuery::id(root, "old-next".to_owned());
    let requests = [source_dependency_request(
        &query,
        None,
        StyloSourceDependencyRequestRequirement::relative_previous_sibling(),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[], &[root]),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].target(),
        &StyleInvalidationSourceTarget::stylesheet(source_id)
    );
    assert_eq!(
        target_queries[0].structural_boundary_cleanup_root_count(),
        1
    );
}

#[test]
fn source_dependency_request_translation_prefers_cause_fallback_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let cause_root = host.create_element("section");
    assert!(host.append_child(document, cause_root));
    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata =
        stylo_source_metadata_for_css_text(".marker + .target { color: red; }", &base_url);
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![document],
    )];
    let query = RetainedStyleInvalidationQuery::class(document, "marker".to_owned());
    let requests = [exact_source_dependency_request(&query, None)];
    let source_local_plan =
        super::target_plan::pending_target_queries_for_source_dependency_requests(
            &host,
            &matching_sources,
            &requests,
            &[],
            StyloSourceDependencyBoundaryRoots::default(),
        );
    assert!(source_local_plan.boundary_fallback().is_none());
    let source_local_targets = source_local_plan.target_queries();
    assert_eq!(source_local_targets.len(), 1);
    assert!(
        source_local_targets[0]
            .exact_safety_fallback_root_set_for_test()
            .contains(&document)
    );
    assert!(
        !source_local_targets[0]
            .exact_safety_fallback_root_set_for_test()
            .contains(&cause_root)
    );
    assert!(
        source_local_targets[0]
            .reasoned_fallback_root_set_for_test()
            .is_empty()
    );

    let cause_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[cause_root],
        StyloSourceDependencyBoundaryRoots::default(),
    );
    assert!(cause_plan.boundary_fallback().is_none());
    let cause_targets = cause_plan.target_queries();
    assert_eq!(cause_targets.len(), 1);
    assert!(
        cause_targets[0]
            .exact_safety_fallback_root_set_for_test()
            .contains(&cause_root)
    );
    assert!(
        !cause_targets[0]
            .exact_safety_fallback_root_set_for_test()
            .contains(&document)
    );
    assert!(
        cause_targets[0]
            .reasoned_fallback_root_set_for_test()
            .is_empty()
    );
}

#[test]
fn source_dependency_request_translation_uses_context_roots_for_nth_of_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let first = host.create_element("div");
    let middle = host.create_element("div");
    let target = host.create_element("div");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, first));
    assert!(host.append_child(parent, middle));
    assert!(host.append_child(parent, target));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "section > div:nth-child(odd of :not(.c)) { color: red; }",
        &base_url,
    );
    let dependency = metadata
        .dependency_summary
        .query_class(&style::Atom::from("c"));
    assert!(dependency.requires_fallback(), "{dependency:?}");
    assert!(dependency.has_sibling_dependency(), "{dependency:?}");
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![document],
    )];
    let query = RetainedStyleInvalidationQuery::class(middle, "c".to_owned());
    let requests = [exact_source_dependency_request(
        &query,
        Some(moli_selector::StyloDependencyInvalidationFallbackContext::default()),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::ContextFallback
    );
    assert!(
        target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );
    assert!(
        target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&middle)
    );
    assert!(
        target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&target)
    );
    assert!(
        !target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&first)
    );
    assert!(
        !target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&document)
    );
}

#[test]
fn source_dependency_request_translation_tries_nested_relative_with_context_safety_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let first = host.create_element("div");
    let middle = host.create_element("div");
    let target = host.create_element("div");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, first));
    assert!(host.append_child(parent, middle));
    assert!(host.append_child(parent, target));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "section:has(:is(.item + .item + .item > .child)) { color: red; }",
        &base_url,
    );
    let dependency = metadata
        .dependency_summary
        .query_class(&style::Atom::from("item"));
    assert!(dependency.requires_fallback(), "{dependency:?}");
    assert!(dependency.has_sibling_dependency(), "{dependency:?}");
    let matching_sources = vec![matching_source(
        source_id,
        &metadata.dependency_summary,
        vec![document],
    )];
    let query = RetainedStyleInvalidationQuery::class(middle, "item".to_owned());
    let requests = [exact_source_dependency_request(
        &query,
        Some(
            moli_selector::StyloDependencyInvalidationFallbackContext::from_mutation_relation(
                Some(parent),
                Some(first),
                Some(target),
            ),
        ),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries
    );
    assert!(
        target_queries[0]
            .retained_queries_for_test()
            .is_some_and(|queries| queries.contains(&query))
    );
    assert!(target_queries[0].fallback_reasons_for_test().is_empty());
    assert!(
        target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .is_empty()
    );
    assert!(
        target_queries[0]
            .exact_safety_fallback_root_set_for_test()
            .contains(&target)
    );
    assert!(
        !target_queries[0]
            .exact_safety_fallback_root_set_for_test()
            .contains(&document)
    );
}

#[test]
fn source_dependency_request_translation_keeps_nth_fallback_while_trying_nested_relative_exactly() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let first = host.create_element("div");
    let middle = host.create_element("div");
    let target = host.create_element("div");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, first));
    assert!(host.append_child(parent, middle));
    assert!(host.append_child(parent, target));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "section > div:nth-child(odd of :not(.c)) { color: red; }
         section:has(:is(.item + .item + .item > .child)) { background-color: blue; }",
        &base_url,
    );
    let matching_sources = vec![matching_source(
        source_id,
        &metadata.dependency_summary,
        vec![document],
    )];
    let nth_query = RetainedStyleInvalidationQuery::class(middle, "c".to_owned());
    let nested_query = RetainedStyleInvalidationQuery::class(middle, "item".to_owned());
    let context = moli_selector::StyloDependencyInvalidationFallbackContext::from_mutation_relation(
        Some(parent),
        Some(first),
        Some(target),
    );
    let requests = [
        exact_source_dependency_request(&nth_query, Some(context)),
        exact_source_dependency_request(&nested_query, Some(context)),
    ];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries
    );
    assert!(
        target_queries[0]
            .retained_queries_for_test()
            .is_some_and(|queries| !queries.contains(&nth_query) && queries.contains(&nested_query))
    );
    let reasons = target_queries[0].fallback_reasons_for_test();
    assert!(reasons.contains(&StyloSourceInvalidationFallbackReason::NthOfDependency));
    assert!(
        !reasons.contains(&StyloSourceInvalidationFallbackReason::NestedRelativeSelectorDependency)
    );
    assert!(
        !target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&document)
    );
    assert!(
        !target_queries[0]
            .exact_safety_fallback_root_set_for_test()
            .contains(&document)
    );
}

#[test]
fn source_dependency_request_translation_preserves_context_fallback_kind_with_retained_queries() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let first = host.create_element("div");
    let middle = host.create_element("div");
    let target = host.create_element("div");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, first));
    assert!(host.append_child(parent, middle));
    assert!(host.append_child(parent, target));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let style_text = ".exact { color: green; }
         section > div:nth-child(odd of :not(.c)) { color: red; }";
    let metadata = stylo_source_metadata_for_css_text(style_text, &base_url);
    let exact_dependency = metadata
        .dependency_summary
        .query_class(&style::Atom::from("exact"));
    assert!(
        exact_dependency.has_any_dependency(),
        "{exact_dependency:?}"
    );
    assert!(
        !exact_dependency.requires_fallback(),
        "{exact_dependency:?}"
    );
    let nth_dependency = metadata
        .dependency_summary
        .query_class(&style::Atom::from("c"));
    assert!(nth_dependency.requires_fallback(), "{nth_dependency:?}");
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        Vec::new(),
    )];
    let exact_query = RetainedStyleInvalidationQuery::class(target, "exact".to_owned());
    let nth_query = RetainedStyleInvalidationQuery::class(middle, "c".to_owned());
    let requests = [
        exact_source_dependency_request(&exact_query, None),
        exact_source_dependency_request(
            &nth_query,
            Some(
                moli_selector::StyloDependencyInvalidationFallbackContext::from_mutation_relation(
                    Some(parent),
                    Some(first),
                    Some(target),
                ),
            ),
        ),
    ];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries
    );
    assert_eq!(target_queries[0].retained_query_count(), 1);
    assert!(
        target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );
    let retained_source_input = retained_source_input_for_test(
        target_queries[0]
            .retained_source_invalidation_input()
            .into_stylo_input(None),
    );
    assert_eq!(
        retained_source_input.retained_fallback_kind,
        Some(Some(
            StyloRetainedSourceStyleInvalidationKind::ContextFallback
        ))
    );

    let mut engine = MoliStyleEngine::new();
    let source = StyloStylesheetSource::new(style_text.into(), base_url.clone())
        .with_source_id(Some(source_id));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);

    let application = engine
        .with_retained_style_system_for_document_for_test(document, |retained| {
            retained_source_invalidation_outcome_for_document_for_test(
                &engine,
                &host,
                document,
                StyleSourceDocumentContext::for_root_document(document),
                Some(retained),
                target_queries,
                false,
            )
        })
        .finalize(&host);
    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert_eq!(
        application.diagnostic_target_results()[0].kind(),
        StyleInvalidationDiagnosticTargetResultKind::ContextFallback
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .context_fallback_target_count(),
        1
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_scope_fallback_target_count(),
        0
    );
}

#[test]
fn source_dependency_request_translation_does_not_merge_exact_context_roots_into_context_fallback()
{
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let marker = host.create_element("div");
    let exact_context_only = host.create_element("div");
    let nth_subject = host.create_element("div");
    let nth_later = host.create_element("div");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, marker));
    assert!(host.append_child(parent, exact_context_only));
    assert!(host.append_child(parent, nth_subject));
    assert!(host.append_child(parent, nth_later));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let style_text = ".marker ~ .target { color: green; }
         section > div:nth-child(odd of :not(.c)) { color: red; }";
    let metadata = stylo_source_metadata_for_css_text(style_text, &base_url);
    let exact_dependency = metadata
        .dependency_summary
        .query_class(&style::Atom::from("marker"));
    assert!(
        exact_dependency.has_sibling_dependency(),
        "{exact_dependency:?}"
    );
    assert!(
        !exact_dependency.requires_fallback(),
        "{exact_dependency:?}"
    );
    let nth_dependency = metadata
        .dependency_summary
        .query_class(&style::Atom::from("c"));
    assert!(nth_dependency.requires_fallback(), "{nth_dependency:?}");

    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        Vec::new(),
    )];
    let exact_query = RetainedStyleInvalidationQuery::class(marker, "marker".to_owned());
    let nth_query = RetainedStyleInvalidationQuery::class(nth_subject, "c".to_owned());
    let requests = [
        exact_source_dependency_request(
            &exact_query,
            Some(
                moli_selector::StyloDependencyInvalidationFallbackContext::from_mutation_relation(
                    Some(parent),
                    None,
                    Some(exact_context_only),
                ),
            ),
        ),
        exact_source_dependency_request(
            &nth_query,
            Some(
                moli_selector::StyloDependencyInvalidationFallbackContext::from_mutation_relation(
                    Some(parent),
                    Some(exact_context_only),
                    Some(nth_later),
                ),
            ),
        ),
    ];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries
    );
    assert!(
        target_query
            .retained_queries_for_test()
            .is_some_and(|queries| queries.contains(&exact_query))
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );
    assert!(
        !target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&exact_context_only),
        "exact sibling query fallback roots must not be merged into the context fallback roots for another request"
    );
    assert!(
        target_query
            .exact_safety_fallback_root_set_for_test()
            .contains(&exact_context_only),
        "exact sibling query fallback roots must remain available when retained exact invalidation cannot run"
    );
}

#[test]
fn retained_unavailable_uses_exact_safety_roots_without_broadening_context_fallback() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let marker = host.create_element("div");
    let exact_context_only = host.create_element("div");
    let nth_subject = host.create_element("div");
    let nth_later = host.create_element("div");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, marker));
    assert!(host.append_child(parent, exact_context_only));
    assert!(host.append_child(parent, nth_subject));
    assert!(host.append_child(parent, nth_later));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let style_text = ".marker ~ .target { color: green; }
         section > div:nth-child(odd of :not(.c)) { color: red; }";
    let metadata = stylo_source_metadata_for_css_text(style_text, &base_url);
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        Vec::new(),
    )];
    let exact_query = RetainedStyleInvalidationQuery::class(marker, "marker".to_owned());
    let nth_query = RetainedStyleInvalidationQuery::class(nth_subject, "c".to_owned());
    let requests = [
        exact_source_dependency_request(
            &exact_query,
            Some(
                moli_selector::StyloDependencyInvalidationFallbackContext::from_mutation_relation(
                    Some(parent),
                    None,
                    Some(exact_context_only),
                ),
            ),
        ),
        exact_source_dependency_request(
            &nth_query,
            Some(
                moli_selector::StyloDependencyInvalidationFallbackContext::from_mutation_relation(
                    Some(parent),
                    Some(exact_context_only),
                    Some(nth_later),
                ),
            ),
        ),
    ];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert!(
        !target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&exact_context_only),
        "normal context fallback cleanup must not include exact-query safety roots"
    );
    assert!(
        target_query
            .exact_safety_fallback_root_set_for_test()
            .contains(&exact_context_only)
    );

    let mut engine = MoliStyleEngine::new();
    engine.set_document_adopted_style_sheet_sources(document, Vec::new());
    let application = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
        None,
        target_queries,
        false,
    )
    .finalize(&host);

    assert_eq!(application.diagnostic_target_results().len(), 1);
    let target_result = &application.diagnostic_target_results()[0];
    assert_eq!(
        target_result.kind(),
        StyleInvalidationDiagnosticTargetResultKind::MissingRetainedStyleSystem
    );
    assert!(
        target_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem)
    );
    assert!(
        target_result
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::SourceFallbackSubtreeRoots(roots)
            if roots.contains(&exact_context_only) && !roots.contains(&document)
    ));
}

#[test]
fn source_dependency_request_translation_marks_relative_previous_sibling_cleanup_from_stylo_plan() {
    let host = test_host();
    let document = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata =
        stylo_source_metadata_for_css_text("#subject:has(+ #old-next) { color: red; }", &base_url);
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![document],
    )];
    let query = RetainedStyleInvalidationQuery::id(document, "old-next".to_owned());
    let requests = [source_dependency_request(
        &query,
        None,
        StyloSourceDependencyRequestRequirement::relative_previous_sibling(),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[], &[document]),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].structural_boundary_cleanup_root_count(),
        1
    );
    assert!(
        target_queries[0]
            .retained_queries_for_test()
            .is_some_and(|queries| queries.contains(&query))
    );
}

#[test]
fn source_dependency_request_translation_skips_empty_target_fallback_for_non_structural_request() {
    let host = test_host();
    let document = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text("section:empty { color: red; }", &base_url);
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![document],
    )];
    let query = RetainedStyleInvalidationQuery::class(document, "unrelated".to_owned());
    let requests = [exact_source_dependency_request(&query, None)];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[document], &[]),
    );

    assert!(target_plan.boundary_fallback().is_none());
    assert!(target_plan.target_queries().is_empty());
}

#[test]
fn source_dependency_structural_request_keeps_empty_target_boundary_fallback() {
    let mut host = test_host();
    let document = host.document_handle();
    let empty_target_root = host.create_element("section");
    let relative_cleanup_root = host.create_element("article");
    assert!(host.append_child(document, empty_target_root));
    assert!(host.append_child(document, relative_cleanup_root));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text("section:empty { color: red; }", &base_url);
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![document],
    )];
    let query =
        RetainedStyleInvalidationQuery::element_type(empty_target_root, "section".to_owned());
    let requests = [source_dependency_request(
        &query,
        None,
        StyloSourceDependencyRequestRequirement::child_list_structural(),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[empty_target_root], &[relative_cleanup_root]),
    );

    assert!(target_plan.boundary_fallback().is_some());
    let resolution = target_plan.into_target_queries(&host, &PendingCauseFallback::default());
    let super::target_plan::SourceDependencyTargetPlanResolution::Finalized(target_queries) =
        resolution
    else {
        panic!("empty-target fallback should finalize target queries before base roots apply");
    };
    assert_eq!(target_queries.len(), 1);
    assert!(
        target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&empty_target_root)
    );
    assert!(
        !target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&relative_cleanup_root)
    );
    assert!(
        target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::InexactEmptyResult)
    );
}

#[test]
fn user_agent_structural_boundary_fallback_coexists_with_author_source_target() {
    let mut host = test_host();
    let document = host.document_handle();
    let details = host.create_element("details");
    let widget = host.create_element("div");
    assert!(host.append_child(document, details));
    assert!(host.append_child(document, widget));

    let source_scope = StyleSourceScope::for_document(document);
    let user_agent_source = super::source_record::MatchingStyleDependencySource::user_agent(
        &host,
        document,
        super::stylesheet::moli_user_agent_source_dependency_summary(),
        &source_scope,
    )
    .expect("document UA source should participate in document scope");
    let author_source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let author_metadata = stylo_source_metadata_for_css_text(".widget { color: red; }", &base_url);
    let matching_sources = vec![
        user_agent_source,
        matching_source(
            author_source_id.clone(),
            &author_metadata.dependency_summary,
            vec![document],
        ),
    ];
    let structural_query =
        RetainedStyleInvalidationQuery::element_type(details, "details".to_owned());
    let author_query = RetainedStyleInvalidationQuery::class(widget, "widget".to_owned());
    let requests = [
        source_dependency_request(
            &structural_query,
            None,
            StyloSourceDependencyRequestRequirement::child_list_structural(),
        ),
        exact_source_dependency_request(&author_query, None),
    ];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[details], &[]),
    );

    assert!(target_plan.boundary_fallback().is_some());
    assert!(target_plan.target_queries().iter().any(|query| {
        query.target() == &StyleInvalidationSourceTarget::stylesheet(author_source_id.clone())
    }));

    let resolution = target_plan.into_target_queries(&host, &PendingCauseFallback::default());
    let super::target_plan::SourceDependencyTargetPlanResolution::PendingBaseRoots(target_queries) =
        resolution
    else {
        panic!("structural fallback should coexist with source-local target work");
    };
    assert!(target_queries.iter().any(|query| {
        matches!(
            query.target(),
            StyleInvalidationSourceTarget::FallbackRoot { root, .. } if *root == details
        )
    }));
    assert!(target_queries.iter().any(|query| {
        query.target() == &StyleInvalidationSourceTarget::stylesheet(author_source_id.clone())
    }));
}

#[test]
fn user_agent_structural_metadata_skips_unrelated_boundary_type() {
    let mut host = test_host();
    let document = host.document_handle();
    let section = host.create_element("section");
    assert!(host.append_child(document, section));

    let source_scope = StyleSourceScope::for_document(document);
    let matching_sources = vec![
        super::source_record::MatchingStyleDependencySource::user_agent(
            &host,
            document,
            super::stylesheet::moli_user_agent_source_dependency_summary(),
            &source_scope,
        )
        .expect("document UA source should participate in document scope"),
    ];
    let query = RetainedStyleInvalidationQuery::element_type(section, "section".to_owned());
    let requests = [source_dependency_request(
        &query,
        None,
        StyloSourceDependencyRequestRequirement::child_list_structural(),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[section], &[]),
    );

    assert!(target_plan.target_queries().is_empty());
    assert!(target_plan.boundary_fallback().is_none());
}

#[test]
fn source_dependency_structural_request_without_boundary_roots_uses_source_fallback() {
    let host = test_host();
    let document = host.document_handle();

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text("section:empty { color: red; }", &base_url);
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![document],
    )];
    let query = RetainedStyleInvalidationQuery::element_type(document, "section".to_owned());
    let requests = [source_dependency_request(
        &query,
        None,
        StyloSourceDependencyRequestRequirement::child_list_structural(),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[], &[]),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::FallbackOnly
    );
    assert!(
        target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&document)
    );
    assert!(
        target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::InexactEmptyResult)
    );
}

#[test]
fn source_dependency_structural_request_without_any_fallback_roots_is_explicit_missing_roots() {
    let host = test_host();
    let document = host.document_handle();

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text("section:empty { color: red; }", &base_url);
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        Vec::new(),
    )];
    let query = RetainedStyleInvalidationQuery::element_type(document, "section".to_owned());
    let requests = [source_dependency_request(
        &query,
        None,
        StyloSourceDependencyRequestRequirement::child_list_structural(),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[], &[]),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots
    );
    assert!(
        target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::InexactEmptyResult)
    );
    assert!(
        target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
    );
}

#[test]
fn source_dependency_structural_request_without_boundary_roots_prefers_source_with_roots() {
    let host = test_host();
    let document = host.document_handle();

    let missing_source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let rooted_source_id = StyleSourceId::document_adopted_style_sheet(document, 1);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text("section:empty { color: red; }", &base_url);
    let matching_sources = vec![
        matching_source(
            missing_source_id.clone(),
            &metadata.dependency_summary,
            Vec::new(),
        ),
        matching_source(
            rooted_source_id.clone(),
            &metadata.dependency_summary,
            vec![document],
        ),
    ];
    let query = RetainedStyleInvalidationQuery::element_type(document, "section".to_owned());
    let requests = [source_dependency_request(
        &query,
        None,
        StyloSourceDependencyRequestRequirement::child_list_structural(),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[], &[]),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].target(),
        &StyleInvalidationSourceTarget::stylesheet(rooted_source_id)
    );
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::FallbackOnly
    );
    assert!(
        target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&document)
    );
    assert!(
        !target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
    );
}

#[test]
fn source_dependency_request_translation_keeps_exact_queries_after_structural_context_fallback() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let previous_sibling = host.create_element("div");
    let root = host.create_element("x-stateful");
    let next_sibling = host.create_element("div");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, previous_sibling));
    assert!(host.append_child(parent, root));
    assert!(host.append_child(parent, next_sibling));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "*:has(+ *) { color: red; }
         .widget:state(--active) { color: blue; }",
        &base_url,
    );
    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![document],
    )];
    let structural_query = RetainedStyleInvalidationQuery::universal(root);
    let custom_state_query =
        RetainedStyleInvalidationQuery::custom_state(root, "--active".to_owned());
    let context = moli_selector::StyloDependencyInvalidationFallbackContext::from_mutation_relation(
        Some(parent),
        Some(previous_sibling),
        Some(next_sibling),
    );
    let requests = [
        source_dependency_request(
            &structural_query,
            Some(context),
            StyloSourceDependencyRequestRequirement::child_list_structural(),
        ),
        exact_source_dependency_request(&custom_state_query, None),
    ];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::new(&[parent], &[]),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries
    );
    assert!(
        target_query
            .retained_queries_for_test()
            .is_some_and(|queries| queries.contains(&custom_state_query))
    );
    assert!(
        !target_query
            .retained_queries_for_test()
            .is_some_and(|queries| queries.contains(&structural_query))
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&previous_sibling)
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::InexactEmptyResult)
    );
}

#[test]
fn custom_state_source_dependency_request_stays_exact_without_unsupported_fallback() {
    let mut host = test_host();
    let document = host.document_handle();
    let root = host.create_element("x-stateful");
    assert!(host.append_child(document, root));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "x-stateful:state(--active) { color: blue; }",
        &base_url,
    );
    let dependency = metadata
        .dependency_summary
        .query_custom_state(&style::values::AtomIdent::from("--active"));
    assert!(dependency.has_any_dependency());
    assert!(!dependency.requires_fallback());

    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![document],
    )];
    let custom_state_query =
        RetainedStyleInvalidationQuery::custom_state(root, "--active".to_owned());
    let requests = [exact_source_dependency_request(&custom_state_query, None)];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries
    );
    assert!(
        target_query
            .retained_queries_for_test()
            .is_some_and(|queries| queries.contains(&custom_state_query))
    );
    assert!(target_query.fallback_reasons_for_test().is_empty());
}

#[test]
fn custom_state_nth_of_source_dependency_request_uses_context_fallback_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let root = host.create_element("x-stateful");
    let adjacent = host.create_element("p");
    let later = host.create_element("x-stateful");
    let later_adjacent = host.create_element("p");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, root));
    assert!(host.append_child(parent, adjacent));
    assert!(host.append_child(parent, later));
    assert!(host.append_child(parent, later_adjacent));

    let source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        ":nth-child(2 of :state(--active)) { color: green; }
         :nth-child(2 of :state(--active)) + p { color: blue; }",
        &base_url,
    );
    let dependency = metadata
        .dependency_summary
        .query_custom_state(&style::values::AtomIdent::from("--active"));
    assert!(dependency.has_any_dependency(), "{dependency:?}");
    assert!(dependency.has_sibling_dependency(), "{dependency:?}");

    let matching_sources = vec![matching_source(
        source_id.clone(),
        &metadata.dependency_summary,
        vec![document],
    )];
    let custom_state_query =
        RetainedStyleInvalidationQuery::custom_state(root, "--active".to_owned());
    let context = moli_selector::StyloDependencyInvalidationFallbackContext::from_mutation_relation(
        Some(parent),
        None,
        Some(adjacent),
    );
    let requests = [exact_source_dependency_request(
        &custom_state_query,
        Some(context),
    )];
    let target_plan = super::target_plan::pending_target_queries_for_source_dependency_requests(
        &host,
        &matching_sources,
        &requests,
        &[],
        StyloSourceDependencyBoundaryRoots::default(),
    );

    assert!(target_plan.boundary_fallback().is_none());
    let target_queries = target_plan.target_queries();
    assert_eq!(target_queries.len(), 1);
    let target_query = &target_queries[0];
    assert_eq!(
        target_query.kind(),
        StyloRetainedSourceStyleInvalidationKind::ContextFallback
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&adjacent),
        "{target_query:?}"
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&later),
        "{target_query:?}"
    );
    assert!(
        target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&later_adjacent),
        "{target_query:?}"
    );
    assert!(
        !target_query
            .reasoned_fallback_root_set_for_test()
            .contains(&document),
        "{target_query:?}"
    );
    assert!(
        target_query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency),
        "{target_query:?}"
    );
}

#[test]
fn pending_target_query_merge_promotes_fallback_payload_to_retained_source() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let mut target_queries = vec![
        PendingStyleInvalidationTargetQueries::source_fallback_for_test(
            source_id.clone(),
            [root],
            None,
        ),
    ];

    super::target_queries::merge_pending_target_queries(
        &mut target_queries,
        vec![PendingStyleInvalidationTargetQueries::retained_source(
            source_id,
            indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(root)]),
        )],
    );

    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries
    );
    assert_eq!(target_queries[0].retained_query_count(), 1);
    assert!(
        target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .contains(&root)
    );
}

#[test]
fn pending_target_query_merge_preserves_missing_root_availability_with_retained_source() {
    let host = test_host();
    let root = host.document_handle();
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let mut target_queries = vec![
        PendingStyleInvalidationTargetQueries::source_fallback_missing_roots_for_test(
            source_id.clone(),
            Some(StyloSourceInvalidationFallbackReason::FullSelector),
        ),
    ];

    super::target_queries::merge_pending_target_queries(
        &mut target_queries,
        vec![PendingStyleInvalidationTargetQueries::retained_source(
            source_id,
            indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(root)]),
        )],
    );

    assert_eq!(target_queries.len(), 1);
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries
    );
    assert!(
        target_queries[0]
            .reasoned_fallback_root_set_for_test()
            .is_empty()
    );
    assert!(
        target_queries[0]
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
    );
}

#[test]
fn style_invalidation_drain_summary_counts_pending_work() {
    let mut host = test_host();
    let root = host.document_handle();
    let shadow_host = host.create_element("div");
    assert!(host.append_child(root, shadow_host));
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("shadow root should attach");
    let source_id = StyleSourceId::document_adopted_style_sheet(root, 0);
    let shadow_source_id = StyleSourceId::shadow_root_adopted_style_sheet(shadow_root, 0);
    let fallback_target =
        StyleInvalidationSourceTarget::fallback_root(&host, root).expect("document target");
    let mut retained_target_query = PendingStyleInvalidationTargetQueries::retained_source(
        source_id.clone(),
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::class(
            root,
            "active".to_owned(),
        )]),
    );
    retained_target_query.extend_structural_boundary_cleanup_roots([root]);
    retained_target_query.extend_reasoned_fallback_roots_for_test([root]);
    let shadow_target_query = PendingStyleInvalidationTargetQueries::retained_source(
        shadow_source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::class(
            shadow_root,
            "shadow-active".to_owned(),
        )]),
    );
    let work_items = vec![
        PendingStyleInvalidationWork::for_test(
            PendingStyleInvalidationWorkKind::FocusChange,
            vec![retained_target_query, shadow_target_query],
        ),
        PendingStyleInvalidationWork::for_test(
            PendingStyleInvalidationWorkKind::Mutation,
            vec![
                PendingStyleInvalidationTargetQueries::fallback_roots_for_test(
                    fallback_target,
                    [root],
                ),
            ],
        ),
    ];

    let summary = StyleInvalidationDrainSummary::from_work_items(
        &host,
        root,
        StyleInvalidationDrainBoundary::ComputedStyleRead,
        &work_items,
    );

    assert_eq!(summary.document_for_test(), root);
    assert_eq!(
        summary.boundary_for_test(),
        StyleInvalidationDrainBoundary::ComputedStyleRead
    );
    assert_eq!(
        summary.counts_for_test(),
        [
            ("work_item_count", 2),
            ("mutation_work_item_count", 1),
            ("state_work_item_count", 0),
            ("custom_state_work_item_count", 0),
            ("focus_work_item_count", 1),
            ("target_work_item_count", 0),
            ("target_query_count", 3),
            ("retained_query_count", 2),
            ("mutation_snapshot_count", 0),
            ("structural_boundary_cleanup_root_count", 1),
            ("fallback_root_bucket_count", 2),
            ("source_scope_fallback_root_count", 3),
        ]
    );
    assert_eq!(
        summary.source_scope_fallback_roots_for_test(),
        vec![root, shadow_root, shadow_host]
    );
}
#[test]
fn retained_style_system_cache_hit_preserves_computed_and_retained_generations() {
    let engine = MoliStyleEngine::new();
    let host = test_host();
    let document = host.document_handle();
    let inputs = FullStyleWorldSnapshot::default();
    let key = StyleWorldKey::new(&inputs, None);

    engine.ensure_retained_style_system_for_document(
        &host,
        host.document_handle(),
        key.clone(),
        &inputs,
    );
    let computed_generation_after_first_build =
        engine.computed_cache_generation_for_document_for_test(document);
    let retained_generation_after_first_build =
        engine.retained_style_system_generation_for_document_for_test(document);
    engine.ensure_retained_style_system_for_document(&host, host.document_handle(), key, &inputs);

    assert_eq!(computed_generation_after_first_build, 1);
    assert_eq!(retained_generation_after_first_build, 1);
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        computed_generation_after_first_build
    );
    assert_eq!(
        engine.retained_style_system_generation_for_document_for_test(document),
        retained_generation_after_first_build
    );
}

#[test]
fn viewport_change_updates_the_persistent_style_world_in_place() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("main");
    let sibling = host.create_element("aside");
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, sibling));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![StyloStylesheetSource::new(
            "main { color: rgb(1, 2, 3); }\n\
             aside { color: rgb(7, 8, 9); }\n\
             @media (max-width: 700px) { main { color: rgb(4, 5, 6); } }"
                .to_owned(),
            document_url.clone(),
        )],
        ..Default::default()
    };
    let initial_viewport = StyleViewport::from_width(Some(800.0));
    let next_viewport = StyleViewport::from_width(Some(640.0));
    let initial_key = StyleWorldKey::new(&inputs, initial_viewport);
    let viewport_key = StyleWorldKey::new(&inputs, next_viewport);
    let mismatch = initial_key.mismatch_trace(&viewport_key);
    assert!(mismatch.viewport_changed);

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &inputs,
            initial_viewport,
        ),
        Some("rgb(1, 2, 3)".to_owned())
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            sibling,
            "color",
            None,
            &inputs,
            initial_viewport,
        ),
        Some("rgb(7, 8, 9)".to_owned())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    let computed_generation_after_cached_reads =
        engine.computed_cache_generation_for_document_for_test(document);
    let source_set_generation_after_cached_reads =
        engine.source_set_generation_for_document_for_test(document);
    let retained_generation_after_cached_reads =
        engine.retained_style_system_generation_for_document_for_test(document);
    let retained_identity_after_cached_reads =
        engine.retained_stylist_identity_for_document_for_test(document);
    let retained_rebuilds_after_cached_reads =
        engine.retained_style_system_rebuild_count_for_document_for_test(document);
    let retained_updates_after_cached_reads =
        engine.retained_style_system_update_count_for_document_for_test(document);

    engine.ensure_retained_style_system_for_document(&host, document, viewport_key, &inputs);

    assert_eq!(
        engine.source_set_generation_for_document_for_test(document),
        source_set_generation_after_cached_reads,
        "a device update must not advance source-set generation"
    );
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        computed_generation_after_cached_reads,
        "an in-place device update must not create a full-cache generation boundary"
    );
    assert!(
        engine.retained_style_system_generation_for_document_for_test(document)
            > retained_generation_after_cached_reads,
        "the observation generation must record the in-place device update"
    );
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        retained_identity_after_cached_reads,
        "viewport changes must retain the Document Stylist"
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        retained_rebuilds_after_cached_reads
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        retained_updates_after_cached_reads + 1
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &inputs,
            next_viewport,
        ),
        Some("rgb(4, 5, 6)".to_owned()),
        "the retained world must recascade media-query-dependent values"
    );
}
#[test]
fn stylesheet_source_clones_share_css_text_and_base_url() {
    let base_url = url::Url::parse("https://example.test/assets/app.css").unwrap();
    let source = StyloStylesheetSource::new(".probe { color: green; }".to_owned(), base_url);
    let cloned = source.clone();

    assert!(source.shares_source_storage_for_test(&cloned));
    assert!(StdArc::ptr_eq(&source.base_url, &cloned.base_url));
    assert_eq!(source.cache_key(), cloned.cache_key());
}
#[test]
fn source_scope_fallback_roots_preserve_unrelated_document_cache() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let engine = MoliStyleEngine::new();
    let active_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &active_url,
                active,
                "display",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &active_url,
                detached,
                "display",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );

    let source_scope = StyleSourceScope::for_document(document);
    let fallback_roots = source_scope_fallback_roots_for_test(&host, &source_scope);
    assert!(fallback_roots.contains(&document));
    assert!(!fallback_roots.contains(&detached_document));
    assert!(engine.invalidate_style_subtrees(&host, fallback_roots.iter().copied()));

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            detached_document,
            detached
        )
    );
}
#[test]
fn source_scope_fallback_roots_preserve_active_document_cache_for_detached_document() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let engine = MoliStyleEngine::new();
    let active_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &active_url,
                active,
                "display",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &active_url,
                detached,
                "display",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );

    let source_scope = StyleSourceScope::for_handle(&host, detached);
    let fallback_roots = source_scope_fallback_roots_for_test(&host, &source_scope);
    assert!(fallback_roots.contains(&detached_document));
    assert!(!fallback_roots.contains(&document));
    assert!(engine.invalidate_style_subtrees(&host, fallback_roots.iter().copied()));

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        0
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(
            detached_document,
            detached
        )
    );
}
#[test]
fn unsupported_state_change_without_snapshot_uses_source_scope_fallback() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("button");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let mut engine = MoliStyleEngine::new();
    let active_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [active, detached] {
        assert!(
            engine
                .computed_style_property_value(
                    &host,
                    &active_url,
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let source_scope = StyleSourceScope::for_handle(&host, active);
    let target_queries = {
        let world = engine.world_for_document(document);
        let linked_sources = world.linked_stylesheet_sources.borrow();
        let document_adopted_sources = world.adopted_style_sheet_sources.borrow();
        super::planner::target_queries_for_pending_cause_with_document_adopted_sources(
            &host,
            &linked_sources,
            &document_adopted_sources,
            &engine.dom_adapter,
            &media,
            StyleViewport::default(),
            host.document_handle(),
            &PendingStyleInvalidationCause::StateChange {
                element: active,
                state: StyloElementState::HOVER,
                old_state: None,
            },
            &source_scope,
        )
    };
    assert!(target_queries.iter().any(|query| {
        query
            .fallback_reasons_for_test()
            .contains(&StyloSourceInvalidationFallbackReason::UnsupportedStateDependency)
    }));

    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        active,
        StyloElementState::HOVER,
        None,
        &media,
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
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            detached_document,
            detached
        )
    );
}
#[test]
fn cache_key_source_matching_skips_inactive_owner_sources() {
    let mut host = test_host();
    let mut engine = MoliStyleEngine::new();
    let document = host.document_handle();
    let document_url = host.document_url().expect("test document url").clone();
    let css_text = ".shared { color: green; }";

    let connected_style = host.create_element("style");
    assert!(host.append_child(document, connected_style));
    engine.set_owner_style_sheet_text_with_host(&host, connected_style, css_text.into());
    let connected_style_id =
        StyleSourceId::owner_style_sheet(&host, connected_style).expect("connected style id");

    let disconnected_style = host.create_element("style");
    engine.set_owner_style_sheet_text_with_host(&host, disconnected_style, css_text.into());
    let disconnected_style_id =
        StyleSourceId::owner_style_sheet(&host, disconnected_style).expect("disconnected style id");

    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            css_text.to_owned(),
            document_url.clone(),
        ));

    engine.ensure_retained_style_system_for_document(
        &host,
        host.document_handle(),
        StyleWorldKey::new(&inputs, None),
        &inputs,
    );
    engine.with_retained_style_system_for_document_for_test(host.document_handle(), |retained| {
        assert!(
            retained
                .source_cascade_data
                .contains_key(&connected_style_id),
            "connected source should match the cache-key input source"
        );
        assert!(
            !retained
                .source_cascade_data
                .contains_key(&disconnected_style_id),
            "disconnected active-document source must not match by cache key"
        );
    });
}
