use super::*;

#[test]
fn inherited_ancestor_change_clears_descendant_cache_via_subtree_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let ancestor = host.create_element("section");
    let descendant = host.create_element("span");
    let unrelated = host.create_element("aside");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, descendant));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        ".theme { color: rgb(1, 2, 3); }".into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    for handle in [descendant, unrelated] {
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

    assert!(host.set_attribute(ancestor, "class", "theme"));
    let effects = [StyleMutationEffect::Attribute {
        element: ancestor,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, descendant),
        "inherited ancestor changes must clear descendant cache until direct-handle cleanup has a safety class"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn custom_property_ancestor_change_clears_descendant_cache_via_subtree_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let ancestor = host.create_element("section");
    let descendant = host.create_element("span");
    let unrelated = host.create_element("aside");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, descendant));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        ".theme { --accent: rgb(1, 2, 3); } span { color: var(--accent, black); }".into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    for handle in [descendant, unrelated] {
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

    assert!(host.set_attribute(ancestor, "class", "theme"));
    let effects = [StyleMutationEffect::Attribute {
        element: ancestor,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, descendant),
        "custom-property ancestor changes must clear descendant var() consumers before direct-handle cleanup is safe"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn non_inherited_exact_change_still_clears_descendant_cache_via_subtree_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let ancestor = host.create_element("section");
    let descendant = host.create_element("span");
    let unrelated = host.create_element("aside");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, descendant));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        ".theme { background-color: rgb(1, 2, 3); }".into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    for handle in [descendant, unrelated] {
        assert!(
            engine
                .computed_style_property_value(
                    &host,
                    &document_url,
                    handle,
                    "background-color",
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

    assert!(host.set_attribute(ancestor, "class", "theme"));
    let effects = [StyleMutationEffect::Attribute {
        element: ancestor,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, descendant),
        "even non-inherited exact-looking changes currently clear descendant cache until SelfOnly direct-handle cleanup exists"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn shadow_host_inherited_change_clears_shadow_descendant_cache_via_subtree_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_child = host.create_element("span");
    let unrelated = host.create_element("aside");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_child));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        ".theme { color: rgb(1, 2, 3); }".into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    for handle in [shadow_child, unrelated] {
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

    assert!(host.set_attribute(shadow_host, "class", "theme"));
    let effects = [StyleMutationEffect::Attribute {
        element: shadow_host,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child),
        "inherited host changes must clear shadow descendant cache before direct-handle cleanup is safe"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn lazy_pseudo_inherited_change_clears_pseudo_cache_via_subtree_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let ancestor = host.create_element("section");
    let list_item = host.create_element("li");
    let unrelated = host.create_element("aside");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, list_item));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        ".theme { color: rgb(1, 2, 3); } li { display: list-item; } li::marker { color: inherit; }"
            .into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                list_item,
                "color",
                Some("marker"),
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
                "color",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );
    assert_eq!(
        engine
            .computed_style_cache_entry_count_for_handle_for_document_for_test(document, list_item),
        2
    );
    assert_eq!(
        engine
            .computed_style_cache_entry_count_for_handle_for_document_for_test(document, unrelated),
        1
    );

    assert!(host.set_attribute(ancestor, "class", "theme"));
    let effects = [StyleMutationEffect::Attribute {
        element: ancestor,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, list_item),
        "inherited ancestor changes must clear lazy pseudo cache entries keyed by descendant handles"
    );
    assert_eq!(
        engine
            .computed_style_cache_entry_count_for_handle_for_document_for_test(document, list_item),
        0
    );
    assert_eq!(
        engine
            .computed_style_cache_entry_count_for_handle_for_document_for_test(document, unrelated),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn lazy_pseudo_custom_property_change_clears_pseudo_cache_via_subtree_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let ancestor = host.create_element("section");
    let list_item = host.create_element("li");
    let unrelated = host.create_element("aside");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, list_item));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        ".theme { --marker-color: rgb(1, 2, 3); } li { display: list-item; } li::marker { color: var(--marker-color, black); }"
            .into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                list_item,
                "color",
                Some("marker"),
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
                "color",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );
    assert_eq!(
        engine
            .computed_style_cache_entry_count_for_handle_for_document_for_test(document, list_item),
        2
    );
    assert_eq!(
        engine
            .computed_style_cache_entry_count_for_handle_for_document_for_test(document, unrelated),
        1
    );

    assert!(host.set_attribute(ancestor, "class", "theme"));
    let effects = [StyleMutationEffect::Attribute {
        element: ancestor,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, list_item),
        "custom-property ancestor changes must clear lazy pseudo var() consumers before direct-handle cleanup is safe"
    );
    assert_eq!(
        engine
            .computed_style_cache_entry_count_for_handle_for_document_for_test(document, list_item),
        0
    );
    assert_eq!(
        engine
            .computed_style_cache_entry_count_for_handle_for_document_for_test(document, unrelated),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

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
    let inputs = StyloComputedStyleInputs::default();
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

    let source_scope = StyleSourceScope::for_handle(&host, shadow_child);
    let fallback_roots =
        shadow_root_source_scope_fallback_roots_for_test(&host, shadow_root, &source_scope);
    assert!(fallback_roots.contains(&shadow_root));
    assert!(fallback_roots.contains(&shadow_host));
    assert!(!fallback_roots.contains(&document));
    assert!(!fallback_roots.contains(&sibling_shadow_root));
    assert!(!fallback_roots.contains(&sibling_shadow_host));
    assert!(engine.invalidate_style_subtrees(&host, fallback_roots.iter().copied()));

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            sibling_shadow_child
        )
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
    let mut first_inputs = StyloComputedStyleInputs::default();
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
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );

    let second_source_id = StyleSourceId::shadow_root_adopted_style_sheet(shadow_root, 0);
    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![second_source_id]
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
    let mut second_inputs = StyloComputedStyleInputs::default();
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
    let first_inputs = StyloComputedStyleInputs::default();

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

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );

    let source_id = StyleSourceId::shadow_root_adopted_style_sheet(shadow_root, 0);
    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![source_id]
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
    let mut second_inputs = StyloComputedStyleInputs::default();
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
    let mut first_inputs = StyloComputedStyleInputs::default();
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
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );

    let source_id = StyleSourceId::shadow_root_adopted_style_sheet(shadow_root, 0);
    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![source_id]
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
    let second_inputs = StyloComputedStyleInputs::default();
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

    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![
            StyleSourceId::shadow_root_adopted_style_sheet(first_root, 0),
            StyleSourceId::shadow_root_adopted_style_sheet(second_root, 0),
        ]
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
fn shadow_source_rebuild_full_clears_on_uncovered_shadow_root_order_change() {
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
    let mut first_inputs = StyloComputedStyleInputs::default();
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
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, first_child)
    );

    let mut second_inputs = StyloComputedStyleInputs::default();
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

    assert!(
        engine.computed_cache_generation_for_document_for_test(document)
            > generation_after_first_build,
        "shadow root list reordering is not proven source-local by dirty roots"
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, outside),
        "full rebuild should clear unrelated computed entries when list reordering is not covered"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, first_child)
    );
}

#[test]
fn document_adopted_stylesheet_rebuild_uses_scoped_dirty_root() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("main");
    let target = host.create_element("section");
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        "section { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        document,
        vec![first_source.clone()],
    );
    let mut first_inputs = StyloComputedStyleInputs::default();
    first_inputs.document_stylesheet_sources.push(first_source);

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
            target,
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

    let second_source = StyloStylesheetSource::new(
        "section { color: rgb(4, 5, 6); }".to_owned(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        document,
        vec![second_source.clone()],
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let second_source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![second_source_id]
    );
    assert_eq!(
        engine.source_dirty_scope_ids_for_document_for_test(document),
        vec![StyleScopeId::Document(document)]
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::DocumentAdoptedStyleSheets]
    );
    assert_eq!(
        engine.source_dirty_scope_roots_for_document_for_test(document),
        vec![document]
    );
    let mut second_inputs = StyloComputedStyleInputs::default();
    second_inputs
        .document_stylesheet_sources
        .push(second_source);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
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
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}

