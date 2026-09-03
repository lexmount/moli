use super::*;

#[test]
fn retained_stylo_invalidator_refreshes_nth_child_of_selector_list_on_class_change() {
    let mut host = test_host();
    let document = host.document_handle();
    let body = host.create_element("body");
    let container = host.create_element("section");
    let first = host.create_element("div");
    let middle = host.create_element("div");
    let target = host.create_element("div");

    assert!(host.set_attribute(middle, "class", "c"));
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, container));
    assert!(host.append_child(container, first));
    assert!(host.append_child(container, middle));
    assert!(host.append_child(container, target));

    let mut engine = MoliStyleEngine::new();
    let style_text =
        "section > div:nth-child(odd of :not(.c)) { background-color: rgb(192, 192, 192); }
         section > div { color: rgb(1, 2, 3); }
         .c * { color: rgb(1, 2, 3); }";
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let metadata = stylo_source_metadata_for_css_text(style_text, &document_url);
    assert!(
        metadata
            .dependency_summary
            .query_class(&style::Atom::from("c"))
            .has_sibling_dependency()
    );
    let source = StyloStylesheetSource::new(style_text.into(), document_url.clone())
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(192, 192, 192)".into())
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgba(0, 0, 0, 0)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    assert!(host.set_attribute(middle, "class", ""));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: middle,
            name: "class".into(),
            old_value: Some("c".into()),
            new_value: Some("".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(192, 192, 192)".into())
    );
}

#[test]
fn retained_stylo_invalidator_refreshes_nth_child_of_custom_state_with_structural_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let body = host.create_element("body");
    let container = host.create_element("section");
    let first = host.create_element("x-stateful");
    let middle = host.create_element("x-stateful");
    let target = host.create_element("x-stateful");
    let unrelated = host.create_element("aside");

    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    assert!(host.insert_custom_state(first, "--active"));
    assert!(host.insert_custom_state(target, "--active"));
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, container));
    assert!(host.append_child(container, first));
    assert!(host.append_child(container, middle));
    assert!(host.append_child(container, target));
    assert!(host.append_child(body, unrelated));

    let mut engine = MoliStyleEngine::new();
    let style_text = "section > x-stateful:nth-child(odd of :state(--active)) {
             background-color: rgb(192, 192, 192);
         }
         #unrelated { color: rgb(1, 2, 3); }";
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(style_text.into(), document_url.clone())
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgba(0, 0, 0, 0)".into())
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
        Some("rgb(1, 2, 3)".into())
    );

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let old_custom_states = host.custom_state_names(middle);
    assert!(host.insert_custom_state(middle, "--active"));
    let source_scope = crate::style_engine::scope::source_scope_for_custom_state_change(
        &host,
        middle,
        &["--active".to_owned()],
    )
    .expect("custom state scope");
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
            &PendingStyleInvalidationCause::CustomStateChange {
                element: middle,
                state_names: vec!["--active".to_owned()],
                old_custom_states: old_custom_states.clone(),
            },
            &source_scope,
        )
    };
    assert_eq!(target_queries.len(), 1, "{target_queries:#?}");
    assert_eq!(
        target_queries[0].kind(),
        StyloRetainedSourceStyleInvalidationKind::RetainedQueries,
        "{target_queries:#?}"
    );
    assert!(
        target_queries[0]
            .structural_boundary_cleanup_roots_for_test()
            .contains(&middle)
    );
    assert!(
        target_queries[0]
            .structural_boundary_cleanup_roots_for_test()
            .contains(&target)
    );
    assert!(
        !target_queries[0]
            .structural_boundary_cleanup_roots_for_test()
            .contains(&unrelated)
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
        StyleInvalidationCleanupTargetKind::MixedSubtreeRoots
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::MixedSubtreeRoots(groups)
            if groups.structural_boundary_roots().contains(&target)
                && !groups.structural_boundary_roots().contains(&unrelated)
                && !groups.exact_affected_roots().contains(&unrelated)
    ));
    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert!(application.diagnostic_target_results()[0].exact());
    assert!(
        !application.diagnostic_target_results()[0]
            .fallback_reasons()
            .contains(&StyloSourceInvalidationFallbackReason::NthOfDependency)
    );
    let telemetry = application.fallback_telemetry();
    assert_eq!(telemetry.exact_target_result_count(), 1);
    assert_eq!(telemetry.fallback_target_result_count(), 0);
    assert_eq!(
        telemetry
            .fallback_reason_target_count(StyloSourceInvalidationFallbackReason::NthOfDependency,),
        0
    );

    engine.invalidate_for_custom_state_change(
        &host,
        middle,
        vec!["--active".to_owned()],
        old_custom_states,
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(192, 192, 192)".into())
    );
}

