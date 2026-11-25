use super::*;

#[test]
fn inline_style_subtree_invalidation_uses_root_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let mut engine = MoliStyleEngine::new();
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

    engine.invalidate_inline_style_subtree(&host, detached);

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
fn detached_document_mutation_pending_work_uses_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".active { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(detached_document, vec![source.clone()]);
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs.document_stylesheet_sources.push(source);
    let active_inputs = FullStyleWorldSnapshot::default();

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
                "color",
                None,
                &active_inputs,
                None,
            )
            .is_some()
    );
    let before = engine
        .computed_style_property_value(
            &host,
            &document_url,
            detached,
            "color",
            None,
            &detached_inputs,
            None,
        )
        .expect("detached style should compute before mutation");
    assert_ne!(before, "rgb(1, 2, 3)");
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );

    assert!(host.set_attribute(detached, "class", "active"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: detached,
            namespace: None,
            name: "class".to_owned(),
            old_value: None,
            new_value: Some("active".to_owned()),
        }],
        &media,
    );

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(detached_document),
        1
    );

    let after = engine
        .computed_style_property_value(
            &host,
            &document_url,
            detached,
            "color",
            None,
            &detached_inputs,
            None,
        )
        .expect("detached style should recompute after owner-world drain");
    assert_eq!(after, "rgb(1, 2, 3)");
    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(detached_document),
        0
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            detached_document,
            detached
        )
    );
}

#[test]
fn detached_document_focus_change_pending_work_uses_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "section:focus { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(detached_document, vec![source.clone()]);
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs.document_stylesheet_sources.push(source);
    let active_inputs = FullStyleWorldSnapshot::default();

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
                "color",
                None,
                &active_inputs,
                None,
            )
            .is_some()
    );
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                detached,
                "color",
                None,
                &detached_inputs,
                None,
            )
            .is_some()
    );

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_focus_change(&host, None, Some(detached), &media);

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(detached_document),
        1
    );
}

#[test]
fn detached_document_target_change_pending_work_uses_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));
    assert!(host.set_document_target_element(detached_document, Some(detached)));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "section:target { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(detached_document, vec![source.clone()]);
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs.document_stylesheet_sources.push(source);
    let active_inputs = FullStyleWorldSnapshot::default();

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
                "color",
                None,
                &active_inputs,
                None,
            )
            .is_some()
    );
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                detached,
                "color",
                None,
                &detached_inputs,
                None,
            )
            .is_some()
    );

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_target_change(&host, None, Some(detached), &media);

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(detached_document),
        1
    );
}

#[test]
fn empty_focus_change_does_not_use_active_document_world() {
    let host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_focus_change(&host, None, None, &media);

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        0
    );
}

#[test]
fn empty_target_change_does_not_use_active_document_world() {
    let host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_target_change(&host, None, None, &media);

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        0
    );
}

#[test]
fn detached_document_adopted_stylesheet_change_uses_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(detached_document, vec![first_source.clone()]);
    assert_eq!(
        engine.adopted_style_sheet_source_owner_counts_for_document_for_test(document),
        (0, 0)
    );
    assert_eq!(
        engine.adopted_style_sheet_source_owner_counts_for_document_for_test(detached_document),
        (1, 0)
    );
    assert_eq!(
        engine
            .adopted_style_sheet_sources_for_document(document)
            .len(),
        0
    );
    assert_eq!(
        engine
            .adopted_style_sheet_sources_for_document(detached_document)
            .len(),
        1
    );
    let mut first_detached_inputs = FullStyleWorldSnapshot::default();
    first_detached_inputs
        .document_stylesheet_sources
        .push(first_source);
    let active_inputs = FullStyleWorldSnapshot::default();
    assert!(host.set_attribute(detached, "class", "target"));

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
                "color",
                None,
                &active_inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            detached,
            "color",
            None,
            &first_detached_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );

    let second_source = StyloStylesheetSource::new(
        ".target { color: rgb(4, 5, 6); }".to_owned(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(detached_document, vec![second_source.clone()]);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1,
        "the owner document keeps its published style until observation"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            detached_document,
            detached
        )
    );
    assert_only_source_document_is_dirty(&engine, detached_document, document);

    let mut second_detached_inputs = FullStyleWorldSnapshot::default();
    second_detached_inputs
        .document_stylesheet_sources
        .push(second_source);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            detached,
            "color",
            None,
            &second_detached_inputs,
            None,
        ),
        Some("rgb(4, 5, 6)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
}

#[test]
fn detached_document_owner_stylesheet_change_uses_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_style = host.create_element("style");
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached_style));
    assert!(host.append_child(detached_document, detached));
    assert!(host.set_attribute(detached, "class", "target"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        detached_style,
        ".target { color: rgb(1, 2, 3); }".to_owned(),
    );
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    let detached_source = engine
        .owner_style_sheet_source_with_host(&host, detached_style)
        .expect("detached owner style source");
    assert_eq!(
        detached_source.serialized_css_text().as_ref(),
        ".target { color: rgb(1, 2, 3); }"
    );
    detached_inputs
        .document_stylesheet_sources
        .push(detached_source);
    let active_inputs = FullStyleWorldSnapshot::default();

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
                "color",
                None,
                &active_inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            detached,
            "color",
            None,
            &detached_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );

    engine.set_owner_style_sheet_text_with_host(
        &host,
        detached_style,
        ".target { color: rgb(4, 5, 6); }".to_owned(),
    );

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1,
        "the detached owner document invalidates at observation time"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            detached_document,
            detached
        )
    );
    assert_only_source_document_is_dirty(&engine, detached_document, document);
}

#[test]
fn ownerless_owner_stylesheet_change_does_not_use_active_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    assert!(host.append_child(document, active));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
                "color",
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

    // Use the highest representable DOM handle as an owner that cannot belong
    // to this tiny test document. Native node IDs deliberately reject larger
    // sentinels at construction time.
    let ownerless = DomHandle::new(u32::MAX as usize - 1);
    engine.set_owner_style_sheet_text_with_host(
        &host,
        ownerless,
        "main { color: rgb(1, 2, 3); }".to_owned(),
    );

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
}