#[test]
fn document_source_scoped_rebuild_clears_same_document_shadow_cascade_data() {
    let mut host = test_host();
    let document = host.document_handle();
    let active_shadow_host = host.create_element("section");
    assert!(host.append_child(document, active_shadow_host));
    let active_shadow_root = host
        .attach_shadow_root(active_shadow_host, "open")
        .expect("active host should accept a shadow root");

    let detached_document = host.create_detached_html_document();
    let detached_shadow_host = host.create_element("article");
    assert!(host.append_child(detached_document, detached_shadow_host));
    let detached_shadow_root = host
        .attach_shadow_root(detached_shadow_host, "open")
        .expect("detached host should accept a shadow root");

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_document_source = StyloStylesheetSource::new(
        "section { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    let second_document_source = StyloStylesheetSource::new(
        "section { color: rgb(4, 5, 6); }".to_owned(),
        document_url.clone(),
    );
    let active_shadow_source =
        StyloStylesheetSource::new(":host { display: block; }".to_owned(), document_url.clone());
    let detached_shadow_source =
        StyloStylesheetSource::new(":host { display: flex; }".to_owned(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        document,
        vec![first_document_source.clone()],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        active_shadow_root,
        vec![active_shadow_source.clone()],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        detached_shadow_root,
        vec![detached_shadow_source.clone()],
    );

    let mut active_inputs = StyloComputedStyleInputs::default();
    active_inputs
        .document_stylesheet_sources
        .push(first_document_source);
    active_inputs
        .shadow_stylesheet_sources
        .push((active_shadow_root, vec![active_shadow_source]));
    let active_key = StyleSystemCacheKey::new(&document_url, &active_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, active_key, &active_inputs);

    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs
        .shadow_stylesheet_sources
        .push((detached_shadow_root, vec![detached_shadow_source]));
    let detached_key = StyleSystemCacheKey::new(&document_url, &detached_inputs, None);
    engine.ensure_retained_style_system_for_document(
        &host,
        detached_document,
        detached_key,
        &detached_inputs,
    );

    let active_cascade_data =
        engine.with_retained_style_system_for_document_for_test(document, |retained| {
            retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == active_shadow_root)
                .expect("active retained system should track the active shadow root")
                .1
                .clone()
        });
    let detached_cascade_data =
        engine.with_retained_style_system_for_document_for_test(detached_document, |retained| {
            retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == detached_shadow_root)
                .expect("detached retained system should track the detached shadow root")
                .1
                .clone()
        });
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            active_shadow_root,
            active_cascade_data,
        );
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            detached_document,
            detached_shadow_root,
            detached_cascade_data,
        );
    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(active_shadow_root)
    );
    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(detached_shadow_root)
    );

    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        document,
        vec![second_document_source],
    );

    assert!(
        !engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(active_shadow_root)
    );
    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(detached_shadow_root)
    );
}

#[test]
fn clear_all_fallback_record_prevents_scoped_retained_rebuild() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("main");
    let target = host.create_element("section");
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        "section { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        document,
        vec![first_source.clone()],
    );
    let mut first_inputs = StyloComputedStyleInputs::default();
    first_inputs.document_stylesheet_sources.push(first_source);

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
            target,
            "color",
            None,
            &first_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    let generation_after_first_build =
        engine.computed_cache_generation_for_document_for_test(document);

    let second_source = StyloStylesheetSource::new(
        "section { color: rgb(4, 5, 6); }".to_owned(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        document,
        vec![second_source.clone()],
    );
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
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![
            StyleSourceDirtyReason::DocumentAdoptedStyleSheets,
            StyleSourceDirtyReason::InvalidationClearAllFallback,
        ]
    );

    let mut second_inputs = StyloComputedStyleInputs::default();
    second_inputs
        .document_stylesheet_sources
        .push(second_source);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &second_inputs,
            None,
        ),
        Some("rgb(4, 5, 6)".into())
    );

    assert!(
        engine.computed_cache_generation_for_document_for_test(document)
            > generation_after_first_build,
        "mixed clear-all fallback diagnostics should force full retained rebuild"
    );
    assert!(
        engine
            .source_dirty_scope_reasons_for_document_for_test(document)
            .is_empty()
    );
}

#[test]
fn owner_stylesheet_rebuild_uses_document_dirty_root() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let outside = host.create_element("main");
    let target = host.create_element("section");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "section { color: rgb(1, 2, 3); }".to_owned(),
    );
    let first_source_id = StyleSourceId::owner_style_sheet(&host, style)
        .expect("document owner stylesheet source id");
    let first_source = engine
        .owner_style_sheet_source_with_host(&host, style)
        .expect("document owner stylesheet source")
        .with_source_id(Some(first_source_id));
    let mut first_inputs = StyloComputedStyleInputs::default();
    first_inputs.document_stylesheet_sources.push(first_source);

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
            target,
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

    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "section { color: rgb(4, 5, 6); }".to_owned(),
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let second_source_id = StyleSourceId::owner_style_sheet(&host, style)
        .expect("document owner stylesheet source id");
    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![second_source_id.clone()]
    );
    assert_eq!(
        engine.source_dirty_scope_ids_for_document_for_test(document),
        vec![StyleScopeId::Document(document)]
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::OwnerStyleSheet]
    );
    assert_eq!(
        engine.source_dirty_scope_roots_for_document_for_test(document),
        vec![document]
    );
    let second_source = engine
        .owner_style_sheet_source_with_host(&host, style)
        .expect("document owner stylesheet source")
        .with_source_id(Some(second_source_id));
    let mut second_inputs = StyloComputedStyleInputs::default();
    second_inputs
        .document_stylesheet_sources
        .push(second_source);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
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
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}

#[test]
fn repeated_owner_stylesheet_changes_reuse_applied_document_cleanup() {
    let mut host = test_host();
    let document = host.document_handle();
    let first_style = host.create_element("style");
    let second_style = host.create_element("style");
    let target = host.create_element("section");
    assert!(host.append_child(document, first_style));
    assert!(host.append_child(document, second_style));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        first_style,
        "section { color: rgb(1, 2, 3); }".to_owned(),
    );
    let epoch_after_first_cleanup = engine.target_context_epoch_for_document_for_test(document);

    engine.set_owner_style_sheet_text_with_host(
        &host,
        second_style,
        "section { color: rgb(4, 5, 6); }".to_owned(),
    );

    assert_eq!(
        engine.target_context_epoch_for_document_for_test(document),
        epoch_after_first_cleanup,
        "the already-cleaned document root must not be walked and invalidated again"
    );
    let first_source_id = StyleSourceId::owner_style_sheet(&host, first_style).unwrap();
    let second_source_id = StyleSourceId::owner_style_sheet(&host, second_style).unwrap();
    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![first_source_id.clone(), second_source_id.clone()]
    );

    let mut inputs = StyloComputedStyleInputs::default();
    for (owner, source_id) in [
        (first_style, first_source_id),
        (second_style, second_source_id),
    ] {
        inputs.document_stylesheet_sources.push(
            engine
                .owner_style_sheet_source_with_host(&host, owner)
                .unwrap()
                .with_source_id(Some(source_id)),
        );
    }
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
        Some("rgb(4, 5, 6)".into())
    );
    assert!(
        engine
            .source_dirty_scope_source_ids_for_document_for_test(document)
            .is_empty()
    );
}

#[test]
fn shadow_owner_stylesheet_rebuild_uses_scoped_dirty_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("main");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_style = host.create_element("style");
    let shadow_child = host.create_element("span");
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_style));
    assert!(host.append_child(shadow_root, shadow_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        shadow_style,
        "span { color: rgb(1, 2, 3); }".to_owned(),
    );
    let first_source_id = StyleSourceId::owner_style_sheet(&host, shadow_style)
        .expect("shadow owner stylesheet source id");
    let first_source = engine
        .owner_style_sheet_source_with_host(&host, shadow_style)
        .expect("shadow owner stylesheet source")
        .with_source_id(Some(first_source_id));
    let mut first_inputs = StyloComputedStyleInputs::default();
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
    let generation_after_first_build =
        engine.computed_cache_generation_for_document_for_test(document);

    engine.set_owner_style_sheet_text_with_host(
        &host,
        shadow_style,
        "span { color: rgb(4, 5, 6); }".to_owned(),
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );

    let second_source_id = StyleSourceId::owner_style_sheet(&host, shadow_style)
        .expect("shadow owner stylesheet source id");
    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![second_source_id.clone()]
    );
    assert_eq!(
        engine.source_dirty_scope_ids_for_document_for_test(document),
        vec![StyleScopeId::ShadowRoot(shadow_root)]
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::OwnerStyleSheet]
    );
    assert_eq!(
        engine.source_dirty_scope_roots_for_document_for_test(document),
        vec![shadow_root, shadow_host]
    );
    let second_source = engine
        .owner_style_sheet_source_with_host(&host, shadow_style)
        .expect("shadow owner stylesheet source")
        .with_source_id(Some(second_source_id));
    let mut second_inputs = StyloComputedStyleInputs::default();
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
        generation_after_first_build
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
}

