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
    assert_eq!(
        engine.pending_style_invalidation_work_kind_names_for_document_for_test(document),
        vec!["mutation"]
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
fn pending_mutation_invalidations_keep_separate_work_items_until_drain() {
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
        2
    );
    assert_eq!(
        engine.pending_style_invalidation_work_kind_names_for_document_for_test(document),
        vec!["mutation", "mutation"]
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
    assert_eq!(
        engine.pending_style_invalidation_work_kind_names_for_document_for_test(document),
        vec!["mutation"]
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
#[test]
fn source_metadata_requires_registered_style_source_metadata() {
    let mut host = test_host();
    let style = host.create_element("style");
    let text = host.create_text_node("body:has(.marker) .target { color: green; }");
    assert!(host.append_child(style, text));
    assert!(host.append_child(host.document_handle(), style));

    let engine = MoliStyleEngine::new();
    let source_scope = StyleSourceScope::for_document(host.document_handle());
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    assert!(
        !engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );
}
#[test]
fn source_metadata_uses_registered_style_source_metadata() {
    let mut host = test_host();
    let style = host.create_element("style");
    let text = host.create_text_node("body:has(.marker) .target { color: green; }");
    assert!(host.append_child(style, text));
    assert!(host.append_child(host.document_handle(), style));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "body:has(.marker) .target { color: green; }".into(),
    );
    let source_scope = StyleSourceScope::for_document(host.document_handle());
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    assert!(
        engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );
}
#[test]
fn source_metadata_skips_disconnected_active_document_style_sources() {
    let mut host = test_host();
    let style = host.create_element("style");
    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "body:has(.marker) .target { color: green; }".into(),
    );
    let source_scope = StyleSourceScope::for_document(host.document_handle());
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    assert!(
        !engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );
}
#[test]
fn has_selector_detection_is_ascii_case_insensitive() {
    let mut host = test_host();
    let style = host.create_element("style");
    let text = host.create_text_node("body:HAS(.marker) .target { color: green; }");
    assert!(host.append_child(style, text));
    assert!(host.append_child(host.document_handle(), style));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "body:HAS(.marker) .target { color: green; }".into(),
    );
    let source_scope = StyleSourceScope::for_document(host.document_handle());
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    assert!(
        engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );
}
#[test]
fn has_selector_detection_ignores_dom_style_comments_and_strings() {
    let mut host = test_host();
    let style = host.create_element("style");
    let text = host.create_text_node(
        r#"
              /* :has(#marker) should not force full invalidation. */
              #outside { color: rgb(1, 2, 3); }
              #unused::before { content: ":has(#marker)"; }
            "#,
    );
    assert!(host.append_child(style, text));
    assert!(host.append_child(host.document_handle(), style));

    let engine = MoliStyleEngine::new();
    let source_scope = StyleSourceScope::for_document(host.document_handle());
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    assert!(
        !engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );
}
#[test]
fn source_metadata_uses_explicitly_installed_linked_stylesheet_owners() {
    let mut host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let has_source = StyloStylesheetSource::new(
        "body:has(.marker) .target { color: green; }".into(),
        url::Url::parse("https://example.test/has.css").unwrap(),
    );
    let detached_document = host.create_detached_html_document();
    engine.set_document_adopted_style_sheet_sources(detached_document, vec![has_source]);

    let linked_url = url::Url::parse("https://example.test/app.css").unwrap();
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &linked_url,
        StyloStylesheetSource::new(
            "body:has(.linked) .target { color: red; }".into(),
            linked_url.clone(),
        ),
    );

    let source_scope = StyleSourceScope::for_document(host.document_handle());
    assert!(
        !engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );

    let link = host.create_element("link");
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "app.css"));
    assert!(host.set_attribute(link, "media", "print"));
    assert!(host.append_child(host.document_handle(), link));

    assert!(
        !engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );
    assert!(engine.install_recorded_linked_stylesheet_source_for_test(&host, link, &linked_url,));
    assert!(
        !engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );
    assert!(host.set_attribute(link, "media", "screen"));
    assert!(
        engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );
}
#[test]
fn source_metadata_respects_style_element_media() {
    let mut host = test_host();
    let style = host.create_element("style");
    let text = host.create_text_node("body:has(.marker) .target { color: green; }");
    assert!(host.set_attribute(style, "media", "print"));
    assert!(host.append_child(style, text));
    assert!(host.append_child(host.document_handle(), style));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "body:has(.marker) .target { color: green; }".into(),
    );
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let source_scope = StyleSourceScope::for_document(host.document_handle());

    assert!(
        !engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );
    assert!(host.set_attribute(style, "media", "screen"));
    assert!(
        engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        )
    );
}

