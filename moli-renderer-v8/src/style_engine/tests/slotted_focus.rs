use super::*;

#[test]
fn slotted_dependency_invalidation_targets_assigned_nodes() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let slot = host.create_element("slot");
    let assigned = host.create_element("span");

    assert!(host.set_attribute(assigned, "class", "item"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, slot));
    assert!(host.append_child(shadow_host, assigned));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![StyloStylesheetSource::new(
            ".slot::slotted(.item) { color: red; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [outside, assigned] {
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

    assert!(host.set_attribute(slot, "class", "slot"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: slot,
            name: "class".into(),
            old_value: None,
            new_value: Some("slot".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, assigned));
}

#[test]
fn slotted_dependency_change_clears_assigned_descendant_cache_via_subtree_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let slot = host.create_element("slot");
    let assigned = host.create_element("span");
    let assigned_child = host.create_element("em");

    assert!(host.set_attribute(assigned, "class", "item"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, slot));
    assert!(host.append_child(shadow_host, assigned));
    assert!(host.append_child(assigned, assigned_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ".slot::slotted(.item) { color: rgb(1, 2, 3); }".into(),
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

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                assigned_child,
                "color",
                None,
                &inputs,
                None,
            )
            .is_some_and(|color| color != "rgb(1, 2, 3)")
    );
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                outside,
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

    assert!(host.set_attribute(slot, "class", "slot"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: slot,
            name: "class".into(),
            old_value: None,
            new_value: Some("slot".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2,
        "the assigned descendant is retained until it is observed"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, assigned_child)
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            assigned_child,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
}

#[test]
fn slotted_relative_anchor_invalidation_preserves_unmatched_assigned_node() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let slot = host.create_element("slot");
    let marker = host.create_element("b");
    let assigned = host.create_element("span");
    let unmatched_assigned = host.create_element("span");

    assert!(host.set_attribute(slot, "class", "slot"));
    assert!(host.set_attribute(marker, "class", "other"));
    assert!(host.set_attribute(assigned, "class", "item"));
    assert!(host.set_attribute(unmatched_assigned, "class", "other"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, slot));
    assert!(host.append_child(slot, marker));
    assert!(host.append_child(shadow_host, assigned));
    assert!(host.append_child(shadow_host, unmatched_assigned));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ".slot:has(.marker)::slotted(.item) { color: rgb(1, 2, 3); }".into(),
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

    for handle in [outside, assigned, unmatched_assigned] {
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
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: marker,
            name: "class".into(),
            old_value: Some("other".into()),
            new_value: Some("marker".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, assigned));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            unmatched_assigned
        )
    );
}

#[test]
fn manual_slot_assignment_invalidation_uses_assigned_node_snapshot() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let slot = host.create_element("slot");
    let previous_assigned = host.create_element("span");
    let assigned = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(slot, "class", "slot"));
    assert!(host.set_attribute(previous_assigned, "class", "item"));
    assert!(host.set_attribute(assigned, "class", "item"));
    assert!(host.set_attribute(unrelated, "class", "item"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, slot));
    assert!(host.append_child(shadow_host, previous_assigned));
    assert!(host.append_child(shadow_host, assigned));
    assert!(host.append_child(shadow_host, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ".slot::slotted(.item) { color: red; }".into(),
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
    for handle in [outside, previous_assigned, assigned, unrelated] {
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
        4
    );

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::SlotAssignment {
            slot,
            previous_assigned_nodes: Some(vec![previous_assigned]),
            assigned_nodes: Some(vec![assigned]),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            previous_assigned
        )
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, assigned));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn shadow_slot_insertion_uses_retargeted_assignment_snapshots() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let old_slot = host.create_element("slot");
    let new_slot = host.create_element("slot");
    let assigned = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(old_slot, "name", "content"));
    assert!(host.set_attribute(old_slot, "class", "slot"));
    assert!(host.set_attribute(new_slot, "name", "content"));
    assert!(host.set_attribute(new_slot, "class", "slot"));
    assert!(host.set_attribute(assigned, "slot", "content"));
    assert!(host.set_attribute(assigned, "class", "item"));
    assert!(host.set_attribute(unrelated, "class", "item"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, old_slot));
    assert!(host.append_child(shadow_host, assigned));
    assert!(host.append_child(shadow_host, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ".slot::slotted(.item) { color: rgb(1, 2, 3); }".into(),
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
    for handle in [assigned, unrelated, outside] {
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

    let effects = host.insert_before_effects(shadow_root, new_slot, Some(old_slot));
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);
    assert!(style_effects.iter().all(|effect| {
        matches!(
            effect,
            StyleMutationEffect::SlotAssignment {
                previous_assigned_nodes: Some(_),
                assigned_nodes: Some(_),
                ..
            } | StyleMutationEffect::ChildList { .. }
                | StyleMutationEffect::ConnectedSubtree { .. }
        )
    }));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &style_effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, assigned));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
}