#[test]
fn retained_stylo_invalidator_custom_state_snapshot_avoids_source_fallback() {
    let mut host = test_host();
    let document = host.document_handle();
    let body = host.create_element("body");
    let subject = host.create_element("section");
    let target = host.create_element("x-stateful");
    let unrelated = host.create_element("aside");

    assert!(host.set_attribute(subject, "id", "subject"));
    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, subject));
    assert!(host.append_child(subject, target));
    assert!(host.append_child(body, unrelated));

    let mut engine = MoliStyleEngine::new();
    let style_text = "#subject { background-color: rgb(255, 0, 0); }
         #subject:has(:state(--active)) { background-color: rgb(0, 128, 0); }
         #unrelated { color: rgb(1, 2, 3); }";
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let metadata = stylo_source_metadata_for_css_text(style_text, &document_url);
    let dependency = metadata
        .dependency_summary
        .query_custom_state(&style::values::AtomIdent::from("--active"));
    assert!(dependency.has_any_dependency(), "{dependency:?}");
    assert!(!dependency.requires_fallback(), "{dependency:?}");
    assert!(
        dependency.has_relative_ancestors_dependency(),
        "{dependency:?}"
    );
    let source = StyloStylesheetSource::new(style_text.into(), document_url.clone())
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            subject,
            "background-color",
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
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let old_custom_states = host.custom_state_names(target);
    assert!(host.insert_custom_state(target, "--active"));
    let source_scope = crate::style_engine::scope::source_scope_for_custom_state_change(
        &host,
        target,
        &["--active".to_owned()],
    )
    .expect("custom state scope");
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
            &PendingStyleInvalidationCause::CustomStateChange {
                element: target,
                state_names: vec!["--active".to_owned()],
                old_custom_states: old_custom_states.clone(),
            },
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
        StyleInvalidationCleanupTargetKind::ExactAffectedSubtreeRoots
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::ExactAffectedSubtreeRoots(roots)
            if roots.contains(&subject) && !roots.contains(&unrelated)
    ));
    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert!(application.diagnostic_target_results()[0].exact());
    assert!(
        application.diagnostic_target_results()[0]
            .fallback_reasons()
            .is_empty()
    );

    engine.invalidate_for_custom_state_change(
        &host,
        target,
        vec!["--active".to_owned()],
        old_custom_states,
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            subject,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 128, 0)".into())
    );

    let old_custom_states = host.custom_state_names(target);
    assert!(host.remove_custom_state(target, "--active"));
    engine.invalidate_for_custom_state_change(
        &host,
        target,
        vec!["--active".to_owned()],
        old_custom_states,
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            subject,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(255, 0, 0)".into())
    );
}