#[test]
fn mixed_document_and_shadow_source_rebuild_uses_document_dirty_root() {
    let mut host = test_host();
    let document = host.document_handle();
    let document_style = host.create_element("style");
    let outside = host.create_element("main");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_style = host.create_element("style");
    let shadow_child = host.create_element("span");
    assert!(host.append_child(document, document_style));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_style));
    assert!(host.append_child(shadow_root, shadow_child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        document_style,
        "main { color: rgb(1, 2, 3); }".to_owned(),
    );
    engine.set_owner_style_sheet_text_with_host(
        &host,
        shadow_style,
        "span { color: rgb(4, 5, 6); }".to_owned(),
    );
    let first_document_source_id = StyleSourceId::owner_style_sheet(&host, document_style)
        .expect("document owner stylesheet source id");
    let first_shadow_source_id = StyleSourceId::owner_style_sheet(&host, shadow_style)
        .expect("shadow owner stylesheet source id");
    let first_document_source = engine
        .owner_style_sheet_source_with_host(&host, document_style)
        .expect("document owner stylesheet source")
        .with_source_id(Some(first_document_source_id));
    let first_shadow_source = engine
        .owner_style_sheet_source_with_host(&host, shadow_style)
        .expect("shadow owner stylesheet source")
        .with_source_id(Some(first_shadow_source_id));
    let mut first_inputs = StyloComputedStyleInputs::default();
    first_inputs
        .document_stylesheet_sources
        .push(first_document_source);
    first_inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![first_shadow_source]));

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            outside,
            "color",
            None,
            &first_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
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
        Some("rgb(4, 5, 6)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    let generation_after_first_build =
        engine.computed_cache_generation_for_document_for_test(document);

    engine.set_owner_style_sheet_text_with_host(
        &host,
        document_style,
        "main { color: rgb(7, 8, 9); }".to_owned(),
    );
    engine.set_owner_style_sheet_text_with_host(
        &host,
        shadow_style,
        "span { color: rgb(10, 11, 12); }".to_owned(),
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    let second_document_source_id = StyleSourceId::owner_style_sheet(&host, document_style)
        .expect("document owner stylesheet source id");
    let second_shadow_source_id = StyleSourceId::owner_style_sheet(&host, shadow_style)
        .expect("shadow owner stylesheet source id");
    assert_eq!(
        engine.source_dirty_scope_ids_for_document_for_test(document),
        vec![
            StyleScopeId::Document(document),
            StyleScopeId::ShadowRoot(shadow_root)
        ]
    );
    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![
            second_document_source_id.clone(),
            second_shadow_source_id.clone()
        ]
    );
    assert_eq!(
        engine.source_dirty_scope_roots_for_document_for_test(document),
        vec![document, shadow_root, shadow_host]
    );

    let second_document_source = engine
        .owner_style_sheet_source_with_host(&host, document_style)
        .expect("document owner stylesheet source")
        .with_source_id(Some(second_document_source_id));
    let second_shadow_source = engine
        .owner_style_sheet_source_with_host(&host, shadow_style)
        .expect("shadow owner stylesheet source")
        .with_source_id(Some(second_shadow_source_id));
    let mut second_inputs = StyloComputedStyleInputs::default();
    second_inputs
        .document_stylesheet_sources
        .push(second_document_source);
    second_inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![second_shadow_source]));

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            outside,
            "color",
            None,
            &second_inputs,
            None,
        ),
        Some("rgb(7, 8, 9)".into())
    );
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
        Some("rgb(10, 11, 12)".into())
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
}

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
    let inputs = StyloComputedStyleInputs::default();
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
    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        detached_document,
        vec![source.clone()],
    );
    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs.document_stylesheet_sources.push(source);
    let active_inputs = StyloComputedStyleInputs::default();

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
    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        detached_document,
        vec![source.clone()],
    );
    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs.document_stylesheet_sources.push(source);
    let active_inputs = StyloComputedStyleInputs::default();

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
    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        detached_document,
        vec![source.clone()],
    );
    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs.document_stylesheet_sources.push(source);
    let active_inputs = StyloComputedStyleInputs::default();

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
    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        detached_document,
        vec![first_source.clone()],
    );
    assert_eq!(
        engine.adopted_style_sheet_source_owner_counts_for_document_for_test(document),
        (0, 0)
    );
    assert_eq!(
        engine.adopted_style_sheet_source_owner_counts_for_document_for_test(detached_document),
        (1, 0)
    );
    assert!(engine.document_adopted_style_sheet_tracks_document_for_test(detached_document));
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
    let mut first_detached_inputs = StyloComputedStyleInputs::default();
    first_detached_inputs
        .document_stylesheet_sources
        .push(first_source);
    let active_inputs = StyloComputedStyleInputs::default();
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
    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        detached_document,
        vec![second_source.clone()],
    );

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

    let mut second_detached_inputs = StyloComputedStyleInputs::default();
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
    let mut detached_inputs = StyloComputedStyleInputs::default();
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
    let active_inputs = StyloComputedStyleInputs::default();

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
fn ownerless_owner_stylesheet_change_does_not_use_active_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    assert!(host.append_child(document, active));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();

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

#[test]
fn explicit_linked_stylesheet_install_tracks_document_buckets() {
    let mut host = test_host();
    let document = host.document_handle();
    let active_link = host.create_element("link");
    let detached_document = host.create_detached_html_document();
    let detached_link = host.create_element("link");
    assert!(host.append_child(document, active_link));
    assert!(host.append_child(detached_document, detached_link));
    for link in [active_link, detached_link] {
        assert!(host.set_attribute(link, "rel", "stylesheet"));
        assert!(host.set_attribute(link, "href", "linked.css"));
    }

    let mut engine = MoliStyleEngine::new();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    let linked_source = StyloStylesheetSource::new(
        ".linked { color: rgb(1, 2, 3); }".to_owned(),
        linked_url.clone(),
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        linked_source.clone(),
        &[active_link, detached_link],
    );

    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1)
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(detached_document),
        (1, 1)
    );

    assert!(host.append_child(document, detached_link));
    engine.install_linked_stylesheet_source_with_host(
        &host,
        detached_link,
        &linked_url,
        linked_source,
    );

    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 2)
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(detached_document),
        (0, 0)
    );
}

#[test]
fn linked_stylesheet_sources_are_document_world_local() {
    let mut host = test_host();
    let document = host.document_handle();
    let active_link = host.create_element("link");
    let detached_document = host.create_detached_html_document();
    let detached_link = host.create_element("link");
    assert!(host.append_child(document, active_link));
    assert!(host.append_child(detached_document, detached_link));
    for link in [active_link, detached_link] {
        assert!(host.set_attribute(link, "rel", "stylesheet"));
        assert!(host.set_attribute(link, "href", "shared.css"));
    }

    let mut engine = MoliStyleEngine::new();
    let linked_url = url::Url::parse("https://example.test/shared.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(
            ".active { color: rgb(1, 2, 3); }".to_owned(),
            linked_url.clone(),
        )
        .with_origin_clean(false),
        &[active_link],
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(
            ".detached { color: rgb(4, 5, 6); }".to_owned(),
            linked_url.clone(),
        ),
        &[detached_link],
    );

    let active_source = engine
        .stylesheet_source_for_url_for_document_for_test(document, &linked_url)
        .expect("active document linked source");
    let detached_source = engine
        .stylesheet_source_for_url_for_document_for_test(detached_document, &linked_url)
        .expect("detached document linked source");
    assert_eq!(
        active_source.serialized_css_text().as_ref(),
        ".active { color: rgb(1, 2, 3); }"
    );
    assert!(!active_source.origin_clean());
    assert_eq!(
        detached_source.serialized_css_text().as_ref(),
        ".detached { color: rgb(4, 5, 6); }"
    );
    assert!(detached_source.origin_clean());
}

#[test]
fn removed_linked_stylesheet_owner_lifecycle_clears_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let link = host.create_element("link");
    assert!(host.append_child(document, active));
    assert!(host.append_child(document, link));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "linked.css"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new("main { color: rgb(1, 2, 3); }".into(), linked_url.clone()),
        &[link],
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1)
    );

    let inputs = StyloComputedStyleInputs::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
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

    let effects = host.remove_child_effects(document, link);
    engine.apply_stylesheet_owner_changes_with_host(&host, effects.stylesheet_owners().changes());
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &style_effects, &media);

    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (0, 0)
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}

#[test]
fn linked_stylesheet_owner_lifecycle_uses_final_remove_in_same_batch() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let link = host.create_element("link");
    assert!(host.append_child(document, active));
    assert!(host.append_child(document, link));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "linked.css"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new("main { color: rgb(1, 2, 3); }".into(), linked_url.clone()),
        &[link],
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1)
    );

    let inputs = StyloComputedStyleInputs::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
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

    let mut effects = host.remove_child_effects(document, link);
    effects.merge(host.append_child_effects(document, link));
    effects.merge(host.remove_child_effects(document, link));
    engine.apply_stylesheet_owner_changes_with_host(&host, effects.stylesheet_owners().changes());
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &style_effects, &media);

    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (0, 0)
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}

#[test]
fn inactive_linked_stylesheet_owner_lifecycle_clears_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let link = host.create_element("link");
    assert!(host.append_child(document, active));
    assert!(host.append_child(document, link));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "linked.css"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new("main { color: rgb(1, 2, 3); }".into(), linked_url.clone()),
        &[link],
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1)
    );

    let inputs = StyloComputedStyleInputs::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
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

    let effects = host.set_attribute_effects(link, "rel", "preload");
    engine.apply_stylesheet_owner_changes_with_host(&host, effects.stylesheet_owners().changes());
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &style_effects, &media);

    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (0, 0)
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}

#[test]
fn first_unknown_owner_linked_stylesheet_url_record_preserves_document_worlds() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
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
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                detached,
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
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );

    let linked_url = url::Url::parse("https://example.test/unknown-owner.css").unwrap();
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &linked_url,
        StyloStylesheetSource::new("main { color: green; }".into(), linked_url.clone()),
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (0, 0)
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(detached_document),
        (0, 0)
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
fn no_client_linked_stylesheet_url_update_preserves_document_worlds() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/unknown-owner.css").unwrap();
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &linked_url,
        StyloStylesheetSource::new("main { color: green; }".into(), linked_url.clone()),
    );

    let inputs = StyloComputedStyleInputs::default();
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
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                detached,
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
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );

    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &linked_url,
        StyloStylesheetSource::new("main { color: blue; }".into(), linked_url.clone()),
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
fn explicit_linked_stylesheet_install_uses_captured_url_not_live_href() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let link = host.create_element("link");
    assert!(host.append_child(document, active));
    assert!(host.append_child(document, link));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "current.css"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
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

    let stale_url = url::Url::parse("https://example.test/stale.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &stale_url,
        StyloStylesheetSource::new("main { color: rgb(1, 2, 3); }".into(), stale_url.clone()),
        &[link],
    );

    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1)
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
}

