use super::*;

#[test]
fn retained_stylo_invalidator_narrows_sibling_cache_invalidation() {
    let mut host = test_host();
    let document = host.document_handle();
    let marker = host.create_element("div");
    let target = host.create_element("p");
    let unrelated = host.create_element("p");
    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, marker));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".marker + .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
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
                &document_url,
                unrelated,
                "display",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    assert!(host.set_attribute(marker, "class", ""));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: Some("marker".into()),
        new_value: Some(String::new()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_narrows_class_attribute_cache_invalidation() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("p");
    let unrelated = host.create_element("p");
    assert!(host.set_attribute(target, "class", "active"));
    assert!(host.set_attribute(unrelated, "class", "active"));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(".active { color: red; }".into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    for handle in [target, unrelated] {
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
        2
    );

    assert!(host.set_attribute(target, "class", ""));
    let effects = [StyleMutationEffect::Attribute {
        element: target,
        name: "class".into(),
        old_value: Some("active".into()),
        new_value: Some(String::new()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_accepts_empty_normal_sibling_result() {
    let mut host = test_host();
    let document = host.document_handle();
    let marker = host.create_element("div");
    let spacer = host.create_element("p");
    let target = host.create_element("p");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, marker));
    assert!(host.append_child(document, spacer));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".marker + .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
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

    assert!(host.set_attribute(marker, "class", "marker"));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: None,
        new_value: Some("marker".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}

#[test]
fn retained_stylo_invalidator_accepts_empty_scope_sibling_result() {
    let mut host = test_host();
    let document = host.document_handle();
    let marker = host.create_element("div");
    let spacer = host.create_element("p");
    let target = host.create_element("p");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, marker));
    assert!(host.append_child(document, spacer));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ":scope .marker + .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
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

    assert!(host.set_attribute(marker, "class", "marker"));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: None,
        new_value: Some("marker".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let source_scope = source_scope_for_mutations(&host, &effects).expect("source scope");
    let target_queries = {
        let world = engine.world_for_document(document);
        let linked_sources = world.linked_stylesheet_sources.borrow();
        let owner_sources = world.owner_style_sheet_sources.borrow();
        let adopted_sources = world.adopted_style_sheet_sources.borrow();
        super::planner::target_queries_for_pending_cause_with_adopted_sources(
            &host,
            &linked_sources,
            &owner_sources,
            &adopted_sources,
            &engine.dom_adapter,
            &media,
            StyleViewport::default(),
            document,
            &PendingStyleInvalidationCause::Mutation(effects.to_vec()),
            &source_scope,
        )
    };
    let application = prepared_retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
        &target_queries,
        false,
    )
    .finalize(&host);
    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::Noop
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::Noop
    );
    assert!(application.diagnostic_target_results()[0].exact());
    assert!(
        application.diagnostic_target_results()[0]
            .fallback_reasons()
            .is_empty()
    );

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}

#[test]
fn retained_stylo_invalidator_narrows_at_scope_dependency_to_affected_target() {
    let mut host = test_host();
    let document = host.document_handle();
    let scope = host.create_element("section");
    let marker = host.create_element("div");
    let target = host.create_element("p");
    let unrelated = host.create_element("p");
    assert!(host.set_attribute(scope, "class", "scope"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, scope));
    assert!(host.append_child(scope, marker));
    assert!(host.append_child(scope, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".target { color: rgb(0, 0, 255); }
         @scope (.scope) {
             .marker + .target { color: rgb(255, 0, 0); }
         }"
        .into(),
        document_url.clone(),
    )
    .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
        document, 0,
    )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    for handle in [target, unrelated] {
        assert_eq!(
            engine.computed_style_property_value(
                &host,
                &document_url,
                handle,
                "color",
                None,
                &inputs,
                None,
            ),
            Some("rgb(0, 0, 255)".into())
        );
    }
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    assert!(host.set_attribute(marker, "class", "marker"));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: None,
        new_value: Some("marker".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let source_scope = source_scope_for_mutations(&host, &effects).expect("source scope");
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
            document,
            &PendingStyleInvalidationCause::Mutation(effects.to_vec()),
            &source_scope,
        )
    };
    assert_eq!(target_queries.len(), 1, "{target_queries:#?}");
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries,
        "{target_queries:#?}"
    );
    let application = prepared_retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
        &target_queries,
        false,
    )
    .finalize(&host);
    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::ExactAffectedSubtreeRoots
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::ExactAffectedSubtreeRoots(roots)
            if roots.contains(&target)
                && !roots.contains(&scope)
                && !roots.contains(&unrelated)
                && !roots.contains(&document)
    ));
    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert!(application.diagnostic_target_results()[0].exact());
    assert!(
        application.diagnostic_target_results()[0]
            .fallback_reasons()
            .is_empty()
    );

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(255, 0, 0)".into())
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            unrelated,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 0, 255)".into())
    );
}

