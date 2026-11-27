use super::*;

#[test]
fn lang_selector_does_not_match_more_specific_range() {
    let mut host = test_host();
    host.reset_html_document_shell();
    let head = host.document_head_handle().expect("document head");
    let body = host.document_body_handle().expect("document body");
    let style = host.create_element("style");
    let wrapper = host.create_element("div");
    let target = host.create_element("div");
    assert!(host.set_attribute(wrapper, "class", "test"));
    assert!(host.set_attribute(target, "id", "box"));
    assert!(host.set_attribute(target, "lang", "es"));
    assert!(host.append_child(head, style));
    assert!(host.append_child(body, wrapper));
    assert!(host.append_child(wrapper, target));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        ".test div { color: green; width: 50px; } #box:lang(es-MX) { width: 100px; }".into(),
    );
    let source_id = StyleSourceId::owner_style_sheet(&host, style).expect("owner source id");
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        engine
            .owner_style_sheet_source_with_host(&host, style)
            .expect("owner stylesheet source")
            .with_source_id(Some(source_id)),
    );

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
            target,
            "width",
            None,
            &inputs,
            None,
        ),
        Some("50px".into())
    );
}

#[test]
fn standalone_subtree_context_invalidation_preserves_unrelated_cache_entries() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("div");
    let target = host.create_element("section");
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
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
        2
    );
    let generation = engine.computed_cache_generation_for_document_for_test(document);
    let rebuilds = engine.retained_style_system_rebuild_count_for_document_for_test(document);

    let effects = [StyleMutationEffect::ConnectedSubtrees {
        roots: vec![target].into(),
    }];

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);

    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds
    );
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
}
#[test]
fn pending_mutation_invalidations_drain_before_computed_style_read() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("div");
    let target = host.create_element("section");
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
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
        2
    );

    let effects = [StyleMutationEffect::ConnectedSubtrees {
        roots: vec![target].into(),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        1
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
        1
    );
    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        0
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
}
#[test]
fn pending_mutation_invalidations_batch_distinct_targets_until_drain() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("div");
    let first_target = host.create_element("section");
    let second_target = host.create_element("article");
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, first_target));
    assert!(host.append_child(document, second_target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [outside, first_target, second_target] {
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ConnectedSubtrees {
            roots: vec![first_target].into(),
        }],
        &media,
    );
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ConnectedSubtrees {
            roots: vec![second_target].into(),
        }],
        &media,
    );

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
}
#[test]
fn pending_attribute_mutation_invalidations_merge_for_batch_drain() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let first = host.create_element("section");
    let second = host.create_element("article");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, first));
    assert!(host.append_child(document, second));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        ".first { color: red; } .second { color: blue; }".into(),
    );
    let inputs = FullStyleWorldSnapshot::default();
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, host.document_handle(), key, &inputs);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: first,
            namespace: None,
            name: "class".into(),
            old_value: None,
            new_value: Some("first".into()),
        }],
        &media,
    );
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: second,
            namespace: None,
            name: "class".into(),
            old_value: None,
            new_value: Some("second".into()),
        }],
        &media,
    );

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        1
    );
}
#[test]
fn style_subtree_membership_crosses_shadow_root_to_host_without_crossing_siblings() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let light_child = host.create_element("div");
    let sibling = host.create_element("aside");
    let shadow_root = host
        .attach_shadow_root(parent, "open")
        .expect("section should host a shadow root");
    let shadow_child = host.create_element("span");

    assert!(host.append_child(document, parent));
    assert!(host.append_child(document, sibling));
    assert!(host.append_child(parent, light_child));
    assert!(host.append_child(shadow_root, shadow_child));

    let parent_roots = HashSet::from([parent]);
    assert!(handle_is_in_style_subtrees(
        &host,
        light_child,
        &parent_roots
    ));
    assert!(handle_is_in_style_subtrees(
        &host,
        shadow_root,
        &parent_roots
    ));
    assert!(handle_is_in_style_subtrees(
        &host,
        shadow_child,
        &parent_roots
    ));
    assert!(!handle_is_in_style_subtrees(&host, sibling, &parent_roots));

    let shadow_roots = HashSet::from([shadow_root]);
    assert!(handle_is_in_style_subtrees(
        &host,
        shadow_child,
        &shadow_roots
    ));
    assert!(!handle_is_in_style_subtrees(&host, parent, &shadow_roots));
}
