use super::*;

#[test]
fn retained_stylo_invalidator_narrows_assigned_node_slotted_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let slot = host.create_element("slot");
    let assigned = host.create_element("span");
    let unrelated_assigned = host.create_element("span");

    assert!(host.set_attribute(slot, "class", "slot"));
    assert!(host.set_attribute(unrelated_assigned, "class", "other"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, slot));
    assert!(host.append_child(shadow_host, assigned));
    assert!(host.append_child(shadow_host, unrelated_assigned));

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
    let inputs = FullStyleWorldSnapshot {
        shadow_stylesheet_sources: vec![(
            shadow_root,
            engine.shadow_root_adopted_style_sheet_sources_with_host(&host, shadow_root),
        )],
        ..Default::default()
    };
    for handle in [outside, assigned, unrelated_assigned] {
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

    assert!(host.set_attribute(assigned, "class", "item"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: assigned,
            namespace: None,
            name: "class".into(),
            old_value: None,
            new_value: Some("item".into()),
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
            unrelated_assigned
        )
    );
}

#[test]
fn retained_stylo_invalidator_accepts_empty_slotted_result() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let slot = host.create_element("slot");
    let assigned = host.create_element("span");

    assert!(host.set_attribute(assigned, "class", "other"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, slot));
    assert!(host.append_child(shadow_host, assigned));

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
            namespace: None,
            name: "class".into(),
            old_value: None,
            new_value: Some("slot".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, assigned));
}

#[test]
fn retained_stylo_invalidator_narrows_part_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let part = host.create_element("span");
    let unrelated_shadow_child = host.create_element("span");

    assert!(host.set_attribute(part, "part", "label"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, part));
    assert!(host.append_child(shadow_root, unrelated_shadow_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".host::part(label) { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [outside, part, unrelated_shadow_child] {
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

    assert!(host.set_attribute(shadow_host, "class", "host"));
    let effects = [StyleMutationEffect::Attribute {
        element: shadow_host,
        namespace: None,
        name: "class".into(),
        old_value: None,
        new_value: Some("host".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, part));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            unrelated_shadow_child
        )
    );
}

#[test]
fn retained_stylo_invalidator_accepts_empty_part_result() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("aside");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let part = host.create_element("span");

    assert!(host.set_attribute(part, "part", "other"));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, part));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".host::part(label) { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
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
            namespace: None,
            name: "class".into(),
            old_value: None,
            new_value: Some("host".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, part));
}