#[test]
fn source_metadata_uses_the_stylesheet_runtime_media_list() {
    use style::{context::QuirksMode, stylesheets::AllowImportRules};

    let mut host = test_host();
    let style = host.create_element("style");
    assert!(host.set_attribute(style, "media", "print"));
    assert!(host.append_child(host.document_handle(), style));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "body:has(.marker) .target { color: green; }".into(),
    );
    let registry = crate::live_stylesheet::LiveStylesheetRegistry::default();
    let stylesheet = registry.create(
        "body:has(.marker) .target { color: green; }",
        url::Url::parse("https://example.test/runtime-media.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::No,
        engine.author_shared_lock(),
    );
    stylesheet.set_media_text("screen");
    assert!(engine.install_owner_live_stylesheet_with_host(&host, style, stylesheet.clone()));

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let source_scope = StyleSourceScope::for_document(host.document_handle());
    assert_eq!(host.get_attribute(style, "media").as_deref(), Some("print"));
    assert!(
        engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        ),
        "CSSOM runtime media must be authoritative even when the owner attribute differs",
    );

    stylesheet.set_media_text("print");
    assert!(engine.refresh_owner_live_stylesheet_with_host(&host, style, stylesheet.id(),));
    assert!(
        !engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &source_scope,
            &media,
        ),
        "an inactive runtime MediaList must not participate in dependency invalidation",
    );
}

#[test]
fn stylo_source_dependency_summary_reads_invalidation_metadata() {
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        ".ancestor:has(.marker) .target { color: green; }
         .marker + .target { color: red; }
         .target:focus-within { color: blue; }
         #target:target { color: purple; }
         .item:empty { color: orange; }",
        &base_url,
    );
    let summary = &metadata.dependency_summary;

    assert!(summary.has_relative_selector_dependency());
    assert!(summary.has_sibling_dependency());
    assert!(summary.has_focus_dependency());
    assert!(summary.has_focus_within_dependency());
    assert!(summary.has_target_dependency());
    assert!(summary.has_child_list_structural_dependency());
}
#[test]
fn stylo_source_dependency_summary_includes_inactive_media_rule_metadata() {
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        "@media print {
            .print:has(.marker) .target { color: green; }
        }
        @media (max-width: 1px) {
            .narrow + .target { color: red; }
        }
        @media (width: 777px) {
            .exact + .target { color: red; }
        }
        @media (prefers-color-scheme: dark) {
            .dark:focus-within { color: blue; }
            #target:target { color: purple; }
        }",
        &base_url,
    );
    let summary = &metadata.dependency_summary;

    assert!(summary.has_relative_selector_dependency());
    assert!(summary.has_sibling_dependency());
    assert!(summary.has_focus_dependency());
    assert!(summary.has_focus_within_dependency());
    assert!(summary.has_target_dependency());

    assert!(
        metadata
            .dependency_summary
            .query_class(&Atom::from("narrow"))
            .has_sibling_dependency()
    );
    assert!(
        metadata
            .dependency_summary
            .query_class(&Atom::from("exact"))
            .has_sibling_dependency()
    );
}

#[test]
fn live_stylesheet_dependency_summary_is_lazy_and_shared_per_revision() {
    use style::{context::QuirksMode, shared_lock::SharedRwLock, stylesheets::AllowImportRules};

    crate::live_stylesheet::reset_live_stylesheet_dependency_summary_projection_count_for_test();
    let registry = crate::live_stylesheet::LiveStylesheetRegistry::default();
    let stylesheet = registry.create(
        ".item-0 + .target { color: red; }",
        url::Url::parse("https://example.test/live.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::No,
        SharedRwLock::new(),
    );

    let initial_source = StyloStylesheetSource::from_live_stylesheet(&stylesheet);
    let same_revision_source = StyloStylesheetSource::from_live_stylesheet(&stylesheet);
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_dependency_summary_projection_count_for_test(),
        0
    );

    let initial_summary = initial_source.source_dependency_summary();
    let same_revision_summary = same_revision_source.source_dependency_summary();
    assert!(std::sync::Arc::ptr_eq(
        &initial_summary,
        &same_revision_summary
    ));
    assert!(
        initial_summary
            .query_class(&Atom::from("item-0"))
            .has_sibling_dependency()
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_dependency_summary_projection_count_for_test(),
        1
    );

    let mut latest_source = same_revision_source;
    for index in 1..=1_000 {
        stylesheet
            .replace_rule(&format!(".item-{index} + .target {{ color: red; }}"), 0)
            .unwrap();
        latest_source = StyloStylesheetSource::from_live_stylesheet(&stylesheet);
    }
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_dependency_summary_projection_count_for_test(),
        1
    );

    let latest_summary = latest_source.source_dependency_summary();
    assert!(
        latest_summary
            .query_class(&Atom::from("item-1000"))
            .has_sibling_dependency()
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_dependency_summary_projection_count_for_test(),
        2
    );
}