#[test]
fn ownerless_dom_link_url_update_does_not_register_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_link = host.create_element("link");
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached_link));
    assert!(host.append_child(detached_document, detached));
    assert!(host.set_attribute(detached_link, "rel", "stylesheet"));
    assert!(host.set_attribute(detached_link, "href", "discovered.css"));
    assert!(host.set_attribute(detached, "class", "target"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/discovered.css").unwrap();
    let first_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".to_owned(),
        linked_url.clone(),
    );
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &linked_url,
        first_source.clone(),
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (0, 0)
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(detached_document),
        (0, 0)
    );
    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs
        .document_stylesheet_sources
        .push(first_source);
    let active_inputs = StyloComputedStyleInputs::default();

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
                "display",
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

    let second_source = StyloStylesheetSource::new(
        ".target { color: rgb(4, 5, 6); }".to_owned(),
        linked_url.clone(),
    );
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &linked_url,
        second_source,
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
fn ownerless_stylesheet_network_result_does_not_discover_dom_link_owner() {
    let mut host = test_host();
    let document = host.document_handle();
    let link = host.create_element("link");
    let target = host.create_element("main");
    assert!(host.append_child(document, link));
    assert!(host.append_child(document, target));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "stale.css"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let stale_url = url::Url::parse("https://example.test/stale.css").unwrap();
    let inputs = StyloComputedStyleInputs::default();

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

    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &stale_url,
        StyloStylesheetSource::new(
            "main { color: rgb(1, 2, 3); }".to_owned(),
            stale_url.clone(),
        ),
        &[],
    );

    assert_eq!(
        engine
            .stylesheet_text_for_url_for_document_for_test(document, &stale_url)
            .as_deref(),
        None
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (0, 0)
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}

#[test]
fn unrelated_document_linked_source_does_not_disable_no_source_fast_path() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_link = host.create_element("link");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached_link));
    assert!(host.set_attribute(detached_link, "rel", "stylesheet"));
    assert!(host.set_attribute(detached_link, "href", "detached.css"));

    let mut engine = MoliStyleEngine::new();
    let linked_url = url::Url::parse("https://example.test/detached.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new("section.active { color: red; }".into(), linked_url.clone()),
        &[detached_link],
    );

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let epoch = engine.target_context_epoch_for_document_for_test(document);
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: active,
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
        0
    );
    assert_eq!(
        engine.target_context_epoch_for_document_for_test(document),
        epoch + 1
    );
}

#[test]
fn uninstalled_link_and_unrelated_url_do_not_disable_no_source_fast_path() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let link = host.create_element("link");
    assert!(host.append_child(document, active));
    assert!(host.append_child(document, link));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "missing.css"));

    let mut engine = MoliStyleEngine::new();
    let unrelated_url = url::Url::parse("https://example.test/unrelated.css").unwrap();
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &unrelated_url,
        StyloStylesheetSource::new("main.active { color: red; }".into(), unrelated_url.clone()),
    );

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let epoch = engine.target_context_epoch_for_document_for_test(document);
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: active,
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
        engine.target_context_epoch_for_document_for_test(document),
        epoch + 1
    );
}

#[test]
fn detached_document_linked_stylesheet_install_uses_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_link = host.create_element("link");
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached_link));
    assert!(host.append_child(detached_document, detached));
    assert!(host.set_attribute(detached_link, "rel", "stylesheet"));
    assert!(host.set_attribute(detached_link, "href", "linked.css"));
    assert!(host.set_attribute(detached, "class", "target"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    let linked_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".to_owned(),
        linked_url.clone(),
    );
    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs
        .document_stylesheet_sources
        .push(linked_source.clone());
    let active_inputs = StyloComputedStyleInputs::default();

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

    engine.install_linked_stylesheet_source_with_host(
        &host,
        detached_link,
        &linked_url,
        linked_source,
    );

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
fn detached_document_linked_stylesheet_source_change_uses_link_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_link = host.create_element("link");
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached_link));
    assert!(host.append_child(detached_document, detached));
    assert!(host.set_attribute(detached_link, "rel", "stylesheet"));
    assert!(host.set_attribute(detached_link, "href", "linked.css"));
    assert!(host.set_attribute(detached, "class", "target"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    let first_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".to_owned(),
        linked_url.clone(),
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        first_source.clone(),
        &[detached_link],
    );
    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs
        .document_stylesheet_sources
        .push(first_source);
    let active_inputs = StyloComputedStyleInputs::default();

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

    let second_source = StyloStylesheetSource::new(
        ".target { color: rgb(4, 5, 6); }".to_owned(),
        linked_url.clone(),
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        second_source,
        &[detached_link],
    );

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
fn linked_stylesheet_source_rebuild_uses_document_dirty_root() {
    let mut host = test_host();
    let document = host.document_handle();
    let link = host.create_element("link");
    let outside = host.create_element("main");
    let target = host.create_element("section");
    assert!(host.append_child(document, link));
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, target));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "linked.css"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(
            "section { color: rgb(1, 2, 3); }".to_owned(),
            linked_url.clone(),
        ),
        &[link],
    );
    let first_source_id =
        StyleSourceId::linked_style_sheet(&host, link).expect("document linked source id");
    let first_source = engine
        .stylesheet_source_for_url_for_document_for_test(document, &linked_url)
        .expect("document linked source")
        .with_source_id(Some(first_source_id));
    let mut first_inputs = StyloComputedStyleInputs::default();
    first_inputs.document_stylesheet_sources.push(first_source);

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
            target,
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

    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(
            "section { color: rgb(4, 5, 6); }".to_owned(),
            linked_url.clone(),
        ),
        &[link],
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let second_source_id =
        StyleSourceId::linked_style_sheet(&host, link).expect("document linked source id");
    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![second_source_id.clone()]
    );
    assert_eq!(
        engine.source_dirty_scope_ids_for_document_for_test(document),
        vec![StyleScopeId::Document(document)]
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::LinkedStyleSheet]
    );
    assert_eq!(
        engine.source_dirty_scope_roots_for_document_for_test(document),
        vec![document]
    );
    let second_source = engine
        .stylesheet_source_for_url_for_document_for_test(document, &linked_url)
        .expect("document linked source")
        .with_source_id(Some(second_source_id));
    let mut second_inputs = StyloComputedStyleInputs::default();
    second_inputs
        .document_stylesheet_sources
        .push(second_source);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
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
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}

#[test]
fn shadow_linked_stylesheet_source_rebuild_uses_scoped_dirty_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let outside = host.create_element("main");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_link = host.create_element("link");
    let shadow_child = host.create_element("span");
    assert!(host.append_child(document, outside));
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_link));
    assert!(host.append_child(shadow_root, shadow_child));
    assert!(host.set_attribute(shadow_link, "rel", "stylesheet"));
    assert!(host.set_attribute(shadow_link, "href", "linked.css"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(
            "span { color: rgb(1, 2, 3); }".to_owned(),
            linked_url.clone(),
        ),
        &[shadow_link],
    );
    let first_source_id =
        StyleSourceId::linked_style_sheet(&host, shadow_link).expect("shadow linked source id");
    let first_source = engine
        .stylesheet_source_for_url_for_document_for_test(document, &linked_url)
        .expect("shadow linked source")
        .with_source_id(Some(first_source_id));
    let mut first_inputs = StyloComputedStyleInputs::default();
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
    let generation_after_first_build =
        engine.computed_cache_generation_for_document_for_test(document);

    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(
            "span { color: rgb(4, 5, 6); }".to_owned(),
            linked_url.clone(),
        ),
        &[shadow_link],
    );

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );

    let second_source_id =
        StyleSourceId::linked_style_sheet(&host, shadow_link).expect("shadow linked source id");
    assert_eq!(
        engine.source_dirty_scope_source_ids_for_document_for_test(document),
        vec![second_source_id.clone()]
    );
    assert_eq!(
        engine.source_dirty_scope_ids_for_document_for_test(document),
        vec![StyleScopeId::ShadowRoot(shadow_root)]
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::LinkedStyleSheet]
    );
    assert_eq!(
        engine.source_dirty_scope_roots_for_document_for_test(document),
        vec![shadow_root, shadow_host]
    );
    let second_source = engine
        .stylesheet_source_for_url_for_document_for_test(document, &linked_url)
        .expect("shadow linked source")
        .with_source_id(Some(second_source_id));
    let mut second_inputs = StyloComputedStyleInputs::default();
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
        generation_after_first_build
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
    );
}

