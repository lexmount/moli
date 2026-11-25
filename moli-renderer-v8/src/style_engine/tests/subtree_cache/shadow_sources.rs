use super::*;

#[test]
fn source_local_fallback_roots_preserve_unrelated_document_cache_for_shadow_source() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("main");
    let shadow_host = host.create_element("section");
    let sibling_shadow_host = host.create_element("article");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let sibling_shadow_root = host
        .attach_shadow_root(sibling_shadow_host, "open")
        .expect("article should host a shadow root");
    let shadow_child = host.create_element("span");
    let sibling_shadow_child = host.create_element("span");
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(document, sibling_shadow_host));
    assert!(host.append_child(shadow_root, shadow_child));
    assert!(host.append_child(sibling_shadow_root, sibling_shadow_child));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [outside, shadow_child, sibling_shadow_child] {
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

    assert!(engine.invalidate_style_subtrees(&host, [shadow_root, shadow_host]));

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        3,
        "a scoped fallback records roots instead of enumerating published descendants"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            sibling_shadow_child
        )
    );
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                shadow_child,
                "display",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
}

#[test]
fn shadow_adopted_stylesheet_rebuild_uses_scoped_dirty_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("main");
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
    let first_source = StyloStylesheetSource::new(
        "span { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![first_source.clone()],
    );
    let first_source_ids =
        engine.shadow_root_adopted_style_sheet_source_ids_for_test(&host, shadow_root);
    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![first_source]));

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                outside,
                "display",
                None,
                &first_inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            shadow_child,
            "color",
            None,
            &first_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    let computed_generation_after_first_build =
        engine.computed_cache_generation_for_document_for_test(document);
    let source_set_generation_after_first_build =
        engine.source_set_generation_for_document_for_test(document);
    let retained_generation_after_first_build =
        engine.retained_style_system_generation_for_document_for_test(document);

    let second_source = StyloStylesheetSource::new(
        "span { color: rgb(4, 5, 6); }".to_owned(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![second_source.clone()],
    );
    let second_source_ids =
        engine.shadow_root_adopted_style_sheet_source_ids_for_test(&host, shadow_root);

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        computed_generation_after_first_build
    );
    assert_eq!(
        engine.retained_style_system_generation_for_document_for_test(document),
        retained_generation_after_first_build
    );
    assert!(
        engine.source_set_generation_for_document_for_test(document)
            > source_set_generation_after_first_build,
        "source-set mutation should advance source-set generation before retained rebuild"
    );
    let source_set_generation_after_source_change =
        engine.source_set_generation_for_document_for_test(document);
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );

    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        first_source_ids
            .into_iter()
            .chain(second_source_ids)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        engine.source_dirty_scope_ids_for_document_for_test(document),
        vec![StyleScopeId::ShadowRoot(shadow_root)]
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::ShadowRootAdoptedStyleSheets]
    );
    assert_eq!(
        engine.source_dirty_scope_roots_for_document_for_test(document),
        vec![shadow_root, shadow_host]
    );
    let mut second_inputs = FullStyleWorldSnapshot::default();
    second_inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![second_source]));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            shadow_child,
            "color",
            None,
            &second_inputs,
            None,
        ),
        Some("rgb(4, 5, 6)".into())
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        computed_generation_after_first_build
    );
    assert_eq!(
        engine.source_set_generation_for_document_for_test(document),
        source_set_generation_after_source_change,
        "consuming retained dirty scopes must not bump source-set generation again"
    );
    assert!(
        engine.retained_style_system_generation_for_document_for_test(document)
            > retained_generation_after_first_build,
        "scoped source rebuild should advance retained style-system generation"
    );
    assert!(
        engine
            .source_dirty_scope_source_ids_for_document_for_test(document)
            .is_empty()
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
}

#[test]
fn shadow_adopted_stylesheet_addition_rebuild_uses_scoped_dirty_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("main");
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
    let first_inputs = FullStyleWorldSnapshot::default();

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                outside,
                "display",
                None,
                &first_inputs,
                None,
            )
            .is_some()
    );
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                shadow_child,
                "display",
                None,
                &first_inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    let generation_after_first_build =
        engine.computed_cache_generation_for_document_for_test(document);

    let source = StyloStylesheetSource::new(
        "span { color: rgb(4, 5, 6); }".to_owned(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![source.clone()],
    );
    let source_ids = engine.shadow_root_adopted_style_sheet_source_ids_for_test(&host, shadow_root);

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );

    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        source_ids
    );
    assert_eq!(
        engine.source_dirty_scope_ids_for_document_for_test(document),
        vec![StyleScopeId::ShadowRoot(shadow_root)]
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::ShadowRootAdoptedStyleSheets]
    );
    assert_eq!(
        engine.source_dirty_scope_roots_for_document_for_test(document),
        vec![shadow_root, shadow_host]
    );
    let mut second_inputs = FullStyleWorldSnapshot::default();
    second_inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![source]));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            shadow_child,
            "color",
            None,
            &second_inputs,
            None,
        ),
        Some("rgb(4, 5, 6)".into())
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert!(
        engine
            .source_dirty_scope_source_ids_for_document_for_test(document)
            .is_empty()
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
}