#[test]
fn slotted_dependency_invalidation_treats_assigned_node_as_shadow_scope() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let slot = host.create_element("slot");
    let assigned = host.create_element("span");

    assert!(host.set_attribute(slot, "class", "slot"));
    assert!(host.set_attribute(assigned, "class", "item"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, slot));
    assert!(host.append_child(shadow_host, assigned));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![StyloStylesheetSource::new(
            ".slot::slotted(.item) { color: red; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [outside, assigned] {
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

    assert!(host.set_attribute(assigned, "class", ""));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: assigned,
            name: "class".into(),
            old_value: Some("item".into()),
            new_value: Some(String::new()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, assigned));
}
#[test]
fn part_dependency_invalidation_targets_shadow_tree() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let part = host.create_element("span");

    assert!(host.set_attribute(part, "part", "label"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, part));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".host::part(label) { color: red; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [outside, part] {
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

    assert!(host.set_attribute(shadow_host, "class", "host"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: shadow_host,
            name: "class".into(),
            old_value: None,
            new_value: Some("host".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, part));
}

#[test]
fn part_relative_anchor_invalidation_preserves_unmatched_part() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let marker = host.create_element("b");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let part = host.create_element("span");
    let unmatched_part = host.create_element("span");

    assert!(host.set_attribute(shadow_host, "class", "host"));
    assert!(host.set_attribute(marker, "class", "other"));
    assert!(host.set_attribute(part, "part", "label"));
    assert!(host.set_attribute(unmatched_part, "part", "other"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_host, marker));
    assert!(host.append_child(shadow_root, part));
    assert!(host.append_child(shadow_root, unmatched_part));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".host:has(.marker)::part(label) { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    for handle in [outside, part, unmatched_part] {
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
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: marker,
            name: "class".into(),
            old_value: Some("other".into()),
            new_value: Some("marker".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, part));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, unmatched_part)
    );
}

#[test]
fn document_part_rule_styles_shadow_part() {
    let mut host = test_host();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let part = host.create_element("span");

    assert!(host.set_attribute(shadow_host, "class", "host"));
    assert!(host.set_attribute(part, "part", "label"));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, part));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".host::part(label) { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            part,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
}

#[test]
fn document_part_rule_styles_after_part_lang_and_invalidates() {
    let mut host = test_host();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let part = host.create_element("span");

    assert!(host.set_attribute(shadow_host, "class", "host"));
    assert!(host.set_attribute(shadow_host, "lang", "en"));
    assert!(host.set_attribute(part, "part", "label"));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, part));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        concat!(
            ".host::part(label) { background-color: rgb(255, 0, 0); }",
            ".host::part(label):lang(en) { background-color: rgb(0, 255, 255); }",
            ".host::part(label):lang(fr) { background-color: rgb(0, 0, 255); }",
        )
        .into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            part,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 255, 255)".into())
    );

    assert!(host.set_attribute(part, "lang", "fr"));
    let effects = [StyleMutationEffect::Attribute {
        element: part,
        name: "lang".into(),
        old_value: None,
        new_value: Some("fr".into()),
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
    let application = engine
        .with_retained_style_system_for_document_for_test(document, |retained| {
            retained_source_invalidation_outcome_for_document_for_test(
                &engine,
                &host,
                document,
                StyleSourceDocumentContext::for_root_document(document),
                Some(retained),
                &target_queries,
                false,
            )
        })
        .finalize(&host);
    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::ExactAffectedSubtreeRoots
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::ExactAffectedSubtreeRoots(roots)
            if roots.contains(&part) && !roots.contains(&shadow_host)
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

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            part,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 0, 255)".into())
    );
}