#[test]
fn linked_stylesheet_final_url_update_uses_current_owner_document_after_owner_moves() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let link = host.create_element("link");
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, link));
    assert!(host.append_child(detached_document, detached));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "linked.css"));
    assert!(host.set_attribute(active, "class", "target"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let request_url = url::Url::parse("https://example.test/linked.css").unwrap();
    let final_url = url::Url::parse("https://cdn.example.test/linked.css").unwrap();
    let first_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".to_owned(),
        final_url.clone(),
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &request_url,
        first_source.clone(),
        &[link],
    );

    assert!(host.append_child(document, link));
    engine.install_linked_stylesheet_source_with_host(
        &host,
        link,
        &request_url,
        first_source.clone(),
    );

    let mut active_inputs = StyloComputedStyleInputs::default();
    active_inputs.document_stylesheet_sources.push(first_source);
    let detached_inputs = StyloComputedStyleInputs::default();

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            active,
            "color",
            None,
            &active_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
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

    let second_source = StyloStylesheetSource::new(
        ".target { color: rgb(4, 5, 6); }".to_owned(),
        final_url.clone(),
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &final_url,
        second_source,
        &[link],
    );

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
fn detached_document_url_source_change_uses_explicit_source_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_link = host.create_element("link");
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached_link));
    assert!(host.append_child(detached_document, detached));
    assert!(host.set_attribute(detached_link, "rel", "stylesheet"));
    assert!(host.set_attribute(detached_link, "href", "linked.css"));
    assert!(host.set_attribute(detached, "class", "target"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    let first_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".to_owned(),
        linked_url.clone(),
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        first_source.clone(),
        &[detached_link],
    );
    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs
        .document_stylesheet_sources
        .push(first_source);
    let active_inputs = StyloComputedStyleInputs::default();

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

    let second_source = StyloStylesheetSource::new(
        ".target { color: rgb(4, 5, 6); }".to_owned(),
        linked_url.clone(),
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        second_source,
        &[detached_link],
    );

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
fn detached_document_url_source_explicit_owner_change_uses_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_link = host.create_element("link");
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached_link));
    assert!(host.append_child(detached_document, detached));
    assert!(host.set_attribute(detached_link, "rel", "stylesheet"));
    assert!(host.set_attribute(detached, "class", "target"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    let source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".to_owned(),
        linked_url.clone(),
    );
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &linked_url,
        source.clone(),
    );
    assert!(host.set_attribute(detached_link, "href", "linked.css"));

    let active_inputs = StyloComputedStyleInputs::default();
    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs
        .document_stylesheet_sources
        .push(source.clone());

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

    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        source,
        &[detached_link],
    );

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
fn explicit_linked_source_install_does_not_rederive_missing_live_href() {
    let mut host = test_host();
    let document = host.document_handle();
    let link = host.create_element("link");
    let target = host.create_element("main");
    assert!(host.append_child(document, link));
    assert!(host.append_child(document, target));
    assert!(host.set_attribute(link, "rel", "stylesheet"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let stale_url = url::Url::parse("https://example.test/stale.css").unwrap();
    let inputs = StyloComputedStyleInputs::default();

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
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &stale_url,
        StyloStylesheetSource::new(
            "main { color: rgb(1, 2, 3); }".to_owned(),
            stale_url.clone(),
        ),
        &[link],
    );

    assert_eq!(
        engine
            .stylesheet_text_for_url_for_document_for_test(document, &stale_url)
            .as_deref(),
        Some("main { color: rgb(1, 2, 3); }")
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1)
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}

#[test]
fn explicit_linked_source_install_is_not_rejected_by_live_disabled_state() {
    let mut host = test_host();
    let document = host.document_handle();
    let link = host.create_element("link");
    let target = host.create_element("main");
    assert!(host.append_child(document, link));
    assert!(host.append_child(document, target));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "linked.css"));
    assert!(host.set_attribute(link, "disabled", ""));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    let inputs = StyloComputedStyleInputs::default();

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
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(
            "main { color: rgb(1, 2, 3); }".to_owned(),
            linked_url.clone(),
        ),
        &[link],
    );

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}

#[test]
fn ownerless_final_url_source_update_preserves_document_worlds() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_link = host.create_element("link");
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached_link));
    assert!(host.append_child(detached_document, detached));
    assert!(host.set_attribute(detached_link, "rel", "stylesheet"));
    assert!(host.set_attribute(detached_link, "href", "linked.css"));
    assert!(host.set_attribute(detached, "class", "target"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let request_url = url::Url::parse("https://example.test/linked.css").unwrap();
    let final_url = url::Url::parse("https://cdn.example.test/linked.css").unwrap();
    let first_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".to_owned(),
        final_url.clone(),
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &request_url,
        first_source.clone(),
        &[detached_link],
    );
    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs
        .document_stylesheet_sources
        .push(first_source);
    let active_inputs = StyloComputedStyleInputs::default();

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

    let second_source = StyloStylesheetSource::new(
        ".target { color: rgb(4, 5, 6); }".to_owned(),
        final_url.clone(),
    );
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &final_url,
        second_source,
    );

    assert_eq!(
        engine.stylesheet_text_for_url_for_document_for_test(document, &final_url),
        Some(".target { color: rgb(4, 5, 6); }".to_owned())
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
fn detached_document_inline_style_metadata_uses_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/page.html").unwrap();
    let detached_base_url = url::Url::parse("https://detached.test/cssom/").unwrap();
    engine.set_inline_style_base_url_with_host(&host, detached, detached_base_url.clone());
    engine.set_inline_style_resolution_text_with_host(
        &host,
        detached,
        "background-image: url(icon.png);".to_owned(),
    );

    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(detached_document),
        1
    );
    assert_eq!(
        engine.inline_style_base_url_with_host(&host, detached),
        Some(detached_base_url)
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            detached,
            "background-image",
            None,
            &StyloComputedStyleInputs::default(),
            None,
        ),
        Some("url(\"https://detached.test/cssom/icon.png\")".into())
    );

    engine.clear_inline_style_base_url_with_host(&host, detached);
    engine.clear_inline_style_resolution_text_with_host(&host, detached);

    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(detached_document),
        0
    );
}

#[test]
fn inline_style_metadata_moves_to_current_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("section");
    let detached_document = host.create_detached_html_document();
    assert!(host.append_child(document, target));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/page.html").unwrap();
    let cssom_base_url = url::Url::parse("https://example.test/cssom/").unwrap();
    engine.set_inline_style_base_url_with_host(&host, target, cssom_base_url);
    engine.set_inline_style_resolution_text_with_host(
        &host,
        target,
        "background-image: url(icon.png);".to_owned(),
    );
    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(document),
        1
    );

    assert!(host.append_child(detached_document, target));
    engine.migrate_inline_style_metadata_subtree_with_host(&host, target);

    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(document),
        0,
        "old document world must not retain moved inline style metadata"
    );
    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(detached_document),
        1
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "background-image",
            None,
            &StyloComputedStyleInputs::default(),
            None,
        ),
        Some("url(\"https://example.test/cssom/icon.png\")".into())
    );
}