#[test]
fn custom_state_batch_invalidation_builds_queries_for_each_changed_state() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("x-stateful");
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let style = host.create_element("style");
    assert!(host.append_child(document, style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "x-stateful:state(--active), x-stateful:state(--enabled) { color: red; }".into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
                "color",
                None,
                &inputs,
                None,
            )
            .is_some()
    );

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let state_names = vec!["--active".to_owned(), "--enabled".to_owned()];
    let source_scope = crate::style_engine::scope::source_scope_for_custom_state_change(
        &host,
        target,
        &state_names,
    )
    .expect("custom state scope");
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
            host.document_handle(),
            &PendingStyleInvalidationCause::CustomStateChange {
                element: target,
                state_names: state_names.clone(),
                old_custom_states: state_names.clone(),
            },
            &source_scope,
        )
    };

    assert_eq!(target_queries.len(), 1);
    let retained_queries = target_queries[0]
        .retained_queries_for_test()
        .expect("custom state batch should use retained queries");
    assert!(
        retained_queries.contains(&RetainedStyleInvalidationQuery::custom_state(
            target,
            "--active".to_owned()
        ))
    );
    assert!(
        retained_queries.contains(&RetainedStyleInvalidationQuery::custom_state(
            target,
            "--enabled".to_owned()
        ))
    );

    let old_custom_states = state_names.clone();
    assert!(host.insert_custom_state(target, "--active"));
    assert!(host.insert_custom_state(target, "--enabled"));
    assert!(host.clear_custom_states(target));
    engine.invalidate_for_custom_state_change(
        &host,
        target,
        state_names,
        old_custom_states,
        &media,
    );
    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.pending_style_invalidation_work_kind_names_for_document_for_test(document),
        vec!["custom-state"]
    );
}

#[test]
fn retained_stylo_invalidator_refreshes_nested_is_sibling_has_on_child_removal() {
    let mut host = test_host();
    let document = host.document_handle();
    let body = host.create_element("body");
    let target = host.create_element("section");
    let first_item = host.create_element("div");
    let second_item = host.create_element("div");
    let third_item = host.create_element("div");
    let first_child = host.create_element("span");
    let second_child = host.create_element("span");
    let third_child = host.create_element("span");
    let unrelated = host.create_element("aside");

    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    for item in [first_item, second_item, third_item] {
        assert!(host.set_attribute(item, "class", "item"));
    }
    for child in [first_child, second_child, third_child] {
        assert!(host.set_attribute(child, "class", "child"));
    }
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, target));
    assert!(host.append_child(target, first_item));
    assert!(host.append_child(target, second_item));
    assert!(host.append_child(target, third_item));
    assert!(host.append_child(third_item, first_child));
    assert!(host.append_child(third_item, second_child));
    assert!(host.append_child(third_item, third_child));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let style_text = "#target { color: rgb(0, 128, 0); }
         #target:has(:is(.item + .item + .item > .child + .child + .child)) {
             color: rgb(192, 192, 192);
         }
         #unrelated { color: rgb(1, 2, 3); }";
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let metadata = stylo_source_metadata_for_css_text(style_text, &document_url);
    assert!(
        metadata
            .dependency_summary
            .query_class(&style::Atom::from("item"))
            .requires_fallback()
    );
    let source = StyloStylesheetSource::new(style_text.into(), document_url.clone())
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
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
        Some("rgb(192, 192, 192)".into())
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
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    let removed_snapshots = removed_element_dependency_snapshots(&host, &[first_item]);
    assert!(host.remove_child(target, first_item));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: target,
            added_nodes: Vec::new(),
            removed_nodes: vec![first_item],
            removed_element_snapshots: removed_snapshots,
            previous_sibling: None,
            next_sibling: Some(second_item),
        }],
        &media,
    );
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
        Some("rgb(0, 128, 0)".into())
    );
}