#[test]
fn retained_stylo_invalidator_accepts_exact_empty_at_scope_dependency_result() {
    let mut host = test_host();
    let document = host.document_handle();
    let scope = host.create_element("section");
    let marker = host.create_element("div");
    let spacer = host.create_element("span");
    let target = host.create_element("p");
    let unrelated = host.create_element("p");
    assert!(host.set_attribute(scope, "class", "scope"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, scope));
    assert!(host.append_child(scope, marker));
    assert!(host.append_child(scope, spacer));
    assert!(host.append_child(scope, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".target { color: rgb(0, 0, 255); }
         @scope (.scope) {
             .marker + .target { color: rgb(255, 0, 0); }
         }"
        .into(),
        document_url.clone(),
    )
    .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
        document, 0,
    )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    for handle in [target, unrelated] {
        assert_eq!(
            engine.computed_style_property_value(
                &host,
                &document_url,
                handle,
                "color",
                None,
                &inputs,
                None,
            ),
            Some("rgb(0, 0, 255)".into())
        );
    }
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    assert!(host.set_attribute(marker, "class", "marker"));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: None,
        new_value: Some("marker".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let source_scope = source_scope_for_mutations(&host, &effects).expect("source scope");
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
            document,
            &PendingStyleInvalidationCause::Mutation(effects.to_vec()),
            &source_scope,
        )
    };
    assert_eq!(target_queries.len(), 1, "{target_queries:#?}");
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries,
        "{target_queries:#?}"
    );
    let application = prepared_retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
        &target_queries,
        false,
    )
    .finalize(&host);
    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::Noop
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::Noop
    );
    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert!(application.diagnostic_target_results()[0].exact());
    assert!(
        application.diagnostic_target_results()[0]
            .fallback_reasons()
            .is_empty()
    );

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_keeps_self_dependency_inheritance_safe() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let parent = host.create_element("section");
    let child = host.create_element("span");
    let unrelated = host.create_element("p");

    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, child));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source =
        StyloStylesheetSource::new(".marker { color: blue; }".into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [outside, parent, child, unrelated] {
        assert!(
            engine
                .computed_style_property_value(
                    &host,
                    &document_url,
                    handle,
                    "color",
                    None,
                    &inputs,
                    None,
                )
                .is_some()
        );
    }
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        4
    );
    assert!(host.set_attribute(parent, "class", "marker"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: parent,
            name: "class".into(),
            old_value: None,
            new_value: Some("marker".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        3,
        "only the matched parent is evicted eagerly; its child inherits the lazy root"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, parent));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, child));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            child,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 0, 255)".into()),
        "reading the child must recascade the matched parent before inheriting"
    );
}