#[test]
fn detached_shadow_adopted_stylesheet_change_uses_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_host = host.create_element("section");
    let detached_shadow_root = host
        .attach_shadow_root(detached_host, "open")
        .expect("detached section should host a shadow root");
    let detached = host.create_element("span");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached_host));
    assert!(host.append_child(detached_shadow_root, detached));
    assert!(host.set_attribute(detached, "class", "target"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        detached_shadow_root,
        vec![first_source.clone()],
    );
    let detached_shadow_sources =
        engine.shadow_root_adopted_style_sheet_sources_with_host(&host, detached_shadow_root);
    assert_eq!(
        detached_shadow_sources[0].serialized_css_text().as_ref(),
        ".target { color: rgb(1, 2, 3); }"
    );
    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs
        .shadow_stylesheet_sources
        .push((detached_shadow_root, detached_shadow_sources));
    let active_inputs = StyloComputedStyleInputs::default();

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

    let second_source =
        StyloStylesheetSource::new(".target { color: rgb(4, 5, 6); }".to_owned(), document_url);
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        detached_shadow_root,
        vec![second_source],
    );

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
fn matching_dependency_sources_have_explicit_source_and_scope_ids() {
    let mut host = test_host();
    let document = host.document_handle();
    let document_style = host.create_element("style");
    let document_style_text = host.create_text_node(".document { color: green; }");
    assert!(host.append_child(document_style, document_style_text));
    assert!(host.append_child(document, document_style));

    let linked_style = host.create_element("link");
    assert!(host.set_attribute(linked_style, "rel", "stylesheet"));
    assert!(host.set_attribute(linked_style, "href", "linked.css"));
    assert!(host.append_child(document, linked_style));

    let shadow_host = host.create_element("section");
    assert!(host.append_child(document, shadow_host));
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_style = host.create_element("style");
    let shadow_style_text = host.create_text_node(".shadow { color: blue; }");
    assert!(host.append_child(shadow_style, shadow_style_text));
    assert!(host.append_child(shadow_root, shadow_style));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        document_style,
        ".document { color: green; }".into(),
    );
    engine.set_owner_style_sheet_text_with_host(
        &host,
        shadow_style,
        ".shadow { color: blue; }".into(),
    );

    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(".linked { color: purple; }".into(), linked_url.clone()),
        &[linked_style],
    );

    engine.set_document_adopted_style_sheet_sources_with_host(
        &host,
        document,
        vec![
            StyloStylesheetSource::new(
                ".document-adopted-a { color: black; }".into(),
                url::Url::parse("https://example.test/a.css").unwrap(),
            ),
            StyloStylesheetSource::new(
                ".document-adopted-b { color: gray; }".into(),
                url::Url::parse("https://example.test/b.css").unwrap(),
            ),
        ],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![StyloStylesheetSource::new(
            ".shadow-adopted { color: orange; }".into(),
            url::Url::parse("https://example.test/shadow.css").unwrap(),
        )],
    );

    let source_scope = StyleSourceScope::for_document_and_connected_shadow_roots(&host, document);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let sources = engine.matching_dependency_source_ids_for_document_for_test(
        &host,
        document,
        &source_scope,
        &media,
    );

    let document_style_id = StyleSourceId {
        scope_id: StyleScopeId::Document(document),
        kind: StyleSourceKind::OwnerStyleSheet {
            owner: document_style,
        },
    };
    let linked_style_id = StyleSourceId {
        scope_id: StyleScopeId::Document(document),
        kind: StyleSourceKind::LinkedStyleSheet {
            owner: linked_style,
        },
    };
    let shadow_style_id = StyleSourceId {
        scope_id: StyleScopeId::ShadowRoot(shadow_root),
        kind: StyleSourceKind::OwnerStyleSheet {
            owner: shadow_style,
        },
    };
    let document_adopted_a_id = StyleSourceId {
        scope_id: StyleScopeId::Document(document),
        kind: StyleSourceKind::DocumentAdoptedStyleSheet { index: 0 },
    };
    let document_adopted_b_id = StyleSourceId {
        scope_id: StyleScopeId::Document(document),
        kind: StyleSourceKind::DocumentAdoptedStyleSheet { index: 1 },
    };
    let shadow_adopted_id = StyleSourceId {
        scope_id: StyleScopeId::ShadowRoot(shadow_root),
        kind: StyleSourceKind::ShadowRootAdoptedStyleSheet { index: 0 },
    };

    for id in [
        document_style_id,
        linked_style_id,
        shadow_style_id.clone(),
        document_adopted_a_id,
        document_adopted_b_id,
        shadow_adopted_id,
    ] {
        assert!(
            sources.iter().any(|(source_id, _)| source_id == &id),
            "missing source id {id:?}; sources={sources:?}"
        );
    }

    let (_, shadow_fallback_roots) = sources
        .iter()
        .find(|(source_id, _)| source_id == &shadow_style_id)
        .expect("shadow style source should be present");
    assert!(shadow_fallback_roots.contains(&shadow_root));
    assert!(shadow_fallback_roots.contains(&shadow_host));
    assert!(!shadow_fallback_roots.contains(&document));
}
#[test]
fn style_system_cache_key_ignores_document_fragment() {
    let old_url = url::Url::parse("https://example.test/page.html#old").unwrap();
    let new_url = url::Url::parse("https://example.test/page.html#new").unwrap();
    let mut inputs = StyloComputedStyleInputs::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { color: green; }".to_owned(),
            old_url.clone(),
        ));
    let mut next_inputs = StyloComputedStyleInputs::default();
    next_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { color: green; }".to_owned(),
            new_url.clone(),
        ));

    assert_eq!(
        StyleSystemCacheKey::new(&old_url, &inputs, None),
        StyleSystemCacheKey::new(&new_url, &next_inputs, None)
    );
}

#[test]
fn style_system_cache_key_changes_when_screen_size_changes() {
    let document_url = url::Url::parse("https://example.test/page.html").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    let viewport =
        StyleViewport::new(Some(800.0), Some(600.0)).with_screen_size(Some(1920.0), Some(1080.0));
    let next_viewport =
        StyleViewport::new(Some(800.0), Some(600.0)).with_screen_size(Some(1366.0), Some(768.0));

    assert_ne!(
        StyleSystemCacheKey::new(&document_url, &inputs, viewport),
        StyleSystemCacheKey::new(&document_url, &inputs, next_viewport)
    );
}

#[test]
fn style_system_cache_key_changes_when_stylesheet_text_changes() {
    let document_url = url::Url::parse("https://example.test/page.html").unwrap();
    let mut inputs = StyloComputedStyleInputs::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { color: green; }".to_owned(),
            document_url.clone(),
        ));
    let mut next_inputs = StyloComputedStyleInputs::default();
    next_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { color: blue; }".to_owned(),
            document_url.clone(),
        ));

    assert_ne!(
        StyleSystemCacheKey::new(&document_url, &inputs, None),
        StyleSystemCacheKey::new(&document_url, &next_inputs, None)
    );
}
#[test]
fn style_system_cache_key_changes_when_stylesheet_base_url_changes() {
    let document_url = url::Url::parse("https://example.test/page.html").unwrap();
    let old_base = url::Url::parse("https://example.test/assets/app.css").unwrap();
    let next_base = url::Url::parse("https://cdn.example.test/assets/app.css").unwrap();
    let mut inputs = StyloComputedStyleInputs::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { background-image: url(icon.png); }".to_owned(),
            old_base,
        ));
    let mut next_inputs = StyloComputedStyleInputs::default();
    next_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { background-image: url(icon.png); }".to_owned(),
            next_base,
        ));

    assert_ne!(
        StyleSystemCacheKey::new(&document_url, &inputs, None),
        StyleSystemCacheKey::new(&document_url, &next_inputs, None)
    );
}
#[test]
fn style_system_cache_key_changes_when_document_quirks_mode_changes() {
    let document_url = url::Url::parse("https://example.test/page.html").unwrap();
    let standards_inputs = StyloComputedStyleInputs::default();
    let quirks_inputs = StyloComputedStyleInputs {
        quirks_mode: style::context::QuirksMode::Quirks,
        ..Default::default()
    };

    assert_ne!(
        StyleSystemCacheKey::new(&document_url, &standards_inputs, None),
        StyleSystemCacheKey::new(&document_url, &quirks_inputs, None)
    );
}

#[test]
fn style_system_cache_key_mismatch_trace_records_changed_dimensions() {
    let previous_url = url::Url::parse("https://example.test/old.html").unwrap();
    let next_url = url::Url::parse("https://example.test/new.html").unwrap();
    let shadow_root = NativeNodeId::new(10);
    let removed_shadow_root = NativeNodeId::new(11);
    let added_shadow_root = NativeNodeId::new(12);

    let mut previous_inputs = StyloComputedStyleInputs::default();
    previous_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { color: rgb(1, 2, 3); }".to_owned(),
            previous_url.clone(),
        ));
    previous_inputs.shadow_stylesheet_sources.push((
        shadow_root,
        vec![StyloStylesheetSource::new(
            "#target { color: rgb(4, 5, 6); }".to_owned(),
            previous_url.clone(),
        )],
    ));
    previous_inputs.shadow_stylesheet_sources.push((
        removed_shadow_root,
        vec![StyloStylesheetSource::new(
            "#removed { color: rgb(7, 8, 9); }".to_owned(),
            previous_url.clone(),
        )],
    ));

    let mut next_inputs = previous_inputs.clone();
    next_inputs.environment = StyloStyleEnvironment::from_emulated_media(
        &crate::protocol_types::EmulatedMediaOverrides {
            media: Some("print".to_owned()),
            ..Default::default()
        },
    );
    next_inputs.quirks_mode = style::context::QuirksMode::Quirks;
    next_inputs
        .script_custom_property_registrations
        .push(CssCustomPropertyRegistration {
            name: "--accent".to_owned(),
            syntax: "<color>".to_owned(),
            inherits: false,
            initial_value: Some("red".to_owned()),
        });
    next_inputs.document_stylesheet_sources = vec![StyloStylesheetSource::new(
        ".probe { color: rgb(10, 11, 12); }".to_owned(),
        next_url.clone(),
    )];
    next_inputs.shadow_stylesheet_sources = vec![
        (
            shadow_root,
            vec![StyloStylesheetSource::new(
                "#target { color: rgb(13, 14, 15); }".to_owned(),
                next_url.clone(),
            )],
        ),
        (
            added_shadow_root,
            vec![StyloStylesheetSource::new(
                "#added { color: rgb(16, 17, 18); }".to_owned(),
                next_url.clone(),
            )],
        ),
    ];

    let previous_key = StyleSystemCacheKey::new(
        &previous_url,
        &previous_inputs,
        StyleViewport::new(Some(800.0), Some(600.0)).with_screen_size(Some(1920.0), Some(1080.0)),
    );
    let next_key = StyleSystemCacheKey::new(
        &next_url,
        &next_inputs,
        StyleViewport::new(Some(1024.0), Some(768.0)).with_screen_size(Some(1366.0), Some(768.0)),
    );

    let trace = previous_key.mismatch_trace(&next_key);

    assert!(trace.document_url_changed);
    assert_eq!(trace.previous_document_url, previous_url);
    assert_eq!(trace.next_document_url, next_url);
    assert!(trace.viewport_changed);
    assert!(trace.screen_changed);
    assert!(trace.environment_changed);
    assert!(trace.quirks_mode_changed);
    assert!(trace.custom_property_registrations_changed);
    assert!(trace.document_stylesheet_sources_changed);
    assert_eq!(trace.previous_document_stylesheet_sources.len, 1);
    assert_eq!(trace.next_document_stylesheet_sources.len, 1);
    assert!(trace.shadow_root_list_changed);
    assert_eq!(
        trace.previous_shadow_roots,
        vec![shadow_root, removed_shadow_root]
    );
    assert_eq!(
        trace.next_shadow_roots,
        vec![shadow_root, added_shadow_root]
    );
    assert_eq!(trace.added_shadow_roots, vec![added_shadow_root]);
    assert_eq!(trace.removed_shadow_roots, vec![removed_shadow_root]);
    assert!(trace.shadow_stylesheet_sources_changed);
    assert_eq!(trace.changed_shadow_source_roots, vec![shadow_root]);
    assert_eq!(trace.previous_shadow_stylesheet_sources.len(), 2);
    assert_eq!(trace.next_shadow_stylesheet_sources.len(), 2);
}