#[test]
fn document_part_rule_styles_after_part_dir_and_invalidates() {
    let mut host = test_host();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let part = host.create_element("span");

    assert!(host.set_attribute(shadow_host, "class", "host"));
    assert!(host.set_attribute(part, "part", "label"));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, part));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        concat!(
            ".host::part(label) { background-color: rgb(255, 0, 0); }",
            ".host::part(label):dir(ltr) { background-color: rgb(0, 255, 0); }",
            ".host::part(label):dir(rtl) { background-color: rgb(0, 0, 255); }",
        )
        .into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            part,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 255, 0)".into())
    );

    assert!(host.set_attribute(part, "dir", "rtl"));
    let effects = [StyleMutationEffect::Attribute {
        element: part,
        name: "dir".into(),
        old_value: None,
        new_value: Some("rtl".into()),
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
    let application = engine
        .with_retained_style_system_for_document_for_test(document, |retained| {
            retained_source_invalidation_outcome_for_document_for_test(
                &engine,
                &host,
                document,
                StyleSourceDocumentContext::for_root_document(document),
                Some(retained),
                &target_queries,
                false,
            )
        })
        .finalize(&host);
    assert_eq!(
        application.cleanup_target_kind(),
        StyleInvalidationCleanupTargetKind::ExactAffectedSubtreeRoots
    );
    assert!(matches!(
        application.cleanup_target(),
        StyleInvalidationCleanupTarget::ExactAffectedSubtreeRoots(roots)
            if roots.contains(&part) && !roots.contains(&shadow_host)
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

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            part,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 0, 255)".into())
    );
}

#[test]
fn document_part_rule_styles_exported_nested_shadow_part() {
    let mut host = test_host();
    let document = host.document_handle();
    let outer_host = host.create_element("section");
    let outer_shadow = host
        .attach_shadow_root(outer_host, "open")
        .expect("outer section should host a shadow root");
    let inner_host = host.create_element("article");
    let inner_shadow = host
        .attach_shadow_root(inner_host, "open")
        .expect("inner article should host a shadow root");
    let part = host.create_element("span");

    assert!(host.set_attribute(outer_host, "class", "outer"));
    assert!(host.set_attribute(inner_host, "exportparts", "private: public"));
    assert!(host.set_attribute(part, "part", "private"));
    assert!(host.append_child(document, outer_host));
    assert!(host.append_child(outer_shadow, inner_host));
    assert!(host.append_child(inner_shadow, part));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".outer::part(public) { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            part,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(4, 5, 6)".into())
    );
}