#[test]
fn shadow_adopted_stylesheet_removal_rebuild_uses_scoped_dirty_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("main");
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
    let source = StyloStylesheetSource::new(
        "span { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![source.clone()],
    );
    let source_ids = engine.shadow_root_adopted_style_sheet_source_ids_for_test(&host, shadow_root);
    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![source]));

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                outside,
                "display",
                None,
                &first_inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            shadow_child,
            "color",
            None,
            &first_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    let generation_after_first_build =
        engine.computed_cache_generation_for_document_for_test(document);

    engine.set_shadow_root_adopted_style_sheet_sources_with_host(&host, shadow_root, Vec::new());

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );

    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        source_ids
    );
    assert_eq!(
        engine.source_dirty_scope_ids_for_document_for_test(document),
        vec![StyleScopeId::ShadowRoot(shadow_root)]
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::ShadowRootAdoptedStyleSheets]
    );
    assert_eq!(
        engine.source_dirty_scope_roots_for_document_for_test(document),
        vec![shadow_root, shadow_host]
    );
    let second_inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                shadow_child,
                "display",
                None,
                &second_inputs,
                None,
            )
            .is_some()
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert!(
        engine
            .source_dirty_scope_source_ids_for_document_for_test(document)
            .is_empty()
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
}

#[test]
fn shadow_adopted_stylesheet_dirty_scopes_keep_explicit_shadow_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let first_host = host.create_element("section");
    let second_host = host.create_element("article");
    let first_root = host
        .attach_shadow_root(first_host, "open")
        .expect("section should host a shadow root");
    let second_root = host
        .attach_shadow_root(second_host, "open")
        .expect("article should host a shadow root");
    assert!(host.append_child(document, first_host));
    assert!(host.append_child(document, second_host));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        first_root,
        vec![StyloStylesheetSource::new(
            ":host { color: rgb(1, 2, 3); }".to_owned(),
            document_url.clone(),
        )],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        second_root,
        vec![StyloStylesheetSource::new(
            ":host { color: rgb(4, 5, 6); }".to_owned(),
            document_url,
        )],
    );
    let expected_source_ids = engine
        .shadow_root_adopted_style_sheet_source_ids_for_test(&host, first_root)
        .into_iter()
        .chain(engine.shadow_root_adopted_style_sheet_source_ids_for_test(&host, second_root))
        .collect::<Vec<_>>();

    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        expected_source_ids
    );
    assert_eq!(
        engine.source_dirty_scope_ids_for_document_for_test(document),
        vec![
            StyleScopeId::ShadowRoot(first_root),
            StyleScopeId::ShadowRoot(second_root),
        ]
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::ShadowRootAdoptedStyleSheets]
    );
    assert_eq!(
        engine.source_dirty_scope_roots_for_document_for_test(document),
        vec![first_root, first_host, second_root, second_host]
    );
}

#[test]
fn shadow_scope_reorder_updates_the_retained_world_in_place() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("main");
    let first_host = host.create_element("section");
    let first_root = host
        .attach_shadow_root(first_host, "open")
        .expect("section should host a shadow root");
    let first_child = host.create_element("span");
    let second_host = host.create_element("aside");
    let second_root = host
        .attach_shadow_root(second_host, "open")
        .expect("aside should host a shadow root");
    let second_child = host.create_element("strong");
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, first_host));
    assert!(host.append_child(document, second_host));
    assert!(host.append_child(first_root, first_child));
    assert!(host.append_child(second_root, second_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        "span { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    let second_source = StyloStylesheetSource::new(
        "strong { color: rgb(4, 5, 6); }".to_owned(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        first_root,
        vec![first_source.clone()],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        second_root,
        vec![second_source.clone()],
    );
    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs
        .shadow_stylesheet_sources
        .push((first_root, vec![first_source]));
    first_inputs
        .shadow_stylesheet_sources
        .push((second_root, vec![second_source.clone()]));

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                outside,
                "display",
                None,
                &first_inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_child,
            "color",
            None,
            &first_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    let generation_after_first_build =
        engine.computed_cache_generation_for_document_for_test(document);
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);
    let rebuilds = engine.retained_style_system_rebuild_count_for_document_for_test(document);

    let changed_source = StyloStylesheetSource::new(
        "span { color: rgb(7, 8, 9); }".to_owned(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        first_root,
        vec![changed_source.clone()],
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, first_child)
    );

    let mut second_inputs = FullStyleWorldSnapshot::default();
    second_inputs
        .shadow_stylesheet_sources
        .push((second_root, vec![second_source]));
    second_inputs
        .shadow_stylesheet_sources
        .push((first_root, vec![changed_source]));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_child,
            "color",
            None,
            &second_inputs,
            None,
        ),
        Some("rgb(7, 8, 9)".into())
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build,
        "TreeScope order is metadata, not a reason to replace the document style world"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, outside),
        "a ShadowRoot update must preserve unrelated document styles"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, first_child)
    );
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds
    );
}