#[test]
fn computed_style_read_trace_records_owner_read_and_drain_documents() {
    let mut host = test_host();
    let active_document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(active_document, active));
    assert!(host.append_child(detached_document, detached));
    let document_url = url::Url::parse("https://example.test/trace.html").unwrap();

    let trace = super::super::computed::computed_style_read_trace_for_test(
        &host,
        &document_url,
        detached,
        detached_document,
        "color",
        Some("::before"),
        StyleSourceDocumentContext::for_root_document(active_document),
    )
    .expect("detached element should have an owner document");

    assert_eq!(trace.document_url, document_url);
    assert_eq!(trace.target, detached);
    assert_eq!(trace.owner_document, detached_document);
    assert_eq!(trace.read_document, detached_document);
    assert_eq!(trace.property, "color");
    assert_eq!(trace.pseudo_element.as_deref(), Some("::before"));
    assert_eq!(trace.document_context_documents, vec![active_document]);
    assert_eq!(
        trace.drain_documents,
        vec![active_document, detached_document]
    );
}

#[test]
fn retained_style_system_source_input_trace_records_source_ids_and_shadow_roots() {
    let document = NativeNodeId::new(20);
    let shadow_root = NativeNodeId::new(21);
    let document_url = url::Url::parse("https://example.test/source-input.html").unwrap();
    let document_source_id = StyleSourceId::document_adopted_style_sheet(document, 0);
    let shadow_source_id = StyleSourceId::shadow_root_adopted_style_sheet(shadow_root, 1);
    let mut inputs = StyloComputedStyleInputs {
        document_stylesheet_sources: vec![
            StyloStylesheetSource::new(
                "body { color: rgb(1, 2, 3); }".to_owned(),
                document_url.clone(),
            )
            .with_source_id(Some(document_source_id.clone())),
            StyloStylesheetSource::new(
                ".anonymous { color: rgb(4, 5, 6); }".to_owned(),
                document_url.clone(),
            ),
        ],
        script_custom_property_base_url: document_url.clone(),
        ..Default::default()
    };
    inputs.shadow_stylesheet_sources.push((
        shadow_root,
        vec![
            StyloStylesheetSource::new(
                ":host { color: rgb(7, 8, 9); }".to_owned(),
                document_url.clone(),
            )
            .with_source_id(Some(shadow_source_id.clone())),
        ],
    ));
    inputs
        .script_custom_property_registrations
        .push(CssCustomPropertyRegistration {
            name: "--accent".to_owned(),
            syntax: "<color>".to_owned(),
            inherits: true,
            initial_value: Some("blue".to_owned()),
        });

    let trace = super::super::system::style_source_input_trace_for_test(&inputs);

    assert_eq!(trace.document_stylesheet_source_count, 2);
    assert_eq!(
        trace.document_source_ids,
        vec![Some(document_source_id), None]
    );
    assert_eq!(trace.shadow_stylesheet_sources.len(), 1);
    assert_eq!(trace.shadow_stylesheet_sources[0].root, shadow_root);
    assert_eq!(trace.shadow_stylesheet_sources[0].source_count, 1);
    assert_eq!(
        trace.shadow_stylesheet_sources[0].source_ids,
        vec![Some(shadow_source_id)]
    );
    assert_eq!(trace.script_custom_property_registration_count, 1);
    assert_eq!(trace.script_custom_property_base_url, document_url);
}

#[test]
fn quirks_mode_stylesheet_sources_match_ids_case_insensitively() {
    let mut host = test_host();
    let document = host.document_handle();
    host.set_html_quirks_mode_for_parser(html5ever::tree_builder::QuirksMode::Quirks);

    let target = host.create_element("div");
    assert!(host.set_attribute(target, "id", "foo"));
    assert!(host.append_child(document, target));

    let document_url = url::Url::parse("https://example.test/page.html").unwrap();
    let mut standards_inputs = StyloComputedStyleInputs::default();
    standards_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            "#FoO { background-color: rgb(0, 128, 0); }".to_owned(),
            document_url.clone(),
        ));
    let mut quirks_inputs = standards_inputs.clone();
    quirks_inputs.quirks_mode = style::context::QuirksMode::Quirks;

    let engine = MoliStyleEngine::new();
    let standards_background = engine
        .computed_style_property_value(
            &host,
            &document_url,
            target,
            "background-color",
            None,
            &standards_inputs,
            None,
        )
        .expect("standards-mode background should compute");
    assert_ne!(standards_background, "rgb(0, 128, 0)");

    let quirks_background = engine
        .computed_style_property_value(
            &host,
            &document_url,
            target,
            "background-color",
            None,
            &quirks_inputs,
            None,
        )
        .expect("quirks-mode background should compute");
    assert_eq!(quirks_background, "rgb(0, 128, 0)");
}
#[test]
fn stylesheet_invalidation_clears_retained_style_system() {
    let host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    let key = StyleSystemCacheKey::new(&document_url, &inputs, None);

    engine.ensure_retained_style_system_for_document(
        &host,
        host.document_handle(),
        key.clone(),
        &inputs,
    );

    assert!(engine.retained_style_system_matches_for_document_for_test(document, &key));

    engine.invalidate_author_stylesheet_set_for_document_with_host(&host, document);

    assert!(engine.retained_style_system_is_none_for_document_for_test(document));
    assert!(engine.computed_style_cache_entry_count_for_document_for_test(document) == 0);
}
#[test]
fn style_subtree_invalidation_retains_style_system() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("section");
    assert!(host.append_child(document, target));
    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    let key = StyleSystemCacheKey::new(&document_url, &inputs, None);

    engine.ensure_retained_style_system_for_document(
        &host,
        host.document_handle(),
        key.clone(),
        &inputs,
    );
    engine.invalidate_style_subtree(&host, target);

    assert!(engine.retained_style_system_matches_for_document_for_test(document, &key));
    assert!(engine.computed_style_cache_entry_count_for_document_for_test(document) == 0);
}

#[test]
fn retained_style_system_keeps_cascade_data_for_empty_shadow_scopes() {
    let mut host = test_host();
    let document = host.document_handle();
    let open_host = host.create_element("section");
    let closed_host = host.create_element("article");
    assert!(host.append_child(document, open_host));
    assert!(host.append_child(document, closed_host));
    let open_root = host
        .attach_shadow_root(open_host, "open")
        .expect("open host should accept a shadow root");
    let closed_root = host
        .attach_shadow_root(closed_host, "closed")
        .expect("closed host should accept a shadow root");

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/empty-scopes.html").unwrap();
    let mut inputs = StyloComputedStyleInputs::default();
    inputs
        .shadow_stylesheet_sources
        .push((open_root, Vec::new()));
    inputs
        .shadow_stylesheet_sources
        .push((closed_root, Vec::new()));
    let key = StyleSystemCacheKey::new(&document_url, &inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);

    engine.with_retained_style_system_for_document_for_test(document, |retained| {
        let roots = retained
            .shadow_cascade_data
            .iter()
            .map(|(root, _)| *root)
            .collect::<Vec<_>>();
        assert_eq!(roots, vec![open_root, closed_root]);
    });
}