#[test]
fn shadow_part_rule_does_not_style_local_part_from_same_shadow_scope() {
    let mut host = test_host();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let part = host.create_element("span");

    assert!(host.set_attribute(part, "part", "label"));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, part));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        "span { color: rgb(1, 2, 3); } ::part(label) { color: rgb(255, 0, 0); }".into(),
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

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            part,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
}
#[test]
fn manual_slot_assignment_noop_preserves_cache_entries() {
    let mut host = test_host();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let slot = host.create_element("slot");
    let assigned = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(slot, "class", "slot"));
    assert!(host.set_attribute(assigned, "class", "item"));
    assert!(host.set_attribute(unrelated, "class", "item"));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, slot));
    assert!(host.append_child(shadow_host, assigned));
    assert!(host.append_child(shadow_host, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let shadow_source = StyloStylesheetSource::new(
        ".slot::slotted(.item) { color: red; }".into(),
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
    for handle in [assigned, unrelated] {
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::SlotAssignment {
            slot,
            previous_assigned_nodes: Some(vec![assigned]),
            assigned_nodes: Some(vec![assigned]),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, assigned));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn attribute_relative_dependency_invalidates_outer_sibling_target() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let ancestor = host.create_element("section");
    let marker = host.create_element("div");
    let target = host.create_element("p");
    let unrelated = host.create_element("p");

    assert!(host.set_attribute(ancestor, "class", "ancestor"));
    assert!(host.set_attribute(marker, "class", "other"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, marker));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".ancestor:has(.marker) + .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [outside, ancestor, target, unrelated] {
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
        4
    );
    assert!(host.set_attribute(marker, "class", "marker"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: marker,
            name: "class".into(),
            old_value: Some("other".into()),
            new_value: Some("marker".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, ancestor));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn unrelated_attribute_mutation_ignores_unmatched_has_selector_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let unrelated = host.create_element("div");
    let target = host.create_element("span");

    assert!(host.set_attribute(container, "class", "container"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, unrelated));
    assert!(host.append_child(container, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".container:has(.marker) .target { color: red; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [unrelated, target] {
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: unrelated,
            name: "data-other".into(),
            old_value: None,
            new_value: Some("active".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}
#[test]
fn style_attribute_mutation_keeps_local_subtree_fallback_without_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let element = host.create_element("span");
    assert!(host.append_child(document, element));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                element,
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element,
            name: "style".into(),
            old_value: None,
            new_value: Some("color: red".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}
#[test]
fn attribute_sibling_invalidation_is_per_effect() {
    let mut host = test_host();
    let document = host.document_handle();
    let first_parent = host.create_element("section");
    let first_source = host.create_element("div");
    let first_target = host.create_element("span");
    let second_parent = host.create_element("section");
    let second_source = host.create_element("div");
    let second_target = host.create_element("span");

    assert!(host.set_attribute(first_target, "class", "target"));
    assert!(host.set_attribute(second_target, "class", "target"));
    assert!(host.append_child(document, first_parent));
    assert!(host.append_child(first_parent, first_source));
    assert!(host.append_child(first_parent, first_target));
    assert!(host.append_child(document, second_parent));
    assert!(host.append_child(second_parent, second_source));
    assert!(host.append_child(second_parent, second_target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".marker + .target { color: red; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                first_target,
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
                second_target,
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

    let effects = [
        StyleMutationEffect::Attribute {
            element: first_source,
            name: "class".into(),
            old_value: None,
            new_value: Some("marker".into()),
        },
        StyleMutationEffect::Attribute {
            element: second_source,
            name: "data-other".into(),
            old_value: None,
            new_value: Some("active".into()),
        },
    ];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, second_target)
    );
}
#[test]
fn focus_invalidation_keeps_unrelated_sibling_cache_entries() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let source = host.create_element("button");
    let target = host.create_element("span");

    assert!(host.set_attribute(source, "class", "focusable unrelated"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, source));
    assert!(host.append_child(parent, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".focusable:focus { color: red; }
             .unrelated + .target { color: blue; }"
                .into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_focus_change(&host, None, Some(source), &media);

    assert_eq!(
        engine.pending_style_invalidation_work_kind_names_for_document_for_test(document),
        vec!["focus"]
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}
#[test]
fn focus_within_invalidation_preserves_unrelated_sibling_cache_entries() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("aside");
    let parent = host.create_element("section");
    let source = host.create_element("button");

    assert!(host.set_attribute(parent, "class", "container"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, source));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".container:focus-within { color: red; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [previous, parent] {
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_focus_change(&host, None, Some(source), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, parent));
}
#[test]
fn focus_sibling_invalidation_preserves_previous_sibling_cache() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let previous = host.create_element("span");
    let source = host.create_element("button");
    let target = host.create_element("span");

    assert!(host.set_attribute(source, "class", "focusable"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, previous));
    assert!(host.append_child(parent, source));
    assert!(host.append_child(parent, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".focusable:focus + .target { color: red; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [previous, target] {
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_focus_change(&host, None, Some(source), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}
#[test]
fn focus_invalidation_ignores_unmatched_has_selector_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let source = host.create_element("button");
    let target = host.create_element("span");

    assert!(host.set_attribute(container, "class", "container"));
    assert!(host.set_attribute(source, "class", "focusable"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, source));
    assert!(host.append_child(container, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".focusable:focus { color: red; }
             .container:has(.marker) .target { color: blue; }"
                .into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [source, target] {
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_focus_change(&host, None, Some(source), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}
#[test]
fn focus_invalidation_uses_has_focus_relative_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let source = host.create_element("button");
    let target = host.create_element("span");

    assert!(host.set_attribute(container, "class", "container"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, source));
    assert!(host.append_child(container, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_sheet = StyloStylesheetSource::new(
        ".container:has(:focus) .target { color: blue; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_sheet.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_sheet);
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

    host.set_active_element_handle(Some(source));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_focus_change(&host, None, Some(source), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0,
        "the exact :has(:focus) target is invalidated directly"
    );
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
        Some("rgb(0, 0, 255)".into())
    );
}
#[test]
fn target_invalidation_keeps_pending_work_kind() {
    let mut host = test_host();
    let document = host.document_handle();
    let source = host.create_element("a");
    let target = host.create_element("span");

    assert!(host.set_attribute(source, "id", "current"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, source));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            "#current:target + .target { color: green; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_target_change(&host, None, Some(source), &media);

    assert_eq!(
        engine.pending_style_invalidation_work_kind_names_for_document_for_test(document),
        vec!["target"]
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}
#[test]
fn target_invalidation_ignores_unmatched_has_selector_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let source = host.create_element("a");
    let target = host.create_element("span");

    assert!(host.set_attribute(container, "class", "container"));
    assert!(host.set_attribute(source, "id", "current"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, source));
    assert!(host.append_child(container, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            "#current:target { color: red; }
             .container:has(.marker) .target { color: blue; }"
                .into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [source, target] {
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_target_change(&host, None, Some(source), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}
#[test]
fn target_invalidation_uses_has_target_relative_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let source = host.create_element("a");
    let target = host.create_element("span");

    assert!(host.set_attribute(container, "class", "container"));
    assert!(host.set_attribute(source, "id", "current"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, source));
    assert!(host.append_child(container, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_sheet = StyloStylesheetSource::new(
        ".container:has(:target) .target { color: blue; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_sheet.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_sheet);
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

    assert!(host.set_document_target_element(document, Some(source)));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_target_change(&host, None, Some(source), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0,
        "the exact :has(:target) target is invalidated directly"
    );
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
        Some("rgb(0, 0, 255)".into())
    );
}
#[test]
fn target_sibling_invalidation_preserves_previous_sibling_cache() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("span");
    let source = host.create_element("a");
    let target = host.create_element("span");

    assert!(host.set_attribute(source, "id", "current"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, source));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            "#current:target + .target { color: green; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [previous, target] {
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_target_change(&host, None, Some(source), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}
#[test]
fn attribute_relative_dependency_uses_impacted_shadow_stylesheet_scope() {
    let mut host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let outside = host.create_element("div");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_target = host.create_element("span");

    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_target));
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![StyloStylesheetSource::new(
            ":host:has([data-state='active']) span { color: green; }".into(),
            url::Url::parse("https://example.test/shadow.css").unwrap(),
        )],
    );

    let outside_effect = [StyleMutationEffect::Attribute {
        element: outside,
        name: "data-state".into(),
        old_value: None,
        new_value: Some("active".into()),
    }];
    let outside_scope = source_scope_for_mutations(&host, &outside_effect);
    assert_eq!(
        outside_scope,
        Some(style_source_scope_for_mutation_effects(
            &host,
            &outside_effect
        ))
    );

    let shadow_effect = [StyleMutationEffect::Attribute {
        element: shadow_target,
        name: "data-state".into(),
        old_value: None,
        new_value: Some("active".into()),
    }];
    let shadow_scope = source_scope_for_mutations(&host, &shadow_effect);
    assert_eq!(
        shadow_scope,
        Some(style_source_scope_for_mutation_effects(
            &host,
            &shadow_effect
        ))
    );

    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [outside, shadow_target] {
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

    engine.invalidate_for_mutations(&host, &outside_effect, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_target)
    );

    engine.invalidate_for_mutations(&host, &shadow_effect, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_target)
    );
}