#[test]
fn retained_stylo_invalidator_ignores_unrelated_shadow_cascade_data() {
    let mut host = test_host();
    let document = host.document_handle();
    let marker = host.create_element("div");
    let target = host.create_element("p");
    let unrelated = host.create_element("p");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_child = host.create_element("span");

    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, marker));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let document_source = StyloStylesheetSource::new(
        ".marker + .target { color: red; }".into(),
        document_url.clone(),
    );
    let shadow_source =
        StyloStylesheetSource::new(".shadow-only { color: blue; }".into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![document_source.clone()]);
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(document_source);
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));

    for handle in [target, unrelated] {
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
        2
    );

    assert!(host.set_attribute(marker, "class", ""));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: Some("marker".into()),
        new_value: Some(String::new()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_ignores_out_of_scope_shadow_cascade_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let marker = host.create_element("div");
    let target = host.create_element("p");
    let unrelated = host.create_element("p");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_child = host.create_element("span");

    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, marker));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let document_source = StyloStylesheetSource::new(
        ".marker + .target { color: red; }".into(),
        document_url.clone(),
    );
    let shadow_source =
        StyloStylesheetSource::new(".marker { color: blue; }".into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![document_source.clone()]);
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(document_source);
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));

    for handle in [target, unrelated] {
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
        2
    );

    assert!(host.set_attribute(marker, "class", ""));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: Some("marker".into()),
        new_value: Some(String::new()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_falls_back_for_in_scope_shadow_cascade_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_child = host.create_element("span");

    assert!(host.set_attribute(shadow_child, "class", "marker"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source =
        StyloStylesheetSource::new(".marker { color: blue; }".into(), document_url.clone());
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));

    for handle in [outside, shadow_child] {
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
        2
    );

    assert!(host.set_attribute(shadow_child, "class", ""));
    let effects = [StyleMutationEffect::Attribute {
        element: shadow_child,
        name: "class".into(),
        old_value: Some("marker".into()),
        new_value: Some(String::new()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
}

#[test]
fn retained_stylo_invalidator_narrows_shadow_tree_sibling_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let marker = host.create_element("span");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, marker));
    assert!(host.append_child(shadow_root, target));
    assert!(host.append_child(shadow_root, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ".marker + .target { color: blue; }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));

    for handle in [outside, target, unrelated] {
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
        3
    );

    assert!(host.set_attribute(marker, "class", ""));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: Some("marker".into()),
        new_value: Some(String::new()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_treats_shadow_host_as_shadow_scope() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_child = host.create_element("span");

    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ":host(.marker) span { color: blue; }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));

    for handle in [outside, shadow_child] {
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
        2
    );

    assert!(host.set_attribute(shadow_host, "class", "marker"));
    let effects = [StyleMutationEffect::Attribute {
        element: shadow_host,
        name: "class".into(),
        old_value: None,
        new_value: Some("marker".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
}

#[test]
fn retained_stylo_invalidator_narrows_shadow_host_descendant_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "other"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, target));
    assert!(host.append_child(shadow_root, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ":host(.marker) .target { color: blue; }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));

    for handle in [outside, target, unrelated] {
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
        3
    );

    assert!(host.set_attribute(shadow_host, "class", "marker"));
    let effects = [StyleMutationEffect::Attribute {
        element: shadow_host,
        name: "class".into(),
        old_value: None,
        new_value: Some("marker".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_uses_shadow_relative_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("div");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let marker = host.create_element("span");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(marker, "class", "other"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, marker));
    assert!(host.append_child(shadow_root, target));
    assert!(host.append_child(shadow_root, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ":host:has(.marker) .target { color: green; }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));

    for handle in [outside, target, unrelated] {
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
        3
    );

    assert!(host.set_attribute(marker, "class", "marker"));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: Some("other".into()),
        new_value: Some("marker".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_lazily_refreshes_shadow_tree_has_descendants() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_child = host.create_element("span");

    assert!(host.set_attribute(shadow_child, "class", "other"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ":host:has(.descendant) { color: rgb(0, 128, 0); }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));

    for handle in [outside, shadow_host, shadow_child] {
        assert!(
            engine
                .computed_style_property_value(
                    &host,
                    &document_url,
                    handle,
                    "color",
                    None,
                    &inputs,
                    None,
                )
                .is_some()
        );
    }
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    assert!(host.set_attribute(shadow_child, "class", "descendant"));
    let effects = [StyleMutationEffect::Attribute {
        element: shadow_child,
        name: "class".into(),
        old_value: Some("other".into()),
        new_value: Some("descendant".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2,
        "the shadow child remains published behind the dirty shadow-host root"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_host)
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            shadow_host,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 128, 0)".into())
    );
}

#[test]
fn retained_stylo_invalidator_lazily_refreshes_shadow_host_context_subject() {
    let mut host = test_host();
    let document = host.document_handle();
    let host_parent = host.create_element("div");
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let wrapper = host.create_element("div");
    let subject = host.create_element("span");
    let bar = host.create_element("span");

    assert!(host.set_attribute(subject, "class", "subject"));
    assert!(host.set_attribute(bar, "class", "bar"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, host_parent));
    assert!(host.append_child(host_parent, shadow_host));
    assert!(host.append_child(shadow_root, wrapper));
    assert!(host.append_child(wrapper, subject));
    assert!(host.append_child(subject, bar));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ".subject { color: rgb(255, 0, 0); }
         .subject:has(:is(:host-context(.active) .bar)) { color: rgb(0, 128, 0); }"
            .into(),
        document_url.clone(),
    );
    let metadata =
        stylo_source_metadata_for_css_text(&shadow_source.serialized_css_text(), &document_url);
    assert!(
        metadata
            .dependency_summary
            .query_class(&style::Atom::from("active"))
            .has_any_dependency()
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));

    for handle in [outside, subject] {
        assert!(
            engine
                .computed_style_property_value(
                    &host,
                    &document_url,
                    handle,
                    "color",
                    None,
                    &inputs,
                    None,
                )
                .is_some()
        );
    }
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    assert!(host.set_attribute(host_parent, "class", "active"));
    let effects = [StyleMutationEffect::Attribute {
        element: host_parent,
        name: "class".into(),
        old_value: None,
        new_value: Some("active".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2,
        "the shadow subject remains published until its dirty scope is observed"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            subject,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 128, 0)".into())
    );
}

#[test]
fn retained_stylo_invalidator_keeps_shadow_host_self_dependency_conservative() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_child = host.create_element("span");

    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ":host(.marker) { color: blue; }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));

    for handle in [outside, shadow_host, shadow_child] {
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
        3
    );

    assert!(host.set_attribute(shadow_host, "class", "marker"));
    let effects = [StyleMutationEffect::Attribute {
        element: shadow_host,
        name: "class".into(),
        old_value: None,
        new_value: Some("marker".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2,
        "the host is evicted directly while its shadow child is invalidated lazily"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_host)
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            shadow_child,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 0, 255)".into()),
        "the shadow child must inherit the refreshed host style on demand"
    );
}

#[test]
fn retained_stylo_invalidator_ignores_unrelated_relative_selector_fallback() {
    let mut host = test_host();
    let document = host.document_handle();
    let marker = host.create_element("div");
    let target = host.create_element("p");
    let unrelated = host.create_element("p");
    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, marker));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".marker + .target { color: red; } section:has(*) { display: block; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    for handle in [target, unrelated] {
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
        2
    );

    assert!(host.set_attribute(marker, "class", ""));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: Some("marker".into()),
        new_value: Some(String::new()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_uses_snapshot_roots_for_has_class_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let subject = host.create_element("section");
    let marker = host.create_element("div");
    let unrelated = host.create_element("section");
    assert!(host.set_attribute(subject, "class", "subject"));
    assert!(host.set_attribute(unrelated, "class", "subject"));
    assert!(host.append_child(document, subject));
    assert!(host.append_child(subject, marker));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".subject { color: rgb(0, 0, 255); }
         .subject:has(.marker) { color: rgb(255, 0, 0); }"
            .into(),
        document_url.clone(),
    )
    .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
        document, 0,
    )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    for handle in [subject, unrelated] {
        assert_eq!(
            engine.computed_style_property_value(
                &host,
                &document_url,
                handle,
                "color",
                None,
                &inputs,
                None,
            ),
            Some("rgb(0, 0, 255)".into())
        );
    }
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    assert!(host.set_attribute(marker, "class", "marker"));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: None,
        new_value: Some("marker".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            subject,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(255, 0, 0)".into())
    );
}

#[test]
fn retained_stylo_invalidator_accepts_empty_snapshot_relative_result_for_has_class_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let subject = host.create_element("section");
    let unrelated = host.create_element("section");
    let outside = host.create_element("div");
    assert!(host.set_attribute(subject, "class", "subject"));
    assert!(host.set_attribute(unrelated, "class", "subject"));
    assert!(host.append_child(document, subject));
    assert!(host.append_child(document, unrelated));
    assert!(host.append_child(document, outside));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".subject { color: rgb(0, 0, 255); }
         .subject:has(.marker) { color: rgb(255, 0, 0); }"
            .into(),
        document_url.clone(),
    )
    .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
        document, 0,
    )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    for handle in [subject, unrelated] {
        assert_eq!(
            engine.computed_style_property_value(
                &host,
                &document_url,
                handle,
                "color",
                None,
                &inputs,
                None,
            ),
            Some("rgb(0, 0, 255)".into())
        );
    }
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    assert!(host.set_attribute(outside, "class", "marker"));
    let effects = [StyleMutationEffect::Attribute {
        element: outside,
        name: "class".into(),
        old_value: None,
        new_value: Some("marker".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let source_scope = source_scope_for_mutations(&host, &effects).expect("source scope");
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
            document,
            &PendingStyleInvalidationCause::Mutation(effects.to_vec()),
            &source_scope,
        )
    };
    let application = prepared_retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
        &target_queries,
        false,
    )
    .finalize(&host);
    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::Noop
    );
    assert_eq!(
        application.cleanup_class(),
        StyleInvalidationCleanupClass::Noop
    );
    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert!(application.diagnostic_target_results()[0].exact());
    assert!(
        application.diagnostic_target_results()[0]
            .fallback_reasons()
            .is_empty()
    );

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn target_query_drain_uses_selector_wrapper_fallback_without_retained_system() {
    let mut host = test_host();
    let document = host.document_handle();
    let marker = host.create_element("div");
    let target = host.create_element("p");
    let unrelated = host.create_element("p");
    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, marker));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".marker + .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    for handle in [target, unrelated] {
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
        2
    );

    engine.clear_retained_style_system_for_document_for_test(document);
    assert!(host.set_attribute(marker, "class", ""));
    let effects = [StyleMutationEffect::Attribute {
        element: marker,
        name: "class".into(),
        old_value: Some("marker".into()),
        new_value: Some(String::new()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}