#[test]
fn style_subtree_invalidation_clears_only_affected_shadow_cascade_data() {
    let mut host = test_host();
    let document = host.document_handle();
    let first_shadow_host = host.create_element("section");
    let second_shadow_host = host.create_element("article");
    assert!(host.append_child(document, first_shadow_host));
    assert!(host.append_child(document, second_shadow_host));
    let first_shadow_root = host
        .attach_shadow_root(first_shadow_host, "open")
        .expect("first host should accept a shadow root");
    let second_shadow_root = host
        .attach_shadow_root(second_shadow_host, "open")
        .expect("second host should accept a shadow root");

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        ":host { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    let second_source = StyloStylesheetSource::new(
        ":host { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        first_shadow_root,
        vec![first_source.clone()],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        second_shadow_root,
        vec![second_source.clone()],
    );
    let mut inputs = StyloComputedStyleInputs::default();
    inputs
        .shadow_stylesheet_sources
        .push((first_shadow_root, vec![first_source]));
    inputs
        .shadow_stylesheet_sources
        .push((second_shadow_root, vec![second_source]));
    let key = StyleSystemCacheKey::new(&document_url, &inputs, None);
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

    engine.invalidate_style_subtree(&host, first_shadow_host);

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
fn detached_subtree_invalidation_clears_only_affected_shadow_cascade_data() {
    let mut host = test_host();
    let document = host.document_handle();
    let first_shadow_host = host.create_element("section");
    let second_shadow_host = host.create_element("article");
    assert!(host.append_child(document, first_shadow_host));
    assert!(host.append_child(document, second_shadow_host));
    let first_shadow_root = host
        .attach_shadow_root(first_shadow_host, "open")
        .expect("first host should accept a shadow root");
    let second_shadow_root = host
        .attach_shadow_root(second_shadow_host, "open")
        .expect("second host should accept a shadow root");

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        ":host { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    let second_source = StyloStylesheetSource::new(
        ":host { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        first_shadow_root,
        vec![first_source.clone()],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        second_shadow_root,
        vec![second_source.clone()],
    );
    let mut inputs = StyloComputedStyleInputs::default();
    inputs
        .shadow_stylesheet_sources
        .push((first_shadow_root, vec![first_source]));
    inputs
        .shadow_stylesheet_sources
        .push((second_shadow_root, vec![second_source]));
    let key = StyleSystemCacheKey::new(&document_url, &inputs, None);
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::DisconnectedSubtree {
            root: first_shadow_host,
        }],
        &media,
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
fn document_stylesheet_cleanup_preserves_other_document_shadow_cascade_data() {
    let mut host = test_host();
    let document = host.document_handle();
    let active_shadow_host = host.create_element("section");
    assert!(host.append_child(document, active_shadow_host));
    let active_shadow_root = host
        .attach_shadow_root(active_shadow_host, "open")
        .expect("active host should accept a shadow root");

    let detached_document = host.create_detached_html_document();
    let detached_shadow_host = host.create_element("article");
    assert!(host.append_child(detached_document, detached_shadow_host));
    let detached_shadow_root = host
        .attach_shadow_root(detached_shadow_host, "open")
        .expect("detached host should accept a shadow root");

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let active_source = StyloStylesheetSource::new(
        ":host { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    let detached_source = StyloStylesheetSource::new(
        ":host { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        active_shadow_root,
        vec![active_source.clone()],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        detached_shadow_root,
        vec![detached_source.clone()],
    );

    let mut active_inputs = StyloComputedStyleInputs::default();
    active_inputs
        .shadow_stylesheet_sources
        .push((active_shadow_root, vec![active_source]));
    let active_key = StyleSystemCacheKey::new(&document_url, &active_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, active_key, &active_inputs);

    let mut detached_inputs = StyloComputedStyleInputs::default();
    detached_inputs
        .shadow_stylesheet_sources
        .push((detached_shadow_root, vec![detached_source]));
    let detached_key = StyleSystemCacheKey::new(&document_url, &detached_inputs, None);
    engine.ensure_retained_style_system_for_document(
        &host,
        detached_document,
        detached_key,
        &detached_inputs,
    );

    let active_cascade_data =
        engine.with_retained_style_system_for_document_for_test(document, |retained| {
            retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == active_shadow_root)
                .expect("active retained system should track the active shadow root")
                .1
                .clone()
        });
    let detached_cascade_data =
        engine.with_retained_style_system_for_document_for_test(detached_document, |retained| {
            retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == detached_shadow_root)
                .expect("detached retained system should track the detached shadow root")
                .1
                .clone()
        });
    ensure_adapter_element_data(&engine, &host, active_shadow_host);
    ensure_adapter_element_data(&engine, &host, detached_shadow_host);
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            active_shadow_root,
            active_cascade_data,
        );
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            detached_document,
            detached_shadow_root,
            detached_cascade_data,
        );
    assert!(engine.dom_adapter.has_element_data(active_shadow_host));
    assert!(engine.dom_adapter.has_element_data(detached_shadow_host));
    assert_eq!(
        engine.dom_adapter.shadow_cascade_document_count_for_test(),
        2
    );

    engine.invalidate_author_stylesheet_set_for_document_with_host(&host, detached_document);

    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(active_shadow_root)
    );
    assert!(
        !engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(detached_shadow_root)
    );
    assert!(engine.dom_adapter.has_element_data(active_shadow_host));
    assert!(!engine.dom_adapter.has_element_data(detached_shadow_host));
    assert_eq!(
        engine.dom_adapter.shadow_cascade_document_count_for_test(),
        1
    );
}

#[test]
fn document_replacement_cleanup_preserves_unreplaced_document_world_and_shared_sources() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let active_link = host.create_element("link");
    assert!(host.append_child(document, active));
    assert!(host.append_child(document, active_link));

    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    let detached_link = host.create_element("link");
    assert!(host.append_child(detached_document, detached));
    assert!(host.append_child(detached_document, detached_link));

    for link in [active_link, detached_link] {
        assert!(host.set_attribute(link, "rel", "stylesheet"));
        assert!(host.set_attribute(link, "href", "shared.css"));
    }

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/shared.css").unwrap();
    let linked_source = StyloStylesheetSource::new(
        "main, section { color: rgb(1, 2, 3); }".into(),
        linked_url.clone(),
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        linked_source,
        &[active_link, detached_link],
    );

    let inputs = StyloComputedStyleInputs::default();
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
        ensure_adapter_element_data(&engine, &host, handle);
    }
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1)
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(detached_document),
        (1, 1)
    );
    assert!(engine.dom_adapter.has_element_data(active));
    assert!(engine.dom_adapter.has_element_data(detached));
    let input_cache_key = StyloDocumentComputedStyleInputCacheKey::new(
        None,
        &[],
        &document_url,
        StyleViewport::default(),
        StyloStyleEnvironment::default(),
        &document_url,
    );
    let prepared_inputs = || {
        std::rc::Rc::new(StyloPreparedComputedStyleInputs::new(
            &document_url,
            std::rc::Rc::new(StyloComputedStyleInputs::default()),
            StyleViewport::default(),
        ))
    };
    let document_inputs = prepared_inputs();
    let detached_document_inputs = prepared_inputs();
    engine.cache_document_prepared_style_inputs(
        document,
        input_cache_key.clone(),
        std::rc::Rc::clone(&document_inputs),
    );
    engine.cache_document_prepared_style_inputs(
        detached_document,
        input_cache_key.clone(),
        std::rc::Rc::clone(&detached_document_inputs),
    );
    assert!(
        engine
            .cached_document_prepared_style_inputs(document, &input_cache_key)
            .is_some()
    );
    assert!(
        engine
            .cached_document_prepared_style_inputs(detached_document, &input_cache_key)
            .is_some()
    );
    let document_source_set_generation =
        engine.source_set_generation_for_document_for_test(document);
    let detached_source_set_generation =
        engine.source_set_generation_for_document_for_test(detached_document);
    let detached_generation =
        engine.computed_cache_generation_for_document_for_test(detached_document);

    engine.clear_for_document_replacement(document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
    assert!(
        engine
            .cached_document_prepared_style_inputs(document, &input_cache_key)
            .is_none(),
        "document replacement must clear prepared inputs for the replaced document"
    );
    let retained_detached_inputs = engine
        .cached_document_prepared_style_inputs(detached_document, &input_cache_key)
        .expect("document replacement must preserve other document input caches");
    assert!(std::rc::Rc::ptr_eq(
        &retained_detached_inputs,
        &detached_document_inputs
    ));
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (0, 0)
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(detached_document),
        (1, 1)
    );
    assert!(!engine.dom_adapter.has_element_data(active));
    assert!(engine.dom_adapter.has_element_data(detached));
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(detached_document),
        detached_generation
    );
    assert!(
        engine.source_set_generation_for_document_for_test(document)
            > document_source_set_generation
    );
    assert_eq!(
        engine.source_set_generation_for_document_for_test(detached_document),
        detached_source_set_generation
    );
    assert_eq!(
        engine.stylesheet_text_for_url_for_document_for_test(document, &linked_url),
        None
    );
    assert_eq!(
        engine.stylesheet_text_for_url_for_document_for_test(detached_document, &linked_url),
        Some("main, section { color: rgb(1, 2, 3); }".into())
    );
}

#[test]
fn detached_retained_rebuild_preserves_active_document_adapter_element_data() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = StyloComputedStyleInputs::default();
    let active_key = StyleSystemCacheKey::new(&document_url, &inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, active_key, &inputs);
    ensure_adapter_element_data(&engine, &host, active);
    assert!(engine.dom_adapter.has_element_data(active));

    let detached_source = StyloStylesheetSource::new(
        "section { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    let mut first_detached_inputs = StyloComputedStyleInputs::default();
    first_detached_inputs
        .document_stylesheet_sources
        .push(detached_source);
    let first_detached_key = StyleSystemCacheKey::new(&document_url, &first_detached_inputs, None);
    engine.ensure_retained_style_system_for_document(
        &host,
        detached_document,
        first_detached_key,
        &first_detached_inputs,
    );

    let next_detached_source = StyloStylesheetSource::new(
        "section { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    );
    let mut next_detached_inputs = StyloComputedStyleInputs::default();
    next_detached_inputs
        .document_stylesheet_sources
        .push(next_detached_source);
    let next_detached_key = StyleSystemCacheKey::new(&document_url, &next_detached_inputs, None);
    engine.ensure_retained_style_system_for_document(
        &host,
        detached_document,
        next_detached_key,
        &next_detached_inputs,
    );

    assert!(
        engine.dom_adapter.has_element_data(active),
        "rebuilding detached document retained style system must not clear active document adapter data"
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