#[test]
fn stylo_dependency_summary_exposes_keyed_dependency_kinds() {
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        ".marker + .target { color: red; }
         #anchor ~ .target { color: blue; }
         [data-state] > .target { color: green; }
         .focusable:focus + .target { color: orange; }
         .container:focus-within .target { color: teal; }
         #current:target + .target { color: purple; }",
        &base_url,
    );
    let summary = &metadata.dependency_summary;

    assert!(
        summary
            .query_class(&Atom::from("marker"))
            .has_sibling_dependency()
    );
    assert!(
        summary
            .query_id(&Atom::from("anchor"))
            .has_sibling_dependency()
    );
    let attribute = summary.query_attribute(&LocalName::from("data-state"));
    assert!(attribute.has_any_dependency());
    assert!(!attribute.requires_fallback());
    assert!(attribute.has_descendants_dependency());
    assert!(summary.query_focus().has_sibling_dependency());
    let focus_within = summary.query_focus_within();
    assert!(focus_within.has_any_dependency());
    assert!(!focus_within.requires_fallback());
    assert!(!focus_within.has_sibling_dependency());
    assert!(summary.query_target().has_sibling_dependency());
}
#[test]
fn stylo_dependency_summary_exposes_shadow_dependency_kinds() {
    let base_url = url::Url::parse("https://example.test/app.css").unwrap();
    let metadata = stylo_source_metadata_for_css_text(
        ".slot::slotted(.item) { color: red; }
         .host::part(label) { color: blue; }
         [data-state]::part(label) { color: green; }",
        &base_url,
    );
    let summary = &metadata.dependency_summary;

    let slot_class = summary.query_class(&Atom::from("slot"));
    assert!(slot_class.has_slotted_elements_dependency());
    let item_class = summary.query_class(&Atom::from("item"));
    assert!(item_class.has_any_dependency());

    let host_class = summary.query_class(&Atom::from("host"));
    assert!(host_class.has_parts_dependency());

    let data_state = summary.query_attribute(&LocalName::from("data-state"));
    assert!(data_state.has_parts_dependency());
}
#[test]
fn source_metadata_ignores_disconnected_active_document_subtree_mutations() {
    let mut host = test_host();
    let style = host.create_element("style");
    let text = host.create_text_node("body:has(.marker) .target { color: green; }");
    let connected_parent = host.create_element("section");
    let disconnected_parent = host.create_element("section");
    let disconnected_child = host.create_element("span");

    assert!(host.append_child(style, text));
    assert!(host.append_child(host.document_handle(), style));
    assert!(host.append_child(host.document_handle(), connected_parent));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "body:has(.marker) .target { color: green; }".into(),
    );
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    let disconnected_effect = [StyleMutationEffect::ChildList {
        parent: disconnected_parent,
        added_nodes: vec![disconnected_child],
        removed_nodes: vec![],
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    let disconnected_scope = style_source_scope_for_mutation_effects(&host, &disconnected_effect);
    assert!(
        !engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &disconnected_scope,
            &media
        )
    );

    let connected_effect = [StyleMutationEffect::ChildList {
        parent: connected_parent,
        added_nodes: vec![],
        removed_nodes: vec![],
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    let connected_scope = style_source_scope_for_mutation_effects(&host, &connected_effect);
    assert!(
        engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &connected_scope,
            &media
        )
    );
}
#[test]
fn source_metadata_matches_detached_document_styles_without_matching_active_disconnected_subtrees()
{
    let mut host = test_host();
    let active_disconnected_parent = host.create_element("section");
    let active_disconnected_child = host.create_element("span");
    let detached_document = host.create_detached_html_document();
    let detached_style = host.create_element("style");
    let detached_text = host.create_text_node("body:has(.marker) .target { color: green; }");
    let detached_parent = host.create_element("section");
    let detached_child = host.create_element("span");

    assert!(host.append_child(active_disconnected_parent, active_disconnected_child));
    assert!(host.append_child(detached_style, detached_text));
    assert!(host.append_child(detached_document, detached_style));
    assert!(host.append_child(detached_document, detached_parent));
    assert!(host.append_child(detached_parent, detached_child));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        detached_style,
        "body:has(.marker) .target { color: green; }".into(),
    );
    let mut adopted_engine = MoliStyleEngine::new();
    adopted_engine.set_document_adopted_style_sheet_sources(
        detached_document,
        vec![StyloStylesheetSource::new(
            "body:has(.marker) .target { color: green; }".into(),
            url::Url::parse("https://detached.example.test/document.css").unwrap(),
        )],
    );
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    let active_disconnected_effect = [StyleMutationEffect::ChildList {
        parent: active_disconnected_parent,
        added_nodes: vec![active_disconnected_child],
        removed_nodes: vec![],
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    let active_disconnected_scope =
        style_source_scope_for_mutation_effects(&host, &active_disconnected_effect);
    let active_disconnected_fallback_roots =
        source_scope_fallback_roots_for_test(&host, &active_disconnected_scope);
    assert!(!active_disconnected_fallback_roots.contains(&host.document_handle()));
    assert!(
        !engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &active_disconnected_scope,
            &media
        )
    );
    assert!(
        !adopted_engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &active_disconnected_scope,
            &media
        )
    );

    let detached_effect = [StyleMutationEffect::ChildList {
        parent: detached_parent,
        added_nodes: vec![detached_child],
        removed_nodes: vec![],
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    let detached_scope = style_source_scope_for_mutation_effects(&host, &detached_effect);
    let detached_fallback_roots = source_scope_fallback_roots_for_test(&host, &detached_scope);
    assert!(detached_fallback_roots.contains(&detached_document));
    assert!(
        engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            detached_document,
            &detached_scope,
            &media
        )
    );
    assert!(
        adopted_engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            detached_document,
            &detached_scope,
            &media
        )
    );
}
#[test]
fn source_metadata_matches_detached_shadow_styles_without_matching_active_disconnected_shadow_tree()
{
    let mut host = test_host();
    let active_host = host.create_element("section");
    let active_shadow_root = host
        .attach_shadow_root(active_host, "open")
        .expect("section should host a shadow root");
    let active_shadow_style = host.create_element("style");
    let active_shadow_child = host.create_element("span");
    let detached_document = host.create_detached_html_document();
    let detached_host = host.create_element("section");
    let detached_shadow_root = host
        .attach_shadow_root(detached_host, "open")
        .expect("section should host a shadow root");
    let detached_shadow_style = host.create_element("style");
    let detached_shadow_child = host.create_element("span");

    assert!(host.append_child(active_shadow_root, active_shadow_style));
    assert!(host.append_child(active_shadow_root, active_shadow_child));
    assert!(host.append_child(detached_document, detached_host));
    assert!(host.append_child(detached_shadow_root, detached_shadow_style));
    assert!(host.append_child(detached_shadow_root, detached_shadow_child));

    let mut owner_engine = MoliStyleEngine::new();
    owner_engine.set_owner_style_sheet_text_with_host(
        &host,
        detached_shadow_style,
        ":host:has(.marker) span { color: green; }".into(),
    );
    owner_engine.set_owner_style_sheet_text_with_host(
        &host,
        active_shadow_style,
        ":host:has(.marker) span { color: red; }".into(),
    );
    let mut adopted_engine = MoliStyleEngine::new();
    adopted_engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        detached_shadow_root,
        vec![StyloStylesheetSource::new(
            ":host:has(.marker) span { color: green; }".into(),
            url::Url::parse("https://detached.example.test/shadow.css").unwrap(),
        )],
    );
    adopted_engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        active_shadow_root,
        vec![StyloStylesheetSource::new(
            ":host:has(.marker) span { color: red; }".into(),
            url::Url::parse("https://example.test/shadow.css").unwrap(),
        )],
    );
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    let active_disconnected_effect = [StyleMutationEffect::ChildList {
        parent: active_shadow_root,
        added_nodes: vec![active_shadow_child],
        removed_nodes: vec![],
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    let active_disconnected_scope =
        style_source_scope_for_mutation_effects(&host, &active_disconnected_effect);
    assert!(
        shadow_root_source_scope_fallback_roots_for_test(
            &host,
            active_shadow_root,
            &active_disconnected_scope,
        )
        .is_empty()
    );
    assert!(
        !owner_engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &active_disconnected_scope,
            &media
        )
    );
    assert!(
        !adopted_engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            host.document_handle(),
            &active_disconnected_scope,
            &media
        )
    );

    let detached_effect = [StyleMutationEffect::ChildList {
        parent: detached_shadow_root,
        added_nodes: vec![detached_shadow_child],
        removed_nodes: vec![],
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    let detached_scope = style_source_scope_for_mutation_effects(&host, &detached_effect);
    let detached_fallback_roots = source_scope_fallback_roots_for_test(&host, &detached_scope);
    assert!(detached_fallback_roots.contains(&detached_document));
    assert_eq!(
        shadow_root_source_scope_fallback_roots_for_test(
            &host,
            detached_shadow_root,
            &detached_scope
        ),
        vec![detached_shadow_root, detached_host]
    );
    assert!(
        owner_engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            detached_document,
            &detached_scope,
            &media
        )
    );
    assert!(
        adopted_engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            detached_document,
            &detached_scope,
            &media
        )
    );
}