#[test]
fn retained_stylo_invalidator_refreshes_nested_is_sibling_has_on_middle_insertion() {
    let mut host = test_host();
    let document = host.document_handle();
    let body = host.create_element("body");
    let outer = host.create_element("div");
    let first = host.create_element("div");
    let previous = host.create_element("div");
    let parent = host.create_element("div");
    let target = host.create_element("section");
    let child = host.create_element("div");
    let descendant = host.create_element("div");
    let unrelated = host.create_element("aside");

    assert!(host.set_attribute(first, "class", "p"));
    assert!(host.set_attribute(previous, "id", "parent_previous"));
    assert!(host.set_attribute(previous, "class", "c_has_scope"));
    assert!(host.set_attribute(parent, "class", "d"));
    assert!(host.set_attribute(target, "id", "has_scope"));
    assert!(host.set_attribute(target, "class", "green d"));
    assert!(host.set_attribute(child, "class", "d"));
    assert!(host.set_attribute(descendant, "id", "descendant"));
    assert!(host.set_attribute(descendant, "class", "e"));
    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, outer));
    assert!(host.append_child(outer, first));
    assert!(host.append_child(outer, previous));
    assert!(host.append_child(outer, parent));
    assert!(host.append_child(parent, target));
    assert!(host.append_child(target, child));
    assert!(host.append_child(child, descendant));
    assert!(host.append_child(body, unrelated));

    let mut engine = MoliStyleEngine::new();
    let style_text = "#has_scope { color: rgb(128, 128, 128); }
         .green:has(#descendant:is(.p + .c_has_scope ~ .d .e)) {
             color: rgb(0, 128, 0);
         }
         #unrelated { color: rgb(1, 2, 3); }";
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let metadata = stylo_source_metadata_for_css_text(style_text, &document_url);
    assert!(
        metadata
            .dependency_summary
            .query_class(&style::Atom::from("c_has_scope"))
            .requires_fallback()
    );
    let source = StyloStylesheetSource::new(style_text.into(), document_url.clone())
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
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
        Some("rgb(0, 128, 0)".into())
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
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    let inserted = host.create_element("div");
    assert!(host.set_attribute(inserted, "class", "invalid"));
    assert!(host.insert_before(outer, inserted, Some(previous)));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: outer,
            added_nodes: vec![inserted],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: Some(first),
            next_sibling: Some(previous),
        }],
        &media,
    );
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
        Some("rgb(128, 128, 128)".into())
    );
}

#[test]
fn retained_stylo_invalidator_refreshes_has_any_link_on_href_insertion() {
    let mut host = test_host();
    let document = host.document_handle();
    let body = host.create_element("body");
    let target = host.create_element("section");

    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, target));

    let mut engine = MoliStyleEngine::new();
    let style_text = "#target { color: rgb(0, 0, 255); }
         #target:has(:any-link) { color: rgb(0, 128, 0); }";
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let metadata = stylo_source_metadata_for_css_text(style_text, &document_url);
    assert!(
        metadata
            .dependency_summary
            .query_attribute(&LocalName::from("href"))
            .has_any_dependency()
    );
    let source = StyloStylesheetSource::new(style_text.into(), document_url.clone())
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
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
        Some("rgb(0, 0, 255)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let link = host.create_element("a");
    assert!(host.set_attribute(link, "href", "https://example.test/link"));
    assert!(host.append_child(target, link));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: target,
            added_nodes: vec![link],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: None,
            next_sibling: None,
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
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
        Some("rgb(0, 128, 0)".into())
    );
}

#[test]
fn retained_stylo_invalidator_refreshes_has_not_any_link_on_plain_child_insertion() {
    let mut host = test_host();
    let document = host.document_handle();
    let body = host.create_element("body");
    let grandparent = host.create_element("section");
    let style_text = "#parent { color: rgb(0, 0, 255); }
         #grandparent { color: rgb(0, 0, 255); }
         #parent:has(> :not(:link)) { color: rgb(128, 128, 128); }
         #parent:has(> :link) { color: rgb(0, 128, 0); }
         #parent:has(> :visited) { color: rgb(255, 0, 0); }
         #grandparent:has(:not(:any-link)) { color: rgb(128, 128, 128); }
         #grandparent:has(:any-link) { color: rgb(0, 128, 0); }";

    assert!(host.set_attribute(grandparent, "id", "grandparent"));
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, grandparent));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let metadata = stylo_source_metadata_for_css_text(style_text, &document_url);
    assert!(
        metadata
            .dependency_summary
            .query_universal()
            .has_any_dependency()
    );
    let source = StyloStylesheetSource::new(style_text.into(), document_url.clone())
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        )));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            grandparent,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 0, 255)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let parent = host.create_element("div");
    assert!(host.set_attribute(parent, "id", "parent"));
    assert!(host.append_child(grandparent, parent));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let effects = [StyleMutationEffect::ChildList {
        parent: grandparent,
        added_nodes: vec![parent],
        removed_nodes: Vec::new(),
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, grandparent)
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            grandparent,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(128, 128, 128)".into())
    );
}
